use async_graphql::Schema;
use scryer_application::{AppUseCase, application_upgrade::InstallationAssessment};

use crate::{mutation::MutationRoot, query::QueryRoot, subscription::SubscriptionRoot};

pub use scryer_interface_core::{
    ApiContext, ApiKeyManagementSession, AuthRuntimeStateHandle, AuthRuntimeStateSnapshot,
    AuthlessDefaultSession, ConnectionAuthEpoch, InteractiveSession, LogBuffer,
    LoginAttemptLimiter, LoginAttemptPrincipal, MfaVerification, OAuthActorSession,
    RequestSessionPersistence, RestoreContext, RestoreDatastoreConfig, RestoreDatastoreEngine,
    RestoreDatastoreHandle, RestoreMigrationMode, RestoreRestartHandle,
    RestoreSqliteDatastoreRequest, actor_from_ctx, actor_has_any_library_permission,
    actor_has_app_permission, app_from_ctx, auth_runtime_from_ctx, current_user_from_ctx,
    login_attempt_limiter_from_ctx, mfa_verification_from_ctx, oauth_actor_session_from_ctx,
    require_app_permission, require_config_app_permission, restore_context_from_ctx, to_gql_error,
    to_login_gql_error, to_login_gql_error_after_timing,
};

pub type ApiSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;
/// Recursion-depth ceiling for executable documents, shared by the schema's
/// global limit and the transport-level authentication classifier so the two
/// can never disagree. First-party clients nest at most 5 levels deep, so 32
/// leaves ample headroom for API consumers.
pub const GRAPHQL_RECURSIVE_DEPTH_LIMIT: usize = 32;

pub fn export_schema_sdl() -> String {
    Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        SubscriptionRoot,
    )
    .limit_recursive_depth(GRAPHQL_RECURSIVE_DEPTH_LIMIT)
    .finish()
    .sdl()
}

pub fn build_schema(app: AppUseCase, auth_runtime: AuthRuntimeStateHandle) -> ApiSchema {
    build_schema_with_log_buffer_and_restore(app, auth_runtime, None, None)
}

pub fn build_schema_with_log_buffer(
    app: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    log_buffer: Option<LogBuffer>,
) -> ApiSchema {
    build_schema_with_log_buffer_and_restore(app, auth_runtime, log_buffer, None)
}

pub fn build_schema_with_log_buffer_and_restore(
    app: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    log_buffer: Option<LogBuffer>,
    restore: Option<RestoreContext>,
) -> ApiSchema {
    build_schema_with_log_buffer_and_restore_and_application_upgrade(
        app,
        auth_runtime,
        log_buffer,
        restore,
        InstallationAssessment::default(),
    )
}

pub fn build_schema_with_log_buffer_and_restore_and_application_upgrade(
    app: AppUseCase,
    auth_runtime: AuthRuntimeStateHandle,
    log_buffer: Option<LogBuffer>,
    restore: Option<RestoreContext>,
    application_upgrade_assessment: InstallationAssessment,
) -> ApiSchema {
    let mut builder = Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        SubscriptionRoot,
    )
    .limit_recursive_depth(GRAPHQL_RECURSIVE_DEPTH_LIMIT)
    .data(ApiContext {
        app,
        auth_runtime,
        restore,
        application_upgrade_assessment,
    });
    if let Some(buf) = log_buffer {
        builder = builder.data(buf);
    }
    builder.finish()
}
