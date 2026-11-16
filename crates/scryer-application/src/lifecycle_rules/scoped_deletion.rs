//! Resumable policy deletion using the catalog's per-file deletion owner.
use super::action_execution::{
    ActionResult, LifecycleExecutionContext, SafetyDecision, execution_reason,
};
use crate::library::user_delete::PolicyMediaFileDeletePlan;
use crate::{AppError, AppResult, AppUseCase, PolicyDeleteAuthorization};
use scryer_domain::{
    LifecycleActionRun, LifecycleCandidate, LifecycleRuleSubjectKind as Scope, Title, User,
};
use scryer_rules::lifecycle::{LifecycleFileDoc, LifecycleInput, Observation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ScopeFactSnapshot {
    monitored: Observation<bool>,
    has_file: Observation<bool>,
    file_count: Observation<i64>,
    total_file_size_bytes: Observation<i64>,
    files: Observation<Vec<LifecycleFileDoc>>,
    first_imported_at: Observation<String>,
    episode_file_count: Observation<i64>,
    monitored_episode_count: Observation<i64>,
}
impl ScopeFactSnapshot {
    fn capture(input: &LifecycleInput) -> Self {
        let facts = input.subject_facts();
        Self {
            monitored: facts.monitored.clone(),
            has_file: facts.has_file.clone(),
            file_count: facts.file_count.clone(),
            total_file_size_bytes: facts.total_file_size_bytes.clone(),
            files: facts.files.clone(),
            first_imported_at: facts.first_imported_at.clone(),
            episode_file_count: facts.episode_file_count.clone(),
            monitored_episode_count: facts.monitored_episode_count.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileCheckpoint {
    file_id: String,
    file_path: String,
    catalog_proof: String,
    /// Exact catalog size captured with this target. Older journals lack the
    /// field and therefore never project a size baseline forward.
    #[serde(default)]
    file_size_bytes: Option<i64>,
    plan: Option<PolicyMediaFileDeletePlan>,
    /// Episode coverage that this exact manifest contributed to rolling
    /// retention before it was unlinked. Legacy journals omit it safely.
    #[serde(default)]
    retention_episode_ids: Vec<String>,
    completed: bool,
    error: Option<String>,
    /// Stable reason code for `error` when the failure carried one, so the
    /// journal can be read without matching its sentence. Older journals
    /// lack the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_reason: Option<String>,
}

impl FileCheckpoint {
    /// Record why this target was not deleted: a reason code alone, or a
    /// failure's sentence beside the code it carried.
    fn record_reason(&mut self, reason: &str) {
        self.error = Some(reason.to_string());
        self.error_reason = Some(reason.to_string());
    }

    fn record_failure(&mut self, error: &AppError) {
        self.error = Some(error.to_string());
        self.error_reason = error.validation_reason().map(str::to_string);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ScopedDeletionCheckpoint {
    scoped_deletion: u32,
    /// A policy-deletion manifest belongs to one candidate generation. A later
    /// match after a cancelled partial deletion must earn its own grace period
    /// and preview, not resume this manifest.
    #[serde(default)]
    match_generation: i64,
    #[serde(default)]
    action_kind: String,
    /// A v2 sequence carries its stable step identity through the existing
    /// file journal. Legacy journals intentionally omit this field.
    #[serde(default)]
    sequence_step_id: Option<String>,
    subject_kind: String,
    subject_id: String,
    subject_label: String,
    grace_deadline: String,
    total_size_bytes: i64,
    pub baseline: ScopeFactSnapshot,
    mutation_started: bool,
    files: Vec<FileCheckpoint>,
    /// The most recent hold that stopped this journal's run before it
    /// finished. The reason itself is the run's `hold_reason`; this keeps the
    /// hold's detail without replacing the journal. Progress clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hold: Option<serde_json::Value>,
}

impl ScopedDeletionCheckpoint {
    fn from_detail(detail: &str) -> Option<Self> {
        serde_json::from_str::<Self>(detail)
            .ok()
            .filter(|checkpoint| checkpoint.scoped_deletion == 1)
    }
}

/// Record a hold or cancellation's detail on `run` without losing a scoped
/// deletion journal. The run's journal is the only record of which targets
/// were already removed, and the next pass resumes from it: a hold that
/// returns the journal itself is written as is, a hold with its own detail is
/// kept inside the journal, and a run without a journal takes the detail.
pub(super) fn record_hold_detail(run: &mut LifecycleActionRun, hold_detail: &serde_json::Value) {
    let hold_is_journal = serde_json::from_value::<ScopedDeletionCheckpoint>(hold_detail.clone())
        .is_ok_and(|checkpoint| checkpoint.scoped_deletion == 1);
    if !hold_is_journal
        && let Some(mut journal) = ScopedDeletionCheckpoint::from_detail(&run.detail)
    {
        journal.hold = Some(hold_detail.clone());
        if let Ok(detail) = serde_json::to_string(&journal) {
            run.detail = detail;
            return;
        }
    }
    run.detail = hold_detail.to_string();
}

impl ScopedDeletionCheckpoint {
    pub(super) fn completed_retention_episode_ids(&self) -> BTreeSet<String> {
        if self.mutation_started {
            self.files
                .iter()
                .filter(|file| file.completed)
                .flat_map(|file| file.retention_episode_ids.iter().cloned())
                .collect()
        } else {
            BTreeSet::new()
        }
    }

    /// Restore only file-count and size deltas which can be attributed to the
    /// exact completed journal targets. The live values must equal the
    /// baseline minus those targets first; imports, independent removals, or
    /// any unproven journal remain live instead of being masked.
    pub(super) fn restore_sequence_owned_file_deltas(&self, input: &mut LifecycleInput) {
        if !self.mutation_started {
            return;
        }
        let completed_count = self.files.iter().filter(|file| file.completed).count() as i64;
        let Some(completed_size) = self
            .files
            .iter()
            .filter(|file| file.completed)
            .map(|file| file.file_size_bytes)
            .collect::<Option<Vec<_>>>()
            .map(|sizes| sizes.into_iter().sum::<i64>())
        else {
            return;
        };
        let restore_has_file = if let (
            Observation::Known {
                value: baseline_count,
                ..
            },
            Observation::Known {
                value: current_count,
                ..
            },
            Observation::Known {
                value: baseline_has_file,
                ..
            },
        ) = (
            &self.baseline.file_count,
            &input.subject_facts().file_count,
            &self.baseline.has_file,
        ) && *current_count == *baseline_count - completed_count
        {
            Some(*baseline_has_file)
        } else {
            None
        };
        let count_restored = restore_known_delta(
            &self.baseline.file_count,
            &mut input.subject_facts_mut().file_count,
            completed_count,
        );
        let size_restored = restore_known_delta(
            &self.baseline.total_file_size_bytes,
            &mut input.subject_facts_mut().total_file_size_bytes,
            completed_size,
        );
        if let Some(baseline_has_file) = restore_has_file {
            input.subject_facts_mut().has_file = Observation::known(baseline_has_file);
        }
        // A season's or episode's files are also its title's, and the rule
        // reads the title's facts beside the subject's. Once the subject shows
        // nothing but this delete's own removals, the same removals are added
        // back to the title's live figures; any other change to the title's
        // files still reads as it is.
        let narrow_subject = !std::ptr::eq(input.subject_facts(), &input.facts);
        if narrow_subject && count_restored && size_restored && completed_count > 0 {
            add_known_delta(&mut input.facts.file_count, completed_count);
            add_known_delta(&mut input.facts.total_file_size_bytes, completed_size);
            if matches!(input.facts.has_file, Observation::Known { .. }) {
                input.facts.has_file = Observation::known(true);
            }
        }
    }
}

/// Put `delta` back on `current` when it reads exactly `baseline - delta`.
/// Returns whether it did.
fn restore_known_delta(
    baseline: &Observation<i64>,
    current: &mut Observation<i64>,
    delta: i64,
) -> bool {
    if let (
        Observation::Known {
            value: baseline, ..
        },
        Observation::Known {
            value: current_value,
            ..
        },
    ) = (baseline, current)
        && *current_value == *baseline - delta
    {
        *current_value = *baseline;
        return true;
    }
    false
}

fn add_known_delta(current: &mut Observation<i64>, delta: i64) {
    if let Observation::Known { value, .. } = current {
        *value += delta;
    }
}
fn catalog_proof(file: &crate::types::TitleMediaFile) -> String {
    format!(
        "{}:{}:{}:{}:{:?}:{:?}",
        file.title_id,
        file.file_path,
        file.created_at,
        file.size_bytes,
        file.source_signature_scheme,
        file.source_signature_value
    )
}

impl AppUseCase {
    /// The candidate's saved deletion journal for one sequence step, or
    /// `None` when that deletion has not begun. A journal that does not
    /// belong to exactly this candidate, generation, step and subject is an
    /// identity mismatch and never resumes.
    pub(super) async fn lifecycle_deletion_checkpoint_for_step(
        &self,
        candidate: &LifecycleCandidate,
        sequence_step_id: Option<&str>,
    ) -> AppResult<Option<ScopedDeletionCheckpoint>> {
        let Some(run) = self
            .services
            .customization
            .lifecycle_evaluation
            .latest_scoped_deletion_action_run(
                &candidate.id,
                candidate.match_generation,
                &candidate.action_kind,
            )
            .await?
        else {
            return Ok(None);
        };
        let checkpoint: ScopedDeletionCheckpoint = serde_json::from_str(&run.detail)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        if run.rule_set_id != candidate.rule_set_id
            || run.revision_number != candidate.revision_number
            || run.match_generation != candidate.match_generation
            || run.action_kind != candidate.action_kind
            || checkpoint.scoped_deletion != 1
            || checkpoint.match_generation != candidate.match_generation
            || checkpoint.action_kind != candidate.action_kind
            || checkpoint.sequence_step_id.as_deref() != sequence_step_id
            || checkpoint.subject_kind != candidate.subject_kind
            || checkpoint.subject_id != candidate.subject_id
        {
            return Err(AppError::Repository(
                "lifecycle deletion checkpoint identity mismatch".into(),
            ));
        }
        Ok(Some(checkpoint))
    }

    /// Reconcile only a catalog row proven absent by its immutable file id.
    /// Subject association lookups are intentionally insufficient: a still
    /// present row could have become linked or reassociated and must hold.
    async fn reconcile_absent_scoped_deletion_catalog_rows(
        &self,
        checkpoint: &mut ScopedDeletionCheckpoint,
    ) -> AppResult<bool> {
        let mut changed = false;
        for saved in checkpoint.files.iter_mut().filter(|file| !file.completed) {
            if self
                .services
                .library
                .media_files
                .get_media_file_by_id(&saved.file_id)
                .await?
                .is_none()
            {
                saved.completed = true;
                saved.error = None;
                saved.error_reason = None;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Unmonitor every persisted monitoring layer before a title-preserving
    /// deletion. The explicit child writes make future discovery inherit the
    /// title's disabled state and make the postcondition independently
    /// observable before the first filesystem mutation.
    async fn unmonitor_lifecycle_title(&self, title: &Title) -> AppResult<()> {
        self.unmonitor_lifecycle_title_with_descendants(title, true)
            .await
    }

    /// Unmonitor a title, optionally including the monitoring records of its
    /// persisted descendants. Sequence actions expose the option explicitly;
    /// the legacy deletion workflow always retains its full-title behavior.
    pub(super) async fn unmonitor_lifecycle_title_with_descendants(
        &self,
        title: &Title,
        include_descendants: bool,
    ) -> AppResult<()> {
        let actor = User::system_execution_actor();
        let (collections, episodes) = if include_descendants {
            let collections = self
                .services
                .catalog
                .shows
                .list_collections_for_title(&title.id)
                .await?;
            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await?;
            if collections
                .iter()
                .any(|collection| collection.title_id != title.id)
                || episodes.iter().any(|episode| episode.title_id != title.id)
            {
                return Err(AppError::Repository(
                    "lifecycle title child ownership mismatch".into(),
                ));
            }
            (collections, episodes)
        } else {
            (Vec::new(), Vec::new())
        };

        if title.monitored {
            self.set_title_monitored(&actor, &title.id, false).await?;
        }
        for collection in collections.iter().filter(|collection| collection.monitored) {
            self.set_collection_monitored(&actor, &collection.id, false)
                .await?;
        }
        for episode in episodes.iter().filter(|episode| episode.monitored) {
            self.set_episode_monitored(&actor, &episode.id, false)
                .await?;
        }

        let fresh_title = self
            .services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {}", title.id),
                )
            })?;
        if fresh_title.monitored {
            return Err(AppError::Repository(
                "lifecycle title unmonitoring was not confirmed; files preserved".into(),
            ));
        }
        if include_descendants {
            let fresh_collections = self
                .services
                .catalog
                .shows
                .list_collections_for_title(&title.id)
                .await?;
            let fresh_episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await?;
            if fresh_collections
                .iter()
                .any(|collection| collection.title_id != title.id || collection.monitored)
                || fresh_episodes
                    .iter()
                    .any(|episode| episode.title_id != title.id || episode.monitored)
            {
                return Err(AppError::Repository(
                    "lifecycle title unmonitoring was not confirmed; files preserved".into(),
                ));
            }
        }
        Ok(())
    }

    /// Build the title-wide manifest from actual associations. Direct title
    /// files and episode files are included only when their catalog ownership
    /// is unambiguous; series-movie links are deliberately left untouched.
    /// Unambiguously title-owned files for storage summaries and policy
    /// manifests. Linked movies and files with any foreign episode association
    /// are excluded rather than treated as title coverage.
    pub(crate) async fn lifecycle_title_deletion_files(
        &self,
        title: &Title,
    ) -> AppResult<Vec<crate::types::TitleMediaFile>> {
        let mut files: HashMap<String, crate::types::TitleMediaFile> = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?
            .into_iter()
            .filter(|file| file.title_id == title.id && file.series_movie_link_ids.is_empty())
            .map(|file| (file.id.clone(), file))
            .collect();
        let mut disqualified_file_ids = HashSet::new();
        if title.facet == scryer_domain::MediaFacet::Movie {
            let mut owned: Vec<_> = files.into_values().collect();
            owned.sort_by(|left, right| left.id.cmp(&right.id));
            return Ok(owned);
        }

        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        let episode_ids: HashSet<&str> =
            episodes.iter().map(|episode| episode.id.as_str()).collect();
        if episodes.iter().any(|episode| episode.title_id != title.id) {
            return Err(AppError::Repository(
                "lifecycle title episode ownership mismatch".into(),
            ));
        }
        let mut proven_file_ids = HashSet::new();
        for batch in episodes.chunks(100) {
            let ids: Vec<String> = batch.iter().map(|episode| episode.id.clone()).collect();
            for scoped in self
                .services
                .library
                .media_files
                .list_live_media_files_for_episode_ids(&title.id, &ids)
                .await?
            {
                let file = scoped.media_file;
                // A shared episode file may only be deleted by an action whose
                // subject owns every association. Title scope owns all of this
                // title's episodes, but never a linked movie or another title.
                if file.title_id != title.id
                    || !file.series_movie_link_ids.is_empty()
                    || scoped
                        .episode_ids
                        .iter()
                        .any(|episode_id| !episode_ids.contains(episode_id.as_str()))
                {
                    // A direct title-file listing is not proof that every
                    // association belongs to this title. Once an association
                    // fails, quarantine the id so a later valid row cannot
                    // put it back into the manifest.
                    files.remove(&file.id);
                    disqualified_file_ids.insert(file.id);
                    continue;
                }
                if disqualified_file_ids.contains(&file.id) {
                    continue;
                }
                proven_file_ids.insert(file.id.clone());
                files.insert(file.id.clone(), file);
            }
        }
        files.retain(|file_id, _| proven_file_ids.contains(file_id));
        let mut files: Vec<_> = files.into_values().collect();
        files.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(files)
    }

    pub(super) async fn lifecycle_policy_deletion_files(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<(String, Vec<crate::types::TitleMediaFile>)> {
        if candidate.subject_kind == Scope::Title.as_storage_str() {
            return Ok((
                title.name.clone(),
                self.lifecycle_title_deletion_files(title).await?,
            ));
        }
        let subject = self
            .lifecycle_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        if subject.ambiguous_files {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::FILE_COVERAGE_AMBIGUOUS,
                "ambiguous lifecycle file coverage",
            ));
        }
        Ok((subject.label, subject.files))
    }

    pub(super) async fn unmonitor_lifecycle_child(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<()> {
        let subject = self
            .lifecycle_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        let actor = User::system_execution_actor();
        match subject.kind {
            Scope::Episode => {
                self.set_episode_monitored(&actor, &subject.id, false)
                    .await?;
            }
            Scope::Season => {
                self.set_collection_monitored(&actor, &subject.id, false)
                    .await?;
            }
            Scope::Title => {
                return Err(AppError::validation_refused(
                    crate::error_reasons::lifecycle::CHILD_DELETION_SUBJECT_REQUIRED,
                    "child deletion requires an episode or season",
                ));
            }
        }
        // Confirm the persisted state, including every future episode of a season.
        let fresh = self
            .lifecycle_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        if fresh.monitored || fresh.episodes.iter().any(|episode| episode.monitored) {
            return Err(AppError::Repository(
                "lifecycle unmonitoring was not confirmed; files preserved".into(),
            ));
        }
        Ok(())
    }

    /// Read the persisted monitoring state required by a sequence's earlier
    /// `unmonitor` step. This deliberately accepts no baseline: an external
    /// re-monitor between steps is live evidence that invalidates a later file
    /// deletion, not a value a sequence may write over in memory.
    pub(super) async fn lifecycle_unmonitoring_is_confirmed(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
        title_includes_descendants: bool,
    ) -> AppResult<bool> {
        if candidate.title_id != title.id
            || (candidate.subject_kind == Scope::Title.as_storage_str()
                && candidate.subject_id != title.id)
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::CANDIDATE_TITLE_MISMATCH,
                "lifecycle unmonitoring candidate identity does not match its title",
            ));
        }
        if candidate.subject_kind != Scope::Title.as_storage_str() {
            let fresh = self
                .lifecycle_resolve_subject(
                    &candidate.subject_kind,
                    &candidate.subject_id,
                    title,
                    &[],
                )
                .await?;
            return Ok(!fresh.monitored && fresh.episodes.iter().all(|episode| !episode.monitored));
        }

        let Some(fresh_title) = self.services.catalog.titles.get_by_id(&title.id).await? else {
            return Err(AppError::not_found_reason(
                crate::error_reasons::catalog::TITLE_NOT_FOUND,
                format!("title {}", title.id),
            ));
        };
        if fresh_title.monitored || !title_includes_descendants {
            return Ok(!fresh_title.monitored);
        }
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        Ok(collections
            .iter()
            .all(|collection| collection.title_id == title.id && !collection.monitored)
            && episodes
                .iter()
                .all(|episode| episode.title_id == title.id && !episode.monitored))
    }

    async fn save_scoped_deletion_checkpoint(
        &self,
        run: &mut LifecycleActionRun,
        checkpoint: &mut ScopedDeletionCheckpoint,
    ) -> AppResult<()> {
        // Progress supersedes any hold the journal carried from a past pass.
        checkpoint.hold = None;
        run.detail = serde_json::to_string(checkpoint)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        // The run remains Running; this is durable progress, not a success report.
        self.services
            .customization
            .lifecycle_evaluation
            .finish_action_run(run)
            .await
    }

    pub(super) async fn execute_scoped_lifecycle_deletion(
        &self,
        context: &mut LifecycleExecutionContext<'_>,
        sequence_step_id: Option<&str>,
        unmonitor_before_delete: bool,
        sequence_unmonitor_includes_descendants: Option<bool>,
    ) -> AppResult<ActionResult> {
        let candidate = context.candidate;
        let title = context.title;
        let input = context.input;
        let run = &mut *context.run;
        // A rule that reads storage capacity decides on a moving condition:
        // every unlink changes it. Such a deletion is serialized with every
        // other one and re-checks the rule before each file.
        let reads_storage =
            super::storage::rule_reads_storage(&context.detail.revision.rego_source);
        let detail = context.detail;
        let evaluator = &mut *context.evaluator;
        let libraries = context.libraries;
        let tag_conflicts = context.tag_conflicts;
        let storage_roots = context.storage_roots;
        let (subject_label, subject_files) = self
            .lifecycle_policy_deletion_files(candidate, title)
            .await?;
        let existing_checkpoint = self
            .lifecycle_deletion_checkpoint_for_step(candidate, sequence_step_id)
            .await?;
        let mut checkpoint = match existing_checkpoint {
            // The read above refuses a journal that is not exactly this
            // candidate's rule, revision, generation, action, step and
            // subject. One that passes resumes from the files it saved, and
            // the subject's current files must all be among them (below).
            Some(checkpoint) => checkpoint,
            None => {
                let retention_inventory = if title.facet != scryer_domain::MediaFacet::Movie {
                    Some(self.lifecycle_load_title_episode_inventory(title).await?)
                } else {
                    None
                };
                let mut files = Vec::new();
                for file in &subject_files {
                    let retention_episode_ids = retention_inventory
                        .as_ref()
                        .map(|inventory| {
                            inventory
                                .retention_episode_ids_for_file_ids(&HashSet::from([file
                                    .id
                                    .clone()]))
                                .into_iter()
                                .collect()
                        })
                        .unwrap_or_default();
                    let (plan, error) = match self.prepare_policy_media_file_delete(&file.id).await
                    {
                        Ok(plan) => (Some(plan), None),
                        Err(error) => (None, Some(error.to_string())),
                    };
                    files.push(FileCheckpoint {
                        file_id: file.id.clone(),
                        file_path: file.file_path.clone(),
                        catalog_proof: catalog_proof(file),
                        file_size_bytes: Some(file.size_bytes),
                        plan,
                        retention_episode_ids,
                        completed: false,
                        error,
                        error_reason: None,
                    });
                }
                ScopedDeletionCheckpoint {
                    scoped_deletion: 1,
                    match_generation: candidate.match_generation,
                    action_kind: candidate.action_kind.clone(),
                    sequence_step_id: sequence_step_id.map(str::to_string),
                    subject_kind: candidate.subject_kind.clone(),
                    subject_id: candidate.subject_id.clone(),
                    subject_label,
                    grace_deadline: candidate.due_at.to_rfc3339(),
                    total_size_bytes: subject_files.iter().map(|file| file.size_bytes).sum(),
                    baseline: ScopeFactSnapshot::capture(input),
                    mutation_started: false,
                    files,
                    hold: None,
                }
            }
        };
        self.reconcile_absent_scoped_deletion_catalog_rows(&mut checkpoint)
            .await?;
        if subject_files.iter().any(|file| {
            !checkpoint
                .files
                .iter()
                .any(|saved| saved.file_id == file.id && saved.catalog_proof == catalog_proof(file))
        }) {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::FILE_TARGETS_CHANGED,
                "lifecycle file targets changed; another evaluation is required",
            ));
        }
        let mut guards = Vec::new();
        if reads_storage {
            // Capacity can change after each unlink. Until configured roots
            // carry a portable volume identity, serialize all storage-pressure
            // deletions rather than letting two roots on the same filesystem
            // make decisions from the same stale measurement.
            let Some(guard) = self
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire("maintenance-storage-deletion")
                .await
            else {
                self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                    .await?;
                return Ok(ActionResult::Held {
                    reason: execution_reason::STORAGE_DELETION_IN_PROGRESS,
                    detail: serde_json::to_value(checkpoint)
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                });
            };
            guards.push(guard);
        }
        for file in &checkpoint.files {
            let Some(guard) = self
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire(&format!("media-file:{}", file.file_id))
                .await
            else {
                if reads_storage {
                    self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                        .await?;
                    return Ok(ActionResult::Held {
                        reason: execution_reason::STORAGE_DELETION_IN_PROGRESS,
                        detail: serde_json::to_value(checkpoint)
                            .map_err(|error| AppError::Repository(error.to_string()))?,
                    });
                }
                return Err(AppError::Repository(
                    "another deletion owns this media file".into(),
                ));
            };
            guards.push(guard);
        }
        // Persist intent before either the monitoring or filesystem mutation.
        checkpoint.mutation_started = true;
        self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
            .await?;
        if unmonitor_before_delete {
            if candidate.subject_kind == Scope::Title.as_storage_str() {
                self.unmonitor_lifecycle_title(title).await?;
            } else {
                self.unmonitor_lifecycle_child(candidate, title).await?;
            }
        }
        let authorization = PolicyDeleteAuthorization {
            rule_set_id: candidate.rule_set_id.clone(),
            candidate_id: candidate.id.clone(),
            revision_number: candidate.revision_number,
        };
        let actor = User::system_execution_actor();
        let mut failed = false;
        for index in 0..checkpoint.files.len() {
            if checkpoint.files[index].completed {
                continue;
            }
            let saved = checkpoint.files[index].clone();
            if let Some(include_descendants) = sequence_unmonitor_includes_descendants
                && !self
                    .lifecycle_unmonitoring_is_confirmed(candidate, title, include_descendants)
                    .await?
            {
                checkpoint.files[index].record_reason(execution_reason::NO_MATCH_AT_EXECUTION);
                self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                    .await?;
                return Ok(ActionResult::Canceled {
                    reason: execution_reason::NO_MATCH_AT_EXECUTION,
                    detail: serde_json::to_value(checkpoint)
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                });
            }
            // Storage pressure is a moving condition. The first safety check
            // authorized unmonitoring and the manifest; every subsequent file
            // gets a new capacity probe and Rego evaluation before unlinking.
            if reads_storage || sequence_step_id.is_some() {
                match self
                    .lifecycle_execution_safety_checks(
                        detail,
                        context.sequence,
                        evaluator,
                        candidate,
                        libraries,
                        tag_conflicts,
                        storage_roots,
                    )
                    .await
                {
                    SafetyDecision::Proceed(_, _) => {}
                    SafetyDecision::Cancel(reason) => {
                        checkpoint.files[index].record_reason(reason);
                        self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                            .await?;
                        return Ok(ActionResult::Canceled {
                            reason,
                            detail: serde_json::to_value(checkpoint)
                                .map_err(|error| AppError::Repository(error.to_string()))?,
                        });
                    }
                    SafetyDecision::Excluded => {
                        checkpoint.files[index].record_reason(
                            crate::lifecycle_rules::evaluation::candidate_reason::EXCLUDED,
                        );
                        self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                            .await?;
                        return Ok(ActionResult::Canceled {
                            reason: crate::lifecycle_rules::evaluation::candidate_reason::EXCLUDED,
                            detail: serde_json::to_value(checkpoint)
                                .map_err(|error| AppError::Repository(error.to_string()))?,
                        });
                    }
                    SafetyDecision::Hold(reason) => {
                        checkpoint.files[index].record_reason(reason);
                        self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                            .await?;
                        return Ok(ActionResult::Held {
                            reason,
                            detail: serde_json::to_value(checkpoint)
                                .map_err(|error| AppError::Repository(error.to_string()))?,
                        });
                    }
                }
            }
            // Re-read ownership after monitoring and before every file
            // mutation.
            let (_, fresh_files) = self
                .lifecycle_policy_deletion_files(candidate, title)
                .await?;
            let Some(plan) = saved.plan.clone() else {
                // A single target may have failed its pre-mutation preview.
                // Keep its error in the journal but continue independently
                // authorized siblings, as the legacy scoped deletion contract
                // requires.
                failed = true;
                checkpoint.files[index].error = Some(
                    saved
                        .error
                        .unwrap_or_else(|| "file preview unavailable".to_string()),
                );
                self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                    .await?;
                continue;
            };
            let result = async {
                let Some(file) = self
                    .services
                    .library
                    .media_files
                    .get_media_file_by_id(&saved.file_id)
                    .await?
                else {
                    return Ok(());
                };
                if catalog_proof(&file) != saved.catalog_proof
                    || !fresh_files.iter().any(|current| current.id == file.id)
                {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::lifecycle::FILE_OWNERSHIP_CHANGED,
                        "lifecycle file ownership changed",
                    ));
                }
                self.delete_media_file_authorized(
                    &actor,
                    file,
                    crate::catalog_workflow::MediaFileDiskDeletion::DeleteByPolicy {
                        plan,
                        authorization: authorization.clone(),
                    },
                )
                .await
            }
            .await;
            match result {
                Ok(()) => {
                    checkpoint.files[index].completed = true;
                    checkpoint.files[index].error = None;
                    checkpoint.files[index].error_reason = None;
                }
                Err(error) => {
                    failed = true;
                    checkpoint.files[index].record_failure(&error);
                }
            }
            self.save_scoped_deletion_checkpoint(run, &mut checkpoint)
                .await?;
        }
        drop(guards);
        if failed {
            return Err(AppError::Repository(
                "some episode files could not be deleted; see per-file results".into(),
            ));
        }
        Ok(ActionResult::Executed {
            detail: serde_json::to_value(checkpoint)
                .map_err(|error| AppError::Repository(error.to_string()))?,
        })
    }
}

#[cfg(test)]
mod hold_detail_tests {
    use super::{ScopedDeletionCheckpoint, record_hold_detail};
    use scryer_domain::{LifecycleActionRun, LifecycleActionRunStatus};

    /// A minimal journal: no provenance and no hold.
    fn minimal_journal() -> serde_json::Value {
        let absent = serde_json::json!({ "status": "absent" });
        serde_json::json!({
            "scoped_deletion": 1,
            "subject_kind": "title",
            "subject_id": "synthetic-title",
            "subject_label": "Synthetic Title",
            "grace_deadline": "2026-01-01T00:00:00+00:00",
            "total_size_bytes": 10,
            "baseline": {
                "monitored": absent,
                "has_file": absent,
                "file_count": absent,
                "total_file_size_bytes": absent,
                "files": absent,
                "first_imported_at": absent,
                "episode_file_count": absent,
                "monitored_episode_count": absent,
            },
            "mutation_started": true,
            "files": [{
                "file_id": "synthetic-file",
                "file_path": "/synthetic/file.mkv",
                "catalog_proof": "synthetic-proof",
                "plan": null,
                "completed": false,
                "error": null,
            }],
        })
    }

    fn run_with_detail(detail: String) -> LifecycleActionRun {
        let now = chrono::Utc::now();
        LifecycleActionRun {
            id: "synthetic-run".to_string(),
            candidate_id: "synthetic-candidate".to_string(),
            rule_set_id: "synthetic-rules".to_string(),
            revision_number: 1,
            title_id: "synthetic-title".to_string(),
            subject_kind: "title".to_string(),
            subject_id: "synthetic-title".to_string(),
            action_kind: "delete_files".to_string(),
            match_generation: 1,
            idempotency_key: "synthetic-key".to_string(),
            attempt: 1,
            status: LifecycleActionRunStatus::Running,
            hold_reason: None,
            error: None,
            detail,
            started_at: now,
            finished_at: None,
            created_at: now,
        }
    }

    #[test]
    fn a_minimal_journal_without_a_hold_still_reads_and_writes_none() {
        let journal = minimal_journal().to_string();
        let checkpoint =
            ScopedDeletionCheckpoint::from_detail(&journal).expect("a minimal journal reads");
        assert!(checkpoint.hold.is_none());
        let written = serde_json::to_value(&checkpoint).expect("write journal");
        assert!(written.get("hold").is_none(), "{written}");
    }

    #[test]
    fn a_hold_keeps_the_journal_and_records_its_detail_inside_it() {
        let journal = minimal_journal();
        let mut run = run_with_detail(journal.to_string());
        let hold = serde_json::json!({ "reason": "synthetic_hold" });

        record_hold_detail(&mut run, &hold);

        let mut kept: serde_json::Value = serde_json::from_str(&run.detail).expect("parse detail");
        assert_eq!(kept["hold"], hold);
        kept.as_object_mut().expect("journal object").remove("hold");
        let original =
            ScopedDeletionCheckpoint::from_detail(&journal.to_string()).expect("journal reads");
        assert_eq!(
            kept,
            serde_json::to_value(&original).expect("write journal"),
            "the journal itself is unchanged"
        );
    }

    #[test]
    fn a_hold_that_returns_the_journal_writes_that_journal() {
        let mut run = run_with_detail(minimal_journal().to_string());
        let mut progressed = minimal_journal();
        progressed["files"][0]["completed"] = serde_json::json!(true);

        record_hold_detail(&mut run, &progressed);

        let written: serde_json::Value = serde_json::from_str(&run.detail).expect("parse detail");
        assert_eq!(written["files"][0]["completed"], true);
        assert!(written.get("hold").is_none(), "{written}");
    }

    #[test]
    fn a_run_without_a_journal_takes_the_hold_detail() {
        let mut run = run_with_detail(r#"{"preview":"synthetic"}"#.to_string());
        let hold = serde_json::json!({ "reason": "synthetic_unavailable" });

        record_hold_detail(&mut run, &hold);

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&run.detail).expect("parse detail"),
            hold
        );
    }
}

#[cfg(test)]
mod title_restore_tests {
    use super::{FileCheckpoint, ScopeFactSnapshot, ScopedDeletionCheckpoint};
    use scryer_domain::MediaFacet;
    use scryer_rules::lifecycle::{
        LifecycleInput, LifecycleNarrowSubjectDoc, LifecycleSubjectKind, Observation,
    };
    use std::collections::HashMap;

    /// A show's title input with known file figures.
    fn title_input(file_count: i64, total_size: i64) -> LifecycleInput {
        let mut show =
            crate::lifecycle_rules::facts::tests::faceted_title(None, MediaFacet::Series);
        show.id = "synthetic-show".to_string();
        let usernames = HashMap::new();
        let watch = crate::lifecycle_rules::facts::LifecycleWatchContext::default();
        let mut input = crate::lifecycle_rules::facts::build_title_input(
            chrono::Utc::now(),
            &show,
            &crate::lifecycle_rules::facts::LifecycleLibraryRef {
                id: "synthetic-library".to_string(),
                name: "Synthetic Shows".to_string(),
            },
            &[],
            crate::lifecycle_rules::facts::LifecycleTitlePeople {
                requester_user_ids: None,
                usernames: &usernames,
            },
            crate::lifecycle_rules::facts::LifecycleTitleWatch {
                context: &watch,
                signals: None,
                audience: None,
            },
            crate::lifecycle_rules::facts::LifecycleTitleClaims::none(),
            &[],
        );
        input.facts.has_file = Observation::known(true);
        input.facts.file_count = Observation::known(file_count);
        input.facts.total_file_size_bytes = Observation::known(total_size);
        input
    }

    /// `title` with one season as the subject, read beside the title.
    fn season_input(
        title: &LifecycleInput,
        season_id: &str,
        file_count: i64,
        total_size: i64,
    ) -> LifecycleInput {
        let mut input = title.clone();
        let mut facts = title.facts.clone();
        facts.has_file = Observation::known(file_count > 0);
        facts.file_count = Observation::known(file_count);
        facts.total_file_size_bytes = Observation::known(total_size);
        input.subject.kind = LifecycleSubjectKind::Season;
        input.season = Some(LifecycleNarrowSubjectDoc {
            id: season_id.to_string(),
            collection_id: Some(season_id.to_string()),
            season_number: Some(1),
            episode_number: None,
            facts,
        });
        input
    }

    /// A season journal whose delete removed `completed_sizes` from a
    /// season that held `baseline_count` files of `baseline_size` bytes.
    fn season_journal(
        title: &LifecycleInput,
        season_id: &str,
        baseline_count: i64,
        baseline_size: i64,
        completed_sizes: &[i64],
    ) -> ScopedDeletionCheckpoint {
        let baseline = season_input(title, season_id, baseline_count, baseline_size);
        ScopedDeletionCheckpoint {
            scoped_deletion: 1,
            match_generation: 1,
            action_kind: "sequence".to_string(),
            sequence_step_id: Some("delete_files".to_string()),
            subject_kind: "season".to_string(),
            subject_id: season_id.to_string(),
            subject_label: "Synthetic Season".to_string(),
            grace_deadline: "2026-01-01T00:00:00+00:00".to_string(),
            total_size_bytes: baseline_size,
            baseline: ScopeFactSnapshot::capture(&baseline),
            mutation_started: true,
            files: completed_sizes
                .iter()
                .enumerate()
                .map(|(index, size)| FileCheckpoint {
                    file_id: format!("{season_id}-file-{index}"),
                    file_path: format!("/synthetic/{season_id}/{index}.mkv"),
                    catalog_proof: format!("{season_id}-proof-{index}"),
                    file_size_bytes: Some(*size),
                    plan: None,
                    retention_episode_ids: Vec::new(),
                    completed: true,
                    error: None,
                    error_reason: None,
                })
                .collect(),
            hold: None,
        }
    }

    fn known(observation: &Observation<i64>) -> i64 {
        match observation {
            Observation::Known { value, .. } => *value,
            other => panic!("expected a known figure, got {other:?}"),
        }
    }

    /// The season's count reads exactly its baseline minus the journal's
    /// removal, but its size moved some other way: the count comes back, the
    /// size stays live, and the title's figures get nothing added.
    #[test]
    fn a_restorable_count_with_an_unrestorable_size_adds_nothing_to_the_title() {
        let title = title_input(3, 20);
        let journal = season_journal(&title, "synthetic-season-1", 2, 10, &[5]);
        let mut input = season_input(&title, "synthetic-season-1", 1, 7);

        journal.restore_sequence_owned_file_deltas(&mut input);

        assert_eq!(known(&input.subject_facts().file_count), 2);
        assert_eq!(known(&input.subject_facts().total_file_size_bytes), 7);
        assert_eq!(known(&input.facts.file_count), 3);
        assert_eq!(known(&input.facts.total_file_size_bytes), 20);
    }

    /// Two seasons of one show each removed one file. Each season is judged
    /// on its own copy of the title's facts, with only its own removal added
    /// back: neither reads the other's removal as restored.
    #[test]
    fn two_started_seasons_each_add_back_only_their_own_removal() {
        let title = title_input(2, 10);
        let first = season_journal(&title, "synthetic-season-1", 2, 10, &[5]);
        let second = season_journal(&title, "synthetic-season-2", 2, 10, &[5]);

        let mut first_input = season_input(&title, "synthetic-season-1", 1, 5);
        first.restore_sequence_owned_file_deltas(&mut first_input);
        let mut second_input = season_input(&title, "synthetic-season-2", 1, 5);
        second.restore_sequence_owned_file_deltas(&mut second_input);

        for input in [&first_input, &second_input] {
            assert_eq!(known(&input.subject_facts().file_count), 2);
            assert_eq!(known(&input.subject_facts().total_file_size_bytes), 10);
            assert_eq!(known(&input.facts.file_count), 3);
            assert_eq!(known(&input.facts.total_file_size_bytes), 15);
        }
        assert_eq!(
            known(&title.facts.file_count),
            2,
            "the title input is untouched"
        );

        // Restoring the same journal again finds the figures no longer read
        // baseline minus its removal, and adds nothing a second time.
        first.restore_sequence_owned_file_deltas(&mut first_input);
        assert_eq!(known(&first_input.facts.file_count), 3);
        assert_eq!(known(&first_input.facts.total_file_size_bytes), 15);
    }
}
