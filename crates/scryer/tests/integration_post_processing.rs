#![recursion_limit = "256"]

mod common;

use std::path::PathBuf;

use common::TestContext;
use scryer_application::{
    ActivityKind, ActivitySeverity, DomainEventActor, PostProcessingContext, TitleRepository,
    run_post_processing,
};
use scryer_domain::{
    AppPermission, AppPermissionMask, ConfigurationChangeAction, DomainEventFilter,
    DomainEventPayload, DomainEventType, LibraryPermission, LibraryPermissionMask, MediaFacet,
    PostProcessingScript, ScriptRunStatus, ScriptType, User,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn admin() -> User {
    let mut user = User::new_admin("admin");
    user.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::from_permissions([AppPermission::ManageCatalogSettings]),
        default_library: LibraryPermissionMask::from_permissions([
            LibraryPermission::View,
            LibraryPermission::ManageTitles,
            LibraryPermission::ResolveImports,
            LibraryPermission::ManageLibrary,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    user
}

/// Create a post-processing script in the DB for the given facet.
async fn create_script(
    ctx: &TestContext,
    facet: MediaFacet,
    command: &str,
    timeout_secs: i64,
    debug: bool,
) {
    create_script_with_type(ctx, facet, ScriptType::Inline, command, timeout_secs, debug).await;
}

async fn create_script_with_type(
    ctx: &TestContext,
    facet: MediaFacet,
    script_type: ScriptType,
    content: &str,
    timeout_secs: i64,
    debug: bool,
) -> String {
    let facet_str = facet.as_str();
    let script_id = format!(
        "pp-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let script = PostProcessingScript {
        id: script_id.clone(),
        name: format!("Test script for {facet_str}"),
        description: String::new(),
        script_type,
        script_content: content.to_string(),
        applied_facets: vec![facet_str.to_string()],
        execution_mode: scryer_domain::ExecutionMode::Blocking,
        timeout_secs,
        priority: 0,
        enabled: true,
        debug,
        language: scryer_domain::ScriptLanguage::Shell,
        trigger: scryer_domain::ScriptTrigger::PostImport,
        schedule: None,
        run_on_startup: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let actor = admin();
    ctx.app
        .create_post_processing_script(&actor, script)
        .await
        .expect("create script");
    script_id
}

#[cfg(unix)]
fn write_executable_script(path: &std::path::Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, content).expect("write script");
    let mut permissions = std::fs::metadata(path)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod script");
}

async fn seed_title(ctx: &TestContext, id: &str, name: &str, facet: MediaFacet) {
    let root_folder_path = match facet {
        MediaFacet::Movie => "/data/movies",
        MediaFacet::Series => "/data/series",
        MediaFacet::Anime => "/data/anime",
    };
    TitleRepository::create(
        &ctx.titles,
        scryer_domain::Title {
            id: id.to_string(),
            name: name.to_string(),
            facet: facet.clone(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            root_folder_id: scryer_domain::root_folder_id_for_path(root_folder_path),
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
            runtime_minutes: None,
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
            folder_path: None,
        },
    )
    .await
    .expect("seed title");
}

/// Build a PostProcessingContext for a movie import.
fn movie_context(
    app: &scryer_application::AppUseCase,
    dest: &std::path::Path,
) -> PostProcessingContext {
    PostProcessingContext {
        app: app.clone(),
        actor: DomainEventActor::system(),
        title_id: "title-pp-test".to_string(),
        title_name: "Test Movie".to_string(),
        facet: MediaFacet::Movie,
        dest_path: dest.to_path_buf(),
        year: Some(2024),
        imdb_id: Some("tt1234567".to_string()),
        tvdb_id: None,
        season: None,
        episode: None,
        quality: Some("1080p".to_string()),
    }
}

/// Retrieve the most recent activity events and find one matching PostProcessingCompleted.
async fn last_post_processing_event(
    app: &scryer_application::AppUseCase,
) -> Option<scryer_application::ActivityEvent> {
    let actor = admin();
    let events = app
        .recent_activity(&actor, 10, 0)
        .await
        .expect("recent activity");
    events
        .into_iter()
        .find(|e| e.kind == ActivityKind::PostProcessingCompleted)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When no scripts are configured, post-processing is a no-op.
#[tokio::test]
async fn skips_when_no_script_configured() {
    let ctx = TestContext::new().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    assert!(
        last_post_processing_event(&ctx.app).await.is_none(),
        "no activity event expected when no scripts configured"
    );
}

/// A script that exits 0 produces a Success activity event.
#[tokio::test]
async fn successful_script_records_success_event() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;
    create_script(&ctx, MediaFacet::Movie, "true", 300, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Success);
    assert!(event.message.contains("Test Movie"));
}

/// A script that exits non-zero produces a Warning activity event.
#[tokio::test]
async fn failed_script_records_warning_with_stderr() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;
    create_script(
        &ctx,
        MediaFacet::Movie,
        "echo 'oh no' >&2; exit 42",
        300,
        true,
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
}

/// A script that exceeds the timeout is killed and produces a timeout warning.
#[tokio::test]
async fn timeout_kills_script_and_records_warning() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;
    create_script(&ctx, MediaFacet::Movie, "sleep 60", 1, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
}

#[cfg(unix)]
#[tokio::test]
async fn file_script_timeout_kills_script_and_records_timeout_run() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;

    let script_dir = tempfile::tempdir().expect("tempdir");
    let script_path = script_dir.path().join("sleep-post-process.sh");
    write_executable_script(&script_path, "#!/bin/sh\nsleep 60\n");
    let script_id = create_script_with_type(
        &ctx,
        MediaFacet::Movie,
        ScriptType::File,
        script_path.to_str().expect("utf-8 script path"),
        1,
        false,
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let actor = admin();
    let runs = ctx
        .app
        .list_post_processing_script_runs(&actor, &script_id, 1)
        .await
        .expect("list script runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, ScriptRunStatus::Timeout);
}

/// The script receives SCRYER_METADATA and legacy environment variables.
#[tokio::test]
async fn script_receives_environment_variables() {
    let ctx = TestContext::new().await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let env_dump = output_dir.path().join("env_dump.txt");
    let script = format!("env | grep ^SCRYER_ | sort > '{}'", env_dump.display());
    create_script(&ctx, MediaFacet::Movie, &script, 300, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor: DomainEventActor::system(),
        title_id: "title-env-test".to_string(),
        title_name: "Env Test Movie".to_string(),
        facet: MediaFacet::Movie,
        dest_path: dest_file.clone(),
        year: Some(2024),
        imdb_id: Some("tt9999999".to_string()),
        tvdb_id: Some("12345".to_string()),
        season: None,
        episode: None,
        quality: Some("720p".to_string()),
    };
    run_post_processing(pp_ctx).await.expect("run");

    let content = std::fs::read_to_string(&env_dump).expect("read env dump");
    assert!(
        content.contains("SCRYER_EVENT=post_import"),
        "content:\n{content}"
    );
    assert!(
        content.contains("SCRYER_FACET=movie"),
        "content:\n{content}"
    );
    assert!(
        content.contains(&format!("SCRYER_FILE_PATH={}", dest_file.display())),
        "content:\n{content}"
    );
    assert!(
        content.contains("SCRYER_TITLE_NAME=Env Test Movie"),
        "content:\n{content}"
    );
    assert!(
        content.contains("SCRYER_METADATA="),
        "should have JSON metadata: {content}"
    );
}

/// The script's working directory is set to the parent of the imported file.
#[tokio::test]
async fn script_working_directory_is_file_parent() {
    let ctx = TestContext::new().await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let cwd_dump = output_dir.path().join("cwd.txt");
    let script = format!("pwd > '{}'", cwd_dump.display());
    create_script(&ctx, MediaFacet::Movie, &script, 300, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let cwd = std::fs::read_to_string(&cwd_dump)
        .expect("read cwd dump")
        .trim()
        .to_string();

    let expected = dest_dir.path().canonicalize().expect("canonicalize dest");
    let actual = PathBuf::from(&cwd)
        .canonicalize()
        .expect("canonicalize cwd");
    assert_eq!(actual, expected);
}

#[cfg(unix)]
#[tokio::test]
async fn file_script_executes_direct_path_with_environment_and_cwd() {
    let ctx = TestContext::new().await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let script_path = output_dir.path().join("direct-post-process.sh");
    let env_dump = output_dir.path().join("file_env_dump.txt");
    let cwd_dump = output_dir.path().join("file_cwd.txt");
    write_executable_script(
        &script_path,
        &format!(
            "#!/bin/sh\nenv | grep ^SCRYER_ | sort > '{}'\npwd > '{}'\n",
            env_dump.display(),
            cwd_dump.display()
        ),
    );
    create_script_with_type(
        &ctx,
        MediaFacet::Movie,
        ScriptType::File,
        script_path.to_str().expect("utf-8 script path"),
        300,
        false,
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let env_content = std::fs::read_to_string(&env_dump).expect("read env dump");
    assert!(
        env_content.contains("SCRYER_EVENT=post_import"),
        "content:\n{env_content}"
    );
    assert!(
        env_content.contains("SCRYER_FACET=movie"),
        "content:\n{env_content}"
    );
    assert!(
        env_content.contains(&format!("SCRYER_FILE_PATH={}", dest_file.display())),
        "content:\n{env_content}"
    );

    let cwd = std::fs::read_to_string(&cwd_dump)
        .expect("read cwd dump")
        .trim()
        .to_string();
    let expected = dest_dir.path().canonicalize().expect("canonicalize dest");
    let actual = PathBuf::from(&cwd)
        .canonicalize()
        .expect("canonicalize cwd");
    assert_eq!(actual, expected);
}

#[cfg(unix)]
#[tokio::test]
async fn file_script_content_is_executable_path_not_shell_command() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let script_path = output_dir.path().join("file-script-no-shell.sh");
    let marker = output_dir.path().join("marker.txt");
    write_executable_script(
        &script_path,
        &format!("#!/bin/sh\necho ran > '{}'\n", marker.display()),
    );

    create_script_with_type(
        &ctx,
        MediaFacet::Movie,
        ScriptType::File,
        &format!("{} --not-an-argument", script_path.display()),
        300,
        true,
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    assert!(
        !marker.exists(),
        "file script content must be treated as one executable path, not a shell command with arguments"
    );
    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
}

#[tokio::test]
async fn file_script_bare_command_records_failure_without_path_lookup() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;

    let script_id =
        create_script_with_type(&ctx, MediaFacet::Movie, ScriptType::File, "true", 300, true).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let actor = admin();
    let runs = ctx
        .app
        .list_post_processing_script_runs(&actor, &script_id, 1)
        .await
        .expect("list script runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, ScriptRunStatus::Failed);
    assert!(
        runs[0]
            .stderr_tail
            .as_deref()
            .is_some_and(|stderr| stderr.contains("file script path must be absolute")),
        "stderr tail should explain absolute path requirement: {:?}",
        runs[0].stderr_tail
    );
}

/// Series facet uses series-targeted scripts.
#[tokio::test]
async fn series_facet_uses_series_script() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-series-pp", "Test Show", MediaFacet::Series).await;
    create_script(&ctx, MediaFacet::Series, "true", 300, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Show.S01E01.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor: DomainEventActor::system(),
        title_id: "title-series-pp".to_string(),
        title_name: "Test Show".to_string(),
        facet: MediaFacet::Series,
        dest_path: dest_file,
        year: None,
        imdb_id: None,
        tvdb_id: Some("54321".to_string()),
        season: Some(1),
        episode: Some(1),
        quality: Some("1080p".to_string()),
    };
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Success);
    assert!(event.message.contains("Test Show"));
}

/// Anime facet uses anime-targeted scripts.
#[tokio::test]
async fn anime_facet_uses_anime_script() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-anime-pp", "Test Anime", MediaFacet::Anime).await;
    create_script(&ctx, MediaFacet::Anime, "true", 300, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Anime.S01E01.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor: DomainEventActor::system(),
        title_id: "title-anime-pp".to_string(),
        title_name: "Test Anime".to_string(),
        facet: MediaFacet::Anime,
        dest_path: dest_file,
        year: None,
        imdb_id: None,
        tvdb_id: None,
        season: Some(1),
        episode: Some(5),
        quality: None,
    };
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Success);
    assert!(event.message.contains("Test Anime"));
}

/// A script that references an invalid binary records a failure.
#[tokio::test]
async fn invalid_command_records_spawn_failure() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;
    create_script(
        &ctx,
        MediaFacet::Movie,
        "/nonexistent/binary_that_does_not_exist_12345",
        300,
        false,
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
}

#[tokio::test]
async fn script_configuration_changes_are_audited_without_script_content() {
    let ctx = TestContext::new().await;
    let actor = admin();
    let mut audit_actor = admin();
    audit_actor.authorization.app = AppPermissionMask::from_permissions([
        AppPermission::ManageCatalogSettings,
        AppPermission::ManageSystemSettings,
    ]);
    let script_id = "pp-audit-inline-script".to_string();
    let secret_content = "echo audit-secret-content";
    let now = chrono::Utc::now();
    let script = PostProcessingScript {
        id: script_id.clone(),
        name: "Audited inline script".to_string(),
        description: String::new(),
        script_type: ScriptType::Inline,
        script_content: secret_content.to_string(),
        applied_facets: vec!["movie".to_string()],
        execution_mode: scryer_domain::ExecutionMode::Blocking,
        timeout_secs: 300,
        priority: 0,
        enabled: true,
        debug: false,
        language: scryer_domain::ScriptLanguage::Shell,
        trigger: scryer_domain::ScriptTrigger::PostImport,
        schedule: None,
        run_on_startup: false,
        created_at: now,
        updated_at: now,
    };

    let mut updated = ctx
        .app
        .create_post_processing_script(&actor, script)
        .await
        .expect("create script");
    updated.description = "updated".to_string();
    updated.updated_at = chrono::Utc::now();
    ctx.app
        .update_post_processing_script(&actor, updated)
        .await
        .expect("update script");
    ctx.app
        .toggle_post_processing_script(&actor, &script_id)
        .await
        .expect("toggle script");
    ctx.app
        .delete_post_processing_script(&actor, &script_id)
        .await
        .expect("delete script");

    let events = ctx
        .app
        .audit_log(
            &audit_actor,
            &DomainEventFilter {
                event_types: Some(vec![DomainEventType::ConfigurationChanged]),
                limit: 20,
                ..DomainEventFilter::default()
            },
        )
        .await
        .expect("list events");

    let mut actions = Vec::new();
    for event in events {
        let payload_json = serde_json::to_string(&event.payload).expect("payload json");
        assert!(
            !payload_json.contains(secret_content),
            "audit payload should not include script content: {payload_json}"
        );
        if let DomainEventPayload::ConfigurationChanged(data) = event.payload
            && data.resource_id.as_deref() == Some(&script_id)
        {
            assert_eq!(data.resource_type, "post_processing_inline_script");
            actions.push(data.action);
        }
    }

    assert!(actions.contains(&ConfigurationChangeAction::Saved));
    assert!(actions.contains(&ConfigurationChangeAction::Updated));
    assert!(actions.contains(&ConfigurationChangeAction::Deleted));
    assert_eq!(
        actions
            .iter()
            .filter(|action| **action == ConfigurationChangeAction::Updated)
            .count(),
        2,
        "update and toggle should both emit updated audit events"
    );
}

const SCRIPT_INTERPRETER_KEYS: [&str; 4] = [
    scryer_application::SCRIPT_INTERPRETER_PYTHON_KEY,
    scryer_application::SCRIPT_INTERPRETER_POWERSHELL_KEY,
    scryer_application::SCRIPT_INTERPRETER_BATCH_KEY,
    scryer_application::SCRIPT_INTERPRETER_GO_KEY,
];

async fn seed_script_interpreter_setting_definitions(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(
            SCRIPT_INTERPRETER_KEYS
                .iter()
                .map(
                    |key_name| scryer_infrastructure_sql::types::SettingDefinitionSeed {
                        category: "general".into(),
                        scope: scryer_application::SETTINGS_SCOPE_SYSTEM.into(),
                        key_name: (*key_name).into(),
                        data_type: "string".into(),
                        default_value_json: "null".into(),
                        is_sensitive: false,
                        validation_json: None,
                    },
                )
                .collect(),
        )
        .await
        .expect("seed script interpreter setting definitions");
}

async fn script_interpreter_gql(
    ctx: &TestContext,
    query: &str,
    variables: serde_json::Value,
) -> serde_json::Value {
    let response = ctx
        .http_client()
        .post(ctx.graphql_url())
        .json(&serde_json::json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("graphql request should succeed");
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("valid JSON body");
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
    body
}

#[tokio::test]
async fn graphql_script_interpreter_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_script_interpreter_setting_definitions(&ctx).await;
    let read = "query { scriptInterpreterSettings { python powershell batch go } }";
    let update = r#"mutation($input: ScriptInterpreterSettingsInput!) {
        updateScriptInterpreterSettings(input: $input) { python powershell batch go }
    }"#;

    let body = script_interpreter_gql(&ctx, read, serde_json::json!({})).await;
    assert_eq!(
        body["data"]["scriptInterpreterSettings"],
        serde_json::json!({ "python": null, "powershell": null, "batch": null, "go": null })
    );

    let body = script_interpreter_gql(
        &ctx,
        update,
        serde_json::json!({ "input": {
            "python": "/opt/synthetic/python3",
            "powershell": "  /opt/synthetic/pwsh  ",
            "batch": "",
            "go": "/opt/synthetic/go",
        } }),
    )
    .await;
    let expected = serde_json::json!({
        "python": "/opt/synthetic/python3",
        "powershell": "/opt/synthetic/pwsh",
        "batch": null,
        "go": "/opt/synthetic/go",
    });
    assert_eq!(body["data"]["updateScriptInterpreterSettings"], expected);
    let body = script_interpreter_gql(&ctx, read, serde_json::json!({})).await;
    assert_eq!(body["data"]["scriptInterpreterSettings"], expected);

    // Omitted fields keep their pins; null and blank clear only their own.
    let body = script_interpreter_gql(
        &ctx,
        update,
        serde_json::json!({ "input": { "python": null } }),
    )
    .await;
    let expected = serde_json::json!({
        "python": null,
        "powershell": "/opt/synthetic/pwsh",
        "batch": null,
        "go": "/opt/synthetic/go",
    });
    assert_eq!(body["data"]["updateScriptInterpreterSettings"], expected);

    let body = script_interpreter_gql(
        &ctx,
        update,
        serde_json::json!({ "input": { "powershell": "", "batch": "cmd.exe" } }),
    )
    .await;
    let expected = serde_json::json!({
        "python": null,
        "powershell": null,
        "batch": "cmd.exe",
        "go": "/opt/synthetic/go",
    });
    assert_eq!(body["data"]["updateScriptInterpreterSettings"], expected);
    let body = script_interpreter_gql(&ctx, read, serde_json::json!({})).await;
    assert_eq!(body["data"]["scriptInterpreterSettings"], expected);
}

#[tokio::test]
async fn script_interpreter_settings_require_system_settings_permission() {
    let ctx = TestContext::new().await;
    seed_script_interpreter_setting_definitions(&ctx).await;

    // The catalog-settings admin used by the other tests lacks system settings.
    let read_error = ctx
        .app
        .get_script_interpreter_settings(&admin())
        .await
        .expect_err("reading requires system settings");
    assert!(
        matches!(read_error, scryer_application::AppError::Unauthorized(_)),
        "unexpected error: {read_error:?}"
    );
    let update_error = ctx
        .app
        .update_script_interpreter_settings(
            &admin(),
            scryer_application::UpdateScriptInterpreterSettings {
                python: Some(Some("/opt/synthetic/python3".to_string())),
                ..Default::default()
            },
        )
        .await
        .expect_err("updating requires system settings");
    assert!(
        matches!(update_error, scryer_application::AppError::Unauthorized(_)),
        "unexpected error: {update_error:?}"
    );
}

#[tokio::test]
async fn script_interpreter_pins_must_be_absolute_paths_or_command_names() {
    let ctx = TestContext::new().await;
    seed_script_interpreter_setting_definitions(&ctx).await;
    let mut system_admin = admin();
    system_admin.authorization.app =
        AppPermissionMask::from_permissions([AppPermission::ManageSystemSettings]);

    for rejected in ["bin/python3", "./python3", "..\\tools\\pwsh.exe"] {
        let error = ctx
            .app
            .update_script_interpreter_settings(
                &system_admin,
                scryer_application::UpdateScriptInterpreterSettings {
                    go: Some(Some("/opt/synthetic/go".to_string())),
                    python: Some(Some(rejected.to_string())),
                    ..Default::default()
                },
            )
            .await
            .expect_err("relative interpreter paths are rejected");
        assert!(
            matches!(error, scryer_application::AppError::Validation(_)),
            "{rejected:?} gave {error:?}"
        );
    }
    // A rejected update writes none of its pins.
    let settings = ctx
        .app
        .get_script_interpreter_settings(&system_admin)
        .await
        .expect("read settings");
    assert_eq!(settings, Default::default());

    let settings = ctx
        .app
        .update_script_interpreter_settings(
            &system_admin,
            scryer_application::UpdateScriptInterpreterSettings {
                python: Some(Some("python3".to_string())),
                go: Some(Some("/opt/synthetic/go".to_string())),
                ..Default::default()
            },
        )
        .await
        .expect("absolute paths and command names are accepted");
    assert_eq!(settings.python, Some(std::path::PathBuf::from("python3")));
    assert_eq!(
        settings.go,
        Some(std::path::PathBuf::from("/opt/synthetic/go"))
    );
}

/// A file script with a `.py` entry point is launched through the configured
/// Python interpreter.
#[cfg(unix)]
#[tokio::test]
async fn file_python_script_runs_through_the_configured_interpreter() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-pp-test", "Test Movie", MediaFacet::Movie).await;
    seed_script_interpreter_setting_definitions(&ctx).await;

    let script_dir = tempfile::tempdir().expect("tempdir");
    let fake_python = script_dir.path().join("fake-python");
    write_executable_script(
        &fake_python,
        "#!/bin/sh\nfor a in \"$@\"; do printf 'arg=%s\\n' \"$a\"; done\n",
    );
    let script_path = script_dir.path().join("synthetic-job.py");
    std::fs::write(&script_path, "print('synthetic')\n").expect("write script");

    let mut system_admin = admin();
    system_admin.authorization.app =
        AppPermissionMask::from_permissions([AppPermission::ManageSystemSettings]);
    ctx.app
        .update_script_interpreter_settings(
            &system_admin,
            scryer_application::UpdateScriptInterpreterSettings {
                python: Some(Some(fake_python.to_string_lossy().into_owned())),
                ..Default::default()
            },
        )
        .await
        .expect("configure python interpreter");

    let script_id = create_script_with_type(
        &ctx,
        MediaFacet::Movie,
        ScriptType::File,
        script_path.to_str().expect("utf-8 script path"),
        300,
        true,
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");
    run_post_processing(movie_context(&ctx.app, &dest_file))
        .await
        .expect("run");

    let runs = ctx
        .app
        .list_post_processing_script_runs(&admin(), &script_id, 1)
        .await
        .expect("list script runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, ScriptRunStatus::Success);
    assert_eq!(
        runs[0].stdout_tail.as_deref(),
        Some(format!("arg={}", script_path.display()).as_str())
    );
}
fn system_admin() -> User {
    let mut user = admin();
    user.authorization.app = AppPermissionMask::from_permissions([
        AppPermission::ManageCatalogSettings,
        AppPermission::ManageSystemSettings,
    ]);
    user
}

fn scheduled_script(id: &str, schedule: scryer_domain::ScriptSchedule) -> PostProcessingScript {
    let now = chrono::Utc::now();
    PostProcessingScript {
        id: id.to_string(),
        name: format!("Scheduled fixture {id}"),
        description: String::new(),
        script_type: ScriptType::Inline,
        script_content: "echo scheduled-fixture".to_string(),
        applied_facets: vec![],
        execution_mode: scryer_domain::ExecutionMode::Blocking,
        timeout_secs: 60,
        priority: 0,
        enabled: true,
        debug: false,
        language: scryer_domain::ScriptLanguage::Shell,
        trigger: scryer_domain::ScriptTrigger::Schedule,
        schedule: Some(schedule),
        run_on_startup: false,
        created_at: now,
        updated_at: now,
    }
}

fn assert_unauthorized<T: std::fmt::Debug>(result: Result<T, scryer_application::AppError>) {
    match result {
        Err(scryer_application::AppError::Unauthorized(_)) => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

#[tokio::test]
async fn scheduled_scripts_require_system_settings_permission() {
    let ctx = TestContext::new().await;
    let system = system_admin();
    let catalog_only = admin();
    let interval = scryer_domain::ScriptSchedule::Interval { every_seconds: 600 };

    let stored = ctx
        .app
        .create_post_processing_script(
            &system,
            scheduled_script("pp-scheduled-guarded", interval.clone()),
        )
        .await
        .expect("system admin creates scheduled script");
    create_script(&ctx, MediaFacet::Movie, "echo import-fixture", 60, false).await;

    assert_unauthorized(
        ctx.app
            .create_post_processing_script(
                &catalog_only,
                scheduled_script("pp-scheduled-refused", interval.clone()),
            )
            .await,
    );
    let mut edited = stored.clone();
    edited.description = "edited".to_string();
    assert_unauthorized(
        ctx.app
            .update_post_processing_script(&catalog_only, edited.clone())
            .await,
    );
    // The stored trigger decides, so relabelling the row does not get past the check.
    edited.trigger = scryer_domain::ScriptTrigger::PostImport;
    assert_unauthorized(
        ctx.app
            .update_post_processing_script(&catalog_only, edited)
            .await,
    );
    assert_unauthorized(
        ctx.app
            .toggle_post_processing_script(&catalog_only, &stored.id)
            .await,
    );
    assert_unauthorized(
        ctx.app
            .delete_post_processing_script(&catalog_only, &stored.id)
            .await,
    );
    assert_unauthorized(
        ctx.app
            .validate_script_schedule(&catalog_only, &interval)
            .await,
    );
    assert_unauthorized(
        ctx.app
            .list_post_processing_scripts_by_trigger(
                &catalog_only,
                scryer_domain::ScriptTrigger::Schedule,
            )
            .await,
    );

    let visible = ctx
        .app
        .list_post_processing_scripts(&catalog_only)
        .await
        .expect("catalog admin lists scripts");
    assert!(!visible.is_empty(), "import scripts stay visible");
    assert!(
        visible
            .iter()
            .all(|script| script.trigger == scryer_domain::ScriptTrigger::PostImport),
        "scheduled scripts are hidden from a catalog-only actor"
    );
    let all = ctx
        .app
        .list_post_processing_scripts(&system)
        .await
        .expect("system admin lists scripts");
    assert!(all.iter().any(|script| script.id == stored.id));

    let still_enabled = ctx
        .app
        .list_post_processing_scripts_by_trigger(&system, scryer_domain::ScriptTrigger::Schedule)
        .await
        .expect("list scheduled");
    assert!(
        still_enabled
            .iter()
            .any(|script| script.id == stored.id && script.enabled)
    );
}

#[tokio::test]
async fn trigger_cannot_change_on_update() {
    let ctx = TestContext::new().await;
    let system = system_admin();
    let stored = ctx
        .app
        .create_post_processing_script(
            &system,
            scheduled_script("pp-trigger-fixed", scryer_domain::ScriptSchedule::Manual),
        )
        .await
        .expect("create scheduled script");
    let mut relabelled = stored;
    relabelled.trigger = scryer_domain::ScriptTrigger::PostImport;
    match ctx
        .app
        .update_post_processing_script(&system, relabelled)
        .await
    {
        Err(scryer_application::AppError::Validation(_)) => {}
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn cron_schedule_that_never_fires_is_rejected_on_create() {
    let ctx = TestContext::new().await;
    let result = ctx
        .app
        .create_post_processing_script(
            &system_admin(),
            scheduled_script(
                "pp-cron-never",
                scryer_domain::ScriptSchedule::Cron {
                    expression: "0 0 30 2 *".to_string(),
                },
            ),
        )
        .await;
    match result {
        Err(scryer_application::AppError::Validation(_)) => {}
        other => panic!("expected Validation, got {other:?}"),
    }
}
