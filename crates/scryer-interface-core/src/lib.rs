use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_graphql::{Context, Error, ErrorExtensions, Result as GqlResult, value};
use scryer_application::{
    AppError, AppUseCase, BackupRestorePreparedBundle, JwtSessionScope, LoginFailureTimingClass,
    OAuthAuthorizationSource, application_upgrade::InstallationAssessment,
};
use scryer_domain::{AppPermission, Id, LibraryPermission, User};
use tokio::sync::{broadcast, watch};

pub mod loaders;

const AUTHENTICATION_REQUIRED_MESSAGE: &str = "authentication required";
const AUTHENTICATION_REQUIRED_CODE: &str = "AUTHENTICATION_REQUIRED";
const INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal server error";
const INTERNAL_ERROR_CODE: &str = "INTERNAL_ERROR";
pub const LOGIN_FAILED_MESSAGE: &str = "Sign-in failed. Check your sign-in details and try again.";

/// Opaque handle to a log snapshot provider and subscription source.
/// The `scryer` crate constructs this from its `LogRingBuffer`.
#[derive(Clone)]
pub struct LogBuffer {
    snapshot_fn: Arc<dyn Fn(usize) -> Vec<String> + Send + Sync>,
    subscribe_fn: Arc<dyn Fn() -> broadcast::Receiver<String> + Send + Sync>,
}

impl LogBuffer {
    pub fn new(
        snapshot: impl Fn(usize) -> Vec<String> + Send + Sync + 'static,
        subscribe: impl Fn() -> broadcast::Receiver<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            snapshot_fn: Arc::new(snapshot),
            subscribe_fn: Arc::new(subscribe),
        }
    }

    pub fn snapshot(&self, limit: usize) -> Vec<String> {
        (self.snapshot_fn)(limit)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        (self.subscribe_fn)()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRuntimeStateSnapshot {
    pub form_login_enabled: bool,
    pub skip_login_for_local_ips: bool,
    pub effective_form_login_enabled: bool,
    pub webauthn_configured: bool,
    pub passkey_enabled: bool,
    pub env_override_active: bool,
    pub env_override_description: Option<String>,
    pub epoch: u64,
}

#[derive(Clone)]
pub struct AuthRuntimeStateHandle {
    snapshot: Arc<RwLock<AuthRuntimeStateSnapshot>>,
    epoch_tx: watch::Sender<u64>,
}

impl AuthRuntimeStateHandle {
    pub fn new(snapshot: AuthRuntimeStateSnapshot) -> Self {
        let (epoch_tx, _) = watch::channel(snapshot.epoch);
        Self {
            snapshot: Arc::new(RwLock::new(snapshot)),
            epoch_tx,
        }
    }

    pub fn snapshot(&self) -> AuthRuntimeStateSnapshot {
        self.snapshot
            .read()
            .expect("auth runtime snapshot lock poisoned")
            .clone()
    }

    pub fn apply_saved_security_settings(
        &self,
        form_login_enabled: bool,
        skip_login_for_local_ips: bool,
    ) -> AuthRuntimeStateSnapshot {
        let next_snapshot = {
            let mut snapshot = self
                .snapshot
                .write()
                .expect("auth runtime snapshot lock poisoned");
            let previous_policy = (
                snapshot.effective_form_login_enabled,
                snapshot.effective_form_login_enabled && snapshot.skip_login_for_local_ips,
                snapshot.passkey_enabled,
            );
            snapshot.form_login_enabled = form_login_enabled;
            snapshot.skip_login_for_local_ips = skip_login_for_local_ips;
            if !snapshot.env_override_active {
                snapshot.effective_form_login_enabled = form_login_enabled;
            }
            snapshot.passkey_enabled =
                snapshot.webauthn_configured && snapshot.effective_form_login_enabled;
            let next_policy = (
                snapshot.effective_form_login_enabled,
                snapshot.effective_form_login_enabled && snapshot.skip_login_for_local_ips,
                snapshot.passkey_enabled,
            );
            if next_policy != previous_policy {
                snapshot.epoch += 1;
            }
            snapshot.clone()
        };

        let _ = self.epoch_tx.send(next_snapshot.epoch);
        next_snapshot
    }

    pub fn subscribe_epoch(&self) -> watch::Receiver<u64> {
        self.epoch_tx.subscribe()
    }

    pub fn invalidate_connections(&self) -> u64 {
        let next_epoch = {
            let mut snapshot = self
                .snapshot
                .write()
                .expect("auth runtime snapshot lock poisoned");
            snapshot.epoch += 1;
            snapshot.epoch
        };
        let _ = self.epoch_tx.send(next_epoch);
        next_epoch
    }
}

#[derive(Clone, Copy)]
pub struct ConnectionAuthEpoch(pub u64);

#[derive(Clone, Default)]
pub struct MfaVerification {
    pub verified_until: Option<i64>,
    pub step_up_verified_until: Option<i64>,
    pub security_action_verified_until: Option<i64>,
    pub session_scope: JwtSessionScope,
    pub persist_session: bool,
    pub auth_session_version: Option<String>,
    pub password_change_required_after_enrollment: bool,
    pub oauth_authorization_source: OAuthAuthorizationSource,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum LoginAttemptPrincipal {
    Local {
        username: String,
    },
    Jellyfin {
        connection_id: String,
        username: String,
    },
    Emby {
        connection_id: String,
        username: String,
    },
}

impl LoginAttemptPrincipal {
    pub fn local(username: &str) -> Option<Self> {
        let username = username.trim();
        (!username.is_empty()).then(|| Self::Local {
            username: username.to_string(),
        })
    }

    pub fn jellyfin(connection_id: &str, username: &str) -> Option<Self> {
        Self::external(connection_id, username, |connection_id, username| {
            Self::Jellyfin {
                connection_id,
                username,
            }
        })
    }

    pub fn emby(connection_id: &str, username: &str) -> Option<Self> {
        Self::external(connection_id, username, |connection_id, username| {
            Self::Emby {
                connection_id,
                username,
            }
        })
    }

    fn external(
        connection_id: &str,
        username: &str,
        build: impl FnOnce(String, String) -> Self,
    ) -> Option<Self> {
        let connection_id = connection_id.trim();
        let username = username.trim();
        (!connection_id.is_empty() && !username.is_empty())
            .then(|| build(connection_id.to_string(), username.to_ascii_lowercase()))
    }
}

type LoginAttemptCheck = dyn Fn(&LoginAttemptPrincipal) -> GqlResult<()> + Send + Sync;
type LoginAttemptOutcome = dyn Fn(&LoginAttemptPrincipal) + Send + Sync;

#[derive(Clone)]
pub struct LoginAttemptLimiter {
    check: Arc<LoginAttemptCheck>,
    record_failure: Arc<LoginAttemptOutcome>,
    clear_success: Arc<LoginAttemptOutcome>,
}

impl LoginAttemptLimiter {
    pub fn new(
        check: impl Fn(&LoginAttemptPrincipal) -> GqlResult<()> + Send + Sync + 'static,
        record_failure: impl Fn(&LoginAttemptPrincipal) + Send + Sync + 'static,
        clear_success: impl Fn(&LoginAttemptPrincipal) + Send + Sync + 'static,
    ) -> Self {
        Self {
            check: Arc::new(check),
            record_failure: Arc::new(record_failure),
            clear_success: Arc::new(clear_success),
        }
    }

    pub fn check(&self, principal: &LoginAttemptPrincipal) -> GqlResult<()> {
        (self.check)(principal)
    }

    pub fn record_failure(&self, principal: &LoginAttemptPrincipal) {
        (self.record_failure)(principal);
    }

    pub fn clear_success(&self, principal: &LoginAttemptPrincipal) {
        (self.clear_success)(principal);
    }
}

#[derive(Clone)]
pub struct ApiContext {
    pub app: AppUseCase,
    pub auth_runtime: AuthRuntimeStateHandle,
    pub restore: Option<RestoreContext>,
    pub application_upgrade_assessment: InstallationAssessment,
}

pub fn login_attempt_limiter_from_ctx<'a>(ctx: &'a Context<'a>) -> Option<&'a LoginAttemptLimiter> {
    ctx.data_opt::<LoginAttemptLimiter>()
}

/// Per-HTTP-request session persistence policy. This is intentionally absent
/// from schema-level contexts, where callers must fail closed.
#[derive(Clone, Copy)]
pub struct RequestSessionPersistence {
    pub default_persist_session: bool,
}

pub fn default_persist_session_from_ctx(ctx: &Context<'_>) -> bool {
    ctx.data_opt::<RequestSessionPersistence>()
        .is_some_and(|policy| policy.default_persist_session)
}

pub fn persist_session_or_default(requested: Option<bool>, default: bool) -> bool {
    requested.unwrap_or(default)
}

#[cfg(test)]
mod request_session_persistence_tests {
    use super::persist_session_or_default;

    #[test]
    fn explicit_preference_overrides_the_request_default() {
        assert!(persist_session_or_default(Some(true), false));
        assert!(!persist_session_or_default(Some(false), true));
    }

    #[test]
    fn omitted_preference_uses_the_request_default() {
        assert!(persist_session_or_default(None, true));
        assert!(!persist_session_or_default(None, false));
    }
}

#[derive(Clone)]
pub struct RestoreRestartHandle {
    schedule_fn: Arc<dyn Fn() + Send + Sync>,
}

pub struct RestoreSqliteDatastoreRequest {
    pub target_db_path: PathBuf,
    pub migration_mode: RestoreMigrationMode,
    pub bundle_path: PathBuf,
    pub passphrase: Option<String>,
}

#[derive(Clone)]
pub struct RestoreDatastoreHandle {
    restore_sqlite_fn: Arc<
        dyn Fn(RestoreSqliteDatastoreRequest) -> Result<BackupRestorePreparedBundle, AppError>
            + Send
            + Sync,
    >,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreDatastoreEngine {
    Sqlite,
    Postgres,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreMigrationMode {
    ValidateOnly,
    Apply,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreDatastoreConfig {
    pub engine: RestoreDatastoreEngine,
    pub migration_mode: RestoreMigrationMode,
}

impl RestoreRestartHandle {
    pub fn new(schedule: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            schedule_fn: Arc::new(schedule),
        }
    }

    pub fn schedule_restart(&self) {
        (self.schedule_fn)();
    }
}

impl RestoreDatastoreHandle {
    pub fn new(
        restore_sqlite: impl Fn(
            RestoreSqliteDatastoreRequest,
        ) -> Result<BackupRestorePreparedBundle, AppError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            restore_sqlite_fn: Arc::new(restore_sqlite),
        }
    }

    pub fn unavailable() -> Self {
        Self::new(|_| {
            Err(AppError::Validation(
                "restore datastore operations are not configured".into(),
            ))
        })
    }

    pub fn restore_sqlite_bundle_to_path(
        &self,
        request: RestoreSqliteDatastoreRequest,
    ) -> Result<BackupRestorePreparedBundle, AppError> {
        (self.restore_sqlite_fn)(request)
    }
}

#[derive(Clone)]
pub struct RestoreContext {
    pub data_dir: PathBuf,
    pub datastore_config: RestoreDatastoreConfig,
    pub datastore: RestoreDatastoreHandle,
    pub restart: RestoreRestartHandle,
}

pub fn app_from_ctx(ctx: &Context<'_>) -> GqlResult<AppUseCase> {
    Ok(ctx.data_unchecked::<ApiContext>().app.clone())
}

pub fn application_upgrade_assessment_from_ctx(ctx: &Context<'_>) -> InstallationAssessment {
    ctx.data_unchecked::<ApiContext>()
        .application_upgrade_assessment
}

pub fn auth_runtime_from_ctx(ctx: &Context<'_>) -> AuthRuntimeStateHandle {
    ctx.data_unchecked::<ApiContext>().auth_runtime.clone()
}

pub fn restore_context_from_ctx(ctx: &Context<'_>) -> GqlResult<RestoreContext> {
    ctx.data_unchecked::<ApiContext>()
        .restore
        .clone()
        .ok_or_else(|| to_gql_error(AppError::Validation("restore is not configured".into())))
}

pub fn to_gql_error(err: AppError) -> Error {
    match err {
        AppError::Unauthorized(message) => {
            coded_gql_error(format!("unauthorized: {message}"), "UNAUTHORIZED")
        }
        AppError::Validation(message) => {
            coded_gql_error(format!("validation: {message}"), "VALIDATION_ERROR")
        }
        // A refused location plan is a validation failure the client acts on:
        // `stale_plan` means re-preview, `blocked_items` means unblock the
        // selection, `insufficient_space` means neither will help and the user
        // has to free room. The reason travels as a code so the client never
        // has to parse the sentence (FR-016, FR-080, FR-081).
        AppError::LocationPlanRefused { message, code } => {
            Error::new(format!("validation: {message}")).extend_with(|_, extensions| {
                extensions.set("code", "LOCATION_PLAN_REFUSED");
                extensions.set("refusalCode", code.as_str());
            })
        }
        // A root-scoped workflow refused the request before planning: the
        // destination is not admissible, the source root cannot be read, or the
        // user asked the wrong half of FR-020's one control. The code is the
        // application's own vocabulary and travels typed, so the client
        // cross-routes between "change root" and "consolidate" without reading
        // a sentence (FR-020 to FR-029).
        AppError::LocationRootRefused { message, code } => {
            Error::new(format!("validation: {message}")).extend_with(|_, extensions| {
                extensions.set("code", "LOCATION_ROOT_REFUSED");
                extensions.set("refusalCode", code);
            })
        }
        // The retired direct root write (FR-077). Its own code, so a client can
        // route the user into the move workflow instead of surfacing a generic
        // validation failure, and the title id so a bulk edit can name the row
        // that refused.
        AppError::DirectRootWriteRetired { message, title_id } => {
            Error::new(format!("validation: {message}")).extend_with(|_, extensions| {
                extensions.set("code", "DIRECT_ROOT_WRITE_RETIRED");
                extensions.set("titleId", title_id);
            })
        }
        AppError::NoAutoEligibleRelease {
            candidate_count,
            reasons,
        } => {
            Error::new("validation: no auto-eligible release found").extend_with(|_, extensions| {
                extensions.set("code", "VALIDATION_ERROR");
                extensions.set("autoCandidateCount", candidate_count as i64);
                extensions.set(
                    "autoDecisionReasons",
                    reasons
                        .into_iter()
                        .map(|reason| {
                            value!({
                                "code": reason.code,
                                "summary": reason.summary,
                                "count": reason.count as i64,
                            })
                        })
                        .collect::<Vec<_>>(),
                );
            })
        }
        AppError::DownloadFeedbackTimeout(message) => {
            coded_gql_error(message, "DOWNLOAD_FEEDBACK_TIMEOUT")
        }
        // Failover exhaustion is a distinct internal kind for diagnostics but
        // the same external contract: the submission is retryable later.
        AppError::DownloadSubmitUnavailable(message)
        | AppError::DownloadSubmitFailoverExhausted(message)
        | AppError::DownloadSourceGone(message) => {
            coded_gql_error(message, "DOWNLOAD_SUBMIT_UNAVAILABLE")
        }
        AppError::ArchiveExtractionPluginRequired {
            message,
            source_path,
        } => Error::new(message).extend_with(|_, extensions| {
            extensions.set("code", "ARCHIVE_EXTRACTION_PLUGIN_REQUIRED");
            if let Some(source_path) = source_path {
                extensions.set("sourcePath", source_path);
            }
        }),
        AppError::ArchiveExtractionTimedOut { message } => {
            coded_gql_error(message, "ARCHIVE_EXTRACTION_TIMED_OUT")
        }
        AppError::TemporaryUnavailable {
            message,
            retry_after,
            ..
        } => Error::new(message).extend_with(|_, extensions| {
            extensions.set("code", "TEMPORARY_UNAVAILABLE");
            if let Some(delay) = retry_after {
                extensions.set("retryAfterSeconds", delay.as_secs());
            }
        }),
        AppError::PluginInstallInProgress(message) => {
            coded_gql_error(message, "PLUGIN_INSTALL_IN_PROGRESS")
        }
        AppError::NotFound(message) => {
            coded_gql_error(format!("not found: {message}"), "NOT_FOUND")
        }
        AppError::DownloadSubmitAmbiguous(message) => {
            coded_gql_error(message, "DOWNLOAD_SUBMIT_AMBIGUOUS")
        }
        AppError::DownloadSubmitAmbiguousWithClient { message, .. } => {
            coded_gql_error(message, "DOWNLOAD_SUBMIT_AMBIGUOUS")
        }
        AppError::DownloadSubmitRejected(message) => {
            coded_gql_error(message, "DOWNLOAD_SUBMIT_REJECTED")
        }
        AppError::MfaStepUpRequired(message) => coded_gql_error(message, "MFA_STEP_UP_REQUIRED"),
        AppError::ReauthenticationRequired(message) => {
            coded_gql_error(message, "REAUTHENTICATION_REQUIRED")
        }
        AppError::TotpEnrollmentRequired(message) => {
            coded_gql_error(message, "TOTP_ENROLLMENT_REQUIRED")
        }
        AppError::MfaEnrollmentRequired(message) => {
            coded_gql_error(message, "MFA_ENROLLMENT_REQUIRED")
        }
        AppError::PasswordChangeRequired(message) => {
            coded_gql_error(message, "PASSWORD_CHANGE_REQUIRED")
        }
        AppError::TotpInvalidCode(message) => coded_gql_error(message, "TOTP_INVALID_CODE"),
        AppError::TotpRecoveryCodeUsed(message) => {
            coded_gql_error(message, "TOTP_RECOVERY_CODE_USED")
        }
        AppError::Canceled(message) => coded_gql_error(message, "CANCELED"),
        AppError::ManualReconciliationRequired(message) => {
            coded_gql_error(message, "MANUAL_RECONCILIATION_REQUIRED")
        }
        AppError::ImportEvidenceUnavailable(message) => repository_gql_error(message),
        error @ AppError::ImportSourceInspection { .. }
        | error @ AppError::UnsupportedImportSource { .. }
        | error @ AppError::ImportSourceChanged { .. } => repository_gql_error(error.to_string()),
        AppError::Repository(message) => repository_gql_error(message),
    }
}

fn coded_gql_error(message: impl Into<String>, code: &'static str) -> Error {
    Error::new(message).extend_with(|_, extensions| {
        extensions.set("code", code);
    })
}

fn repository_gql_error(message: String) -> Error {
    let error_id = Id::new().0;
    tracing::error!(
        error_id = %error_id,
        error_kind = "Repository",
        error = %message,
        "masked internal repository error"
    );
    Error::new(INTERNAL_SERVER_ERROR_MESSAGE).extend_with(|_, extensions| {
        extensions.set("code", INTERNAL_ERROR_CODE);
        extensions.set("errorId", error_id);
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginErrorClassification {
    Progression,
    InfrastructureFailure,
    MaskedPrimaryFailure,
}

pub fn classify_login_error(err: &AppError) -> LoginErrorClassification {
    if login_progression_error(err) {
        LoginErrorClassification::Progression
    } else if matches!(err, AppError::Repository(_)) {
        LoginErrorClassification::InfrastructureFailure
    } else {
        LoginErrorClassification::MaskedPrimaryFailure
    }
}

fn login_progression_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::MfaStepUpRequired(_)
            | AppError::TotpEnrollmentRequired(_)
            | AppError::MfaEnrollmentRequired(_)
            | AppError::PasswordChangeRequired(_)
            | AppError::TotpInvalidCode(_)
            | AppError::TotpRecoveryCodeUsed(_)
    )
}

fn app_error_kind(err: &AppError) -> &'static str {
    match err {
        AppError::Unauthorized(_) => "Unauthorized",
        AppError::Validation(_) => "Validation",
        AppError::LocationPlanRefused { .. } => "LocationPlanRefused",
        AppError::LocationRootRefused { .. } => "LocationRootRefused",
        AppError::DirectRootWriteRetired { .. } => "DirectRootWriteRetired",
        AppError::NoAutoEligibleRelease { .. } => "NoAutoEligibleRelease",
        AppError::PluginInstallInProgress(_) => "PluginInstallInProgress",
        AppError::NotFound(_) => "NotFound",
        AppError::DownloadFeedbackTimeout(_) => "DownloadFeedbackTimeout",
        AppError::DownloadSubmitAmbiguous(_) => "DownloadSubmitAmbiguous",
        AppError::DownloadSubmitAmbiguousWithClient { .. } => "DownloadSubmitAmbiguous",
        AppError::DownloadSubmitRejected(_) => "DownloadSubmitRejected",
        AppError::DownloadSubmitUnavailable(_) => "DownloadSubmitUnavailable",
        AppError::DownloadSourceGone(_) => "DownloadSourceGone",
        AppError::DownloadSubmitFailoverExhausted(_) => "DownloadSubmitFailoverExhausted",
        AppError::ArchiveExtractionPluginRequired { .. } => "ArchiveExtractionPluginRequired",
        AppError::ArchiveExtractionTimedOut { .. } => "ArchiveExtractionTimedOut",
        AppError::TemporaryUnavailable { .. } => "TemporaryUnavailable",
        AppError::MfaStepUpRequired(_) => "MfaStepUpRequired",
        AppError::ReauthenticationRequired(_) => "ReauthenticationRequired",
        AppError::TotpEnrollmentRequired(_) => "TotpEnrollmentRequired",
        AppError::MfaEnrollmentRequired(_) => "MfaEnrollmentRequired",
        AppError::PasswordChangeRequired(_) => "PasswordChangeRequired",
        AppError::TotpInvalidCode(_) => "TotpInvalidCode",
        AppError::TotpRecoveryCodeUsed(_) => "TotpRecoveryCodeUsed",
        AppError::Canceled(_) => "Canceled",
        AppError::ManualReconciliationRequired(_) => "ManualReconciliationRequired",
        AppError::ImportEvidenceUnavailable(_) => "ImportEvidenceUnavailable",
        AppError::ImportSourceInspection { .. } => "ImportSourceInspection",
        AppError::UnsupportedImportSource { .. } => "UnsupportedImportSource",
        AppError::ImportSourceChanged { .. } => "ImportSourceChanged",
        AppError::Repository(_) => "Repository",
    }
}

pub fn to_login_gql_error(method: &'static str, err: AppError) -> Error {
    let error_kind = app_error_kind(&err);
    match classify_login_error(&err) {
        LoginErrorClassification::Progression => to_gql_error(err),
        LoginErrorClassification::InfrastructureFailure => {
            let AppError::Repository(message) = err else {
                return to_gql_error(err);
            };
            repository_gql_error(message)
        }
        LoginErrorClassification::MaskedPrimaryFailure => {
            tracing::debug!(login_method = method, error_kind, "masked login failure");
            Error::new(LOGIN_FAILED_MESSAGE).extend_with(|_, extensions| {
                extensions.set("code", "LOGIN_FAILED");
            })
        }
    }
}

pub fn login_verification_required_gql_error(
    challenge_id: &str,
    expires_at: &str,
    has_passkey: bool,
    has_totp: bool,
) -> Error {
    Error::new("Additional verification is required.").extend_with(|_, extensions| {
        extensions.set("code", "MFA_STEP_UP_REQUIRED");
        extensions.set("loginChallengeId", challenge_id);
        extensions.set("expiresAt", expires_at);
        extensions.set("hasPasskey", has_passkey);
        extensions.set("hasTotp", has_totp);
        extensions.set(
            "preferredFactor",
            if has_passkey { "PASSKEY" } else { "TOTP" },
        );
    })
}

pub async fn to_login_gql_error_after_timing(
    method: &'static str,
    timing_class: LoginFailureTimingClass,
    started_at: Instant,
    err: AppError,
) -> Error {
    if classify_login_error(&err) == LoginErrorClassification::Progression {
        return to_gql_error(err);
    }

    AppUseCase::apply_login_failure_timing(timing_class, started_at).await;
    to_login_gql_error(method, err)
}

pub fn actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    match mfa_verification_from_ctx(ctx).session_scope {
        JwtSessionScope::MfaEnrollment => {
            return Err(to_gql_error(AppError::MfaEnrollmentRequired(
                "MFA enrollment must be completed before accessing Scryer".into(),
            )));
        }
        JwtSessionScope::PasswordChangeRequired => {
            return Err(to_gql_error(AppError::PasswordChangeRequired(
                "password replacement must be completed before accessing Scryer".into(),
            )));
        }
        JwtSessionScope::Full => {}
    }
    current_user_any_scope_from_ctx(ctx).ok_or_else(authentication_required_error)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthActorSession {
    pub client_id: String,
    pub grant_id: String,
}

/// Marker added only for browser or native interactive sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractiveSession;

/// Marker for the intentional unauthenticated default actor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthlessDefaultSession;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApiKeyManagementSession;

/// Returns the actor only when the request may manage its own API keys.
pub fn api_key_management_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if ctx.data_opt::<ApiKeyManagementSession>().is_none() {
        return Err(to_gql_error(AppError::Unauthorized(
            "an interactive session is required for this operation".into(),
        )));
    }
    actor_from_ctx(ctx)
}

/// Returns the actor only when the request was authenticated by an interactive session.
pub fn interactive_session_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if ctx.data_opt::<InteractiveSession>().is_none() {
        return Err(to_gql_error(AppError::Unauthorized(
            "an interactive session is required for this operation".into(),
        )));
    }
    actor_from_ctx(ctx)
}

/// Returns the actor allowed to manage TOTP factors.
///
/// The intentionally configured authless default actor retains this legacy
/// capability; API-key and OAuth actors do not receive either marker.
pub fn totp_management_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if ctx.data_opt::<InteractiveSession>().is_none()
        && ctx.data_opt::<AuthlessDefaultSession>().is_none()
    {
        return Err(to_gql_error(AppError::Unauthorized(
            "an interactive session is required for this operation".into(),
        )));
    }
    actor_from_ctx(ctx)
}

/// Starts TOTP enrollment only with an account-security grant, except for the
/// intentional authless default actor's established management behavior.
pub fn totp_enrollment_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if ctx.data_opt::<AuthlessDefaultSession>().is_some() {
        return actor_from_ctx(ctx);
    }
    account_security_actor_from_ctx(ctx)
}

/// Returns the interactive actor only while its account-security freshness grant is valid.
pub fn account_security_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    let actor = actor_from_ctx(ctx)?;
    if ctx.data_opt::<InteractiveSession>().is_none() {
        return Err(to_gql_error(AppError::Unauthorized(
            "interactive session authentication is required".into(),
        )));
    }
    if mfa_verification_from_ctx(ctx)
        .security_action_verified_until
        .is_some_and(|expires_at| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|now| expires_at > now.as_secs() as i64)
                .unwrap_or(false)
        })
    {
        return Ok(actor);
    }

    Err(to_gql_error(AppError::ReauthenticationRequired(
        "reauthentication is required before changing authentication factors".into(),
    )))
}

pub fn oauth_actor_session_from_ctx(ctx: &Context<'_>) -> Option<OAuthActorSession> {
    ctx.data_opt::<OAuthActorSession>().cloned()
}

pub fn mfa_enrollment_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if mfa_verification_from_ctx(ctx).session_scope != JwtSessionScope::MfaEnrollment {
        return Err(to_gql_error(AppError::MfaEnrollmentRequired(
            "MFA enrollment session required".into(),
        )));
    }
    current_user_any_scope_from_ctx(ctx).ok_or_else(authentication_required_error)
}

pub fn password_change_required_actor_from_ctx(ctx: &Context<'_>) -> GqlResult<User> {
    if mfa_verification_from_ctx(ctx).session_scope != JwtSessionScope::PasswordChangeRequired {
        return Err(to_gql_error(AppError::PasswordChangeRequired(
            "password-replacement session required".into(),
        )));
    }
    current_user_any_scope_from_ctx(ctx).ok_or_else(authentication_required_error)
}

fn authentication_required_error() -> Error {
    Error::new(AUTHENTICATION_REQUIRED_MESSAGE).extend_with(|_, extensions| {
        extensions.set("code", AUTHENTICATION_REQUIRED_CODE);
    })
}

pub async fn require_app_permission(
    ctx: &Context<'_>,
    permission: AppPermission,
) -> GqlResult<User> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.require_app_permission(&actor, permission)
        .await
        .map_err(to_gql_error)?;
    Ok(actor)
}

pub async fn require_config_app_permission(
    ctx: &Context<'_>,
    permission: AppPermission,
) -> GqlResult<User> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.require_app_permission(&actor, permission)
        .await
        .map_err(to_gql_error)?;
    if !auth_runtime_from_ctx(ctx)
        .snapshot()
        .effective_form_login_enabled
    {
        return Ok(actor);
    }
    let mfa = mfa_verification_from_ctx(ctx);
    app.require_mfa_step_up(&actor, mfa.step_up_verified_until)
        .await
        .map_err(to_gql_error)?;
    Ok(actor)
}

pub async fn actor_has_app_permission(
    ctx: &Context<'_>,
    permission: AppPermission,
) -> GqlResult<bool> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.has_app_permission(&actor, permission)
        .await
        .map_err(to_gql_error)
}

pub async fn actor_has_any_library_permission(
    ctx: &Context<'_>,
    permission: LibraryPermission,
) -> GqlResult<bool> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.has_any_library_permission(&actor, permission)
        .await
        .map_err(to_gql_error)
}

pub fn current_user_from_ctx(ctx: &Context<'_>) -> Option<User> {
    if mfa_verification_from_ctx(ctx).session_scope != JwtSessionScope::Full {
        return None;
    }
    current_user_any_scope_from_ctx(ctx)
}

fn current_user_any_scope_from_ctx(ctx: &Context<'_>) -> Option<User> {
    if let Some(connection_epoch) = ctx.data_opt::<ConnectionAuthEpoch>()
        && connection_epoch.0 != auth_runtime_from_ctx(ctx).snapshot().epoch
    {
        return None;
    }

    ctx.data_opt::<User>().cloned()
}

pub fn mfa_verification_from_ctx(ctx: &Context<'_>) -> MfaVerification {
    ctx.data_opt::<MfaVerification>()
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graphql_error_extension_string<'a>(error: &'a Error, key: &str) -> Option<&'a str> {
        error
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get(key))
            .and_then(|value| match value {
                async_graphql::Value::String(value) => Some(value.as_str()),
                _ => None,
            })
    }

    fn graphql_error_code(error: &Error) -> Option<&str> {
        graphql_error_extension_string(error, "code")
    }

    #[test]
    fn login_errors_mask_disclosure_details() {
        for err in [
            AppError::Unauthorized("external account is not invited".into()),
            AppError::NotFound("user 00000000-0000-0000-0000-000000000001".into()),
            AppError::Validation("passkeys require a password-backed account".into()),
        ] {
            let error = to_login_gql_error("jellyfin", err);
            assert_eq!(error.message, LOGIN_FAILED_MESSAGE);
            assert_eq!(graphql_error_code(&error), Some("LOGIN_FAILED"));
        }
    }

    #[test]
    fn a_root_scoped_refusal_promotes_its_code_and_carries_no_machine_tail() {
        let error = to_gql_error(AppError::LocationRootRefused {
            message: "that path is already the root “/media/new” of this library; consolidate \
                      instead"
                .into(),
            code: "root_change_destination_is_configured_root",
        });
        assert_eq!(
            error.message,
            "validation: that path is already the root “/media/new” of this library; consolidate instead"
        );
        assert!(
            !error.message.contains('['),
            "the code travels in extensions, never as a bracketed tail on the sentence"
        );
        assert_eq!(graphql_error_code(&error), Some("LOCATION_ROOT_REFUSED"));
        assert_eq!(
            graphql_error_extension_string(&error, "refusalCode"),
            Some("root_change_destination_is_configured_root")
        );
    }

    #[test]
    fn a_plain_validation_failure_from_a_root_workflow_stays_a_validation_error() {
        // Nothing parses prose any more: a sentence that merely *looks* like a
        // coded refusal is still an ordinary validation failure.
        for message in [
            "choose a new path for this root",
            "something went wrong [root_change_paths_overlap]",
        ] {
            let error = to_gql_error(AppError::Validation(message.into()));
            assert_eq!(error.message, format!("validation: {message}"));
            assert_eq!(graphql_error_code(&error), Some("VALIDATION_ERROR"));
            assert_eq!(
                graphql_error_extension_string(&error, "refusalCode"),
                None,
                "a plain validation failure carries no refusal code"
            );
        }
    }

    #[test]
    fn login_error_classification_matches_masking_behavior() {
        assert_eq!(
            classify_login_error(&AppError::Unauthorized("invalid credentials".into())),
            LoginErrorClassification::MaskedPrimaryFailure
        );
        assert_eq!(
            classify_login_error(&AppError::Repository("connection unavailable".into())),
            LoginErrorClassification::InfrastructureFailure
        );
        assert_eq!(
            classify_login_error(&AppError::MfaStepUpRequired("MFA required".into())),
            LoginErrorClassification::Progression
        );
    }

    #[test]
    fn login_errors_preserve_mfa_progression() {
        let error = to_login_gql_error(
            "local",
            AppError::MfaStepUpRequired("MFA code is required for password login".into()),
        );
        assert_eq!(error.message, "MFA code is required for password login");
        assert_eq!(graphql_error_code(&error), Some("MFA_STEP_UP_REQUIRED"));
    }

    #[test]
    fn app_errors_have_stable_graphql_codes() {
        for (err, expected_message, expected_code) in [
            (
                AppError::Unauthorized("settings access is restricted".into()),
                "unauthorized: settings access is restricted",
                "UNAUTHORIZED",
            ),
            (
                AppError::Validation("invalid title id".into()),
                "validation: invalid title id",
                "VALIDATION_ERROR",
            ),
            (
                AppError::PluginInstallInProgress("plugin-a".into()),
                "plugin-a",
                "PLUGIN_INSTALL_IN_PROGRESS",
            ),
            (
                AppError::NotFound("title title-1".into()),
                "not found: title title-1",
                "NOT_FOUND",
            ),
            (
                AppError::DownloadFeedbackTimeout("download feedback timed out".into()),
                "download feedback timed out",
                "DOWNLOAD_FEEDBACK_TIMEOUT",
            ),
            (
                AppError::DownloadSubmitAmbiguous("download submission is ambiguous".into()),
                "download submission is ambiguous",
                "DOWNLOAD_SUBMIT_AMBIGUOUS",
            ),
            (
                AppError::DownloadSubmitRejected("sabnzbd rejected the nzb: Duplicate NZB".into()),
                "sabnzbd rejected the nzb: Duplicate NZB",
                "DOWNLOAD_SUBMIT_REJECTED",
            ),
            (
                AppError::DownloadSubmitUnavailable("download submitter unavailable".into()),
                "download submitter unavailable",
                "DOWNLOAD_SUBMIT_UNAVAILABLE",
            ),
            // A distinct internal kind, the same external contract.
            (
                AppError::download_submit_failover_exhausted(
                    "all prioritized download clients failed to enqueue this release",
                ),
                "all prioritized download clients failed to enqueue this release",
                "DOWNLOAD_SUBMIT_UNAVAILABLE",
            ),
            (
                AppError::temporary_unavailable("provider is deferred", None),
                "provider is deferred",
                "TEMPORARY_UNAVAILABLE",
            ),
            (
                AppError::MfaStepUpRequired("MFA code is required".into()),
                "MFA code is required",
                "MFA_STEP_UP_REQUIRED",
            ),
            (
                AppError::TotpEnrollmentRequired("TOTP enrollment is required".into()),
                "TOTP enrollment is required",
                "TOTP_ENROLLMENT_REQUIRED",
            ),
            (
                AppError::MfaEnrollmentRequired("MFA enrollment is required".into()),
                "MFA enrollment is required",
                "MFA_ENROLLMENT_REQUIRED",
            ),
            (
                AppError::TotpInvalidCode("invalid MFA code".into()),
                "invalid MFA code",
                "TOTP_INVALID_CODE",
            ),
            (
                AppError::TotpRecoveryCodeUsed("recovery code used".into()),
                "recovery code used",
                "TOTP_RECOVERY_CODE_USED",
            ),
        ] {
            let error = to_gql_error(err);
            assert_eq!(error.message, expected_message);
            assert_eq!(graphql_error_code(&error), Some(expected_code));
        }
    }

    #[test]
    fn temporary_unavailable_graphql_error_preserves_retry_after() {
        let error = to_gql_error(AppError::temporary_unavailable(
            "subtitle provider is temporarily deferred",
            Some(std::time::Duration::from_secs(120)),
        ));

        assert_eq!(error.message, "subtitle provider is temporarily deferred");
        assert_eq!(graphql_error_code(&error), Some("TEMPORARY_UNAVAILABLE"));
        let retry_after = error
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("retryAfterSeconds"))
            .expect("retryAfterSeconds extension should be present");
        assert_eq!(retry_after.to_string(), "120");
    }

    #[test]
    fn no_auto_eligible_release_graphql_error_includes_reason_counts() {
        let error = to_gql_error(AppError::NoAutoEligibleRelease {
            candidate_count: 3,
            reasons: vec![scryer_application::AutoEligibilityReason {
                code: "title_mismatch".to_string(),
                summary: "release title does not match the target title".to_string(),
                count: 2,
            }],
        });

        assert_eq!(error.message, "validation: no auto-eligible release found");
        assert_eq!(graphql_error_code(&error), Some("VALIDATION_ERROR"));
        let extensions = error.extensions.as_ref().expect("extensions are present");
        assert_eq!(
            extensions
                .get("autoCandidateCount")
                .expect("candidate count extension is present")
                .to_string(),
            "3"
        );
        assert_eq!(
            extensions
                .get("autoDecisionReasons")
                .expect("reason counts extension is present")
                .to_string(),
            "[{code: \"title_mismatch\", summary: \"release title does not match the target title\", count: 2}]"
        );
    }

    #[test]
    fn repository_errors_are_masked_with_internal_error_code() {
        let error = to_gql_error(AppError::Repository(
            "metadata gateway request failed (502): <html>bad gateway</html>".into(),
        ));

        assert_eq!(error.message, INTERNAL_SERVER_ERROR_MESSAGE);
        assert_eq!(graphql_error_code(&error), Some(INTERNAL_ERROR_CODE));
        assert!(
            graphql_error_extension_string(&error, "errorId")
                .is_some_and(|value| !value.is_empty())
        );
        assert!(!error.message.contains("metadata gateway"));
        assert!(!error.message.contains("<html>"));
    }

    #[test]
    fn login_repository_errors_use_internal_error_masking() {
        let error = to_login_gql_error(
            "local",
            AppError::Repository("login datastore unavailable: sqlite:///secret.db".into()),
        );

        assert_eq!(error.message, INTERNAL_SERVER_ERROR_MESSAGE);
        assert_eq!(graphql_error_code(&error), Some(INTERNAL_ERROR_CODE));
        assert!(
            graphql_error_extension_string(&error, "errorId")
                .is_some_and(|value| !value.is_empty())
        );
        assert!(!error.message.contains("sqlite"));
        assert!(!error.message.contains("secret.db"));
    }

    #[test]
    fn authentication_required_errors_are_coded() {
        let error = authentication_required_error();

        assert_eq!(error.message, AUTHENTICATION_REQUIRED_MESSAGE);
        assert_eq!(
            graphql_error_code(&error),
            Some(AUTHENTICATION_REQUIRED_CODE)
        );
    }
}
