//! Operator-started imports of rules kept by another application: Sonarr and
//! Radarr custom formats, and Maintainerr rule groups. Every import saves its
//! rules disabled so the operator reviews them before they act.
//!
//! A run moves through three steps. `fetch` reads the source over HTTP and
//! keeps the result in memory only; the stored connection is purged as soon as
//! it finishes. `plan` turns the fetched data into one planned item per source
//! object. `save` writes each planned item and records its outcome in the run's
//! per-item report, which survives a restart.
use crate::external_import::{
    ArrQualityProfileFormatScores, ExternalArrClient, ExternalMaintainerrClient,
    RULE_IMPORT_ARR_MAX_RESPONSE_BYTES,
};
use crate::{
    AppError, AppResult, AppUseCase, JobKey, JobRun, JobRunRecord, JobRunStatus, JobTriggerSource,
};
use scryer_domain::{AppPermission, Id, User};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use tokio::sync::Mutex;

pub mod refusal {
    /// The request key, URL or other submitted field is invalid.
    pub const INVALID_REQUEST: &str = "RULE_IMPORT_INVALID_REQUEST";
    /// A repeated request key was submitted with a different source or URL.
    pub const REQUEST_CHANGED: &str = "RULE_IMPORT_REQUEST_CHANGED";
    /// The import run does not exist or belongs to another user.
    pub const NOT_OWNED: &str = "RULE_IMPORT_NOT_OWNED";
    /// Too many rule imports are already running.
    pub const QUEUE_FULL: &str = "RULE_IMPORT_QUEUE_FULL";
    /// Sonarr and Radarr imports need an API key.
    pub const API_KEY_REQUIRED: &str = "RULE_IMPORT_API_KEY_REQUIRED";
    /// Scryer cannot yet translate rules from this source.
    pub const TRANSLATOR_UNAVAILABLE: &str = "RULE_IMPORT_TRANSLATOR_UNAVAILABLE";
    /// The source could not be read; the run-level message says why.
    pub const FETCH_FAILED: &str = "RULE_IMPORT_FETCH_FAILED";
    /// The source holds more objects than one import accepts.
    pub const TOO_MANY_ITEMS: &str = "RULE_IMPORT_TOO_MANY_ITEMS";
    /// The run stopped before this step or item finished.
    pub const INTERRUPTED: &str = "RULE_IMPORT_INTERRUPTED";
    /// A rule with the imported name already exists, so the item was skipped.
    pub const ALREADY_EXISTS: &str = "RULE_IMPORT_ALREADY_EXISTS";
    /// Saving one item failed; the item's message says why.
    pub const ITEM_FAILED: &str = "RULE_IMPORT_ITEM_FAILED";
    /// Maintainerr could not list the quality profiles of one Sonarr or Radarr server.
    pub const PROFILES_UNAVAILABLE: &str = "RULE_IMPORT_PROFILES_UNAVAILABLE";
    /// Maintainerr predates its rule user list, so user names cannot be checked.
    pub const USERNAMES_UNAVAILABLE: &str = "RULE_IMPORT_USERNAMES_UNAVAILABLE";
    /// Rule groups name more Sonarr or Radarr servers than one import reads profiles for.
    pub const TOO_MANY_PROFILE_SOURCES: &str = "RULE_IMPORT_TOO_MANY_PROFILE_SOURCES";
    /// Scryer could not prepare the import from its own data, such as its
    /// quality profiles; the source was read and is not at fault.
    pub const PLAN_FAILED: &str = "RULE_IMPORT_PLAN_FAILED";
    pub const ALL: &[&str] = &[
        INVALID_REQUEST,
        REQUEST_CHANGED,
        NOT_OWNED,
        QUEUE_FULL,
        API_KEY_REQUIRED,
        TRANSLATOR_UNAVAILABLE,
        FETCH_FAILED,
        TOO_MANY_ITEMS,
        INTERRUPTED,
        ALREADY_EXISTS,
        ITEM_FAILED,
        PROFILES_UNAVAILABLE,
        USERNAMES_UNAVAILABLE,
        TOO_MANY_PROFILE_SOURCES,
        PLAN_FAILED,
    ];
    #[cfg(test)]
    mod tests {
        #[test]
        fn rule_import_reasons_are_stable_distinct_codes() {
            let mut seen = std::collections::HashSet::new();
            for code in super::ALL {
                assert!(code.starts_with("RULE_IMPORT_"));
                assert!(
                    code.bytes()
                        .all(|value| value.is_ascii_uppercase() || value == b'_')
                );
                assert!(seen.insert(code));
            }
        }
    }
}

/// Most source objects one import plans.
pub const MAX_ITEMS: usize = 1000;
/// Most Sonarr or Radarr servers whose profiles one Maintainerr import reads.
pub const MAX_PROFILE_SOURCES: usize = 32;
/// Most rule imports running at once.
const MAX_ACTIVE: usize = 4;
/// Fastest rate at which save progress is written while items are saving.
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
const MAX_MESSAGE_CHARS: usize = 500;
const REQUEST_KEY_PREFIX: &str = "rule-import:";

pub(crate) struct RuleImportRuntime {
    submission: Mutex<()>,
    active: Mutex<std::collections::HashSet<String>>,
    changed: tokio::sync::Notify,
}

impl Default for RuleImportRuntime {
    fn default() -> Self {
        Self {
            submission: Mutex::new(()),
            active: Mutex::new(std::collections::HashSet::new()),
            changed: tokio::sync::Notify::new(),
        }
    }
}

/// The application rules are imported from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleImportSource {
    Sonarr,
    Radarr,
    Maintainerr,
}

impl RuleImportSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sonarr => "SONARR",
            Self::Radarr => "RADARR",
            Self::Maintainerr => "MAINTAINERR",
        }
    }

    fn requires_api_key(self) -> bool {
        matches!(self, Self::Sonarr | Self::Radarr)
    }
}

/// What the operator submitted. Stored encrypted as the run's private input
/// until the fetch finishes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleImportConnection {
    pub source: RuleImportSource,
    pub base_url: String,
    /// Required for Sonarr and Radarr; Maintainerr has no API key and ignores it.
    #[serde(default)]
    pub api_key: Option<String>,
}

impl std::fmt::Debug for RuleImportConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleImportConnection")
            .field("source", &self.source)
            .field(
                "base_url",
                &crate::url_redaction::redact_url_credentials(&self.base_url),
            )
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Data read from a Sonarr or Radarr instance.
#[derive(Clone, Debug)]
pub struct FetchedArrRules {
    pub app_name: String,
    pub version: String,
    /// `GET /api/v3/customformat`, unchanged.
    pub custom_formats: Vec<Value>,
    /// Each quality profile with the score it gives every custom format.
    pub quality_profiles: Vec<ArrQualityProfileFormatScores>,
}

/// Data read from a Maintainerr instance.
#[derive(Clone, Debug)]
pub struct FetchedMaintainerrRules {
    pub version: String,
    /// `GET /api/rules`: rule groups with their rules, unchanged.
    pub rule_groups: Vec<Value>,
    /// `GET /api/rules/constants`.
    pub constants: Value,
    /// `GET /api/rules/users`; `None` when this Maintainerr predates it.
    pub usernames: Option<Value>,
    /// Quality profiles per Radarr settings id the rule groups name.
    pub radarr_profiles: Vec<(i64, Value)>,
    /// Quality profiles per Sonarr settings id the rule groups name.
    pub sonarr_profiles: Vec<(i64, Value)>,
}

/// Everything one fetch read. It lives in memory for the run only.
#[derive(Clone, Debug)]
pub enum RuleImportFetched {
    Sonarr(FetchedArrRules),
    Radarr(FetchedArrRules),
    Maintainerr(FetchedMaintainerrRules),
}

/// A coded observation about the run or one item. `details` carries values the
/// client needs, never folded into the message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleImportFact {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl RuleImportFact {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleImportItemState {
    /// Planned and not yet saved.
    Pending,
    /// Saved as an exact translation.
    Saved,
    /// Saved, but the translation needs the operator's review.
    SavedNeedsReview,
    /// Saved as a placeholder because the source rule could not be translated.
    SavedFailedTranslation,
    /// A rule with this name already exists; nothing was written.
    SkippedExists,
    /// Nothing was saved.
    Failed,
}

impl RuleImportItemState {
    fn is_clean(self) -> bool {
        matches!(self, Self::Saved | Self::SkippedExists)
    }
}

/// One source object's line in the report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleImportItem {
    pub key: String,
    pub source_name: String,
    #[serde(default)]
    pub saved_name: Option<String>,
    #[serde(default)]
    pub rule_set_id: Option<String>,
    pub state: RuleImportItemState,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub facts: Vec<RuleImportFact>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleImportPhase {
    Queued,
    Fetching,
    Planning,
    Saving,
    Finished,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleImportCounts {
    pub total: usize,
    pub pending: usize,
    pub saved: usize,
    pub saved_needs_review: usize,
    pub saved_failed_translation: usize,
    pub skipped_exists: usize,
    pub failed: usize,
}

impl RuleImportCounts {
    fn of(items: &[RuleImportItem]) -> Self {
        let mut counts = Self {
            total: items.len(),
            ..Self::default()
        };
        for item in items {
            match item.state {
                RuleImportItemState::Pending => counts.pending += 1,
                RuleImportItemState::Saved => counts.saved += 1,
                RuleImportItemState::SavedNeedsReview => counts.saved_needs_review += 1,
                RuleImportItemState::SavedFailedTranslation => counts.saved_failed_translation += 1,
                RuleImportItemState::SkippedExists => counts.skipped_exists += 1,
                RuleImportItemState::Failed => counts.failed += 1,
            }
        }
        counts
    }
}

/// The run's report, stored as the job run's `progress_json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleImportProgress {
    #[serde(default)]
    pub revision: u64,
    pub source: RuleImportSource,
    pub base_url: String,
    pub phase: RuleImportPhase,
    /// Why the run as a whole stopped short, when it did.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub counts: RuleImportCounts,
    #[serde(default)]
    pub items: Vec<RuleImportItem>,
    /// Observations about the fetch that belong to no single item.
    #[serde(default)]
    pub facts: Vec<RuleImportFact>,
}

/// One source object a translator plans to save.
#[derive(Debug)]
pub struct PlannedRuleImportItem {
    /// Stable within the source, such as `customformat:12` or `rulegroup:3`.
    pub key: String,
    pub source_name: String,
    pub translation: PlannedRuleImportTranslation,
}

/// What a planned item saves as. Each translator adds the variant it produces,
/// for example a disabled scoring rule set or a disabled lifecycle rule, and
/// handles it in [`AppUseCase::save_rule_import_item`].
#[derive(Debug)]
pub enum PlannedRuleImportTranslation {
    /// A disabled scoring rule set for one Sonarr or Radarr custom format.
    ArrScoringRule(crate::rule_import_arr::PlannedArrScoringRule),
}

/// The outcome of saving one planned item.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleImportSaveOutcome {
    pub state: RuleImportItemState,
    pub saved_name: Option<String>,
    pub rule_set_id: Option<String>,
    pub reason: Option<&'static str>,
    pub facts: Vec<RuleImportFact>,
}

/// State loaded once per run before any item saves, such as the names of
/// existing rules, so saves never query per item.
#[derive(Default)]
pub struct RuleImportSaveContext {
    /// Names of existing scoring rule sets, and of every name this run claims.
    pub existing_names: std::collections::HashSet<String>,
    /// Scoring rule sets the run writes together once every item is decided.
    pub staged_rule_sets: Vec<StagedRuleImportRuleSet>,
}

/// A scoring rule set an item saves, written with the run's batch.
#[derive(Debug)]
pub struct StagedRuleImportRuleSet {
    pub item_index: usize,
    /// The item's state once the rule set is written.
    pub state: RuleImportItemState,
    /// Names whose presence, checked again when the batch is written, means
    /// the item was already imported. They include the rule set's own name.
    pub claimed_names: Vec<String>,
    pub rule_set: crate::NewRuleSet,
}

/// Scryer state a translator reads while planning, loaded once per run.
#[derive(Default)]
pub struct RuleImportPlanContext {
    /// Names of Scryer's quality profiles, matched exactly.
    pub quality_profile_names: std::collections::HashSet<String>,
}

impl RuleImportConnection {
    /// Reads everything the source's translator needs. Nothing is written.
    pub async fn fetch(&self) -> AppResult<(RuleImportFetched, Vec<RuleImportFact>)> {
        match self.source {
            RuleImportSource::Sonarr => {
                let client = ExternalArrClient::for_sonarr_v4(
                    self.base_url.clone(),
                    self.api_key.clone().unwrap_or_default(),
                )?
                .with_max_response_bytes(RULE_IMPORT_ARR_MAX_RESPONSE_BYTES);
                Ok((
                    RuleImportFetched::Sonarr(fetch_arr(&client).await?),
                    Vec::new(),
                ))
            }
            RuleImportSource::Radarr => {
                let client = ExternalArrClient::for_radarr_v6(
                    self.base_url.clone(),
                    self.api_key.clone().unwrap_or_default(),
                )?
                .with_max_response_bytes(RULE_IMPORT_ARR_MAX_RESPONSE_BYTES);
                Ok((
                    RuleImportFetched::Radarr(fetch_arr(&client).await?),
                    Vec::new(),
                ))
            }
            RuleImportSource::Maintainerr => {
                let client = ExternalMaintainerrClient::new(self.base_url.clone())?;
                let (fetched, facts) = fetch_maintainerr(&client).await?;
                Ok((RuleImportFetched::Maintainerr(fetched), facts))
            }
        }
    }

    /// Rejects a malformed submission before anything is stored.
    fn validate(&self) -> AppResult<()> {
        if self.base_url.trim().is_empty() || self.base_url.len() > 2048 {
            return Err(AppError::validation_refused(
                refusal::INVALID_REQUEST,
                "a source URL is required",
            ));
        }
        match self.source {
            RuleImportSource::Sonarr | RuleImportSource::Radarr => {
                let key = self.api_key.as_deref().map(str::trim).unwrap_or_default();
                if key.is_empty() {
                    return Err(AppError::validation_refused(
                        refusal::API_KEY_REQUIRED,
                        "Sonarr and Radarr imports need an API key",
                    ));
                }
                if key.len() > 512 {
                    return Err(AppError::validation_refused(
                        refusal::INVALID_REQUEST,
                        "the API key is too long",
                    ));
                }
                let base_url = self.base_url.clone();
                if self.source == RuleImportSource::Sonarr {
                    ExternalArrClient::for_sonarr_v4(base_url, key.into())?;
                } else {
                    ExternalArrClient::for_radarr_v6(base_url, key.into())?;
                }
            }
            RuleImportSource::Maintainerr => {
                ExternalMaintainerrClient::new(self.base_url.clone())?;
            }
        }
        Ok(())
    }

    fn normalized(mut self) -> Self {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.api_key = if self.source.requires_api_key() {
            self.api_key
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty())
        } else {
            None
        };
        self
    }
}

async fn fetch_arr(client: &ExternalArrClient) -> AppResult<FetchedArrRules> {
    let (app_name, version) = client.test_connection().await?;
    let custom_formats = client.list_custom_formats().await?;
    let quality_profiles = client.list_quality_profile_format_scores().await?;
    Ok(FetchedArrRules {
        app_name,
        version,
        custom_formats,
        quality_profiles,
    })
}

async fn fetch_maintainerr(
    client: &ExternalMaintainerrClient,
) -> AppResult<(FetchedMaintainerrRules, Vec<RuleImportFact>)> {
    let mut facts = Vec::new();
    let version = client.test_connection().await?;
    let rule_groups = client.list_rule_groups().await?;
    let constants = client.rule_constants().await?;
    let usernames = match client.rule_usernames().await {
        Ok(usernames) => Some(usernames),
        Err(error)
            if error.reason()
                == Some(crate::error_reasons::external_import::SOURCE_ENDPOINT_MISSING) =>
        {
            facts.push(RuleImportFact::new(
                refusal::USERNAMES_UNAVAILABLE,
                "this Maintainerr version does not list rule users",
            ));
            None
        }
        Err(error) => return Err(error),
    };
    let (radarr_ids, sonarr_ids) = maintainerr_profile_sources(&rule_groups);
    let mut radarr_profiles = Vec::new();
    let mut sonarr_profiles = Vec::new();
    for (application, ids, profiles) in [
        ("radarr", radarr_ids, &mut radarr_profiles),
        ("sonarr", sonarr_ids, &mut sonarr_profiles),
    ] {
        if ids.len() > MAX_PROFILE_SOURCES {
            facts.push(
                RuleImportFact::new(
                    refusal::TOO_MANY_PROFILE_SOURCES,
                    "rule groups name more servers than one import reads profiles for",
                )
                .with_details(serde_json::json!({
                    "application": application,
                    "count": ids.len(),
                    "limit": MAX_PROFILE_SOURCES,
                })),
            );
        }
        for settings_id in ids.into_iter().take(MAX_PROFILE_SOURCES) {
            let result = if application == "radarr" {
                client.radarr_profiles(settings_id).await
            } else {
                client.sonarr_profiles(settings_id).await
            };
            match result {
                Ok(value) => profiles.push((settings_id, value)),
                Err(error) if error.reason().is_some() => facts.push(
                    RuleImportFact::new(refusal::PROFILES_UNAVAILABLE, bounded_message(&error))
                        .with_details(serde_json::json!({
                            "application": application,
                            "settingsId": settings_id,
                            "sourceReason": error.reason(),
                        })),
                ),
                Err(error) => return Err(error),
            }
        }
    }
    Ok((
        FetchedMaintainerrRules {
            version,
            rule_groups,
            constants,
            usernames,
            radarr_profiles,
            sonarr_profiles,
        },
        facts,
    ))
}

/// Distinct Radarr and Sonarr settings ids the rule groups use. Maintainerr
/// keeps them on the group's collection; newer versions repeat them on the group.
fn maintainerr_profile_sources(groups: &[Value]) -> (BTreeSet<i64>, BTreeSet<i64>) {
    let mut radarr = BTreeSet::new();
    let mut sonarr = BTreeSet::new();
    for group in groups {
        for holder in [Some(group), group.get("collection")].into_iter().flatten() {
            if let Some(id) = holder.get("radarrSettingsId").and_then(Value::as_i64) {
                radarr.insert(id);
            }
            if let Some(id) = holder.get("sonarrSettingsId").and_then(Value::as_i64) {
                sonarr.insert(id);
            }
        }
    }
    (radarr, sonarr)
}

/// Turns fetched data into one planned item per source object. A source whose
/// translator has not been written yet is refused, never faked.
pub fn plan_rule_import(
    fetched: &RuleImportFetched,
    context: &RuleImportPlanContext,
) -> AppResult<Vec<PlannedRuleImportItem>> {
    use crate::arr_custom_format::ArrSourceApplication;
    match fetched {
        RuleImportFetched::Sonarr(arr) => {
            crate::rule_import_arr::plan_arr_rule_import(ArrSourceApplication::Sonarr, arr, context)
        }
        RuleImportFetched::Radarr(arr) => {
            crate::rule_import_arr::plan_arr_rule_import(ArrSourceApplication::Radarr, arr, context)
        }
        RuleImportFetched::Maintainerr(_) => Err(AppError::validation_refused(
            refusal::TRANSLATOR_UNAVAILABLE,
            "Scryer cannot translate rules from this source yet",
        )),
    }
}

fn bounded_message(error: &AppError) -> String {
    let message = crate::url_redaction::redact_url_credentials(&error.to_string());
    if message.chars().count() <= MAX_MESSAGE_CHARS {
        return message;
    }
    let mut truncated: String = message.chars().take(MAX_MESSAGE_CHARS).collect();
    truncated.push('…');
    truncated
}

fn encode<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value)
        .map_err(|_| AppError::Repository("rule import state encoding failed".into()))
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> AppResult<T> {
    serde_json::from_str(value)
        .map_err(|_| AppError::Repository("rule import state is invalid".into()))
}

fn apply_summary(run: &mut JobRunRecord, progress: &RuleImportProgress) {
    let counts = progress.counts;
    run.summary_text = Some(format!(
        "Rules: {} saved, {} need review, {} untranslated, {} skipped, {} failed",
        counts.saved,
        counts.saved_needs_review,
        counts.saved_failed_translation,
        counts.skipped_exists,
        counts.failed
    ));
    run.summary_json = serde_json::to_string(&counts).ok();
}

impl AppUseCase {
    /// Accepts an import and starts it in the background. Repeating a request
    /// key returns the run it already started.
    pub async fn start_rule_import(
        &self,
        actor: &User,
        request_key: &str,
        connection: RuleImportConnection,
    ) -> AppResult<JobRunRecord> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        if request_key.trim().is_empty() || request_key.len() > 128 {
            return Err(AppError::validation_refused(
                refusal::INVALID_REQUEST,
                "invalid rule import request key",
            ));
        }
        let connection = connection.normalized();
        connection.validate()?;
        let request_key = format!("{REQUEST_KEY_PREFIX}{request_key}");
        let _guard = self.runtime.imports.rule_import.submission.lock().await;
        if let Some(run) = self
            .services
            .events
            .job_runs
            .find_private_job(&actor.id, &request_key)
            .await?
        {
            return self.existing_rule_import(actor, run, &connection).await;
        }
        if self.restore_coordinator().shutdown_workers.is_cancelled() {
            return Err(AppError::validation_refused(
                refusal::INTERRUPTED,
                "rule import is shutting down",
            ));
        }
        if self.runtime.imports.rule_import.active.lock().await.len() >= MAX_ACTIVE {
            return Err(AppError::validation_refused(
                refusal::QUEUE_FULL,
                "too many rule imports are running",
            ));
        }
        let progress = RuleImportProgress {
            revision: 0,
            source: connection.source,
            base_url: connection.base_url.clone(),
            phase: RuleImportPhase::Queued,
            reason: None,
            message: None,
            counts: RuleImportCounts::default(),
            items: Vec::new(),
            facts: Vec::new(),
        };
        let now = self.runtime.environment.now();
        let run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::ExternalRuleImport,
            operation_type: "external_rule_import".into(),
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
            .accept_private_job(&run, &request_key, &encode(&connection)?)
            .await?;
        if !created {
            return self.existing_rule_import(actor, run, &connection).await;
        }
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
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&run, None))
            .await;
        self.spawn_rule_import(actor.clone(), run.clone(), connection)
            .await;
        Ok(run)
    }

    /// A repeated request answers with its run, provided it names the same source.
    async fn existing_rule_import(
        &self,
        actor: &User,
        run: JobRunRecord,
        connection: &RuleImportConnection,
    ) -> AppResult<JobRunRecord> {
        let (run, progress) = self.rule_import_status(actor, &run.id).await?;
        if progress.source != connection.source || progress.base_url != connection.base_url {
            return Err(AppError::validation_refused(
                refusal::REQUEST_CHANGED,
                "a repeated rule import request cannot change its source",
            ));
        }
        Ok(run)
    }

    /// The caller's run and its report. A run that ended early reports its
    /// unsaved items as interrupted.
    pub async fn rule_import_status(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<(JobRunRecord, RuleImportProgress)> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let run = self
            .services
            .events
            .job_runs
            .get_job_run(run_id)
            .await?
            .filter(|run| {
                run.job_key == JobKey::ExternalRuleImport
                    && run.actor_user_id.as_deref() == Some(actor.id.as_str())
            })
            .ok_or_else(|| {
                AppError::validation_refused(
                    refusal::NOT_OWNED,
                    "rule import is unavailable to this user",
                )
            })?;
        let mut progress: RuleImportProgress =
            decode(run.progress_json.as_deref().unwrap_or("{}"))?;
        if run.status.is_terminal() {
            settle_interrupted(&mut progress);
        }
        Ok((run, progress))
    }

    /// Waits for the run to finish and returns its report.
    pub async fn wait_rule_import(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<RuleImportProgress> {
        loop {
            let changed = self.runtime.imports.rule_import.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let (run, progress) = self.rule_import_status(actor, run_id).await?;
            if run.status.is_terminal() {
                return Ok(progress);
            }
            changed.await;
        }
    }

    async fn spawn_rule_import(
        &self,
        actor: User,
        run: JobRunRecord,
        connection: RuleImportConnection,
    ) {
        self.runtime
            .imports
            .rule_import
            .active
            .lock()
            .await
            .insert(run.id.clone());
        let app = self.clone();
        self.restore_coordinator().spawn_task(async move {
            let id = run.id.clone();
            let result = app.run_rule_import(&actor, run, connection).await;
            if result.is_err() {
                // The run may have stopped before its fetch purged the connection.
                let _ = app
                    .services
                    .events
                    .job_runs
                    .purge_private_job_input(&id)
                    .await;
            }
            if result.is_err()
                && let Ok(Some(mut failed)) = app.services.events.job_runs.get_job_run(&id).await
                && !failed.status.is_terminal()
            {
                failed.status = JobRunStatus::Failed;
                failed.completed_at = Some(app.runtime.environment.now());
                failed.error_text =
                    Some("Rule import interrupted; saved rules were retained".into());
                if let Ok(failed) = app.services.events.job_runs.update_job_run(&failed).await {
                    app.runtime
                        .jobs
                        .job_run_tracker
                        .upsert_active_run(JobRun::from_record(&failed, None))
                        .await;
                }
            }
            app.runtime
                .imports
                .rule_import
                .active
                .lock()
                .await
                .remove(&id);
            app.runtime.imports.rule_import.changed.notify_waiters();
        });
    }

    async fn run_rule_import(
        &self,
        actor: &User,
        mut run: JobRunRecord,
        connection: RuleImportConnection,
    ) -> AppResult<()> {
        let mut progress: RuleImportProgress =
            decode(run.progress_json.as_deref().unwrap_or("{}"))?;
        let shutdown = self.restore_coordinator().shutdown_workers.clone();

        progress.phase = RuleImportPhase::Fetching;
        self.publish_rule_import(&mut run, &mut progress).await?;
        let fetched = tokio::select! {
            fetched = connection.fetch() => fetched,
            _ = shutdown.cancelled() => Err(AppError::validation_refused(
                refusal::INTERRUPTED,
                "rule import stopped while reading the source",
            )),
        };
        drop(connection);
        self.services
            .events
            .job_runs
            .purge_private_job_input(&run.id)
            .await?;

        let outcome = match fetched {
            Err(error) => Err(error),
            Ok((fetched, facts)) => {
                progress.facts = facts;
                progress.phase = RuleImportPhase::Planning;
                self.publish_rule_import(&mut run, &mut progress).await?;
                match self.load_rule_import_plan_context(&fetched).await {
                    // Translation compiles every source object; keep it off
                    // the async workers.
                    Ok(context) => {
                        tokio::task::spawn_blocking(move || plan_rule_import(&fetched, &context))
                            .await
                            .unwrap_or_else(|_| {
                                Err(AppError::RepositoryFailed {
                                    message: "rule import planning failed".into(),
                                    reason: refusal::PLAN_FAILED,
                                })
                            })
                    }
                    // Scryer's own data failed to load; the source is not at fault.
                    Err(error) => Err(AppError::RepositoryFailed {
                        message: format!("Scryer could not prepare the import: {error}"),
                        reason: refusal::PLAN_FAILED,
                    }),
                }
            }
        };
        let planned = match outcome {
            Ok(planned) if planned.len() > MAX_ITEMS => Err(AppError::validation_refused(
                refusal::TOO_MANY_ITEMS,
                "the source holds more objects than one import accepts",
            )),
            other => other,
        };
        let planned = match planned {
            Ok(planned) => planned,
            Err(error) => {
                progress.reason = Some(
                    error
                        .reason()
                        .map(|reason| {
                            if reason.starts_with("RULE_IMPORT_")
                                || reason.starts_with("EXTERNAL_IMPORT_")
                            {
                                reason
                            } else {
                                refusal::FETCH_FAILED
                            }
                        })
                        .unwrap_or(refusal::FETCH_FAILED)
                        .into(),
                );
                progress.message = Some(bounded_message(&error));
                return self
                    .finish_rule_import(actor, &mut run, &mut progress, JobRunStatus::Failed)
                    .await;
            }
        };

        progress.items = planned
            .iter()
            .map(|item| RuleImportItem {
                key: item.key.clone(),
                source_name: item.source_name.clone(),
                saved_name: None,
                rule_set_id: None,
                state: RuleImportItemState::Pending,
                reason: None,
                message: None,
                facts: Vec::new(),
            })
            .collect();
        progress.phase = RuleImportPhase::Saving;
        self.publish_rule_import(&mut run, &mut progress).await?;

        let mut context = self.load_rule_import_save_context(actor).await?;
        let mut last_publish = std::time::Instant::now();
        for (index, item) in planned.into_iter().enumerate() {
            if shutdown.is_cancelled() {
                break;
            }
            let outcome = self
                .save_rule_import_item(actor, &mut context, index, item)
                .await;
            let entry = &mut progress.items[index];
            match outcome {
                Ok(outcome) => {
                    entry.state = outcome.state;
                    entry.saved_name = outcome.saved_name;
                    entry.rule_set_id = outcome.rule_set_id;
                    entry.reason = outcome.reason.map(Into::into);
                    entry.facts = outcome.facts;
                }
                Err(error) => {
                    entry.state = RuleImportItemState::Failed;
                    entry.reason = Some(error.reason().unwrap_or(refusal::ITEM_FAILED).into());
                    entry.message = Some(bounded_message(&error));
                }
            }
            if last_publish.elapsed() >= PROGRESS_INTERVAL {
                self.publish_rule_import(&mut run, &mut progress).await?;
                last_publish = std::time::Instant::now();
            }
        }
        if shutdown.is_cancelled() {
            settle_interrupted(&mut progress);
        } else {
            self.save_staged_rule_import(actor, &mut context, &mut progress)
                .await;
            // Report the written rule sets before anything else can stop the run.
            self.publish_rule_import(&mut run, &mut progress).await?;
        }
        let status = if progress.items.iter().all(|item| item.state.is_clean()) {
            JobRunStatus::Completed
        } else {
            JobRunStatus::Warning
        };
        self.finish_rule_import(actor, &mut run, &mut progress, status)
            .await
    }

    /// Loads, once per run, the Scryer state planning reads.
    async fn load_rule_import_plan_context(
        &self,
        fetched: &RuleImportFetched,
    ) -> AppResult<RuleImportPlanContext> {
        let mut context = RuleImportPlanContext::default();
        if matches!(
            fetched,
            RuleImportFetched::Sonarr(_) | RuleImportFetched::Radarr(_)
        ) {
            context.quality_profile_names = self
                .services
                .config
                .quality_profiles
                .list_quality_profiles(crate::SETTINGS_SCOPE_SYSTEM, None)
                .await?
                .into_iter()
                .map(|profile| profile.name)
                .collect();
        }
        Ok(context)
    }

    /// Loads, once per run, whatever saves need to decide each item.
    async fn load_rule_import_save_context(
        &self,
        _actor: &User,
    ) -> AppResult<RuleImportSaveContext> {
        Ok(RuleImportSaveContext {
            existing_names: self
                .services
                .customization
                .rule_sets
                .list_rule_set_names()
                .await?
                .into_iter()
                .collect(),
            ..RuleImportSaveContext::default()
        })
    }

    /// Saves one planned item as the acting user. Translators add one arm per
    /// [`PlannedRuleImportTranslation`] variant. An item may stage its write
    /// in `context` and report `Pending`; staged writes land together after
    /// the last item.
    async fn save_rule_import_item(
        &self,
        _actor: &User,
        context: &mut RuleImportSaveContext,
        index: usize,
        item: PlannedRuleImportItem,
    ) -> AppResult<RuleImportSaveOutcome> {
        match item.translation {
            PlannedRuleImportTranslation::ArrScoringRule(rule) => Ok(
                crate::rule_import_arr::stage_arr_scoring_rule(context, index, rule),
            ),
        }
    }

    /// Writes the staged scoring rule sets as one batch and settles their items.
    async fn save_staged_rule_import(
        &self,
        actor: &User,
        context: &mut RuleImportSaveContext,
        progress: &mut RuleImportProgress,
    ) {
        let staged = std::mem::take(&mut context.staged_rule_sets);
        if staged.is_empty() {
            return;
        }
        let (targets, drafts): (Vec<_>, Vec<_>) = staged
            .into_iter()
            .map(|staged| {
                (
                    (staged.item_index, staged.state),
                    crate::ClaimedNewRuleSet {
                        rule_set: staged.rule_set,
                        claimed_names: staged.claimed_names,
                    },
                )
            })
            .unzip();
        let results = match self
            .create_rule_sets_batch_claiming_names(actor, drafts)
            .await
        {
            Ok(results) => results.into_iter().map(Some).collect(),
            Err(error) => {
                for (index, _) in &targets {
                    let entry = &mut progress.items[*index];
                    entry.state = RuleImportItemState::Failed;
                    entry.reason = Some(error.reason().unwrap_or(refusal::ITEM_FAILED).into());
                    entry.message = Some(bounded_message(&error));
                }
                Vec::new()
            }
        };
        for ((index, state), result) in targets.into_iter().zip(results) {
            let entry = &mut progress.items[index];
            match result {
                Some(Ok(crate::ClaimedRuleSetOutcome::Created(rule_set))) => {
                    entry.state = state;
                    entry.rule_set_id = Some(rule_set.id);
                }
                // Another run or an operator created the rule since this run
                // read the names; nothing was written for this item.
                Some(Ok(crate::ClaimedRuleSetOutcome::NameTaken { name })) => {
                    entry.state = RuleImportItemState::SkippedExists;
                    entry.saved_name = Some(name);
                    entry.reason = Some(refusal::ALREADY_EXISTS.into());
                }
                Some(Err(error)) => {
                    entry.state = RuleImportItemState::Failed;
                    entry.reason = Some(error.reason().unwrap_or(refusal::ITEM_FAILED).into());
                    entry.message = Some(bounded_message(&error));
                }
                None => {}
            }
        }
    }

    async fn finish_rule_import(
        &self,
        actor: &User,
        run: &mut JobRunRecord,
        progress: &mut RuleImportProgress,
        status: JobRunStatus,
    ) -> AppResult<()> {
        progress.phase = RuleImportPhase::Finished;
        run.status = status;
        run.completed_at = Some(self.runtime.environment.now());
        run.error_text = (status == JobRunStatus::Failed)
            .then(|| progress.message.clone())
            .flatten();
        progress.counts = RuleImportCounts::of(&progress.items);
        apply_summary(run, progress);
        self.publish_rule_import(run, progress).await?;
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

    async fn publish_rule_import(
        &self,
        run: &mut JobRunRecord,
        progress: &mut RuleImportProgress,
    ) -> AppResult<()> {
        progress.revision = progress.revision.saturating_add(1);
        progress.counts = RuleImportCounts::of(&progress.items);
        run.progress_json = Some(encode(progress)?);
        run.updated_at = self.runtime.environment.now();
        *run = self.services.events.job_runs.update_job_run(run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(run, None))
            .await;
        self.runtime.imports.rule_import.changed.notify_waiters();
        Ok(())
    }
}

/// Marks what a stopped run left unfinished as interrupted.
fn settle_interrupted(progress: &mut RuleImportProgress) {
    for item in &mut progress.items {
        if item.state == RuleImportItemState::Pending {
            item.state = RuleImportItemState::Failed;
            item.reason = Some(refusal::INTERRUPTED.into());
        }
    }
    if progress.phase != RuleImportPhase::Finished && progress.reason.is_none() {
        progress.reason = Some(refusal::INTERRUPTED.into());
    }
    progress.counts = RuleImportCounts::of(&progress.items);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(phase: RuleImportPhase, states: &[RuleImportItemState]) -> RuleImportProgress {
        RuleImportProgress {
            revision: 3,
            source: RuleImportSource::Radarr,
            base_url: "http://radarr.example.test".into(),
            phase,
            reason: None,
            message: None,
            counts: RuleImportCounts::default(),
            items: states
                .iter()
                .enumerate()
                .map(|(index, state)| RuleImportItem {
                    key: format!("customformat:{index}"),
                    source_name: format!("Synthetic format {index}"),
                    saved_name: None,
                    rule_set_id: None,
                    state: *state,
                    reason: None,
                    message: None,
                    facts: Vec::new(),
                })
                .collect(),
            facts: Vec::new(),
        }
    }

    #[test]
    fn report_serializes_with_the_documented_shape() {
        let mut report = progress(
            RuleImportPhase::Finished,
            &[RuleImportItemState::SavedNeedsReview],
        );
        report.items[0].facts.push(
            RuleImportFact::new(refusal::ITEM_FAILED, "synthetic")
                .with_details(serde_json::json!({"line": 1})),
        );
        report.counts = RuleImportCounts::of(&report.items);
        let value = serde_json::to_value(&report).expect("report json");
        assert_eq!(value["source"], "RADARR");
        assert_eq!(value["phase"], "FINISHED");
        assert_eq!(value["items"][0]["state"], "SAVED_NEEDS_REVIEW");
        assert_eq!(value["items"][0]["sourceName"], "Synthetic format 0");
        assert_eq!(value["items"][0]["facts"][0]["details"]["line"], 1);
        assert_eq!(value["counts"]["savedNeedsReview"], 1);
        let back: RuleImportProgress = serde_json::from_value(value).expect("round trip");
        assert_eq!(back, report);
    }

    #[test]
    fn a_stopped_run_reports_unsaved_items_as_interrupted() {
        let mut report = progress(
            RuleImportPhase::Saving,
            &[RuleImportItemState::Saved, RuleImportItemState::Pending],
        );
        settle_interrupted(&mut report);
        assert_eq!(report.items[0].state, RuleImportItemState::Saved);
        assert_eq!(report.items[1].state, RuleImportItemState::Failed);
        assert_eq!(
            report.items[1].reason.as_deref(),
            Some(refusal::INTERRUPTED)
        );
        assert_eq!(report.reason.as_deref(), Some(refusal::INTERRUPTED));
        assert_eq!(report.counts.failed, 1);
        assert_eq!(report.counts.saved, 1);

        let mut finished = progress(RuleImportPhase::Finished, &[RuleImportItemState::Saved]);
        settle_interrupted(&mut finished);
        assert_eq!(finished.reason, None);
    }

    #[test]
    fn every_source_without_a_translator_is_refused_with_a_code() {
        let maintainerr = FetchedMaintainerrRules {
            version: "2.19.0".into(),
            rule_groups: Vec::new(),
            constants: Value::Null,
            usernames: None,
            radarr_profiles: Vec::new(),
            sonarr_profiles: Vec::new(),
        };
        for fetched in [RuleImportFetched::Maintainerr(maintainerr)] {
            assert_eq!(
                plan_rule_import(&fetched, &RuleImportPlanContext::default())
                    .expect_err("no translator yet")
                    .validation_reason(),
                Some(refusal::TRANSLATOR_UNAVAILABLE)
            );
        }
    }

    #[test]
    fn profile_sources_come_from_groups_and_their_collections() {
        let groups = vec![
            serde_json::json!({"id": 1, "collection": {"radarrSettingsId": 4, "sonarrSettingsId": null}}),
            serde_json::json!({"id": 2, "sonarrSettingsId": 7, "collection": {"sonarrSettingsId": 7}}),
            serde_json::json!({"id": 3, "radarrSettingsId": 4}),
        ];
        let (radarr, sonarr) = maintainerr_profile_sources(&groups);
        assert_eq!(radarr.into_iter().collect::<Vec<_>>(), vec![4]);
        assert_eq!(sonarr.into_iter().collect::<Vec<_>>(), vec![7]);
    }

    #[test]
    fn connections_require_an_api_key_only_for_arr_sources() {
        for source in [RuleImportSource::Sonarr, RuleImportSource::Radarr] {
            let connection = RuleImportConnection {
                source,
                base_url: "http://arr.example.test".into(),
                api_key: Some("  ".into()),
            }
            .normalized();
            assert_eq!(
                connection
                    .validate()
                    .expect_err("missing key")
                    .validation_reason(),
                Some(refusal::API_KEY_REQUIRED)
            );
        }
        let maintainerr = RuleImportConnection {
            source: RuleImportSource::Maintainerr,
            base_url: "http://maintainerr.example.test/base/".into(),
            api_key: Some("ignored".into()),
        }
        .normalized();
        maintainerr.validate().expect("no key needed");
        assert_eq!(maintainerr.api_key, None);
        assert_eq!(maintainerr.base_url, "http://maintainerr.example.test/base");
    }
}
