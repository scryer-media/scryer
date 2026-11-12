//! Durable source configuration imports. Workflow jobs own progress; private inputs stay encrypted.
use crate::{
    AppError, AppResult, AppUseCase, JobKey, JobRun, JobRunRecord, JobRunStatus, JobTriggerSource,
};
use scryer_domain::{AppPermission, Id, NewDownloadClientConfig, NewIndexerConfig, User};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tokio::sync::Mutex;

pub mod refusal {
    /// The submitted request or selected identity is invalid.
    pub const INVALID_REQUEST: &str = "SOURCE_IMPORT_INVALID_REQUEST";
    /// A duplicate request attempted to change accepted selections.
    pub const SELECTION_CHANGED: &str = "SOURCE_IMPORT_SELECTION_CHANGED";
    /// The source import is not owned by this actor.
    pub const NOT_OWNED: &str = "SOURCE_IMPORT_NOT_OWNED";
    /// The bounded source import capacity is full.
    pub const QUEUE_FULL: &str = "SOURCE_IMPORT_QUEUE_FULL";
    /// The operation still has active workers.
    pub const ACTIVE: &str = "SOURCE_IMPORT_ACTIVE";
    /// An item is not in a state that permits retry or skip.
    pub const ITEM_NOT_RETRYABLE: &str = "SOURCE_IMPORT_ITEM_NOT_RETRYABLE";
    /// Setup still has unresolved selected sources.
    pub const UNRESOLVED: &str = "SOURCE_IMPORT_UNRESOLVED";
    /// A worker was interrupted before completing its item.
    pub const INTERRUPTED: &str = "SOURCE_IMPORT_INTERRUPTED";
    /// A selected source needs an explicit credential.
    pub const CREDENTIAL_REQUIRED: &str = "SOURCE_IMPORT_CREDENTIAL_REQUIRED";
    /// A reserved configuration identity no longer matches the selected source.
    pub const CONFIG_CONFLICT: &str = "SOURCE_IMPORT_CONFIG_CONFLICT";
    /// The source discovery snapshot has not completed.
    pub const SOURCE_NOT_READY: &str = "SOURCE_IMPORT_SOURCE_NOT_READY";
    /// A connection check or configuration save failed.
    pub const ITEM_FAILED: &str = "SOURCE_IMPORT_ITEM_FAILED";
    pub const ALL: &[&str] = &[
        INVALID_REQUEST,
        SELECTION_CHANGED,
        NOT_OWNED,
        QUEUE_FULL,
        ACTIVE,
        ITEM_NOT_RETRYABLE,
        UNRESOLVED,
        INTERRUPTED,
        CREDENTIAL_REQUIRED,
        CONFIG_CONFLICT,
        SOURCE_NOT_READY,
        ITEM_FAILED,
    ];
    #[cfg(test)]
    mod tests {
        #[test]
        fn source_import_reasons_are_stable_distinct_codes() {
            let mut seen = std::collections::HashSet::new();
            for code in super::ALL {
                assert!(code.starts_with("SOURCE_IMPORT_"));
                assert!(
                    code.bytes()
                        .all(|value| value.is_ascii_uppercase() || value == b'_')
                );
                assert!(seen.insert(code));
            }
        }
    }
}

const MAX_ITEMS: usize = 500;
pub const PROBE_CONCURRENCY: usize = 15;

pub(crate) struct SourceImportRuntime {
    submission: Mutex<()>,
    active: Mutex<std::collections::HashMap<String, String>>,
    plugin_install: Mutex<()>,
    download_clients: Mutex<()>,
    probes: std::sync::Arc<tokio::sync::Semaphore>,
    changed: tokio::sync::Notify,
}

impl Default for SourceImportRuntime {
    fn default() -> Self {
        Self {
            submission: Mutex::new(()),
            active: Mutex::new(std::collections::HashMap::new()),
            plugin_install: Mutex::new(()),
            download_clients: Mutex::new(()),
            probes: std::sync::Arc::new(tokio::sync::Semaphore::new(PROBE_CONCURRENCY)),
            changed: tokio::sync::Notify::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceImportInput {
    DownloadClient(NewDownloadClientConfig),
    Indexer(NewIndexerConfig),
    ImportedIndexer {
        name: String,
        provider_type: String,
        base_url: String,
        api_key: Option<String>,
        api_path: Option<String>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SourceImportSelection {
    pub key: String,
    pub name: String,
    pub input: SourceImportInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceImportState {
    Queued,
    Installing,
    Checking,
    Saving,
    Succeeded,
    Failed,
    Interrupted,
    Skipped,
}

impl SourceImportState {
    pub fn resolved(self) -> bool {
        matches!(self, Self::Succeeded | Self::Skipped)
    }
    pub fn retryable(self) -> bool {
        matches!(self, Self::Failed | Self::Interrupted)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SourceImportItem {
    pub key: String,
    pub name: String,
    pub config_id: String,
    pub kind: String,
    pub state: SourceImportState,
    pub reason: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SourceImportProgress {
    #[serde(default)]
    pub revision: u64,
    pub items: Vec<SourceImportItem>,
    #[serde(default)]
    pub plugins_installed: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PrivateItem {
    selection: SourceImportSelection,
    config_id: String,
    #[serde(default)]
    reuse_existing: bool,
}

fn refusal(reason: &'static str, message: &'static str) -> AppError {
    AppError::validation_refused(reason, message)
}
fn encode<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value)
        .map_err(|_| AppError::Repository("source import state encoding failed".into()))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> AppResult<T> {
    serde_json::from_str(value)
        .map_err(|_| AppError::Repository("source import state is invalid".into()))
}

impl AppUseCase {
    pub async fn existing_source_import(
        &self,
        actor: &User,
        request_key: &str,
        mut keys: Vec<String>,
    ) -> AppResult<Option<JobRunRecord>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let Some(run) = self
            .services
            .events
            .job_runs
            .find_private_job(&actor.id, &format!("source-import:{request_key}"))
            .await?
        else {
            return Ok(None);
        };
        let (_, progress) = self.source_import_status(actor, &run.id).await?;
        let mut accepted: Vec<_> = progress.items.into_iter().map(|item| item.key).collect();
        accepted.sort();
        keys.sort();
        keys.dedup();
        if accepted != keys {
            return Err(refusal(
                crate::source_import::refusal::SELECTION_CHANGED,
                "submitted source selections cannot be changed",
            ));
        }
        Ok(Some(run))
    }

    pub(crate) async fn source_import_reserved_providers(&self) -> AppResult<HashSet<String>> {
        let mut providers = HashSet::new();
        for run in self
            .services
            .events
            .job_runs
            .list_unresolved_private_jobs(JobKey::SourceImport)
            .await?
        {
            let Some(owner) = run.actor_user_id.as_deref() else {
                continue;
            };
            let Some(input) = self
                .services
                .events
                .job_runs
                .private_job_input(&run.id, owner)
                .await?
            else {
                continue;
            };
            let private: Vec<PrivateItem> = decode(&input)?;
            let progress: SourceImportProgress =
                decode(run.progress_json.as_deref().unwrap_or("{}"))?;
            for (item, state) in private.iter().zip(&progress.items) {
                if state.state.resolved() {
                    continue;
                }
                match &item.selection.input {
                    SourceImportInput::Indexer(input) => {
                        providers.insert(input.provider_type.clone());
                    }
                    SourceImportInput::ImportedIndexer { provider_type, .. } => {
                        providers.insert(provider_type.clone());
                    }
                    _ => {}
                }
            }
        }
        Ok(providers)
    }
    pub async fn wait_source_import(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<SourceImportProgress> {
        self.wait_source_import_for(actor, run_id, std::time::Duration::from_secs(1800))
            .await?
            .ok_or_else(|| {
                refusal(
                    crate::source_import::refusal::INTERRUPTED,
                    "source import continues in the background",
                )
            })
    }

    /// Waits up to `wait` for the run to settle; `None` means it is still running.
    async fn wait_source_import_for(
        &self,
        actor: &User,
        run_id: &str,
        wait: std::time::Duration,
    ) -> AppResult<Option<SourceImportProgress>> {
        match tokio::time::timeout(wait, async {
            loop {
                let changed = self.runtime.imports.source_import.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                let (run, progress) = self.source_import_status(actor, run_id).await?;
                if run.status.is_terminal() {
                    return Ok(progress);
                }
                changed.await;
            }
        })
        .await
        {
            Ok(result) => result.map(Some),
            Err(_) => Ok(None),
        }
    }
    /// Acceptance commits resolved inputs and reserved IDs before starting network work.
    pub async fn start_source_import(
        &self,
        actor: &User,
        request_key: &str,
        selections: Vec<SourceImportSelection>,
    ) -> AppResult<JobRunRecord> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        if request_key.is_empty() || request_key.len() > 128 || selections.len() > MAX_ITEMS {
            return Err(refusal(
                crate::source_import::refusal::INVALID_REQUEST,
                "invalid source import request",
            ));
        }
        let _guard = self.runtime.imports.source_import.submission.lock().await;
        if let Some(existing) = self
            .existing_source_import(
                actor,
                request_key,
                selections.iter().map(|item| item.key.clone()).collect(),
            )
            .await?
        {
            return Ok(existing);
        }
        if self.restore_coordinator().shutdown_workers.is_cancelled() {
            return Err(refusal(
                crate::source_import::refusal::INTERRUPTED,
                "source import is shutting down",
            ));
        }
        let mut keys = HashSet::new();
        if self.runtime.imports.source_import.active.lock().await.len() >= 16 {
            return Err(refusal(
                crate::source_import::refusal::QUEUE_FULL,
                "source import capacity is full",
            ));
        }
        if selections.iter().any(|item| {
            item.key.is_empty() || item.key.len() > 1024 || !keys.insert(item.key.clone())
        }) {
            return Err(refusal(
                crate::source_import::refusal::INVALID_REQUEST,
                "source import identities must be distinct",
            ));
        }
        let mut private = Vec::new();
        for selection in selections {
            let existing = if let SourceImportInput::Indexer(input) = &selection.input
                && input.provider_type == "prowlarr"
            {
                let wanted: serde_json::Value =
                    decode(input.config_json.as_deref().unwrap_or("{}"))?;
                self.services
                    .integrations
                    .indexer_configs
                    .list(Some("prowlarr".into()))
                    .await?
                    .into_iter()
                    .find(|config| {
                        config
                            .config_json
                            .as_deref()
                            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                            .is_some_and(|value| {
                                stored_config_base_url(&value) == stored_config_base_url(&wanted)
                            })
                    })
            } else {
                None
            };
            private.push(PrivateItem {
                selection,
                config_id: existing
                    .as_ref()
                    .map(|config| config.id.clone())
                    .unwrap_or_else(|| Id::new().0),
                reuse_existing: existing.is_some(),
            });
        }
        let progress = SourceImportProgress {
            revision: 0,
            items: private
                .iter()
                .map(|item| SourceImportItem {
                    key: item.selection.key.clone(),
                    name: item.selection.name.clone(),
                    config_id: item.config_id.clone(),
                    kind: if matches!(item.selection.input, SourceImportInput::DownloadClient(_)) {
                        "download_client"
                    } else {
                        "indexer"
                    }
                    .into(),
                    state: SourceImportState::Queued,
                    reason: None,
                    message: None,
                })
                .collect(),
            plugins_installed: Vec::new(),
        };
        let now = self.runtime.environment.now();
        let run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::SourceImport,
            operation_type: "source_import".into(),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: Some(encode(&progress)?),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        let (run, created) = self
            .services
            .events
            .job_runs
            .accept_private_job(
                &run,
                &format!("source-import:{request_key}"),
                &encode(&private)?,
            )
            .await?;
        if !created {
            let Some(existing) = self.source_import_private_items(actor, &run.id).await? else {
                // The run resolved and its input was purged; the receipt still answers.
                return self
                    .existing_source_import(
                        actor,
                        request_key,
                        private
                            .iter()
                            .map(|item| item.selection.key.clone())
                            .collect(),
                    )
                    .await?
                    .ok_or_else(|| AppError::Repository("source import receipt missing".into()));
            };
            if existing.len() != private.len()
                || existing.iter().zip(&private).any(|(left, right)| {
                    left.selection.key != right.selection.key
                        || left.selection.input != right.selection.input
                })
            {
                return Err(refusal(
                    crate::source_import::refusal::SELECTION_CHANGED,
                    "submitted source selections cannot be changed",
                ));
            }
        } else {
            let _ = self
                .append_domain_event(crate::domain_events::new_job_run_domain_event(
                    crate::DomainEventActor::from(actor),
                    run.id.clone(),
                    scryer_domain::DomainEventPayload::JobRunStarted(
                        scryer_domain::JobRunStartedEventData {
                            run_id: run.id.clone(),
                            job_key: run.job_key.as_str().into(),
                            operation_type: run.operation_type.clone(),
                            trigger_source: run.trigger_source.as_str().into(),
                        },
                    ),
                ))
                .await;
            self.spawn_source_import(actor.clone(), run.clone(), private)
                .await;
        }
        Ok(run)
    }

    async fn source_import_private_items(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<Option<Vec<PrivateItem>>> {
        self.services
            .events
            .job_runs
            .private_job_input(run_id, &actor.id)
            .await?
            .map(|input| decode(&input))
            .transpose()
    }

    /// Private inputs of a run the caller already owns. They are purged once every
    /// item resolves, so a missing input means nothing is left to retry.
    async fn source_import_inputs(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<Vec<PrivateItem>> {
        self.source_import_private_items(actor, run_id)
            .await?
            .ok_or_else(|| {
                refusal(
                    crate::source_import::refusal::ITEM_NOT_RETRYABLE,
                    "imported source credentials were removed after the import resolved",
                )
            })
    }

    /// Erases the stored source credentials once every item has resolved. The
    /// configurations the import saved keep their own copies.
    pub(crate) async fn purge_resolved_source_import(&self, run_id: &str) -> AppResult<()> {
        self.services
            .events
            .job_runs
            .purge_private_job_input(run_id)
            .await
    }

    pub async fn source_import_status(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<(JobRunRecord, SourceImportProgress)> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let run = self
            .services
            .events
            .job_runs
            .get_job_run(run_id)
            .await?
            .filter(|run| {
                run.job_key == JobKey::SourceImport
                    && run.actor_user_id.as_deref() == Some(&actor.id)
            })
            .ok_or_else(|| {
                refusal(
                    crate::source_import::refusal::NOT_OWNED,
                    "source import is unavailable to this user",
                )
            })?;
        let mut progress: SourceImportProgress =
            decode(run.progress_json.as_deref().unwrap_or("{}"))?;
        if run.status.is_terminal() {
            for item in &mut progress.items {
                if !item.state.resolved() && !item.state.retryable() {
                    item.state = SourceImportState::Interrupted;
                    item.reason = Some(crate::source_import::refusal::INTERRUPTED.into());
                }
            }
        }
        Ok((run, progress))
    }

    pub async fn retry_source_import(
        &self,
        actor: &User,
        run_id: &str,
        corrections: Vec<(String, String)>,
    ) -> AppResult<JobRunRecord> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        if self.restore_coordinator().shutdown_workers.is_cancelled() {
            return Err(refusal(
                crate::source_import::refusal::INTERRUPTED,
                "source import is shutting down",
            ));
        }
        let _guard = self.runtime.imports.source_import.submission.lock().await;
        if self.runtime.imports.source_import.active.lock().await.len() >= 16 {
            return Err(refusal(
                crate::source_import::refusal::QUEUE_FULL,
                "source import capacity is full",
            ));
        }
        let (mut run, mut progress) = self.source_import_status(actor, run_id).await?;
        if !run.status.is_terminal() {
            return Err(refusal(
                crate::source_import::refusal::ACTIVE,
                "source import is still active",
            ));
        }
        if !progress.items.iter().any(|item| item.state.retryable()) {
            return Err(refusal(
                crate::source_import::refusal::ITEM_NOT_RETRYABLE,
                "no failed or interrupted sources remain to retry",
            ));
        }
        let mut private = self.source_import_inputs(actor, run_id).await?;
        for (key, secret) in corrections {
            let Some(index) = progress
                .items
                .iter()
                .position(|item| item.key == key && item.state.retryable())
            else {
                return Err(refusal(
                    crate::source_import::refusal::ITEM_NOT_RETRYABLE,
                    "only failed or interrupted items can be retried",
                ));
            };
            private[index].reuse_existing = true;
            match &mut private[index].selection.input {
                SourceImportInput::ImportedIndexer { api_key, .. } => *api_key = Some(secret),
                SourceImportInput::Indexer(input) => {
                    let mut value: serde_json::Value =
                        decode(input.config_json.as_deref().unwrap_or("{}"))?;
                    value["api_key"] = secret.into();
                    input.config_json = Some(encode(&value)?);
                }
                SourceImportInput::DownloadClient(input) => {
                    let mut value: serde_json::Value = decode(&input.config_json)?;
                    value[if matches!(input.client_type.as_str(), "sabnzbd" | "weaver") {
                        "api_key"
                    } else {
                        "password"
                    }] = secret.into();
                    input.config_json = encode(&value)?;
                }
            }
        }
        for item in &mut progress.items {
            if item.state.retryable() {
                item.state = SourceImportState::Queued;
                item.reason = None;
                item.message = None;
            }
        }
        self.services
            .events
            .job_runs
            .update_private_job_input(run_id, &actor.id, &encode(&private)?)
            .await?;
        run.status = JobRunStatus::Running;
        run.completed_at = None;
        run.summary_json = None;
        run.summary_text = None;
        run.error_text = None;
        progress.revision = progress.revision.saturating_add(1);
        run.progress_json = Some(encode(&progress)?);
        let run = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&run, None))
            .await;
        self.spawn_source_import(actor.clone(), run.clone(), private)
            .await;
        Ok(run)
    }

    pub async fn skip_source_import_item(
        &self,
        actor: &User,
        run_id: &str,
        key: &str,
    ) -> AppResult<JobRunRecord> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let _guard = self.runtime.imports.source_import.submission.lock().await;
        let (mut run, mut progress) = self.source_import_status(actor, run_id).await?;
        if !run.status.is_terminal() {
            return Err(refusal(
                crate::source_import::refusal::ACTIVE,
                "source import is still active",
            ));
        }
        let item = progress
            .items
            .iter_mut()
            .find(|item| item.key == key && item.state.retryable())
            .ok_or_else(|| {
                refusal(
                    crate::source_import::refusal::ITEM_NOT_RETRYABLE,
                    "only unresolved terminal items can be skipped",
                )
            })?;
        item.state = SourceImportState::Skipped;
        item.reason = None;
        item.message = None;
        let resolved = progress.items.iter().all(|item| item.state.resolved());
        if resolved {
            run.status = JobRunStatus::Completed;
            run.error_text = None;
        }
        apply_source_import_summary(&mut run, &progress);
        self.publish_source_progress(&mut run, &mut progress)
            .await?;
        if resolved {
            self.purge_resolved_source_import(run_id).await?;
        }
        Ok(run)
    }

    pub async fn require_source_import_resolved(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<()> {
        let (_, progress) = self.source_import_status(actor, run_id).await?;
        if progress.items.iter().any(|item| !item.state.resolved()) {
            return Err(refusal(
                crate::source_import::refusal::UNRESOLVED,
                "resolve or explicitly skip imported sources before finishing setup",
            ));
        }
        Ok(())
    }

    async fn publish_source_progress(
        &self,
        run: &mut JobRunRecord,
        progress: &mut SourceImportProgress,
    ) -> AppResult<()> {
        progress.revision = progress.revision.saturating_add(1);
        run.progress_json = Some(encode(progress)?);
        run.updated_at = self.runtime.environment.now();
        *run = self.services.events.job_runs.update_job_run(run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(run, None))
            .await;
        self.runtime.imports.source_import.changed.notify_waiters();
        Ok(())
    }

    async fn spawn_source_import(&self, actor: User, run: JobRunRecord, private: Vec<PrivateItem>) {
        let attempt = Id::new().0;
        self.runtime
            .imports
            .source_import
            .active
            .lock()
            .await
            .insert(run.id.clone(), attempt.clone());
        let app = self.clone();
        self.restore_coordinator().spawn_task(async move {
            let id = run.id.clone();
            if app
                .run_source_import(&actor, run.clone(), private)
                .await
                .is_err()
                && let Ok(Some(mut failed)) = app.services.events.job_runs.get_job_run(&id).await
            {
                failed.status = JobRunStatus::Failed;
                failed.completed_at = Some(app.runtime.environment.now());
                failed.error_text =
                    Some("Source import interrupted; completed items were retained".into());
                let _ = app.services.events.job_runs.update_job_run(&failed).await;
            }
            let mut active = app.runtime.imports.source_import.active.lock().await;
            if active.get(&id) == Some(&attempt) {
                active.remove(&id);
            }
            drop(active);
            app.runtime.imports.source_import.changed.notify_waiters();
        });
    }

    async fn run_source_import(
        &self,
        actor: &User,
        mut run: JobRunRecord,
        private: Vec<PrivateItem>,
    ) -> AppResult<()> {
        let mut progress: SourceImportProgress =
            decode(run.progress_json.as_deref().unwrap_or("{}"))?;
        let shutdown = self.restore_coordinator().shutdown_workers.clone();
        let mut workers = tokio::task::JoinSet::new();
        let failed_providers = std::sync::Arc::new(Mutex::new(std::collections::HashMap::<
            String,
            &'static str,
        >::new()));
        let (states, mut state_events) = tokio::sync::mpsc::channel(PROBE_CONCURRENCY);
        let (clients, indexers): (Vec<_>, Vec<_>) = private
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !progress.items[*index].state.resolved())
            .partition(|(_, item)| {
                matches!(item.selection.input, SourceImportInput::DownloadClient(_))
            });
        if !clients.is_empty() {
            let app = self.clone();
            let actor = actor.clone();
            let states = states.clone();
            let shutdown = shutdown.clone();
            workers.spawn(async move {
                for (index, item) in clients {
                    if shutdown.is_cancelled() {
                        break;
                    }
                    let SourceImportInput::DownloadClient(input) = item.selection.input else {
                        unreachable!()
                    };
                    let _client = app
                        .runtime
                        .imports
                        .source_import
                        .download_clients
                        .lock()
                        .await;
                    states
                        .send((index, SourceImportState::Saving, None, None))
                        .await
                        .ok();
                    let result = match require_download_client_credential(&input) {
                        Ok(()) => app
                            .create_source_import_download_client(
                                &actor,
                                input,
                                &item.config_id,
                                item.reuse_existing,
                            )
                            .await
                            .map(|_| ()),
                        Err(error) => Err(error),
                    };
                    states
                        .send((index, SourceImportState::Saving, None, Some(result)))
                        .await
                        .ok();
                }
                (None, Ok(()))
            });
        }
        let mut pending = indexers.into_iter();
        let mut indexer_workers = 0usize;
        loop {
            while indexer_workers < PROBE_CONCURRENCY && !shutdown.is_cancelled() {
                let Some((index, item)) = pending.next() else {
                    break;
                };
                let app = self.clone();
                let actor = actor.clone();
                let states = states.clone();
                let shutdown = shutdown.clone();
                let failed_providers = failed_providers.clone();
                workers.spawn(async move {
                    let result = async {
                        let _permit = tokio::select! { permit = app.runtime.imports.source_import.probes.clone().acquire_owned() => permit.map_err(|_| AppError::Repository("source workers closed".into()))?, _ = shutdown.cancelled() => return Err(refusal(crate::source_import::refusal::INTERRUPTED, "source import interrupted")) };
                        let provider = match &item.selection.input {
                            SourceImportInput::Indexer(input) => &input.provider_type,
                            SourceImportInput::ImportedIndexer { provider_type, .. } => provider_type,
                            _ => return Err(refusal(crate::source_import::refusal::CREDENTIAL_REQUIRED, "source credentials are required")),
                        };
                        if !app.available_indexer_provider_types().iter().any(|(kind, _, _, _)| kind == provider) {
                            states.send((index, SourceImportState::Installing, None, None)).await.ok();
                            let _install = app.runtime.imports.source_import.plugin_install.lock().await;
                            if let Some(reason) = failed_providers.lock().await.get(provider).cloned() {
                                return Err(AppError::validation_refused(reason, "provider installation failed; retry the source import"));
                            }
                            let installed = app.ensure_source_import_provider(&actor, provider).await;
                            match installed {
                                Ok(Some(name)) => { states.send((index, SourceImportState::Checking, Some(name), None)).await.ok(); },
                                Ok(None) => {},
                                Err(error) => { failed_providers.lock().await.insert(provider.clone(), error.validation_reason().unwrap_or(crate::source_import::refusal::ITEM_FAILED)); return Err(error); }
                            }

                        }
                        app.require_source_import_indexer_credential(&item.selection.input)?;
                        states.send((index, SourceImportState::Checking, None, None)).await.ok();
                        let config = app.prepare_source_import_indexer(&actor, &item).await?;
                        states.send((index, SourceImportState::Saving, None, None)).await.ok();
                        app.save_prepared_indexer_config(&actor, config).await.map(|_| ())
                    }.await;
                    (Some(index), result)
                });
                indexer_workers += 1;
            }
            if workers.is_empty() || shutdown.is_cancelled() {
                break;
            }
            tokio::select! {
                Some((index, state, installed, outcome)) = state_events.recv() => {
                    if let Some(outcome) = outcome { set_outcome(&mut progress.items[index], outcome); }
                    if let Some(installed) = installed && !progress.plugins_installed.contains(&installed) { progress.plugins_installed.push(installed); }
                    if !progress.items[index].state.resolved() && !progress.items[index].state.retryable() { progress.items[index].state = state; }
                }
                Some(result) = workers.join_next() => {
                    let (index, result) = result.map_err(|_| AppError::Repository("source worker interrupted".into()))?;
                    if let Some(index) = index { indexer_workers -= 1; set_outcome(&mut progress.items[index], result); }
                }
                _ = shutdown.cancelled() => break,
            }
            self.publish_source_progress(&mut run, &mut progress)
                .await?;
        }
        workers.abort_all();
        while let Ok((index, _, installed, outcome)) = state_events.try_recv() {
            if let Some(outcome) = outcome {
                set_outcome(&mut progress.items[index], outcome);
            }
            if let Some(installed) = installed
                && !progress.plugins_installed.contains(&installed)
            {
                progress.plugins_installed.push(installed);
            }
        }
        for item in &mut progress.items {
            if !item.state.resolved() && !item.state.retryable() {
                item.state = SourceImportState::Interrupted;
                item.reason = Some(crate::source_import::refusal::INTERRUPTED.into());
            }
        }
        run.status = if progress.items.iter().all(|item| item.state.resolved()) {
            JobRunStatus::Completed
        } else {
            JobRunStatus::Warning
        };
        run.completed_at = Some(self.runtime.environment.now());
        apply_source_import_summary(&mut run, &progress);
        self.publish_source_progress(&mut run, &mut progress)
            .await?;
        if run.status == JobRunStatus::Completed {
            self.purge_resolved_source_import(&run.id).await?;
        }
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                crate::DomainEventActor::from(actor),
                run.id.clone(),
                scryer_domain::DomainEventPayload::JobRunCompleted(
                    scryer_domain::JobRunCompletedEventData {
                        run_id: run.id.clone(),
                        job_key: run.job_key.as_str().into(),
                        summary_text: run.summary_text.clone(),
                    },
                ),
            ))
            .await;
        Ok(())
    }

    async fn ensure_source_import_provider(
        &self,
        actor: &User,
        provider: &str,
    ) -> AppResult<Option<String>> {
        if self
            .available_indexer_provider_types()
            .iter()
            .any(|(kind, _, _, _)| kind == provider)
        {
            return Ok(None);
        }
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(provider)
            .await?
            .is_some()
        {
            self.reload_plugin_providers().await?;
            return Ok(None);
        }
        let installed = match self.install_plugin(actor, provider).await {
            Ok(installed) => installed,
            Err(error)
                if error.validation_reason()
                    == Some(
                        crate::error_reasons::plugins::NOT_AVAILABLE_PLUGIN_CATALOG_NOT_FOUND,
                    ) =>
            {
                self.refresh_plugin_catalog_internal().await?;
                self.install_plugin(actor, provider).await?
            }
            Err(error) => return Err(error),
        };
        Ok(Some(installed.name))
    }

    /// A selected indexer without its required key fails on its own, so the rest
    /// of the batch still imports and a correction can retry it.
    fn require_source_import_indexer_credential(&self, input: &SourceImportInput) -> AppResult<()> {
        let present = match input {
            // Prowlarr parents always need their key.
            SourceImportInput::Indexer(input) if input.provider_type == "prowlarr" => {
                let value: serde_json::Value =
                    decode(input.config_json.as_deref().unwrap_or("{}"))?;
                config_has_secret(&value, "api_key")
            }
            SourceImportInput::Indexer(_) => true,
            SourceImportInput::ImportedIndexer {
                provider_type,
                api_key,
                ..
            } => {
                api_key
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    || !self
                        .indexer_config_fields_for_provider_type(provider_type)
                        .is_ok_and(|fields| {
                            fields
                                .iter()
                                .any(|field| field.required && is_api_key_field(field))
                        })
            }
            SourceImportInput::DownloadClient(_) => true,
        };
        if present {
            Ok(())
        } else {
            Err(refusal(
                crate::source_import::refusal::CREDENTIAL_REQUIRED,
                "selected indexer requires an API key",
            ))
        }
    }

    async fn prepare_source_import_indexer(
        &self,
        actor: &User,
        item: &PrivateItem,
    ) -> AppResult<scryer_domain::IndexerConfig> {
        let input = match &item.selection.input {
            SourceImportInput::Indexer(input) => input.clone(),
            SourceImportInput::ImportedIndexer {
                name,
                provider_type,
                base_url,
                api_key,
                api_path,
            } => {
                let fields = self.indexer_config_fields_for_provider_type(provider_type)?;
                NewIndexerConfig {
                    name: name.clone(),
                    provider_type: provider_type.clone(),
                    config_json: Some(imported_indexer_config_json(
                        &fields,
                        base_url,
                        api_key.as_deref(),
                        api_path.as_deref(),
                    )),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    max_queries_per_minute: None,
                    max_requests_per_month: None,
                    max_grabs_per_month: None,
                    is_enabled: true,
                    enable_auto_search: true,
                    enable_interactive_search: true,
                    proxy_config_id: None,
                    download_client_id: None,
                }
            }
            _ => unreachable!(),
        };
        if let Some(existing) = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(&item.config_id)
            .await?
        {
            if existing.provider_type != input.provider_type
                || (!item.reuse_existing && existing.name != input.name.trim())
            {
                return Err(refusal(
                    crate::source_import::refusal::CONFIG_CONFLICT,
                    "source configuration identity changed",
                ));
            }
            if item.reuse_existing {
                return self
                    .update_indexer_config(
                        actor,
                        crate::IndexerConfigUpdate {
                            id: item.config_id.clone(),
                            config_json: input.config_json.clone(),
                            is_enabled: Some(true),
                            ..Default::default()
                        },
                    )
                    .await;
            }
            return Ok(existing);
        }
        let mut config = self.prepare_indexer_config(actor, input).await?;
        config.id = item.config_id.clone();
        Ok(config)
    }
}

/// SABnzbd and Weaver need an API key and NZBGet a password; other clients may
/// connect without one. A missing secret fails only this item.
///
/// NZBGet can run with authentication disabled, so an empty password is a
/// real setting there: only a source that carries no password at all lacks it.
fn require_download_client_credential(input: &NewDownloadClientConfig) -> AppResult<()> {
    let value: serde_json::Value = decode(&input.config_json)?;
    let (present, message) = match input.client_type.as_str() {
        "sabnzbd" | "weaver" => (
            config_has_secret(&value, "api_key"),
            "selected download client requires an API key",
        ),
        "nzbget" => (
            value
                .get("password")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "selected download client requires a password",
        ),
        _ => return Ok(()),
    };
    if present {
        Ok(())
    } else {
        Err(refusal(
            crate::source_import::refusal::CREDENTIAL_REQUIRED,
            message,
        ))
    }
}

fn config_has_secret(value: &serde_json::Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .is_some_and(|secret| !secret.trim().is_empty())
}

fn is_api_key_field(field: &scryer_domain::ConfigFieldDef) -> bool {
    field.key == "api_key"
        || (field.field_type == scryer_domain::ConfigFieldType::Password
            && field.key.to_ascii_lowercase().contains("api"))
}

fn apply_source_import_summary(run: &mut JobRunRecord, progress: &SourceImportProgress) {
    let succeeded = progress
        .items
        .iter()
        .filter(|item| item.state == SourceImportState::Succeeded)
        .count();
    let failed = progress
        .items
        .iter()
        .filter(|item| item.state.retryable())
        .count();
    let skipped = progress
        .items
        .iter()
        .filter(|item| item.state == SourceImportState::Skipped)
        .count();
    run.summary_text = Some(format!(
        "Sources: {succeeded} imported, {failed} failed, {skipped} skipped"
    ));
    run.summary_json = Some(
        serde_json::json!({"succeeded":succeeded,"failed":failed,"skipped":skipped}).to_string(),
    );
}

fn set_outcome(item: &mut SourceImportItem, result: AppResult<()>) {
    match result {
        Ok(()) => {
            item.state = SourceImportState::Succeeded;
            item.reason = None;
            item.message = None;
        }
        Err(error) => {
            item.state = SourceImportState::Failed;
            item.reason = Some(
                error
                    .validation_reason()
                    .unwrap_or(crate::source_import::refusal::ITEM_FAILED)
                    .into(),
            );
            item.message = Some(source_import_failure_message(&error));
        }
    }
}

const MAX_FAILURE_MESSAGE_CHARS: usize = 500;

/// The server's own sentence for a failed item, kept for display only. Credentials
/// in URLs are masked because progress is stored in plain text.
fn source_import_failure_message(error: &AppError) -> String {
    let message = crate::url_redaction::redact_url_credentials(&error.to_string());
    if message.chars().count() <= MAX_FAILURE_MESSAGE_CHARS {
        return message;
    }
    let mut truncated: String = message.chars().take(MAX_FAILURE_MESSAGE_CHARS).collect();
    truncated.push('…');
    truncated
}

/// The connection URL stored in a source configuration. Older configurations
/// spell the key `baseUrl`.
fn stored_config_base_url(value: &serde_json::Value) -> Option<String> {
    ["base_url", "baseUrl"]
        .iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .map(|url| url.trim().trim_end_matches('/').to_ascii_lowercase())
        .filter(|url| !url.is_empty())
}

pub fn imported_indexer_config_json(
    fields: &[scryer_domain::ConfigFieldDef],
    base_url: &str,
    api_key: Option<&str>,
    api_path: Option<&str>,
) -> String {
    let mut object = serde_json::Map::new();
    if let Some(field) = fields
        .iter()
        .find(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl))
        && !base_url.trim().is_empty()
    {
        object.insert(field.key.clone(), base_url.trim().into());
    }
    if let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty())
        && let Some(field) = fields.iter().find(|field| is_api_key_field(field))
    {
        object.insert(field.key.clone(), api_key.into());
    }
    if let Some(api_path) = api_path.map(str::trim).filter(|value| !value.is_empty())
        && let Some(field) = fields.iter().find(|field| field.key == "api_path")
    {
        object.insert(field.key.clone(), api_path.into());
    }
    serde_json::Value::Object(object).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_config_base_url_reads_both_key_spellings() {
        let snake = serde_json::json!({"base_url": " https://Prowlarr.example.test/ "});
        let camel = serde_json::json!({"baseUrl": "https://prowlarr.example.test"});
        assert_eq!(
            stored_config_base_url(&snake).as_deref(),
            Some("https://prowlarr.example.test")
        );
        assert_eq!(
            stored_config_base_url(&snake),
            stored_config_base_url(&camel)
        );
        assert_eq!(
            stored_config_base_url(&serde_json::json!({"base_url": " "})),
            None
        );
        assert_eq!(stored_config_base_url(&serde_json::json!({})), None);
    }

    #[test]
    fn download_client_credential_is_checked_per_client_type() {
        let client = |client_type: &str, config: serde_json::Value| NewDownloadClientConfig {
            name: "Fixture".into(),
            client_type: client_type.into(),
            config_json: config.to_string(),
            client_priority: 0,
            is_enabled: true,
            proxy_config_id: None,
        };
        for (client_type, field) in [("sabnzbd", "api_key"), ("weaver", "api_key")] {
            for missing in [serde_json::json!({}), serde_json::json!({ field: "  " })] {
                assert_eq!(
                    require_download_client_credential(&client(client_type, missing))
                        .expect_err("missing secret fails the item")
                        .validation_reason(),
                    Some(refusal::CREDENTIAL_REQUIRED),
                    "{client_type}"
                );
            }
            require_download_client_credential(&client(
                client_type,
                serde_json::json!({ field: "synthetic-secret" }),
            ))
            .expect("present secret passes");
        }
        require_download_client_credential(&client("qbittorrent", serde_json::json!({})))
            .expect("clients without a required secret pass");
        // NZBGet with authentication disabled exports an empty password.
        for password in ["", "  ", "synthetic-secret"] {
            require_download_client_credential(&client(
                "nzbget",
                serde_json::json!({ "password": password }),
            ))
            .expect("an NZBGet password, even empty, passes");
        }
        assert_eq!(
            require_download_client_credential(&client("nzbget", serde_json::json!({})))
                .expect_err("an NZBGet source without a password fails the item")
                .validation_reason(),
            Some(refusal::CREDENTIAL_REQUIRED)
        );
    }

    #[test]
    fn failed_item_keeps_the_server_sentence_and_its_reason() {
        let mut item = SourceImportItem {
            key: "fixture".into(),
            name: "Fixture".into(),
            config_id: "id".into(),
            kind: "indexer".into(),
            state: SourceImportState::Checking,
            reason: None,
            message: None,
        };
        set_outcome(
            &mut item,
            Err(AppError::Repository(
                "connection to https://indexer.example.test/api?t=caps&apikey=synthetic-secret refused".into(),
            )),
        );
        assert_eq!(item.state, SourceImportState::Failed);
        assert_eq!(item.reason.as_deref(), Some(refusal::ITEM_FAILED));
        let message = item.message.clone().expect("failure detail");
        assert!(message.contains("connection to https://indexer.example.test/api"));
        assert!(message.contains("refused"));
        assert!(!message.contains("synthetic-secret"));

        set_outcome(
            &mut item,
            Err(AppError::validation_refused(
                refusal::CONFIG_CONFLICT,
                "source configuration identity changed",
            )),
        );
        assert_eq!(item.reason.as_deref(), Some(refusal::CONFIG_CONFLICT));
        assert!(
            item.message
                .expect("detail")
                .contains("source configuration identity changed")
        );
    }
}
