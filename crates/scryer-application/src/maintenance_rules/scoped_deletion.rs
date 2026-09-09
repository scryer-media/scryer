//! Resumable policy deletion using the catalog's per-file deletion owner.
use super::action_execution::{ActionResult, SafetyDecision, execution_reason};
use super::facts::MaintenanceLibraryRef;
use super::service::MaintenanceRuleSetDetail;
use crate::library::user_delete::PolicyMediaFileDeletePlan;
use crate::{AppError, AppResult, AppUseCase, PolicyDeleteAuthorization};
use scryer_domain::{
    LifecycleActionRun, LifecycleCandidate, MaintenanceRuleSubjectKind as Scope, Title, User,
};
use scryer_rules::maintenance::{
    MaintenanceFileDoc, MaintenanceInput, MaintenanceRulesEvaluator, Observation,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ScopeFactSnapshot {
    monitored: Observation<bool>,
    has_file: Observation<bool>,
    file_count: Observation<i64>,
    total_file_size_bytes: Observation<i64>,
    files: Observation<Vec<MaintenanceFileDoc>>,
    first_imported_at: Observation<String>,
    episode_file_count: Observation<i64>,
    monitored_episode_count: Observation<i64>,
}
impl ScopeFactSnapshot {
    fn capture(input: &MaintenanceInput) -> Self {
        let facts = &input.facts;
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
    /// Only restore facts this action changes. Watch signals, claims, identity,
    /// exclusions, and the rule's authorization are still rechecked live.
    pub fn restore(&self, input: &mut MaintenanceInput) {
        let facts = &mut input.facts;
        facts.monitored = self.monitored.clone();
        facts.has_file = self.has_file.clone();
        facts.file_count = self.file_count.clone();
        facts.total_file_size_bytes = self.total_file_size_bytes.clone();
        facts.files = self.files.clone();
        facts.first_imported_at = self.first_imported_at.clone();
        facts.episode_file_count = self.episode_file_count.clone();
        facts.monitored_episode_count = self.monitored_episode_count.clone();
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileCheckpoint {
    file_id: String,
    file_path: String,
    catalog_proof: String,
    plan: Option<PolicyMediaFileDeletePlan>,
    /// Episode coverage that this exact manifest contributed to rolling
    /// retention before it was unlinked. Legacy journals omit it safely.
    #[serde(default)]
    retention_episode_ids: Vec<String>,
    completed: bool,
    error: Option<String>,
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
    /// `None` is a legacy, unfiltered manifest. It can only resume a rule
    /// which still has no selected storage root.
    #[serde(default)]
    storage_root_id: Option<String>,
    /// Physical root path and filesystem identity captured before monitoring
    /// changed. A selected root id may be reconfigured between retries, which
    /// must hold rather than resume the old manifest at a new location.
    #[serde(default)]
    storage_root_identity: Option<String>,
    subject_kind: String,
    subject_id: String,
    subject_label: String,
    grace_deadline: String,
    total_size_bytes: i64,
    pub baseline: ScopeFactSnapshot,
    mutation_started: bool,
    files: Vec<FileCheckpoint>,
}

impl ScopedDeletionCheckpoint {
    pub(super) fn restore_baseline_if_started(&self, input: &mut MaintenanceInput) {
        if self.mutation_started {
            self.baseline.restore(input);
        }
    }

    pub(super) fn completed_retention_episode_ids(&self) -> BTreeSet<String> {
        self.mutation_started
            .then(|| {
                self.files
                    .iter()
                    .filter(|file| file.completed)
                    .flat_map(|file| file.retention_episode_ids.iter().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn file_matches_catalog(&self, file: &crate::types::TitleMediaFile) -> bool {
        self.files.iter().any(|saved| {
            !saved.completed
                && saved.file_id == file.id
                && saved.catalog_proof == catalog_proof(file)
        })
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
    pub(super) async fn maintenance_deletion_checkpoint(
        &self,
        candidate: &LifecycleCandidate,
        storage_root_id: Option<&str>,
    ) -> AppResult<Option<ScopedDeletionCheckpoint>> {
        let Some(run) = self
            .services
            .customization
            .maintenance_evaluation
            .latest_scoped_deletion_action_run(
                &candidate.id,
                candidate.match_generation,
                &candidate.action_kind,
            )
            .await?
        else {
            return Ok(None);
        };
        let mut checkpoint: ScopedDeletionCheckpoint = serde_json::from_str(&run.detail)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        // Version-one journals did not serialize action/generation. The query
        // already selected those exact columns from the immutable run row, so
        // it can supply their provenance without turning an interrupted
        // pre-upgrade deletion into a permanent hold. New journals carry both
        // values and are checked below.
        if checkpoint.match_generation == 0 && checkpoint.action_kind.is_empty() {
            checkpoint.match_generation = run.match_generation;
            checkpoint.action_kind = run.action_kind.clone();
        }
        if run.rule_set_id != candidate.rule_set_id
            || run.revision_number != candidate.revision_number
            || run.match_generation != candidate.match_generation
            || run.action_kind != candidate.action_kind
            || checkpoint.scoped_deletion != 1
            || checkpoint.match_generation != candidate.match_generation
            || checkpoint.action_kind != candidate.action_kind
            || checkpoint.storage_root_id.as_deref()
                != storage_root_id.map(str::trim).filter(|id| !id.is_empty())
            || checkpoint.subject_kind != candidate.subject_kind
            || checkpoint.subject_id != candidate.subject_id
        {
            return Err(AppError::Repository(
                "maintenance deletion checkpoint identity mismatch".into(),
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
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Determine whether an interrupted storage journal can safely resume when
    /// its own authorized paths disappeared before catalog cleanup or its
    /// checkpoint progress write. This accepts no new or replacement catalog
    /// file: every live row must match one saved proof, and a missing path is
    /// acceptable only through the journal's exact policy plan on an unchanged
    /// selected root.
    pub(super) async fn maintenance_storage_checkpoint_can_resume(
        &self,
        checkpoint: &ScopedDeletionCheckpoint,
        current_files: &[crate::types::TitleMediaFile],
        storage_root_id: Option<&str>,
    ) -> AppResult<bool> {
        if checkpoint.storage_root_identity
            != self
                .maintenance_storage_root_identity(storage_root_id)
                .await?
        {
            return Ok(false);
        }
        // Files outside the selected root were intentionally omitted from the
        // original manifest. They may coexist with its checkpoint and must
        // not block recovery; a newly discovered root-local file still does.
        for file in current_files {
            if checkpoint.file_matches_catalog(file) {
                continue;
            }
            match self
                .maintenance_files_on_root(vec![file.clone()], storage_root_id)
                .await?
            {
                Some(files) if files.is_empty() => continue,
                _ => return Ok(false),
            }
        }
        for saved in checkpoint.files.iter().filter(|file| !file.completed) {
            let Some(current) = self
                .services
                .library
                .media_files
                .get_media_file_by_id(&saved.file_id)
                .await?
            else {
                // The catalog cleanup committed after the authorized unlink;
                // the scoped executor will record this exact id as complete.
                if !checkpoint.mutation_started {
                    return Ok(false);
                }
                continue;
            };
            if catalog_proof(&current) != saved.catalog_proof
                || !current_files.iter().any(|file| {
                    file.id == saved.file_id && catalog_proof(file) == saved.catalog_proof
                })
            {
                return Ok(false);
            }
            match self
                .maintenance_files_on_root(vec![current], storage_root_id)
                .await?
            {
                Some(files) if files.len() == 1 => continue,
                Some(_) => return Ok(false),
                None => {}
            }
            if !checkpoint.mutation_started {
                return Ok(false);
            }
            let Some(plan) = saved.plan.as_ref() else {
                return Ok(false);
            };
            if self
                .maintenance_policy_plan_on_root_or_absent(plan, storage_root_id)
                .await?
                != Some(true)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// True only when every unfinished journal entry has already lost every
    /// exact policy path. Continuing in this state performs catalog cleanup
    /// and checkpoint reconciliation only; it cannot unlink a new path after
    /// storage pressure has recovered.
    pub(super) async fn maintenance_storage_checkpoint_needs_catalog_cleanup(
        &self,
        checkpoint: &ScopedDeletionCheckpoint,
        current_files: &[crate::types::TitleMediaFile],
    ) -> AppResult<bool> {
        if !checkpoint.mutation_started {
            return Ok(false);
        }
        for saved in checkpoint.files.iter().filter(|file| !file.completed) {
            if current_files.iter().all(|file| file.id != saved.file_id) {
                continue;
            }
            let Some(plan) = saved.plan.as_ref() else {
                return Ok(false);
            };
            if plan.paths_are_all_absent().await? != Some(true) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Unmonitor every persisted monitoring layer before a title-preserving
    /// deletion. The explicit child writes make future discovery inherit the
    /// title's disabled state and make the postcondition independently
    /// observable before the first filesystem mutation.
    async fn unmonitor_maintenance_title(&self, title: &Title) -> AppResult<()> {
        let actor = User::system_execution_actor();
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
                "maintenance title child ownership mismatch".into(),
            ));
        }

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
            .ok_or_else(|| AppError::NotFound(format!("title {}", title.id)))?;
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
        if fresh_title.monitored
            || fresh_collections
                .iter()
                .any(|collection| collection.monitored)
            || fresh_episodes.iter().any(|episode| episode.monitored)
        {
            return Err(AppError::Repository(
                "maintenance title unmonitoring was not confirmed; files preserved".into(),
            ));
        }
        Ok(())
    }

    /// Build the title-wide manifest from actual associations. Direct title
    /// files and episode files are included only when their catalog ownership
    /// is unambiguous; series-movie links are deliberately left untouched.
    /// Unambiguously title-owned files for storage summaries and policy
    /// manifests. Linked movies and files with any foreign episode association
    /// are excluded rather than treated as title coverage.
    pub(crate) async fn maintenance_title_deletion_files(
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
                "maintenance title episode ownership mismatch".into(),
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

    pub(super) async fn maintenance_policy_deletion_files(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<(String, Vec<crate::types::TitleMediaFile>)> {
        if candidate.subject_kind == Scope::Title.as_storage_str() {
            return Ok((
                title.name.clone(),
                self.maintenance_title_deletion_files(title).await?,
            ));
        }
        let subject = self
            .maintenance_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        if subject.ambiguous_files {
            return Err(AppError::Validation(
                "ambiguous maintenance file coverage".into(),
            ));
        }
        Ok((subject.label, subject.files))
    }

    pub(super) async fn unmonitor_maintenance_child(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
    ) -> AppResult<()> {
        let subject = self
            .maintenance_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
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
                return Err(AppError::Validation(
                    "child deletion requires an episode or season".into(),
                ));
            }
        }
        // Confirm the persisted state, including every future episode of a season.
        let fresh = self
            .maintenance_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        if fresh.monitored || fresh.episodes.iter().any(|episode| episode.monitored) {
            return Err(AppError::Repository(
                "maintenance unmonitoring was not confirmed; files preserved".into(),
            ));
        }
        Ok(())
    }

    async fn save_scoped_deletion_checkpoint(
        &self,
        run: &mut LifecycleActionRun,
        checkpoint: &ScopedDeletionCheckpoint,
    ) -> AppResult<()> {
        run.detail = serde_json::to_string(checkpoint)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        // The run remains Running; this is durable progress, not a success report.
        self.services
            .customization
            .maintenance_evaluation
            .finish_action_run(run)
            .await
    }

    pub(super) async fn execute_scoped_maintenance_deletion(
        &self,
        candidate: &LifecycleCandidate,
        title: &Title,
        input: &MaintenanceInput,
        run: &mut LifecycleActionRun,
        storage_root_id: Option<&str>,
        detail: &MaintenanceRuleSetDetail,
        evaluator: &mut MaintenanceRulesEvaluator,
        libraries: &HashMap<String, MaintenanceLibraryRef>,
        tag_conflicts: &HashSet<String>,
    ) -> AppResult<ActionResult> {
        let storage_root_identity = if storage_root_id.is_some() {
            let Some(identity) = self
                .maintenance_storage_root_identity(storage_root_id)
                .await?
            else {
                return Ok(ActionResult::Held {
                    reason: execution_reason::UNKNOWN_AT_EXECUTION,
                    detail: serde_json::json!({
                        "storage_root_id": storage_root_id,
                        "reason": "storage_root_identity_unavailable",
                    }),
                });
            };
            Some(identity)
        } else {
            None
        };
        let (subject_label, mut subject_files) = self
            .maintenance_policy_deletion_files(candidate, title)
            .await?;
        if storage_root_id.is_some()
            && self
                .maintenance_storage_root_identity(storage_root_id)
                .await?
                != storage_root_identity
        {
            return Ok(ActionResult::Held {
                reason: execution_reason::UNKNOWN_AT_EXECUTION,
                detail: serde_json::json!({
                    "storage_root_id": storage_root_id,
                    "reason": "storage_root_identity_changed",
                }),
            });
        }
        let existing_checkpoint = self
            .maintenance_deletion_checkpoint(candidate, storage_root_id)
            .await?;
        let mut checkpoint = match existing_checkpoint {
            Some(checkpoint) => {
                if checkpoint.storage_root_identity != storage_root_identity {
                    return Ok(ActionResult::Held {
                        reason: execution_reason::UNKNOWN_AT_EXECUTION,
                        detail: serde_json::json!({
                            "storage_root_id": storage_root_id,
                            "reason": "storage_root_identity_changed",
                        }),
                    });
                }
                if storage_root_id.is_some()
                    && !self
                        .maintenance_storage_checkpoint_can_resume(
                            &checkpoint,
                            &subject_files,
                            storage_root_id,
                        )
                        .await?
                {
                    return Ok(ActionResult::Held {
                        reason: execution_reason::UNKNOWN_AT_EXECUTION,
                        detail: serde_json::json!({
                            "storage_root_id": storage_root_id,
                            "reason": "checkpoint_root_membership_unavailable",
                        }),
                    });
                }
                checkpoint
            }
            None => {
                if storage_root_id.is_some() {
                    let Some(files) = self
                        .maintenance_files_on_root(subject_files, storage_root_id)
                        .await?
                    else {
                        return Ok(ActionResult::Held {
                            reason: execution_reason::UNKNOWN_AT_EXECUTION,
                            detail: serde_json::json!({
                                "storage_root_id": storage_root_id,
                                "remaining_files_held": true,
                            }),
                        });
                    };
                    subject_files = files;
                }
                let retention_inventory = if title.facet != scryer_domain::MediaFacet::Movie {
                    Some(self.maintenance_load_title_episode_inventory(title).await?)
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
                        Ok(plan) if storage_root_id.is_some() => {
                            match self
                                .maintenance_policy_plan_on_root(&plan, storage_root_id)
                                .await?
                            {
                                Some(true) => (Some(plan), None),
                                Some(false) => {
                                    return Ok(ActionResult::Held {
                                        reason: execution_reason::UNKNOWN_AT_EXECUTION,
                                        detail: serde_json::json!({
                                            "storage_root_id": storage_root_id,
                                            "file_id": file.id,
                                            "reason": "policy_delete_path_outside_selected_root",
                                        }),
                                    });
                                }
                                None => {
                                    return Ok(ActionResult::Held {
                                        reason: execution_reason::UNKNOWN_AT_EXECUTION,
                                        detail: serde_json::json!({
                                            "storage_root_id": storage_root_id,
                                            "file_id": file.id,
                                            "reason": "policy_delete_path_root_unavailable",
                                        }),
                                    });
                                }
                            }
                        }
                        Ok(plan) => (Some(plan), None),
                        Err(error) if storage_root_id.is_some() => {
                            return Ok(ActionResult::Held {
                                reason: execution_reason::UNKNOWN_AT_EXECUTION,
                                detail: serde_json::json!({
                                    "storage_root_id": storage_root_id,
                                    "file_id": file.id,
                                    "reason": "policy_delete_plan_unavailable",
                                    "error": error.to_string(),
                                }),
                            });
                        }
                        Err(error) => (None, Some(error.to_string())),
                    };
                    files.push(FileCheckpoint {
                        file_id: file.id.clone(),
                        file_path: file.file_path.clone(),
                        catalog_proof: catalog_proof(file),
                        plan,
                        retention_episode_ids,
                        completed: false,
                        error,
                    });
                }
                ScopedDeletionCheckpoint {
                    scoped_deletion: 1,
                    match_generation: candidate.match_generation,
                    action_kind: candidate.action_kind.clone(),
                    storage_root_id: storage_root_id
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string),
                    storage_root_identity: storage_root_identity.clone(),
                    subject_kind: candidate.subject_kind.clone(),
                    subject_id: candidate.subject_id.clone(),
                    subject_label,
                    grace_deadline: candidate.due_at.to_rfc3339(),
                    total_size_bytes: subject_files.iter().map(|file| file.size_bytes).sum(),
                    baseline: ScopeFactSnapshot::capture(input),
                    mutation_started: false,
                    files,
                }
            }
        };
        self.reconcile_absent_scoped_deletion_catalog_rows(&mut checkpoint)
            .await?;
        if storage_root_id.is_none()
            && subject_files.iter().any(|file| {
                !checkpoint.files.iter().any(|saved| {
                    saved.file_id == file.id && saved.catalog_proof == catalog_proof(file)
                })
            })
        {
            return Err(AppError::Validation(
                "maintenance file targets changed; another evaluation is required".into(),
            ));
        }
        let mut guards = Vec::new();
        if storage_root_id.is_some() {
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
                self.save_scoped_deletion_checkpoint(run, &checkpoint)
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
                if storage_root_id.is_some() {
                    self.save_scoped_deletion_checkpoint(run, &checkpoint)
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
        self.save_scoped_deletion_checkpoint(run, &checkpoint)
            .await?;
        if candidate.subject_kind == Scope::Title.as_storage_str() {
            self.unmonitor_maintenance_title(title).await?;
        } else {
            self.unmonitor_maintenance_child(candidate, title).await?;
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
            // Storage pressure is a moving condition. The first safety check
            // authorized unmonitoring and the manifest; every subsequent file
            // gets a new capacity probe and Rego evaluation before unlinking.
            if storage_root_id.is_some() {
                match self
                    .maintenance_execution_safety_checks(
                        detail,
                        evaluator,
                        candidate,
                        libraries,
                        tag_conflicts,
                    )
                    .await
                {
                    SafetyDecision::Proceed(_, _) => {}
                    SafetyDecision::Cancel(reason) => {
                        checkpoint.files[index].error = Some(reason.to_string());
                        self.save_scoped_deletion_checkpoint(run, &checkpoint)
                            .await?;
                        return Ok(ActionResult::Canceled {
                            reason,
                            detail: serde_json::to_value(checkpoint)
                                .map_err(|error| AppError::Repository(error.to_string()))?,
                        });
                    }
                    SafetyDecision::Excluded => {
                        checkpoint.files[index].error = Some(
                            crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED
                                .to_string(),
                        );
                        self.save_scoped_deletion_checkpoint(run, &checkpoint)
                            .await?;
                        return Ok(ActionResult::Canceled {
                            reason:
                                crate::maintenance_rules::evaluation::candidate_reason::EXCLUDED,
                            detail: serde_json::to_value(checkpoint)
                                .map_err(|error| AppError::Repository(error.to_string()))?,
                        });
                    }
                    SafetyDecision::Hold(reason) => {
                        checkpoint.files[index].error = Some(reason.to_string());
                        self.save_scoped_deletion_checkpoint(run, &checkpoint)
                            .await?;
                        return Ok(ActionResult::Held {
                            reason,
                            detail: serde_json::to_value(checkpoint)
                                .map_err(|error| AppError::Repository(error.to_string()))?,
                        });
                    }
                }
            }
            if storage_root_id.is_some()
                && self
                    .maintenance_storage_root_identity(storage_root_id)
                    .await?
                    != checkpoint.storage_root_identity
            {
                checkpoint.files[index].error =
                    Some(execution_reason::UNKNOWN_AT_EXECUTION.to_string());
                self.save_scoped_deletion_checkpoint(run, &checkpoint)
                    .await?;
                return Ok(ActionResult::Held {
                    reason: execution_reason::UNKNOWN_AT_EXECUTION,
                    detail: serde_json::to_value(checkpoint)
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                });
            }
            // Re-read ownership after monitoring and before every file
            // mutation. An unprovable root/path is a hold, never a partial
            // manifest interpreted as "nothing left".
            let (_, fresh_files) = self
                .maintenance_policy_deletion_files(candidate, title)
                .await?;
            // The primary media file can remain under the selected root while
            // a tracked subtitle or sidecar moves elsewhere. Re-prove every
            // persisted policy path immediately before using the shared
            // deleter, whose manifest may delete all of them together.
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
                self.save_scoped_deletion_checkpoint(run, &checkpoint)
                    .await?;
                continue;
            };
            if storage_root_id.is_some() {
                let target_is_proven_on_root =
                    match fresh_files.iter().find(|file| file.id == saved.file_id) {
                        Some(file) => matches!(
                            self.maintenance_files_on_root(vec![file.clone()], storage_root_id)
                                .await?,
                            Some(files) if files.len() == 1
                        ),
                        None => false,
                    };
                if !target_is_proven_on_root
                    && self
                        .maintenance_policy_plan_on_root_or_absent(&plan, storage_root_id)
                        .await?
                        != Some(true)
                {
                    checkpoint.files[index].error =
                        Some(execution_reason::UNKNOWN_AT_EXECUTION.to_string());
                    self.save_scoped_deletion_checkpoint(run, &checkpoint)
                        .await?;
                    return Ok(ActionResult::Held {
                        reason: execution_reason::UNKNOWN_AT_EXECUTION,
                        detail: serde_json::to_value(checkpoint)
                            .map_err(|error| AppError::Repository(error.to_string()))?,
                    });
                }
            }
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
                    return Err(AppError::Validation(
                        "maintenance file ownership changed".into(),
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
                }
                Err(error) => {
                    failed = true;
                    checkpoint.files[index].error = Some(error.to_string());
                }
            }
            self.save_scoped_deletion_checkpoint(run, &checkpoint)
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
