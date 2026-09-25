#![recursion_limit = "256"]

mod common;

use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use common::TestContext;
use scryer_application::recycle_bin::{RecycleBinConfig, RecycleManifest, recycle_file};
use scryer_application::{
    AppError, InsertMediaFileInput, JobKey, JobRunStatus, LibraryRootDraft, MediaFileRepository,
    MediaFileRole, RECYCLE_BIN_ENABLED_KEY, RECYCLE_BIN_PATH_KEY, RECYCLE_BIN_RETENTION_DAYS_KEY,
    SETTINGS_SCOPE_MEDIA, SETTINGS_SOURCE_TYPED_GRAPHQL, ShowRepository, TitleRepository,
    UpdateRecycleBinSettings,
};
use scryer_domain::{
    AppPermission, AppPermissionMask, Collection, CollectionType, Id, Library, LibraryGrant,
    LibraryPermission, LibraryPermissionMask, MediaFacet, Title, User, UserAuthorization,
};
use scryer_infrastructure_sql::types::SettingDefinitionSeed;
use serde_json::{Value, json};
use tokio::time::{Duration, timeout};

async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

fn actor(
    username: &str,
    app_permissions: impl IntoIterator<Item = AppPermission>,
    library_permissions: impl IntoIterator<Item = (String, LibraryPermissionMask)>,
) -> User {
    let mut user = User::new_admin(username);
    user.authorization = UserAuthorization {
        app: AppPermissionMask::from_permissions(app_permissions),
        libraries: library_permissions.into_iter().collect::<HashMap<_, _>>(),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    user
}

fn config_actor() -> User {
    actor(
        "config",
        [AppPermission::ManageSystemSettings],
        std::iter::empty(),
    )
}

fn catalog_actor() -> User {
    actor(
        "catalog",
        [AppPermission::ManageCatalogSettings],
        std::iter::empty(),
    )
}

fn manage_titles_actor(username: &str, library_ids: &[String]) -> User {
    actor(
        username,
        std::iter::empty(),
        library_ids.iter().map(|library_id| {
            (
                library_id.clone(),
                LibraryPermissionMask::from_permissions([LibraryPermission::ManageTitles]),
            )
        }),
    )
}

async fn persisted_manage_titles_actor(
    ctx: &TestContext,
    username: &str,
    library_ids: &[String],
) -> User {
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default admin");
    let created = ctx
        .app
        .create_user(
            &admin,
            username.to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            library_ids
                .iter()
                .map(|library_id| LibraryGrant {
                    user_id: String::new(),
                    library_id: library_id.clone(),
                    permissions: LibraryPermissionMask::from_permissions([
                        LibraryPermission::ManageTitles,
                    ]),
                })
                .collect(),
        )
        .await
        .expect("create persisted title manager");
    let mut actor = manage_titles_actor(username, library_ids);
    actor.id = created.id;
    actor
}

fn no_permission_actor() -> User {
    actor("none", std::iter::empty(), std::iter::empty())
}

async fn seed_recycle_bin_setting_definition(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "media".into(),
                scope: SETTINGS_SCOPE_MEDIA.into(),
                key_name: RECYCLE_BIN_ENABLED_KEY.into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: SETTINGS_SCOPE_MEDIA.into(),
                key_name: RECYCLE_BIN_PATH_KEY.into(),
                data_type: "string".into(),
                default_value_json: "\"\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: SETTINGS_SCOPE_MEDIA.into(),
                key_name: RECYCLE_BIN_RETENTION_DAYS_KEY.into(),
                data_type: "number".into(),
                default_value_json: "7".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed recycle bin setting definition");
}

fn disabled_settings() -> UpdateRecycleBinSettings {
    UpdateRecycleBinSettings {
        enabled: Some(false),
        ..Default::default()
    }
}

async fn set_custom_recycle_bin_path(ctx: &TestContext, path: &Path) {
    ctx.settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_MEDIA,
            RECYCLE_BIN_PATH_KEY,
            None,
            json!(path.to_string_lossy().to_string()).to_string(),
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            None,
        )
        .await
        .expect("set custom recycle bin path");
}

async fn seed_library(ctx: &TestContext, name: &str, root: &Path) -> Library {
    ctx.app
        .create_library(
            &catalog_actor(),
            MediaFacet::Movie,
            name.to_string(),
            vec![LibraryRootDraft {
                path: root.to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library")
}

async fn seed_title(ctx: &TestContext, id: &str, library: &Library) {
    seed_title_with_folder_path(ctx, id, library, None).await;
}

async fn seed_title_with_folder_path(
    ctx: &TestContext,
    id: &str,
    library: &Library,
    folder_path: Option<String>,
) {
    let title = Title {
        id: id.to_string(),
        name: format!("{} Title", library.name),
        facet: MediaFacet::Movie,
        library_id: library.id.clone(),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
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
        // Root ids are allocated, not derived from a path, so take the library's own.
        root_folder_id: library
            .roots
            .first()
            .map(|root| root.id.clone())
            .unwrap_or_else(|| format!("missing-root-for-{}", library.id)),
        folder_path,
    };
    TitleRepository::create(&ctx.titles, title)
        .await
        .expect("seed title");
}

async fn seed_recycled_file(root: &Path, title_id: &str, name: &str) -> String {
    seed_recycled_file_in_bin(root, &root.join(".scryer-recycle"), title_id, name).await
}

async fn seed_recycled_file_in_bin(
    root: &Path,
    recycle_base_path: &Path,
    title_id: &str,
    name: &str,
) -> String {
    let source_path = root.join(format!("{name}.mkv"));
    std::fs::write(&source_path, format!("{name} content")).expect("write source file");
    let config = RecycleBinConfig {
        enabled: true,
        base_path: recycle_base_path.to_path_buf(),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![root.to_path_buf()],
    };
    let result = recycle_file(
        &config,
        &source_path,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: source_path.to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 128,
            title_id: Some(title_id.to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("recycle file")
    .expect("file recycled");

    result
        .recycled_path
        .parent()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .expect("entry id")
}

#[tokio::test]
async fn recycle_bin_settings_permissions_are_split_between_read_and_update() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Movies A", root.path()).await;
    let manage_actor = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    let config_actor = config_actor();

    assert!(
        ctx.app
            .get_recycle_bin_settings(&manage_actor)
            .await
            .expect("manage-title users can read")
            .enabled
    );
    assert!(
        ctx.app
            .get_recycle_bin_settings(&config_actor)
            .await
            .expect("config users can read")
            .enabled
    );
    assert!(
        ctx.app
            .get_recycle_bin_settings(&no_permission_actor())
            .await
            .is_err(),
        "users without config or manage-title access cannot read"
    );
    assert!(
        ctx.app
            .update_recycle_bin_settings(&manage_actor, disabled_settings(),)
            .await
            .is_err(),
        "manage-title users cannot update the setting"
    );

    let updated = ctx
        .app
        .update_recycle_bin_settings(&config_actor, disabled_settings())
        .await
        .expect("config user updates setting");
    assert!(!updated.enabled);
}

fn default_bin_for(library: &Library) -> String {
    Path::new(&library.roots[0].path)
        .join(".scryer-recycle")
        .to_string_lossy()
        .into_owned()
}

fn bin_settings(path: Option<&str>, retention_days: i64) -> UpdateRecycleBinSettings {
    UpdateRecycleBinSettings {
        enabled: None,
        path: path.map(|path| Some(path.to_string())),
        retention_days: Some(retention_days),
    }
}

#[tokio::test]
async fn recycle_bin_settings_round_trip_custom_path_and_retention() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let bin = tempfile::tempdir().expect("custom bin");
    let bin_path = bin.path().to_string_lossy().into_owned();
    let library = seed_library(&ctx, "Movies A", root.path()).await;
    let config_actor = config_actor();

    let defaults = ctx
        .app
        .get_recycle_bin_settings(&config_actor)
        .await
        .expect("read defaults");
    assert_eq!(defaults.path, None);
    assert_eq!(defaults.retention_days, 7);
    // The test context seeds libraries of its own; only this library's bin is asserted.
    assert!(
        defaults
            .effective_paths
            .contains(&default_bin_for(&library))
    );
    assert_eq!(defaults.validation_error, None);

    let updated = ctx
        .app
        .update_recycle_bin_settings(
            &config_actor,
            bin_settings(Some(&format!("  {bin_path}  ")), 14),
        )
        .await
        .expect("save custom path");
    assert_eq!(updated.path.as_deref(), Some(bin_path.as_str()));
    assert_eq!(updated.retention_days, 14);
    assert_eq!(updated.effective_paths, vec![bin_path.clone()]);
    assert_eq!(updated.validation_error, None);
    assert_eq!(
        ctx.app
            .get_recycle_bin_settings(&config_actor)
            .await
            .expect("read back"),
        updated
    );

    // The bin itself resolves to the saved values.
    let configs = ctx
        .app
        .recycle_bin_configs_for_media_roots(vec![library.roots[0].path.clone()])
        .await;
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].1.base_path, bin.path());
    assert_eq!(configs[0].1.retention_days, 14);
    assert!(configs[0].1.cleanup_enabled);

    let cleared = ctx
        .app
        .update_recycle_bin_settings(&config_actor, bin_settings(Some("   "), 14))
        .await
        .expect("blank path clears the custom bin");
    assert_eq!(cleared.path, None);
    assert!(cleared.effective_paths.contains(&default_bin_for(&library)));
    assert!(!cleared.effective_paths.contains(&bin_path));
    let configs = ctx
        .app
        .recycle_bin_configs_for_media_roots(vec![library.roots[0].path.clone()])
        .await;
    assert_eq!(
        configs[0].1.base_path.to_string_lossy(),
        default_bin_for(&library)
    );
}

#[tokio::test]
async fn recycle_bin_settings_partial_updates_keep_omitted_values() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let bin = tempfile::tempdir().expect("custom bin");
    let bin_path = bin.path().to_string_lossy().into_owned();
    let library = seed_library(&ctx, "Movies A", root.path()).await;
    let manager = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    let config_actor = config_actor();

    ctx.app
        .update_recycle_bin_settings(&config_actor, bin_settings(Some(&bin_path), 30))
        .await
        .expect("save custom bin");

    let toggled = ctx
        .app
        .update_recycle_bin_settings(&config_actor, disabled_settings())
        .await
        .expect("toggle alone");
    assert!(!toggled.enabled);
    assert_eq!(toggled.path.as_deref(), Some(bin_path.as_str()));
    assert_eq!(toggled.retention_days, 30);

    let retention_only = ctx
        .app
        .update_recycle_bin_settings(
            &config_actor,
            UpdateRecycleBinSettings {
                retention_days: Some(20),
                ..Default::default()
            },
        )
        .await
        .expect("retention alone");
    assert!(!retention_only.enabled);
    assert_eq!(retention_only.path.as_deref(), Some(bin_path.as_str()));
    assert_eq!(retention_only.retention_days, 20);

    // A root added later inside the stored bin invalidates it, but an update
    // that does not touch the path still succeeds.
    let nested_root = bin.path().join("nested-library");
    std::fs::create_dir(&nested_root).expect("nested root");
    seed_library(&ctx, "Movies Nested", &nested_root).await;
    let admin_view = ctx
        .app
        .get_recycle_bin_settings(&config_actor)
        .await
        .expect("admin read");
    let admin_error = admin_view
        .validation_error
        .expect("stored bin now conflicts");
    assert!(admin_error.contains("nested-library"), "{admin_error}");

    let reenabled = ctx
        .app
        .update_recycle_bin_settings(
            &config_actor,
            UpdateRecycleBinSettings {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("toggle is not blocked by the stored path");
    assert!(reenabled.enabled);
    assert_eq!(reenabled.path.as_deref(), Some(bin_path.as_str()));

    let manager_view = ctx
        .app
        .get_recycle_bin_settings(&manager)
        .await
        .expect("manager read");
    assert_eq!(
        manager_view.validation_error.as_deref(),
        Some("recycle bin path conflicts with a library root")
    );
}

#[tokio::test]
async fn recycle_bin_settings_reject_invalid_path_and_retention_without_writing() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Movies A", root.path()).await;
    let root_path = library.roots[0].path.clone();
    let config_actor = config_actor();
    let before = ctx
        .app
        .get_recycle_bin_settings(&config_actor)
        .await
        .expect("read defaults");

    let inside_root = Path::new(&root_path)
        .join("bin")
        .to_string_lossy()
        .into_owned();
    let containing_root = Path::new(&root_path)
        .parent()
        .expect("root has a parent")
        .to_string_lossy()
        .into_owned();
    for (path, expected) in [
        ("relative/bin".to_string(), "must be absolute"),
        (inside_root, root_path.as_str()),
        (root_path.clone(), root_path.as_str()),
        (containing_root, root_path.as_str()),
    ] {
        match ctx
            .app
            .update_recycle_bin_settings(&config_actor, bin_settings(Some(&path), 7))
            .await
        {
            Err(AppError::Validation(message)) => assert!(
                message.contains(expected),
                "error for {path} should mention {expected}: {message}"
            ),
            other => panic!("path {path} should be rejected, got {other:?}"),
        }
    }

    for retention_days in [-1, 0, 3651] {
        assert!(
            matches!(
                ctx.app
                    .update_recycle_bin_settings(&config_actor, bin_settings(None, retention_days))
                    .await,
                Err(AppError::Validation(_))
            ),
            "retention {retention_days} should be rejected"
        );
    }

    assert_eq!(
        ctx.app
            .get_recycle_bin_settings(&config_actor)
            .await
            .expect("read after rejections"),
        before,
        "rejected updates must not write anything"
    );
}

#[tokio::test]
async fn recycle_bin_settings_list_only_bins_of_manageable_libraries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let library_a = seed_library(&ctx, "Movies A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Movies B", root_b.path()).await;
    let manager = manage_titles_actor("manager", std::slice::from_ref(&library_a.id));

    let all = ctx
        .app
        .get_recycle_bin_settings(&config_actor())
        .await
        .expect("config read")
        .effective_paths;
    assert!(all.contains(&default_bin_for(&library_a)));
    assert!(all.contains(&default_bin_for(&library_b)));

    let scoped = ctx
        .app
        .get_recycle_bin_settings(&manager)
        .await
        .expect("manager read");
    assert_eq!(scoped.effective_paths, vec![default_bin_for(&library_a)]);
}

#[tokio::test]
async fn graphql_recycle_bin_settings_and_scoped_item_args_work() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;

    let root = tempfile::tempdir().expect("library root");
    let bin = tempfile::tempdir().expect("custom bin");
    let bin_path = bin.path().to_string_lossy().into_owned();
    let library = seed_library(&ctx, "Movies A", root.path()).await;

    let body = gql(
        &ctx,
        "query { recycleBinSettings { enabled path retentionDays effectivePaths validationError } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    let settings = &body["data"]["recycleBinSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["path"], Value::Null);
    assert_eq!(settings["retentionDays"], 7);
    assert!(
        settings["effectivePaths"]
            .as_array()
            .expect("effective paths")
            .contains(&json!(default_bin_for(&library)))
    );
    assert_eq!(settings["validationError"], Value::Null);

    let update = r#"mutation($input: UpdateRecycleBinSettingsInput!) {
            updateRecycleBinSettings(input: $input) {
                enabled path retentionDays effectivePaths validationError
            }
        }"#;
    let body = gql(
        &ctx,
        update,
        json!({ "input": { "enabled": false, "path": bin_path, "retentionDays": 30 } }),
    )
    .await;
    assert_no_errors(&body);
    let settings = &body["data"]["updateRecycleBinSettings"];
    assert_eq!(settings["enabled"], false);
    assert_eq!(settings["path"], json!(bin_path));
    assert_eq!(settings["retentionDays"], 30);
    assert_eq!(settings["effectivePaths"], json!([bin_path]));

    // Omitted fields keep their stored values.
    let body = gql(&ctx, update, json!({ "input": { "enabled": true } })).await;
    assert_no_errors(&body);
    let settings = &body["data"]["updateRecycleBinSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["path"], json!(bin_path));
    assert_eq!(settings["retentionDays"], 30);

    let inside_root = Path::new(&library.roots[0].path)
        .join("bin")
        .to_string_lossy()
        .into_owned();
    for input in [
        json!({ "enabled": false, "path": inside_root, "retentionDays": 30 }),
        json!({ "enabled": false, "path": null, "retentionDays": -1 }),
    ] {
        let body = gql(&ctx, update, json!({ "input": input })).await;
        assert!(
            body.get("errors").is_some(),
            "invalid input {input} should be rejected: {body}"
        );
    }

    let body = gql(
        &ctx,
        update,
        json!({ "input": { "enabled": false, "path": null, "retentionDays": 30 } }),
    )
    .await;
    assert_no_errors(&body);
    let settings = &body["data"]["updateRecycleBinSettings"];
    assert_eq!(settings["path"], Value::Null);
    assert!(
        settings["effectivePaths"]
            .as_array()
            .expect("effective paths")
            .contains(&json!(default_bin_for(&library)))
    );

    let body = gql(
        &ctx,
        r#"query($libraryIds: [ID!]) {
            recycledItems(libraryIds: $libraryIds) {
                totalCount
                items { id libraryId libraryName }
            }
        }"#,
        json!({ "libraryIds": null }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["recycledItems"]["totalCount"], 0);

    let body = gql(
        &ctx,
        r#"mutation($libraryIds: [ID!]) {
            emptyRecycleBin(libraryIds: $libraryIds) {
                jobRun { id jobKey status }
            }
        }"#,
        json!({ "libraryIds": null }),
    )
    .await;
    assert_no_errors(&body);
    let run = &body["data"]["emptyRecycleBin"]["jobRun"];
    assert_eq!(run["jobKey"], "RECYCLE_BIN_PURGE");
    assert_eq!(run["status"], "RUNNING");
    let admin = ctx.app.find_or_create_default_user().await.expect("admin");
    let terminal = wait_for_terminal_job(
        &ctx,
        &admin,
        JobKey::RecycleBinPurge,
        run["id"].as_str().expect("accepted job id"),
    )
    .await;
    assert_eq!(terminal.status, JobRunStatus::Completed);
}

#[tokio::test]
async fn graphql_restore_recycled_item_returns_accepted_job_run() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "GraphQL Restore", root.path()).await;
    seed_title(&ctx, "title-graphql-restore", &library).await;
    let entry_id =
        seed_recycled_file(root.path(), "title-graphql-restore", "graphql-restore").await;

    let body = gql(
        &ctx,
        r#"mutation RestoreRecycledItem($id: ID!) {
            restoreRecycledItem(id: $id) {
                id
                jobRun { id jobKey status }
            }
        }"#,
        json!({ "id": entry_id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["restoreRecycledItem"]["id"], entry_id);
    assert_eq!(
        body["data"]["restoreRecycledItem"]["jobRun"]["jobKey"],
        "RECYCLE_BIN_RESTORE"
    );
    assert_eq!(
        body["data"]["restoreRecycledItem"]["jobRun"]["status"],
        "RUNNING"
    );

    let run_id = body["data"]["restoreRecycledItem"]["jobRun"]["id"]
        .as_str()
        .expect("accepted job id");
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default admin");
    let terminal = wait_for_terminal_job(&ctx, &admin, JobKey::RecycleBinRestore, run_id).await;
    assert_eq!(terminal.status, JobRunStatus::Completed);
    assert!(root.path().join("graphql-restore.mkv").exists());
}

/// Seed a committed entry the way the recycle-bin e2e spec's
/// `seedCommittedRecycleEntries` does: a directory named like `recycle_file`
/// names one, holding the payload plus a hand-written `manifest.json` with
/// exactly the key set the spec emits. Nothing here calls product code, which
/// is the point - it is the spec's bytes, checked against the product's
/// acceptance rules.
async fn seed_recycled_file_like_the_e2e_spec(
    root: &Path,
    title_id: &str,
    entry_id: &str,
    name: &str,
) -> String {
    let recycle_root = root.join(".scryer-recycle");
    std::fs::create_dir_all(&recycle_root).expect("create recycle root");
    let sentinel = recycle_root.join(".scryer-recycle-root");
    if !sentinel.exists() {
        std::fs::write(&sentinel, "scryer.recycle-entry.v1").expect("write sentinel");
    }

    let original_path = root.join(format!("{name}.mkv"));
    let entry_dir = recycle_root.join(entry_id);
    std::fs::create_dir_all(&entry_dir).expect("create entry dir");
    let payload = format!("scryer-e2e-empty-all-{entry_id}\n").repeat(64);
    std::fs::write(entry_dir.join(format!("{name}.mkv")), payload.as_bytes())
        .expect("write payload");

    let manifest = serde_json::json!({
        "schema": "scryer.recycle-entry.v1",
        "entry_id": entry_id,
        "source_operation_id": format!("e2e-empty-all-{entry_id}"),
        "recycled_at": Utc::now().to_rfc3339(),
        "original_path": original_path.to_string_lossy(),
        "size_bytes": payload.len(),
        "title_id": title_id,
        "media_root": root.to_string_lossy(),
        "reason": "title_deleted",
        "status": "committed",
    });
    std::fs::write(
        entry_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).expect("manifest json")
        ),
    )
    .expect("write manifest");

    entry_id.to_string()
}

/// The empty-all e2e writes its own committed entries, because by the time it
/// runs no product path is left that would recycle anything. Those entries must
/// be listed by the product exactly like ones `recycle_file` wrote - if they are
/// not, the spec is asserting against a shape the product does not accept and
/// the e2e failure is real, not a harness timeout.
#[tokio::test]
async fn hand_seeded_committed_entries_are_listed_like_product_written_ones() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Anime", root.path()).await;
    seed_title(&ctx, "title-a", &library).await;

    let product_entry = seed_recycled_file(root.path(), "title-a", "product-written").await;
    let seeded_entry = seed_recycled_file_like_the_e2e_spec(
        root.path(),
        "title-a",
        "20260913_204833089_e2e000",
        "E2E Empty All",
    )
    .await;

    let manager = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    let items = ctx
        .app
        .list_recycled_items(&manager, None)
        .await
        .expect("list recycled items");
    let ids = items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        ids.contains(&product_entry.as_str()),
        "the product-written entry is listed: {ids:?}"
    );
    assert!(
        ids.contains(&seeded_entry.as_str()),
        "the hand-seeded entry must be listed the same way: {ids:?}"
    );
}

#[tokio::test]
async fn recycled_items_are_filtered_to_manage_title_libraries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let library_a = seed_library(&ctx, "Movies A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Movies B", root_b.path()).await;
    seed_title(&ctx, "title-a", &library_a).await;
    seed_title(&ctx, "title-b", &library_b).await;
    seed_recycled_file(root_a.path(), "title-a", "movie-a").await;
    seed_recycled_file(root_b.path(), "title-b", "movie-b").await;

    let manager_a = manage_titles_actor("manager-a", std::slice::from_ref(&library_a.id));
    let manager_both = manage_titles_actor(
        "manager-both",
        &[library_a.id.clone(), library_b.id.clone()],
    );

    let items = ctx
        .app
        .list_recycled_items(&manager_a, None)
        .await
        .expect("list authorized items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].library_id, library_a.id);
    assert_eq!(items[0].library_name, library_a.name);
    let recycled_at =
        chrono::DateTime::parse_from_rfc3339(&items[0].recycled_at).expect("recycled timestamp");
    let scheduled_deletion_at =
        chrono::DateTime::parse_from_rfc3339(&items[0].scheduled_deletion_at)
            .expect("scheduled deletion timestamp");
    assert_eq!(
        scheduled_deletion_at - recycled_at,
        chrono::Duration::days(7)
    );

    let filtered_out = ctx
        .app
        .list_recycled_items(&manager_a, Some(vec![library_b.id.clone()]))
        .await
        .expect("selected unauthorized library is intersected away");
    assert!(filtered_out.is_empty());

    let selected = ctx
        .app
        .list_recycled_items(&manager_both, Some(vec![library_b.id.clone()]))
        .await
        .expect("selected authorized library is listed");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].library_id, library_b.id);

    let config_only = ctx
        .app
        .list_recycled_items(&config_actor(), None)
        .await
        .expect("config-only user can access page but not items");
    assert!(config_only.is_empty());
}

#[tokio::test]
async fn empty_recycle_bin_only_purges_selected_authorized_libraries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let library_a = seed_library(&ctx, "Movies A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Movies B", root_b.path()).await;
    seed_title(&ctx, "title-a", &library_a).await;
    seed_title(&ctx, "title-b", &library_b).await;
    seed_recycled_file(root_a.path(), "title-a", "movie-a").await;
    seed_recycled_file(root_b.path(), "title-b", "movie-b").await;

    let manager_both = manage_titles_actor(
        "manager-both",
        &[library_a.id.clone(), library_b.id.clone()],
    );
    let removed = ctx
        .app
        .empty_recycle_bin(&manager_both, Some(vec![library_a.id.clone()]))
        .await
        .expect("empty selected library");
    assert_eq!(removed, 1);

    let remaining = ctx
        .app
        .list_recycled_items(&manager_both, None)
        .await
        .expect("list remaining items");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].library_id, library_b.id);
}

#[tokio::test]
async fn empty_recycle_bin_job_returns_before_purge_and_preserves_unselected_files() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("root a");
    let root_b = tempfile::tempdir().expect("root b");
    let library_a = seed_library(&ctx, "Async A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Async B", root_b.path()).await;
    seed_title(&ctx, "async-a", &library_a).await;
    seed_title(&ctx, "async-b", &library_b).await;
    let entry_a = seed_recycled_file(root_a.path(), "async-a", "selected").await;
    let entry_b = seed_recycled_file(root_b.path(), "async-b", "unselected").await;
    let unrelated = root_a.path().join("keep.mkv");
    std::fs::write(&unrelated, b"unrelated file").expect("unrelated fixture");
    let malformed = root_a.path().join(".scryer-recycle").join("unrecognized");
    std::fs::create_dir(&malformed).expect("unrecognized entry");
    std::fs::write(malformed.join("keep.mkv"), b"unrecognized file").unwrap();
    let manager = persisted_manage_titles_actor(
        &ctx,
        "async-manager",
        &[library_a.id.clone(), library_b.id.clone()],
    )
    .await;

    assert!(
        ctx.app
            .start_empty_recycle_bin_job(&no_permission_actor(), None)
            .await
            .is_err()
    );
    let accepted = ctx
        .app
        .start_empty_recycle_bin_job(&manager, Some(vec![library_a.id.clone()]))
        .await
        .expect("accept empty job");
    assert_eq!(accepted.status, JobRunStatus::Running);
    // This single-threaded runtime has not yielded to the spawned purge yet.
    assert!(
        root_a
            .path()
            .join(".scryer-recycle")
            .join(&entry_a)
            .exists()
    );
    let admin = ctx.app.find_or_create_default_user().await.expect("admin");
    let terminal = wait_for_terminal_job(&ctx, &admin, JobKey::RecycleBinPurge, &accepted.id).await;
    assert_eq!(terminal.status, JobRunStatus::Completed);
    assert!(!root_a.path().join(".scryer-recycle").join(entry_a).exists());
    assert!(root_b.path().join(".scryer-recycle").join(entry_b).exists());
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated file");
    assert_eq!(
        std::fs::read(malformed.join("keep.mkv")).unwrap(),
        b"unrecognized file"
    );
    let summary: Value = serde_json::from_str(terminal.summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary["succeeded"], 1);
    assert_eq!(summary["action"], "empty");

    // Resolving an inaccessible selection to an empty set must not expand to all libraries.
    let accepted = ctx
        .app
        .start_empty_recycle_bin_job(&manager, Some(vec!["unknown-library".into()]))
        .await
        .expect("accept empty scope");
    let terminal = wait_for_terminal_job(&ctx, &admin, JobKey::RecycleBinPurge, &accepted.id).await;
    assert_eq!(terminal.status, JobRunStatus::Completed);
    assert_eq!(
        ctx.app
            .list_recycled_items(&manager, None)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn custom_recycle_bin_path_lists_entries_once_across_libraries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let recycle_root = tempfile::tempdir().expect("custom recycle root");
    set_custom_recycle_bin_path(&ctx, recycle_root.path()).await;

    let library_a = seed_library(&ctx, "Movies A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Movies B", root_b.path()).await;
    seed_title(&ctx, "title-a", &library_a).await;
    seed_title(&ctx, "title-b", &library_b).await;
    seed_recycled_file_in_bin(root_a.path(), recycle_root.path(), "title-a", "movie-a").await;
    seed_recycled_file_in_bin(root_b.path(), recycle_root.path(), "title-b", "movie-b").await;

    let manager_both = manage_titles_actor(
        "manager-both",
        &[library_a.id.clone(), library_b.id.clone()],
    );
    let items = ctx
        .app
        .list_recycled_items(&manager_both, None)
        .await
        .expect("list custom recycle bin");
    assert_eq!(items.len(), 2);

    let removed = ctx
        .app
        .empty_recycle_bin(&manager_both, Some(vec![library_a.id.clone()]))
        .await
        .expect("empty selected library");
    assert_eq!(removed, 1);

    let remaining = ctx
        .app
        .list_recycled_items(&manager_both, None)
        .await
        .expect("list remaining custom recycle bin");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].library_id, library_b.id);
}

#[tokio::test]
async fn delete_title_purges_recycle_entries_from_all_library_roots() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let library = ctx
        .app
        .create_library(
            &catalog_actor(),
            MediaFacet::Movie,
            "Movies Multi Root".to_string(),
            vec![
                LibraryRootDraft {
                    path: root_a.path().to_string_lossy().to_string(),
                    is_default: true,
                },
                LibraryRootDraft {
                    path: root_b.path().to_string_lossy().to_string(),
                    is_default: false,
                },
            ],
            None,
        )
        .await
        .expect("create multi-root library");
    seed_title(&ctx, "title-multi-root-delete", &library).await;
    let entry_a = seed_recycled_file(root_a.path(), "title-multi-root-delete", "movie-a").await;
    let entry_b = seed_recycled_file(root_b.path(), "title-multi-root-delete", "movie-b").await;

    let manager = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    ctx.app
        .delete_title(&manager, "title-multi-root-delete", false, None)
        .await
        .expect("delete title");

    assert!(
        !root_a.path().join(".scryer-recycle").join(entry_a).exists(),
        "old-root recycle entry should be purged"
    );
    assert!(
        !root_b.path().join(".scryer-recycle").join(entry_b).exists(),
        "new-root recycle entry should be purged"
    );
}

#[tokio::test]
async fn recycle_bin_config_resolution_deduplicates_roots_by_base_path() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let root_path = root.path().to_string_lossy().to_string();

    let configs = ctx
        .app
        .recycle_bin_configs_for_media_roots(vec![root_path.clone(), format!("{root_path}/")])
        .await;
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].0, root_path);
    assert_eq!(configs[0].1.base_path, root.path().join(".scryer-recycle"));
}

#[tokio::test]
async fn recycle_bin_config_resolution_keeps_distinct_default_roots() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");

    let configs = ctx
        .app
        .recycle_bin_configs_for_media_roots(vec![
            root_a.path().to_string_lossy().to_string(),
            root_b.path().to_string_lossy().to_string(),
        ])
        .await;
    assert_eq!(configs.len(), 2);
    assert!(
        configs
            .iter()
            .any(|(_, config)| { config.base_path == root_a.path().join(".scryer-recycle") })
    );
    assert!(
        configs
            .iter()
            .any(|(_, config)| { config.base_path == root_b.path().join(".scryer-recycle") })
    );
}

#[tokio::test]
async fn recycle_bin_config_resolution_deduplicates_custom_path_across_roots() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let recycle_root = tempfile::tempdir().expect("custom recycle root");
    set_custom_recycle_bin_path(&ctx, recycle_root.path()).await;

    let configs = ctx
        .app
        .recycle_bin_configs_for_media_roots(vec![
            root_a.path().to_string_lossy().to_string(),
            root_b.path().to_string_lossy().to_string(),
        ])
        .await;
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].0, "");
    assert_eq!(configs[0].1.base_path, recycle_root.path());
    assert_eq!(configs[0].1.source_roots.len(), 2);
}

#[tokio::test]
async fn restoring_conflict_scans_title_and_tracks_restored_file_as_additional() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let title_dir = root.path().join("Restore Movie Title (2024)");
    std::fs::create_dir(&title_dir).expect("create title folder");
    let library = seed_library(&ctx, "Restore Movie", root.path()).await;
    seed_title_with_folder_path(
        &ctx,
        "title-restore",
        &library,
        Some(title_dir.to_string_lossy().to_string()),
    )
    .await;

    let original_path = title_dir.join("Restore.Movie.Title.2024.720p.WEB-DL.mkv");
    std::fs::write(&original_path, b"recycled original").expect("write original source file");
    let recycle_result = recycle_file(
        &RecycleBinConfig {
            enabled: true,
            base_path: root.path().join(".scryer-recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root.path().to_path_buf()],
        },
        &original_path,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: original_path.to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 17,
            title_id: Some("title-restore".to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("recycle original file")
    .expect("file should be recycled");
    let entry_id = recycle_result.entry_id;
    std::fs::write(&original_path, b"current live file").expect("write replacement live file");
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: "title-restore".to_string(),
            file_path: original_path.to_string_lossy().to_string(),
            size_bytes: 17,
            role: MediaFileRole::Primary,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert current primary media file");
    ctx.shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: "title-restore".to_string(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(original_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create restore movie collection");

    let manager = persisted_manage_titles_actor(
        &ctx,
        "restore-conflict-manager",
        std::slice::from_ref(&library.id),
    )
    .await;
    let accepted = ctx
        .app
        .start_restore_recycled_item_job(&manager, &entry_id)
        .await
        .expect("start recycle restore job");
    assert_eq!(accepted.job_run.job_key, JobKey::RecycleBinRestore);
    assert_eq!(accepted.job_run.status, JobRunStatus::Running);
    let terminal = wait_for_terminal_job(
        &ctx,
        &manager,
        JobKey::RecycleBinRestore,
        &accepted.job_run.id,
    )
    .await;
    assert_eq!(terminal.status, JobRunStatus::Completed);

    let restored_path = title_dir.join("Restore.Movie.Title.2024.720p.WEB-DL-restored.mkv");
    let restored_path_string = restored_path.to_string_lossy().to_string();
    assert!(restored_path.exists(), "restore should divert to sibling");
    let files = ctx
        .media_files
        .list_media_files_for_title("title-restore")
        .await
        .expect("list title media files after restore scan");
    assert!(
        files.iter().any(|file| {
            file.file_path == restored_path_string.as_str()
                && file.role == MediaFileRole::Additional
        }),
        "title scan should track the restored sibling as an additional file: {files:?}"
    );
}

#[tokio::test]
async fn failed_restore_job_keeps_recycle_entry_available() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Restore Failure", root.path()).await;
    seed_title(&ctx, "title-restore-failure", &library).await;

    let parent = root.path().join("blocked-parent");
    std::fs::create_dir_all(&parent).expect("create source parent");
    let source_path = parent.join("Restore.Failure.mkv");
    std::fs::write(&source_path, b"recycled original").expect("write source file");
    let recycle_result = recycle_file(
        &RecycleBinConfig {
            enabled: true,
            base_path: root.path().join(".scryer-recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root.path().to_path_buf()],
        },
        &source_path,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: source_path.to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 17,
            title_id: Some("title-restore-failure".to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("recycle source file")
    .expect("file should be recycled");

    if parent.exists() {
        std::fs::remove_dir(&parent).expect("remove empty source parent");
    }
    std::fs::write(&parent, b"not a directory").expect("block restore parent");
    let manager = persisted_manage_titles_actor(
        &ctx,
        "restore-failure-manager",
        std::slice::from_ref(&library.id),
    )
    .await;
    let accepted = ctx
        .app
        .start_restore_recycled_item_job(&manager, &recycle_result.entry_id)
        .await
        .expect("accept restore job before runtime failure");

    let terminal = wait_for_terminal_job(
        &ctx,
        &manager,
        JobKey::RecycleBinRestore,
        &accepted.job_run.id,
    )
    .await;
    assert_eq!(terminal.status, JobRunStatus::Failed);
    assert!(
        recycle_result.entry_dir.exists(),
        "failed restore should retain the recycle entry"
    );
    assert!(
        recycle_result.recycled_path.exists(),
        "failed restore should retain the recycled file"
    );
}

#[tokio::test]
async fn restoring_out_of_root_manifest_is_refused_and_entry_remains() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let outside = tempfile::tempdir().expect("outside root");
    let library = seed_library(&ctx, "Restore Guard", root.path()).await;
    seed_title(&ctx, "title-restore-guard", &library).await;

    let source_path = root.path().join("Out.Of.Root.Movie.mkv");
    let outside_path = outside.path().join("Out.Of.Root.Movie.mkv");
    std::fs::write(&source_path, b"recycled original").expect("write source file");
    let recycle_result = recycle_file(
        &RecycleBinConfig {
            enabled: true,
            base_path: root.path().join(".scryer-recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root.path().to_path_buf()],
        },
        &source_path,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: outside_path.to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 17,
            title_id: Some("title-restore-guard".to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("recycle source file")
    .expect("file should be recycled");
    let manager = manage_titles_actor("manager", std::slice::from_ref(&library.id));

    let error = ctx
        .app
        .restore_recycled_item(&manager, &recycle_result.entry_id)
        .await
        .expect_err("out-of-root manifest should be refused");
    assert!(
        error
            .to_string()
            .contains("outside the resolved library roots"),
        "unexpected error: {error}"
    );
    assert!(
        !outside_path.exists(),
        "restore must not create the out-of-root destination"
    );
    assert!(
        recycle_result.entry_dir.exists(),
        "refused restore should keep the recycle entry"
    );
    assert!(
        recycle_result.recycled_path.exists(),
        "refused restore should keep the recycled file"
    );
}

#[tokio::test]
async fn restoring_root_equal_manifest_is_refused_and_entry_remains() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Restore Root Guard", root.path()).await;
    seed_title(&ctx, "title-restore-root-guard", &library).await;

    let source_path = root.path().join("Root.Equal.Movie.mkv");
    std::fs::write(&source_path, b"recycled original").expect("write source file");
    let recycle_result = recycle_file(
        &RecycleBinConfig {
            enabled: true,
            base_path: root.path().join(".scryer-recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![root.path().to_path_buf()],
        },
        &source_path,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: root.path().to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 17,
            title_id: Some("title-restore-root-guard".to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("recycle source file")
    .expect("file should be recycled");
    let manager = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    let root_name = root
        .path()
        .file_name()
        .expect("temp root has file name")
        .to_string_lossy();
    let escaped_sibling = root.path().with_file_name(format!("{root_name}-restored"));

    let error = ctx
        .app
        .restore_recycled_item(&manager, &recycle_result.entry_id)
        .await
        .expect_err("root-equal manifest should be refused");
    assert!(
        error
            .to_string()
            .contains("outside the resolved library roots"),
        "unexpected error: {error}"
    );
    assert!(
        !escaped_sibling.exists(),
        "restore must not create a -restored sibling outside the root"
    );
    assert!(
        recycle_result.entry_dir.exists(),
        "refused restore should keep the recycle entry"
    );
    assert!(
        recycle_result.recycled_path.exists(),
        "refused restore should keep the recycled file"
    );
}

#[tokio::test]
async fn disabled_recycle_bin_paths_are_inert_and_direct_delete_new_files() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Movies A", root.path()).await;
    seed_title(&ctx, "title-a", &library).await;
    let entry_id = seed_recycled_file(root.path(), "title-a", "movie-a").await;
    let manager = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    let config_actor = config_actor();

    ctx.app
        .update_recycle_bin_settings(&config_actor, disabled_settings())
        .await
        .expect("disable recycle bin");

    let items = ctx
        .app
        .list_recycled_items(&manager, None)
        .await
        .expect("disabled list is inert");
    assert!(items.is_empty());
    assert_eq!(
        ctx.app
            .empty_recycle_bin(&manager, None)
            .await
            .expect("disabled empty is inert"),
        0
    );
    assert!(
        ctx.app
            .restore_recycled_item(&manager, &entry_id)
            .await
            .is_err(),
        "disabled restore should not traverse stored entries"
    );

    let new_source = root.path().join("new-delete.mkv");
    std::fs::write(&new_source, b"delete directly").expect("write new source");
    let config = ctx
        .app
        .recycle_bin_config_for_media_root(Some(root.path().to_string_lossy().as_ref()))
        .await;
    let result = recycle_file(
        &config,
        &new_source,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: new_source.to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 64,
            title_id: Some("title-a".to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("direct delete succeeds");
    assert!(result.is_none());
    assert!(!new_source.exists());
}

async fn wait_for_terminal_job(
    ctx: &TestContext,
    actor: &User,
    job_key: JobKey,
    run_id: &str,
) -> scryer_application::JobRun {
    let run_id = run_id.to_string();
    timeout(common::WAIT_UNTIL_TIMEOUT, async {
        loop {
            let run = ctx
                .app
                .list_job_runs(actor, job_key, 20)
                .await
                .expect("list recycle-bin job runs")
                .into_iter()
                .find(|run| run.id == run_id);
            if let Some(run) = run
                && run.status.is_terminal()
            {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("recycle-bin job should reach a terminal status")
}

#[tokio::test]
async fn graphql_restore_recycled_items_returns_accepted_batch_job_run() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "GraphQL Batch Restore", root.path()).await;
    seed_title(&ctx, "title-graphql-batch-restore", &library).await;
    let first_entry = seed_recycled_file(
        root.path(),
        "title-graphql-batch-restore",
        "graphql-batch-first",
    )
    .await;
    let second_entry = seed_recycled_file(
        root.path(),
        "title-graphql-batch-restore",
        "graphql-batch-second",
    )
    .await;

    let preview = gql(
        &ctx,
        r#"query PreviewRestoreRecycledItems($ids: [ID!]!) {
            previewRestoreRecycledItems(ids: $ids) {
                fingerprint
                items { id destinationOccupied }
            }
            recycledItems { items { id titleName } }
        }"#,
        json!({ "ids": [first_entry, second_entry] }),
    )
    .await;
    assert_no_errors(&preview);
    assert_eq!(
        preview["data"]["previewRestoreRecycledItems"]["items"]
            .as_array()
            .expect("preview items")
            .len(),
        2
    );
    assert!(
        preview["data"]["recycledItems"]["items"]
            .as_array()
            .expect("recycled items")
            .iter()
            .all(|item| item["titleName"] == "GraphQL Batch Restore Title"),
        "recycle items should expose their title name for UI grouping"
    );
    let fingerprint = preview["data"]["previewRestoreRecycledItems"]["fingerprint"]
        .as_str()
        .expect("preview fingerprint")
        .to_string();

    let body = gql(
        &ctx,
        r#"mutation RestoreRecycledItems($input: RestoreRecycledItemsInput!) {
            restoreRecycledItems(input: $input) {
                ids
                jobRun { id jobKey status }
            }
        }"#,
        json!({
            "input": {
                "ids": [first_entry, second_entry],
                "conflictPolicy": "KEEP_BOTH",
                "previewFingerprint": fingerprint,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let accepted_ids = body["data"]["restoreRecycledItems"]["ids"]
        .as_array()
        .expect("accepted ids");
    assert_eq!(accepted_ids.len(), 2);
    assert!(accepted_ids.iter().any(|id| id == &first_entry));
    assert!(accepted_ids.iter().any(|id| id == &second_entry));
    assert_eq!(
        body["data"]["restoreRecycledItems"]["jobRun"]["jobKey"],
        "RECYCLE_BIN_RESTORE"
    );
    assert_eq!(
        body["data"]["restoreRecycledItems"]["jobRun"]["status"],
        "RUNNING"
    );

    let run_id = body["data"]["restoreRecycledItems"]["jobRun"]["id"]
        .as_str()
        .expect("accepted job id");
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default admin");
    let terminal = wait_for_terminal_job(&ctx, &admin, JobKey::RecycleBinRestore, run_id).await;
    assert_eq!(terminal.status, JobRunStatus::Completed);
    assert!(root.path().join("graphql-batch-first.mkv").exists());
    assert!(root.path().join("graphql-batch-second.mkv").exists());
}

#[tokio::test]
async fn graphql_delete_recycled_items_returns_accepted_purge_job_run() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "GraphQL Batch Purge", root.path()).await;
    seed_title(&ctx, "title-graphql-batch-purge", &library).await;
    let first_entry = seed_recycled_file(
        root.path(),
        "title-graphql-batch-purge",
        "graphql-purge-first",
    )
    .await;
    let second_entry = seed_recycled_file(
        root.path(),
        "title-graphql-batch-purge",
        "graphql-purge-second",
    )
    .await;

    let body = gql(
        &ctx,
        r#"mutation DeleteRecycledItems($input: DeleteRecycledItemsInput!) {
            deleteRecycledItems(input: $input) {
                ids
                jobRun { id jobKey status }
            }
        }"#,
        json!({ "input": { "ids": [first_entry, second_entry] } }),
    )
    .await;
    assert_no_errors(&body);
    let accepted_ids = body["data"]["deleteRecycledItems"]["ids"]
        .as_array()
        .expect("accepted ids");
    assert_eq!(accepted_ids.len(), 2);
    assert!(accepted_ids.iter().any(|id| id == &first_entry));
    assert!(accepted_ids.iter().any(|id| id == &second_entry));
    assert_eq!(
        body["data"]["deleteRecycledItems"]["jobRun"]["jobKey"],
        "RECYCLE_BIN_PURGE"
    );
    assert_eq!(
        body["data"]["deleteRecycledItems"]["jobRun"]["status"],
        "RUNNING"
    );

    let run_id = body["data"]["deleteRecycledItems"]["jobRun"]["id"]
        .as_str()
        .expect("accepted job id");
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default admin");
    let terminal = wait_for_terminal_job(&ctx, &admin, JobKey::RecycleBinPurge, run_id).await;
    assert_eq!(terminal.status, JobRunStatus::Completed);
    assert!(
        ctx.app
            .list_recycled_items(&admin, None)
            .await
            .expect("list recycled items")
            .is_empty(),
        "terminal purge should remove all selected entries"
    );
}

#[tokio::test]
async fn batch_replace_existing_restores_in_place_and_recycles_the_incumbent() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Replace In Place", root.path()).await;
    seed_title(&ctx, "title-replace-in-place", &library).await;
    let entry_id =
        seed_recycled_file(root.path(), "title-replace-in-place", "replace-in-place").await;
    let original_path = root.path().join("replace-in-place.mkv");
    std::fs::write(&original_path, b"current live file").expect("write incumbent file");
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: "title-replace-in-place".to_string(),
            file_path: original_path.to_string_lossy().to_string(),
            size_bytes: 17,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert incumbent media file");
    let manager = persisted_manage_titles_actor(
        &ctx,
        "replace-in-place-manager",
        std::slice::from_ref(&library.id),
    )
    .await;
    let preview = ctx
        .app
        .preview_restore_recycled_items(&manager, vec![entry_id.clone()])
        .await
        .expect("preview replacement restore");
    assert!(preview.items[0].destination_occupied);

    let accepted = ctx
        .app
        .start_restore_recycled_items_job(
            &manager,
            vec![entry_id.clone()],
            scryer_application::RecycleRestoreConflictPolicy::ReplaceExisting,
            &preview.fingerprint,
        )
        .await
        .expect("accept replacement restore job");
    assert_eq!(accepted.job_run.status, JobRunStatus::Running);
    let terminal = wait_for_terminal_job(
        &ctx,
        &manager,
        JobKey::RecycleBinRestore,
        &accepted.job_run.id,
    )
    .await;
    assert_eq!(terminal.status, JobRunStatus::Completed);
    assert_eq!(
        std::fs::read(&original_path).expect("read restored file"),
        b"replace-in-place content"
    );
    assert!(
        !root.path().join("replace-in-place-restored.mkv").exists(),
        "replace-in-place must not create a keep-both sibling"
    );

    let items = ctx
        .app
        .list_recycled_items(&manager, None)
        .await
        .expect("list displaced incumbent");
    let displaced = items
        .iter()
        .find(|item| item.id != entry_id)
        .expect("incumbent should be recycled as a new entry");
    assert_eq!(displaced.reason, "restore_replaced");
    assert_eq!(displaced.original_path, original_path.to_string_lossy());
    let displaced_file = root
        .path()
        .join(".scryer-recycle")
        .join(&displaced.id)
        .join("replace-in-place.mkv");
    assert_eq!(
        std::fs::read(displaced_file).expect("read displaced incumbent"),
        b"current live file"
    );
}

#[tokio::test]
async fn batch_restore_rejects_stale_preview_before_queueing() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Stale Restore Preview", root.path()).await;
    seed_title(&ctx, "title-stale-restore-preview", &library).await;
    let entry_id = seed_recycled_file(
        root.path(),
        "title-stale-restore-preview",
        "stale-restore-preview",
    )
    .await;
    let manager = persisted_manage_titles_actor(
        &ctx,
        "stale-restore-preview-manager",
        std::slice::from_ref(&library.id),
    )
    .await;
    let preview = ctx
        .app
        .preview_restore_recycled_items(&manager, vec![entry_id.clone()])
        .await
        .expect("preview restore");

    std::fs::write(
        root.path().join("stale-restore-preview.mkv"),
        b"new live file",
    )
    .expect("change the restore destination after preview");
    let error = ctx
        .app
        .start_restore_recycled_items_job(
            &manager,
            vec![entry_id.clone()],
            scryer_application::RecycleRestoreConflictPolicy::KeepBoth,
            &preview.fingerprint,
        )
        .await
        .expect_err("stale preview should not queue a restore");
    assert!(error.to_string().contains("preview is stale"));
    assert!(
        root.path()
            .join(".scryer-recycle")
            .join(&entry_id)
            .join("stale-restore-preview.mkv")
            .exists(),
        "a rejected restore must retain its recycle entry"
    );
    assert!(
        ctx.app
            .list_job_runs(&manager, JobKey::RecycleBinRestore, 10)
            .await
            .expect("list restore jobs")
            .is_empty(),
        "stale validation must fail before persisting a job"
    );
}

#[tokio::test]
async fn batch_recycle_jobs_guard_duplicates_without_blocking_independent_entries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Batch Guards", root.path()).await;
    seed_title(&ctx, "title-batch-guards", &library).await;
    let duplicate_entry =
        seed_recycled_file(root.path(), "title-batch-guards", "duplicate-entry").await;
    let first_entry = seed_recycled_file(root.path(), "title-batch-guards", "first-entry").await;
    let second_entry = seed_recycled_file(root.path(), "title-batch-guards", "second-entry").await;
    let manager = persisted_manage_titles_actor(
        &ctx,
        "batch-guard-manager",
        std::slice::from_ref(&library.id),
    )
    .await;
    let duplicate_input_error = ctx
        .app
        .preview_restore_recycled_items(
            &manager,
            vec![duplicate_entry.clone(), duplicate_entry.clone()],
        )
        .await
        .expect_err("a batch must not accept duplicate entry IDs");
    assert!(duplicate_input_error.to_string().contains("more than once"));
    let duplicate_preview = ctx
        .app
        .preview_restore_recycled_items(&manager, vec![duplicate_entry.clone()])
        .await
        .expect("preview duplicate entry");

    let (first_duplicate, second_duplicate) = tokio::join!(
        ctx.app.start_restore_recycled_items_job(
            &manager,
            vec![duplicate_entry.clone()],
            scryer_application::RecycleRestoreConflictPolicy::KeepBoth,
            &duplicate_preview.fingerprint,
        ),
        ctx.app.start_restore_recycled_items_job(
            &manager,
            vec![duplicate_entry.clone()],
            scryer_application::RecycleRestoreConflictPolicy::KeepBoth,
            &duplicate_preview.fingerprint,
        ),
    );
    assert!(
        first_duplicate.is_ok() ^ second_duplicate.is_ok(),
        "exactly one duplicate request should be accepted"
    );
    let duplicate_run = first_duplicate
        .or(second_duplicate)
        .expect("one duplicate request should be accepted");

    let first_preview = ctx
        .app
        .preview_restore_recycled_items(&manager, vec![first_entry.clone()])
        .await
        .expect("preview first independent entry");
    let second_preview = ctx
        .app
        .preview_restore_recycled_items(&manager, vec![second_entry.clone()])
        .await
        .expect("preview second independent entry");
    let (first, second) = tokio::join!(
        ctx.app.start_restore_recycled_items_job(
            &manager,
            vec![first_entry],
            scryer_application::RecycleRestoreConflictPolicy::KeepBoth,
            &first_preview.fingerprint,
        ),
        ctx.app.start_restore_recycled_items_job(
            &manager,
            vec![second_entry],
            scryer_application::RecycleRestoreConflictPolicy::KeepBoth,
            &second_preview.fingerprint,
        ),
    );
    let first = first.expect("first independent entry should be accepted");
    let second = second.expect("second independent entry should be accepted");

    assert_eq!(
        wait_for_terminal_job(
            &ctx,
            &manager,
            JobKey::RecycleBinRestore,
            &duplicate_run.job_run.id,
        )
        .await
        .status,
        JobRunStatus::Completed
    );
    assert_eq!(
        wait_for_terminal_job(&ctx, &manager, JobKey::RecycleBinRestore, &first.job_run.id)
            .await
            .status,
        JobRunStatus::Completed
    );
    assert_eq!(
        wait_for_terminal_job(
            &ctx,
            &manager,
            JobKey::RecycleBinRestore,
            &second.job_run.id
        )
        .await
        .status,
        JobRunStatus::Completed
    );
}
