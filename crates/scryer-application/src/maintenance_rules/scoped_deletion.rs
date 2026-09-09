//! Resumable policy deletion using the catalog's per-file deletion owner.
use super::action_execution::ActionResult;
use crate::library::user_delete::PolicyMediaFileDeletePlan;
use crate::{AppError, AppResult, AppUseCase, PolicyDeleteAuthorization};
use scryer_domain::{
    LifecycleActionRun, LifecycleCandidate, MaintenanceRuleSubjectKind as Scope, Title, User,
};
use scryer_rules::maintenance::{MaintenanceFileDoc, MaintenanceInput, Observation};
use serde::{Deserialize, Serialize};

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
    completed: bool,
    error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ScopedDeletionCheckpoint {
    scoped_deletion: u32,
    subject_kind: String,
    subject_id: String,
    subject_label: String,
    grace_deadline: String,
    total_size_bytes: i64,
    pub baseline: ScopeFactSnapshot,
    mutation_started: bool,
    files: Vec<FileCheckpoint>,
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
    ) -> AppResult<Option<ScopedDeletionCheckpoint>> {
        let Some(run) = self
            .services
            .customization
            .maintenance_evaluation
            .latest_scoped_deletion_action_run(&candidate.id)
            .await?
        else {
            return Ok(None);
        };
        let checkpoint: ScopedDeletionCheckpoint = serde_json::from_str(&run.detail)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        if run.rule_set_id != candidate.rule_set_id
            || run.revision_number != candidate.revision_number
            || checkpoint.scoped_deletion != 1
            || checkpoint.subject_kind != candidate.subject_kind
            || checkpoint.subject_id != candidate.subject_id
        {
            return Err(AppError::Repository(
                "maintenance deletion checkpoint identity mismatch".into(),
            ));
        }
        Ok(Some(checkpoint))
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
    ) -> AppResult<ActionResult> {
        let subject = self
            .maintenance_resolve_subject(&candidate.subject_kind, &candidate.subject_id, title, &[])
            .await?;
        if subject.kind == Scope::Title || subject.ambiguous_files {
            return Err(AppError::Validation(
                "ambiguous maintenance file coverage".into(),
            ));
        }
        let mut checkpoint = match self.maintenance_deletion_checkpoint(candidate).await? {
            Some(checkpoint) => checkpoint,
            None => {
                let mut files = Vec::new();
                for file in &subject.files {
                    let (plan, error) = match self.prepare_policy_media_file_delete(&file.id).await
                    {
                        Ok(plan) => (Some(plan), None),
                        Err(error) => (None, Some(error.to_string())),
                    };
                    files.push(FileCheckpoint {
                        file_id: file.id.clone(),
                        file_path: file.file_path.clone(),
                        catalog_proof: catalog_proof(file),
                        plan,
                        completed: false,
                        error,
                    });
                }
                ScopedDeletionCheckpoint {
                    scoped_deletion: 1,
                    subject_kind: candidate.subject_kind.clone(),
                    subject_id: subject.id.clone(),
                    subject_label: subject.label.clone(),
                    grace_deadline: candidate.due_at.to_rfc3339(),
                    total_size_bytes: subject.files.iter().map(|file| file.size_bytes).sum(),
                    baseline: ScopeFactSnapshot::capture(input),
                    mutation_started: false,
                    files,
                }
            }
        };
        if subject.files.iter().any(|file| {
            !checkpoint
                .files
                .iter()
                .any(|saved| saved.file_id == file.id && saved.catalog_proof == catalog_proof(file))
        }) {
            return Err(AppError::Validation(
                "maintenance file targets changed; another evaluation is required".into(),
            ));
        }
        let mut guards = Vec::new();
        for file in &checkpoint.files {
            guards.push(
                self.runtime
                    .jobs
                    .interactive_operation_guards
                    .try_acquire(&format!("media-file:{}", file.file_id))
                    .await
                    .ok_or_else(|| {
                        AppError::Repository("another deletion owns this media file".into())
                    })?,
            );
        }
        // Persist intent before either the monitoring or filesystem mutation.
        checkpoint.mutation_started = true;
        self.save_scoped_deletion_checkpoint(run, &checkpoint)
            .await?;
        self.unmonitor_maintenance_child(candidate, title).await?;
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
            let saved = &checkpoint.files[index];
            let result = async {
                // Re-read ownership after monitoring and before every file mutation.
                let fresh = self
                    .maintenance_resolve_subject(
                        &candidate.subject_kind,
                        &candidate.subject_id,
                        title,
                        &[],
                    )
                    .await?;
                if fresh.ambiguous_files {
                    return Err(AppError::Validation("episode file coverage changed".into()));
                }
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
                    || !fresh.files.iter().any(|current| current.id == file.id)
                {
                    return Err(AppError::Validation(
                        "maintenance file ownership changed".into(),
                    ));
                }
                // A file without an approved manifest cannot gain paths during
                // a retry. A new evaluation must authorize its fresh preview.
                let plan = saved.plan.clone().ok_or_else(|| {
                    AppError::Validation(format!(
                        "{}; another evaluation is required",
                        saved.error.as_deref().unwrap_or("file preview unavailable")
                    ))
                })?;
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
