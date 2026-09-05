#![recursion_limit = "256"]

mod common;

#[path = "integration_graphql/account_auth_users.rs"]
mod account_auth_users;
#[path = "integration_graphql/activity_history.rs"]
mod activity_history;
#[path = "integration_graphql/auth_runtime_passkeys.rs"]
mod auth_runtime_passkeys;
#[path = "integration_graphql/backups.rs"]
mod backups;
#[path = "integration_graphql/dashboard.rs"]
mod dashboard;
#[path = "integration_graphql/dataloader_enrichment.rs"]
mod dataloader_enrichment;
#[path = "integration_graphql/downloads_housekeeping_system.rs"]
mod downloads_housekeeping_system;
#[path = "integration_graphql/emby_contract.rs"]
mod emby_contract;
#[path = "integration_graphql/external_import_secret_drafts.rs"]
mod external_import_secret_drafts;
#[path = "integration_graphql/folder_match.rs"]
mod folder_match;
#[path = "integration_graphql/library_scan.rs"]
mod library_scan;
#[path = "integration_graphql/location_operations.rs"]
mod location_operations;
#[path = "integration_graphql/media_rename.rs"]
mod media_rename;
#[path = "integration_graphql/metadata_search.rs"]
mod metadata_search;
#[path = "integration_graphql/misc_smoke.rs"]
mod misc_smoke;
#[path = "integration_graphql/quality_routing_settings.rs"]
mod quality_routing_settings;
#[path = "integration_graphql/schema_contract.rs"]
mod schema_contract;
#[path = "integration_graphql/schema_core_queue_import.rs"]
mod schema_core_queue_import;
#[path = "integration_graphql/security_settings.rs"]
mod security_settings;
#[path = "integration_graphql/title_catalog.rs"]
mod title_catalog;
#[path = "integration_graphql/title_credits.rs"]
mod title_credits;
#[path = "integration_graphql/title_image_cache.rs"]
mod title_image_cache;
#[path = "integration_graphql/title_match.rs"]
mod title_match;
#[path = "integration_graphql/typed_settings.rs"]
mod typed_settings;
#[path = "integration_graphql/ui_settings.rs"]
mod ui_settings;

use async_trait::async_trait;
use aws_lc_rs::hmac;
use chrono::{Duration, Utc};
use scryer_application::testing::AppUseCaseTestExt;
use scryer_application::{
    AcquisitionScopeState, AcquisitionScopeStateRepository, AppError, AppResult, BackupInfo,
    BackupStatus, BackupTrigger, BlocklistRepository, CollectionEpisodeProgressSummary,
    CutoffUnmetQualitySummary, DownloadSubmissionRepository, EpisodeScopedMediaFile, EpisodeUpdate,
    InsertMediaFileInput, JwtSessionScope, LibraryRepository, LibraryRootDraft, MediaFileAnalysis,
    MediaFileRepository, MediaFileRole, MediaServerConnectionRepository, PendingRelease,
    PendingReleaseRepository, ReleaseDecision, SettingsRepository, ShowRepository,
    TitleEpisodeProgressSummary, TitleMediaFile, TitleMediaSizeSummary, TitleMovieMediaSummary,
    TitleQualitySummary, TitleRepository, TotpEnrollmentChallengeRecord, TotpFailedAttemptRecord,
    TotpRepository, UserRepository, WebauthnCredentialRecord, WebauthnRepository,
    start_background_download_delete_poller,
};
use scryer_domain::{
    AppPermissionMask, Collection, CollectionType, DomainEventActorKind, DomainEventPayload,
    DomainEventStream, DomainExternalIds, DownloadFailedEventData, Episode, EpisodeType,
    ExternalId, Id, ImportCompletedEventData, Library, LibraryPermission, LibraryPermissionMask,
    MediaFacet, MediaFileAnalyzedEventData, MediaPathUpdate, MediaServerConnection,
    MediaServerProvider, MediaUpdateType, NewDomainEvent, ReleaseBlocklistedEventData, Title,
    TitleContextSnapshot, User, UserAuthorization,
};
use scryer_infrastructure_identity::users::{totp_store::TotpStore, webauthn_store::WebauthnStore};
use scryer_infrastructure_library::media::{
    libraries::renamer::FileSystemLibraryRenamer, search::media_file_store::MediaFileStore,
    servers::MediaServerConnectionStore,
};
use scryer_infrastructure_sql::types::SettingDefinitionSeed;
use scryer_infrastructure_workflow::workflow::stores::DownloadSubmissionStore;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::{BTreeMap, HashMap};
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use common::{TestContext, load_fixture};

const TEST_BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn test_base32_decode_no_pad(input: &str) -> Vec<u8> {
    let mut buffer = 0_u32;
    let mut bits = 0_u8;
    let mut decoded = Vec::new();

    for ch in input
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && *ch != '=')
    {
        let upper = ch.to_ascii_uppercase() as u8;
        let value = TEST_BASE32_ALPHABET
            .iter()
            .position(|candidate| *candidate == upper)
            .expect("valid test base32 secret") as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            decoded.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    decoded
}

fn test_totp_code_for_step_offset(secret_base32: &str, step_offset: i64) -> String {
    let secret = test_base32_decode_no_pad(secret_base32);
    let step = Utc::now().timestamp() / 30 + step_offset;
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &secret);
    let tag = hmac::sign(&key, &(step as u64).to_be_bytes());
    let digest = tag.as_ref();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let value = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);

    format!("{:06}", value % 1_000_000)
}

fn test_totp_code(secret_base32: &str) -> String {
    test_totp_code_for_step_offset(secret_base32, 0)
}

/// Execute a GraphQL operation directly against the schema, without going
/// through the HTTP test server.  This gives full control over what data
/// (e.g. `User`) is attached to the request.
async fn schema_exec(ctx: &TestContext, query: &str, user: Option<scryer_domain::User>) -> Value {
    let mut req = async_graphql::Request::new(query);
    if let Some(u) = user {
        req = req.data(u);
    }
    let resp = ctx.schema.execute(req).await;
    serde_json::to_value(&resp).expect("serialize gql response")
}

fn schema_sdl(ctx: &TestContext) -> String {
    ctx.schema.sdl()
}

/// Helper to execute a GraphQL query and return the parsed JSON body.
async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    ctx.graphql_json(query, variables, None).await
}

async fn gql_with_token(ctx: &TestContext, query: &str, variables: Value, token: &str) -> Value {
    ctx.graphql_json(query, variables, Some(token)).await
}

/// Assert no GraphQL errors in response body.
fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

fn first_graphql_error_message_and_code(body: &Value) -> (String, String) {
    let errors = body["errors"].as_array().expect("graphql errors");
    let first = errors.first().expect("first graphql error");
    let message = first["message"]
        .as_str()
        .expect("graphql error message")
        .to_string();
    let code = first["extensions"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("graphql error code missing: {body}"))
        .to_string();
    (message, code)
}

fn assert_mfa_step_up_required(body: &Value) {
    let (_message, code) = first_graphql_error_message_and_code(body);
    assert_eq!(
        code, "MFA_STEP_UP_REQUIRED",
        "expected MFA_STEP_UP_REQUIRED GraphQL error: {body}"
    );
}

fn assert_graphql_field_denied(body: &Value, field_key: &str) {
    let errors = body["errors"].as_array().expect("expected GraphQL errors");
    assert!(
        !errors.is_empty(),
        "expected GraphQL field {field_key} to be denied: {body}"
    );
    assert!(
        body["data"].is_null() || body["data"][field_key].is_null(),
        "denied GraphQL field {field_key} should not return data: {body}"
    );
}

fn manage_users_actor(username: &str) -> User {
    User {
        id: Id::new().0,
        username: username.to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::from_permissions([scryer_domain::AppPermission::ManageUsers]),
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

async fn enroll_totp_for_test_credentials(ctx: &TestContext, user: &User) -> (String, String) {
    let enrollment = ctx
        .app
        .totp_enrollment_start(user)
        .await
        .expect("start TOTP enrollment");
    let code = test_totp_code(&enrollment.secret_base32);
    let completed = ctx
        .app
        .totp_enrollment_complete(user, &enrollment.challenge_id, &code)
        .await
        .expect("complete TOTP enrollment");
    let recovery_code = completed
        .recovery_codes
        .into_iter()
        .next()
        .expect("TOTP enrollment provides recovery codes");
    (
        test_totp_code_for_step_offset(&enrollment.secret_base32, 1),
        recovery_code,
    )
}

async fn enroll_totp_for_test(ctx: &TestContext, user: &User) {
    let _ = enroll_totp_for_test_credentials(ctx, user).await;
}

async fn enable_form_login_with_config_step_up(
    ctx: &TestContext,
    username: &str,
    password: &str,
) -> (User, String, String) {
    seed_typed_settings_definitions(ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, password.to_string())
        .await
        .expect("set initial default admin password");
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "auth.form_login_enabled",
            None,
            "true",
            "test",
            None,
        )
        .await
        .unwrap();
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "auth.skip_login_for_local_ips",
            None,
            "false",
            "test",
            None,
        )
        .await
        .unwrap();
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "auth.mfa.require_config_step_up",
            None,
            "true",
            "test",
            None,
        )
        .await
        .unwrap();
    ctx.auth_runtime.apply_saved_security_settings(true, false);
    let (totp_code, recovery_code) = enroll_totp_for_test_credentials(ctx, &admin).await;

    let login = gql(
        ctx,
        r#"
        mutation Login($username: String!, $password: String!, $totpCode: String!) {
          login(input: { username: $username, password: $password, totpCode: $totpCode }) {
            token
          }
        }
        "#,
        // Preserve the untouched TOTP code for the step-up assertion below;
        // recovery codes are a supported primary-login fallback.
        json!({ "username": username, "password": password, "totpCode": recovery_code }),
    )
    .await;
    assert_no_errors(&login);
    let token = login["data"]["login"]["token"]
        .as_str()
        .expect("login token")
        .to_string();
    (admin, token, totp_code)
}

async fn seed_test_passkey(ctx: &TestContext, user_id: &str, credential_id: &str) {
    let now = Utc::now().to_rfc3339();
    WebauthnStore::new(ctx.db.datastore())
        .create_credential(WebauthnCredentialRecord {
            id: Id::new().0,
            user_id: user_id.to_string(),
            credential_id: credential_id.to_string(),
            credential_json: "{}".to_string(),
            friendly_name: Some("Test passkey".to_string()),
            created_at: now,
            last_used_at: None,
        })
        .await
        .expect("seed passkey credential");
}

fn write_backup_fixture(ctx: &TestContext, info: BackupInfo, bundle_bytes: &[u8]) {
    let backup_dir = ctx.app.default_backup_dir();
    std::fs::create_dir_all(&backup_dir).expect("create backup dir");
    std::fs::write(backup_dir.join(&info.filename), bundle_bytes).expect("write backup bundle");
    let metadata_path = backup_dir.join(format!("{}.metadata.json", info.filename));
    std::fs::write(
        metadata_path,
        serde_json::to_vec(&info).expect("serialize backup metadata"),
    )
    .expect("write backup metadata");
}

fn backup_dir_is_empty(ctx: &TestContext) -> bool {
    let backup_dir = ctx.app.default_backup_dir();
    !backup_dir.exists()
        || std::fs::read_dir(backup_dir)
            .expect("read backup dir")
            .next()
            .is_none()
}

async fn set_rename_collision_policy(ctx: &TestContext, scope: &str, policy: &str) {
    let body = gql(
        ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            renameCollisionPolicy
          }
        }
        "#,
        json!({
            "input": {
                "scope": scope,
                "renameCollisionPolicy": policy
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["updateMediaSettings"]["renameCollisionPolicy"],
        policy
    );
}

async fn set_folder_template(ctx: &TestContext, scope: &str, template: &str) {
    let body = gql(
        ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            folderTemplate
          }
        }
        "#,
        json!({
            "input": {
                "scope": scope,
                "folderTemplate": template
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["updateMediaSettings"]["folderTemplate"],
        template
    );
}

struct FailingMediaFileRepo {
    inner: MediaFileStore,
    fail_file_id: String,
}

#[async_trait]
impl MediaFileRepository for FailingMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        self.inner.insert_media_file(input).await
    }

    async fn claim_import_destination(
        &self,
        input: &InsertMediaFileInput,
        associations: &scryer_application::MediaFileAssociations,
    ) -> AppResult<scryer_application::ClaimedMediaFile> {
        self.inner
            .claim_import_destination(input, associations)
            .await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        self.inner.link_file_to_episode(file_id, episode_id).await
    }

    async fn link_file_to_series_movie(
        &self,
        file_id: &str,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        self.inner
            .link_file_to_series_movie(file_id, series_movie_link_id)
            .await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        self.inner.list_media_files_for_title(title_id).await
    }

    async fn list_series_movie_link_ids_with_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<String>> {
        self.inner
            .list_series_movie_link_ids_with_files_for_title(title_id)
            .await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        self.inner
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        self.inner.list_title_media_size_summaries(title_ids).await
    }

    async fn collection_media_size_bytes(
        &self,
        title_id: &str,
        ordered_path: &str,
    ) -> AppResult<Option<i64>> {
        self.inner
            .collection_media_size_bytes(title_id, ordered_path)
            .await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        self.inner.list_title_quality_summaries(title_ids).await
    }

    async fn list_title_movie_media_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>> {
        self.inner.list_title_movie_media_summaries(title_ids).await
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        self.inner
            .list_cutoff_unmet_quality_summaries(title_ids)
            .await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        self.inner
            .list_title_episode_progress_summaries(title_ids)
            .await
    }

    async fn list_collection_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>> {
        self.inner
            .list_collection_episode_progress_summaries(title_ids)
            .await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_analysis(file_id, analysis)
            .await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_source_signature(
                file_id,
                size_bytes,
                source_signature_scheme,
                source_signature_value,
            )
            .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        if file_id == self.fail_file_id {
            return Err(AppError::Repository(format!(
                "injected media file path failure for {file_id} -> {file_path}"
            )));
        }

        self.inner.update_media_file_path(file_id, file_path).await
    }

    async fn set_media_file_roles_for_title(
        &self,
        title_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        self.inner
            .set_media_file_roles_for_title(title_id, primary_file_id, additional_file_ids)
            .await
    }

    async fn set_media_file_roles_for_episode(
        &self,
        title_id: &str,
        episode_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        self.inner
            .set_media_file_roles_for_episode(
                title_id,
                episode_id,
                primary_file_id,
                additional_file_ids,
            )
            .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        self.inner.mark_scan_failed(file_id, error).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_id(file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_path(file_path).await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        self.inner.delete_media_file(file_id).await
    }
}

/// Delegating [`SettingsRepository`] double that separates direct explicit
/// reads from scoped batch reads for request-loader regression coverage.
struct CountingSettingsRepo {
    inner: std::sync::Arc<dyn SettingsRepository>,
    direct_explicit_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    batch_explicit_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    global_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl SettingsRepository for CountingSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.global_reads
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.get_setting_json(scope, key_name, scope_id).await
    }

    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.direct_explicit_reads
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .get_setting_json_explicit(scope, key_name, scope_id)
            .await
    }

    async fn list_setting_json_explicit_for_scope_ids(
        &self,
        scope: &str,
        key_name: &str,
        scope_ids: &[String],
    ) -> AppResult<Vec<(String, String)>> {
        self.batch_explicit_reads
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .list_setting_json_explicit_for_scope_ids(scope, key_name, scope_ids)
            .await
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .upsert_setting_json(
                scope,
                key_name,
                scope_id,
                value_json,
                source,
                updated_by_user_id,
            )
            .await
    }

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .delete_setting_value(scope, key_name, scope_id)
            .await
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        self.inner.delete_values_for_scope_id(scope_id).await
    }
}

/// Delegating [`MediaFileRepository`] double that counts how many times the
/// title media-size summary port is invoked. Used to prove that resolving the
/// `sizeBytes` enrichment across N titles in one GraphQL query issues exactly
/// one batched repository call when request-scoped loaders are present, versus
/// one call per title on the loader-absent fallback path.
struct CountingMediaFileRepo {
    inner: MediaFileStore,
    size_summary_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl MediaFileRepository for CountingMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        self.inner.insert_media_file(input).await
    }

    async fn claim_import_destination(
        &self,
        input: &InsertMediaFileInput,
        associations: &scryer_application::MediaFileAssociations,
    ) -> AppResult<scryer_application::ClaimedMediaFile> {
        self.inner
            .claim_import_destination(input, associations)
            .await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        self.inner.link_file_to_episode(file_id, episode_id).await
    }

    async fn link_file_to_series_movie(
        &self,
        file_id: &str,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        self.inner
            .link_file_to_series_movie(file_id, series_movie_link_id)
            .await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        self.inner.list_media_files_for_title(title_id).await
    }

    async fn list_series_movie_link_ids_with_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<String>> {
        self.inner
            .list_series_movie_link_ids_with_files_for_title(title_id)
            .await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        self.inner
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await
    }

    async fn collection_media_size_bytes(
        &self,
        title_id: &str,
        ordered_path: &str,
    ) -> AppResult<Option<i64>> {
        self.inner
            .collection_media_size_bytes(title_id, ordered_path)
            .await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        self.size_summary_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.list_title_media_size_summaries(title_ids).await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        self.inner.list_title_quality_summaries(title_ids).await
    }

    async fn list_title_movie_media_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>> {
        self.inner.list_title_movie_media_summaries(title_ids).await
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        self.inner
            .list_cutoff_unmet_quality_summaries(title_ids)
            .await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        self.inner
            .list_title_episode_progress_summaries(title_ids)
            .await
    }

    async fn list_collection_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>> {
        self.inner
            .list_collection_episode_progress_summaries(title_ids)
            .await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_analysis(file_id, analysis)
            .await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_source_signature(
                file_id,
                size_bytes,
                source_signature_scheme,
                source_signature_value,
            )
            .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        self.inner.update_media_file_path(file_id, file_path).await
    }

    async fn set_media_file_roles_for_title(
        &self,
        title_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        self.inner
            .set_media_file_roles_for_title(title_id, primary_file_id, additional_file_ids)
            .await
    }

    async fn set_media_file_roles_for_episode(
        &self,
        title_id: &str,
        episode_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        self.inner
            .set_media_file_roles_for_episode(
                title_id,
                episode_id,
                primary_file_id,
                additional_file_ids,
            )
            .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        self.inner.mark_scan_failed(file_id, error).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_id(file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_path(file_path).await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        self.inner.delete_media_file(file_id).await
    }
}

/// Helper to add a title and return the title ID.
async fn add_test_title(ctx: &TestContext, name: &str, facet: &str) -> String {
    let tvdb_id = match facet {
        "MOVIE" => "123456",
        "SERIES" | "ANIME" => "345678",
        _ => "123456",
    };
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) { addTitle(input: $input) { title { id name } } }"#,
        json!({
            "input": {
                "name": name,
                "facet": facet,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": tvdb_id }]
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn update_title_catalog_sort_fixture(
    ctx: &TestContext,
    title_id: &str,
    monitored: bool,
    content_status: &str,
    quality_profile_id: &str,
) {
    let tags = serde_json::to_string(&vec![format!(
        "scryer:quality-profile:{quality_profile_id}"
    )])
    .expect("quality profile fixture tags should serialize");
    sqlx::query(
        "UPDATE titles
            SET monitored = ?,
                content_status = ?,
                tags = ?
          WHERE id = ?",
    )
    .bind(if monitored { 1_i64 } else { 0_i64 })
    .bind(content_status)
    .bind(tags)
    .bind(title_id)
    .execute(ctx.db.pool())
    .await
    .expect("title catalog sort fixture should update title row");
}

async fn insert_catalog_sort_collection(
    ctx: &TestContext,
    collection_id: &str,
    title_id: &str,
    collection_index: i64,
    ordered_path: Option<&str>,
) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO collections
            (id, title_id, collection_type, collection_index, label, ordered_path, created_at, updated_at)
         VALUES (?, ?, 'season', ?, NULL, ?, ?, ?)",
    )
    .bind(collection_id)
    .bind(title_id)
    .bind(collection_index.to_string())
    .bind(ordered_path)
    .bind(&now)
    .bind(&now)
    .execute(ctx.db.pool())
    .await
    .expect("catalog sort fixture should insert collection");
}

async fn insert_catalog_sort_media_file(
    ctx: &TestContext,
    title_id: &str,
    file_path: &str,
    size_bytes: i64,
) -> String {
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: file_path.to_string(),
            size_bytes,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("catalog sort fixture should insert media file")
}

async fn seed_title_size_sort_fixture(
    ctx: &TestContext,
    title_id: &str,
    collection_id: &str,
    file_path: &str,
    size_bytes: i64,
) {
    insert_catalog_sort_collection(ctx, collection_id, title_id, 1, Some(file_path)).await;
    insert_catalog_sort_media_file(ctx, title_id, file_path, size_bytes).await;
}

async fn seed_title_episode_sort_fixture(
    ctx: &TestContext,
    title_id: &str,
    collection_id: &str,
    owned_episodes: usize,
    total_episodes: usize,
) {
    insert_catalog_sort_collection(ctx, collection_id, title_id, 1, None).await;
    let now = Utc::now().to_rfc3339();
    for episode_index in 1..=total_episodes {
        let episode_id = format!("{collection_id}-episode-{episode_index}");
        sqlx::query(
            "INSERT INTO episodes
                (id, title_id, collection_id, episode_type, episode_number, season_number,
                 episode_label, title, air_date, duration_seconds, has_multi_audio, has_subtitle,
                 monitored, created_at, updated_at)
             VALUES (?, ?, ?, 'standard', ?, '1', NULL, ?, '2026-01-01', NULL, 0, 0, 1, ?, ?)",
        )
        .bind(&episode_id)
        .bind(title_id)
        .bind(collection_id)
        .bind(episode_index.to_string())
        .bind(format!("Episode {episode_index}"))
        .bind(&now)
        .bind(&now)
        .execute(ctx.db.pool())
        .await
        .expect("catalog sort fixture should insert episode");

        if episode_index <= owned_episodes {
            let file_id = insert_catalog_sort_media_file(
                ctx,
                title_id,
                &format!("/sort-fixtures/{collection_id}/episode-{episode_index}.mkv"),
                0,
            )
            .await;
            ctx.media_files
                .link_file_to_episode(&file_id, &episode_id)
                .await
                .expect("catalog sort fixture should link episode media file");
        }
    }
}

async fn title_catalog_sort_names(ctx: &TestContext, key: &str, direction: &str) -> Vec<String> {
    let body = gql(
        ctx,
        r#"query($sort: TitleCatalogSortInput) {
            titles(sort: $sort) {
                items {
                    name
                    monitored
                    contentStatus
                    qualityProfileId
                    sizeBytes
                    episodesOwned
                    episodesTotal
                }
            }
        }"#,
        json!({ "sort": { "key": key, "direction": direction } }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["titles"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect()
}

async fn seed_typed_settings_definitions(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_api_key".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_username".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_password".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.languages".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.auto_download_on_import".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.minimum_score_series".into(),
                data_type: "number".into(),
                default_value_json: "90".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.minimum_score_movie".into(),
                data_type: "number".into(),
                default_value_json: "70".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.search_interval_hours".into(),
                data_type: "number".into(),
                default_value_json: "6".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.include_ai_translated".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.include_machine_translated".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_threshold_series".into(),
                data_type: "number".into(),
                default_value_json: "90".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_threshold_movie".into(),
                data_type: "number".into(),
                default_value_json: "70".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_max_offset_seconds".into(),
                data_type: "number".into(),
                default_value_json: "60".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.upgrade_cooldown_hours".into(),
                data_type: "number".into(),
                default_value_json: "24".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.same_tier_min_delta".into(),
                data_type: "number".into(),
                default_value_json: "120".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.cross_tier_min_delta".into(),
                data_type: "number".into(),
                default_value_json: "30".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.forced_upgrade_delta_bypass".into(),
                data_type: "number".into(),
                default_value_json: "400".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.poll_interval_seconds".into(),
                data_type: "number".into(),
                default_value_json: "60".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.long_tail_backfill_max_scopes_per_cycle".into(),
                data_type: "number".into(),
                default_value_json: "500".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.long_tail_reconverge_days".into(),
                data_type: "number".into(),
                default_value_json: "0".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.delay_profiles".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "ui.experimental_features_enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "discovery.personalized_enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "imports.srrdb_filename_recovery.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "history.keep_forever".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "history.retention_days".into(),
                data_type: "number".into(),
                default_value_json: "180".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "images.cache.max_size_mb".into(),
                data_type: "number".into(),
                default_value_json: "256".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "plugins.http.ca_bundle_pem".into(),
                data_type: "string".into(),
                default_value_json: "\"\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "security".into(),
                scope: "system".into(),
                key_name: "auth.form_login_enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "security".into(),
                scope: "system".into(),
                key_name: "auth.password_min_length".into(),
                data_type: "integer".into(),
                default_value_json: "8".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "security".into(),
                scope: "system".into(),
                key_name: "auth.skip_login_for_local_ips".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "security".into(),
                scope: "system".into(),
                key_name: "auth.mfa.require_config_step_up".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "security".into(),
                scope: "system".into(),
                key_name: "auth.totp.require_jellyfin_login".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "security".into(),
                scope: "system".into(),
                key_name: "auth.totp.require_emby_login".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "security".into(),
                scope: "system".into(),
                key_name: "auth.mfa.require_password_login".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profiles".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profile_id".into(),
                data_type: "string".into(),
                default_value_json: "\"4k\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.request_profile_ids".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.scoring_persona".into(),
                data_type: "string".into(),
                default_value_json: "\"Balanced\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "audio.required_languages".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "audio.required_languages.title_override".into(),
                data_type: "json".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "download_client.routing".into(),
                data_type: "json".into(),
                default_value_json: "{}".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "indexer.routing".into(),
                data_type: "json".into(),
                default_value_json: "{}".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/movies\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/series\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/anime\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "folder.template".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "folder.season_template".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "folder.specials_template".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"{title} ({year}) - {quality}.{ext}\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.series.global".into(),
                data_type: "string".into(),
                default_value_json:
                    "\"{title} - S{season:2}E{episode:2} - {quality}.{ext}\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.anime.global".into(),
                data_type: "string".into(),
                default_value_json:
                    "\"{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}\""
                        .into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy.global".into(),
                data_type: "string".into(),
                default_value_json: "\"skip\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"skip\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy.global".into(),
                data_type: "string".into(),
                default_value_json: "\"fallback_title\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"fallback_title\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.filler_policy".into(),
                data_type: "string".into(),
                default_value_json: "\"download_all\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.recap_policy".into(),
                data_type: "string".into(),
                default_value_json: "\"download_all\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.monitor_specials".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.inter_season_movies".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.monitor_filler_movies".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.movie".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.series".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.anime".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "plexmatch.write_on_import.series".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "plexmatch.write_on_import.anime".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "metadata_language".into(),
                data_type: "string".into(),
                default_value_json: "\"eng\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "metadata_language.title_override".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.use_season_folders".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "service".into(),
                scope: "system".into(),
                key_name: "tls.cert_path".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "service".into(),
                scope: "system".into(),
                key_name: "tls.key_path".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("settings definitions should seed");
}

async fn mount_smg_mocks(ctx: &TestContext, fixture_path: &str) {
    let fixture = load_fixture(fixture_path);
    let get_fixture = fixture.clone();
    let titles_fixture = load_fixture("smg/titles_movie.json");
    let resolve_titles_fixture = load_fixture("smg/resolve_titles.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(move |request: &wiremock::Request| {
            let operation_name = request
                .url
                .query_pairs()
                .find_map(|(name, value)| (name == "operationName").then(|| value.into_owned()));
            let response = match operation_name.as_deref() {
                Some("ResolveTitles") => &resolve_titles_fixture,
                Some("Titles") => &titles_fixture,
                _ => &get_fixture,
            };
            ResponseTemplate::new(200).set_body_string(response.clone())
        })
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;
}

async fn configure_default_library_root(
    ctx: &TestContext,
    facet: MediaFacet,
    media_root: &std::path::Path,
) -> String {
    let library_id = scryer_domain::default_library_id_for_facet(&facet);
    let library = ctx
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("lookup default library")
        .expect("default library exists");
    let media_root_path = media_root.to_string_lossy().to_string();
    let updated = ctx
        .libraries
        .update(
            &library_id,
            library.name,
            library.slug,
            vec![LibraryRootDraft {
                path: media_root_path.clone(),
                is_default: true,
            }],
        )
        .await
        .expect("configure default library root");
    // Root ids are allocated, not derived from the path, so read the stored id back.
    updated
        .roots
        .iter()
        .find(|root| root.is_default)
        .or_else(|| updated.roots.first())
        .map(|root| root.id.clone())
        .expect("configured library should expose its root")
}

async fn default_library_root_id(ctx: &TestContext, facet: &MediaFacet) -> String {
    let library_id = scryer_domain::default_library_id_for_facet(facet);
    let library = ctx
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("lookup default library")
        .expect("default library exists");
    library
        .roots
        .iter()
        .find(|root| root.is_default)
        .or_else(|| library.roots.first())
        .map(|root| root.id.clone())
        .unwrap_or_else(|| {
            scryer_domain::root_folder_id_for_path(match facet {
                MediaFacet::Movie => "/data/movies",
                MediaFacet::Series => "/data/series",
                MediaFacet::Anime => "/data/anime",
            })
        })
}

async fn create_series_scan_title(
    ctx: &TestContext,
    media_root: &std::path::Path,
    name: &str,
    extra_tags: Vec<String>,
) -> (Title, Collection) {
    let root_folder_id = configure_default_library_root(ctx, MediaFacet::Series, media_root).await;
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let tags = extra_tags;
    let title_dir = media_root.join(name);

    let title = Title {
        id: Id::new().0,
        name: name.to_string(),
        facet: MediaFacet::Series,
        library_id: series_library_id,
        monitored: true,
        tags,
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id,
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: Some(24),
        popularity: None,
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: Some(title_dir.to_string_lossy().to_string()),
    };
    let title = ctx.titles.create(title).await.expect("create series title");

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("10".to_string()),
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .shows
        .create_collection(collection)
        .await
        .expect("create season collection");

    (title, collection)
}

async fn create_catalog_title(
    ctx: &TestContext,
    name: &str,
    facet: MediaFacet,
    external_ids: Vec<ExternalId>,
    tags: Vec<String>,
    monitored: bool,
) -> Title {
    let root_folder_id = default_library_root_id(ctx, &facet).await;
    let title = Title {
        id: Id::new().0,
        name: name.to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        facet,
        monitored,
        tags,
        canonical_tags: vec![],
        external_ids,
        root_folder_id,
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: Some("Original overview".to_string()),
        poster_url: Some("https://example.com/old-poster.jpg".to_string()),
        poster_source_url: None,
        background_url: Some("https://example.com/old-background.jpg".to_string()),
        background_source_url: None,
        sort_title: Some(name.to_string()),
        catalog_sort_key: String::new(),
        slug: Some("old-slug".to_string()),
        imdb_id: Some("tt0000001".to_string()),
        runtime_minutes: Some(100),
        popularity: None,
        content_status: Some("ended".to_string()),
        language: Some("eng".to_string()),
        first_aired: Some("2020-01-01".to_string()),
        network: Some("Old Network".to_string()),
        studio: Some("Old Studio".to_string()),
        country: Some("usa".to_string()),
        aliases: vec!["Legacy Alias".to_string()],
        tagged_aliases: vec![],
        metadata_language: Some("eng".to_string()),
        metadata_fetched_at: Some(Utc::now()),
        min_availability: None,
        digital_release_date: Some("2020-01-01".to_string()),
        folder_path: None,
    };

    ctx.titles.create(title).await.expect("create title")
}

async fn set_title_folder_path(ctx: &TestContext, title_id: &str, path: &std::path::Path) {
    ctx.titles
        .set_folder_path(title_id, &path.to_string_lossy())
        .await
        .expect("set title folder path");
}

async fn activity_kinds_for_title(ctx: &TestContext, title_id: &str) -> Vec<String> {
    let body = gql(ctx, "{ activityEvents { kind titleId } }", json!({})).await;
    assert_no_errors(&body);

    body["data"]["activityEvents"]
        .as_array()
        .expect("activity events array")
        .iter()
        .filter(|event| event["titleId"] == title_id)
        .filter_map(|event| event["kind"].as_str())
        .map(str::to_string)
        .collect()
}

async fn create_series_scan_episode(
    ctx: &TestContext,
    title: &Title,
    collection: &Collection,
    season_number: &str,
    episode_number: &str,
    label: &str,
) -> Episode {
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some(episode_number.to_string()),
        season_number: Some(season_number.to_string()),
        episode_label: Some(label.to_string()),
        title: Some(format!("Episode {episode_number}")),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.shows
        .create_episode(episode)
        .await
        .expect("create episode")
}

async fn create_series_movie_special_episode(
    ctx: &TestContext,
    title: &Title,
    collection: &Collection,
    episode_number: &str,
    episode_title: &str,
    tvdb_id: &str,
) -> Episode {
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Special,
        episode_number: Some(episode_number.to_string()),
        season_number: Some("0".to_string()),
        episode_label: Some(format!("S00E{episode_number:0>2}")),
        title: Some(episode_title.to_string()),
        air_date: None,
        duration_seconds: Some(5400),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: Some(episode_number.to_string()),
        overview: None,
        tvdb_id: Some(tvdb_id.to_string()),
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.shows
        .create_episode(episode)
        .await
        .expect("create series movie special episode")
}

async fn create_test_series_movie_link(
    ctx: &TestContext,
    title: &Title,
    movie_title: &str,
    tvdb_id: &str,
    linked_episode_id: Option<String>,
    legacy_collection_id: Option<String>,
) -> scryer_domain::SeriesMovieLink {
    let now = chrono::Utc::now();
    let link = scryer_domain::SeriesMovieLink {
        id: Id::new().0,
        series_title_id: title.id.clone(),
        movie: scryer_domain::MovieEntity {
            id: Id::new().0,
            title: movie_title.to_string(),
            sort_title: Some(movie_title.to_string()),
            slug: Some(
                movie_title
                    .to_ascii_lowercase()
                    .replace(|ch: char| !ch.is_ascii_alphanumeric(), "-")
                    .trim_matches('-')
                    .to_string(),
            ),
            year: title.year,
            overview: Some(format!("{movie_title} overview")),
            poster_url: Some(format!(
                "https://example.com/{}.jpg",
                movie_title
                    .to_ascii_lowercase()
                    .replace(|ch: char| !ch.is_ascii_alphanumeric(), "-")
                    .trim_matches('-')
            )),
            background_url: None,
            language: Some("eng".to_string()),
            runtime_minutes: Some(95),
            content_status: Some("released".to_string()),
            studio: title.studio.clone(),
            digital_release_date: Some("2024-01-01".to_string()),
            imdb_id: Some(format!("tt{tvdb_id}")),
            tvdb_id: Some(tvdb_id.to_string()),
            tmdb_id: None,
            mal_id: None,
            anidb_id: None,
            ratings: Some(scryer_domain::TitleRatingSummary {
                rating: Some(8.7),
                rating_sources: vec!["tmdb".to_string()],
                external_ratings: vec![scryer_domain::TitleExternalRating {
                    source: "tmdb".to_string(),
                    value: Some(8.7),
                    normalized: 8.7,
                    votes: Some(1_234),
                    url: "https://www.themoviedb.org/movie/fixture".to_string(),
                    ..Default::default()
                }],
            }),
            credits: Some(vec![scryer_domain::TitleCredit {
                kind: "voice_actor".to_string(),
                person_id: "movie-cast-1".to_string(),
                person_name: "Fixture Performer".to_string(),
                person_image_url: "https://images.example.com/private-upstream.jpg".to_string(),
                character_name: "Fixture Character".to_string(),
                language: "eng".to_string(),
                billing_order: 1,
                ..Default::default()
            }]),
            created_at: now,
            updated_at: now,
        },
        placement: None,
        narrative_order: Some("1.0".to_string()),
        after_season: None,
        before_season: None,
        linked_episode_id,
        association_confidence: Some("high".to_string()),
        continuity_status: Some("canonical".to_string()),
        movie_form: Some("movie".to_string()),
        confidence: Some("high".to_string()),
        signal_summary: Some("test fixture".to_string()),
        source: Some("test".to_string()),
        monitoring_override: None,
        metadata_active: true,
        monitored: true,
        legacy_collection_id,
        created_at: now,
        updated_at: now,
    };
    ctx.shows
        .upsert_series_movie_link(link)
        .await
        .expect("create series movie link")
}

const LARGE_GRAPHQL_TEST_STACK_SIZE_BYTES: usize = 8 * 1024 * 1024;

fn run_large_stack_graphql_test<F, Fut>(name: &str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(LARGE_GRAPHQL_TEST_STACK_SIZE_BYTES)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime for large-stack GraphQL test");
            runtime.block_on(test());
        })
        .expect("spawn large-stack GraphQL test thread");

    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
