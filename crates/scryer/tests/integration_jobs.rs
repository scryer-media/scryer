#![recursion_limit = "256"]

mod common;

use chrono::Utc;
use serde_json::Value;
use tokio::time::{Duration, timeout};

use common::TestContext;
use scryer_application::{
    JobKey, JobRunRepository, JobRunStatus, JobTriggerSource, LibraryRepository, LibraryRootDraft,
    MediaFileRepository, TitleRepository, UserRepository,
};
use scryer_domain::{
    ConfigurationChangeAction, DomainEventFilter, DomainEventPayload, DomainEventType, ExternalId,
    Id, Library, LibraryGrant, LibraryPermissionMask, MediaFacet, Title, User,
};
use scryer_infrastructure_sql::types::SettingDefinitionSeed;
use scryer_infrastructure_workflow::workflow::stores::WorkflowOperationStore;

async fn seed_media_path_settings(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
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
        ])
        .await
        .expect("seed media path settings");
}

async fn set_media_path(ctx: &TestContext, key_name: &str, value: &str) {
    ctx.settings_store
        .upsert_setting_value(
            "media",
            key_name,
            None,
            serde_json::to_string(value).expect("serialize setting value"),
            "integration_test",
            None,
        )
        .await
        .expect("upsert setting");

    let (library_id, name, slug) = match key_name {
        "movies.path" => ("movie_default_library", "Movies", "movies"),
        "series.path" => ("series_default_library", "Series", "series"),
        "anime.path" => ("anime_default_library", "Anime", "anime"),
        _ => return,
    };
    ctx.libraries
        .update(
            library_id,
            name.to_string(),
            slug.to_string(),
            vec![LibraryRootDraft {
                path: value.to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("update default library root");
}

/// Root ids are allocated, never derived from a path (FR-078), so a fixture that
/// wants a title attached to the configured root has to read the stored id.
async fn default_library_root_id(ctx: &TestContext, facet: &MediaFacet) -> String {
    let library_id = scryer_domain::default_library_id_for_facet(facet);
    let library = LibraryRepository::get_by_id(&ctx.libraries, &library_id)
        .await
        .expect("default library should load")
        .expect("default library should exist");
    library
        .roots
        .iter()
        .find(|root| root.is_default)
        .or_else(|| library.roots.first())
        .map(|root| root.id.clone())
        .expect("default library should have a root")
}

#[tokio::test]
async fn background_series_refresh_skips_non_relinked_titles_and_completes_job_run() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;
    let series_root_id = default_library_root_id(&ctx, &MediaFacet::Series).await;

    let title = TitleRepository::create(
        &ctx.titles,
        Title {
            id: Id::new().0,
            name: "Pending Series".to_string(),
            facet: MediaFacet::Series,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            monitored: false,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".to_string(),
                value: "345679".to_string(),
            }],
            root_folder_id: series_root_id,
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: Some("Pending hydration title".to_string()),
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: Some("Pending Series".to_string()),
            catalog_sort_key: String::new(),
            slug: Some("pending-series".to_string()),
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
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        },
    )
    .await
    .expect("create pending title");

    let show_dir = media_root.path().join("Pending Series [WEB-DL]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Pending Series</title><tvdbid>345679</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Pending.Series.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    ctx.app
        .run_scheduled_job_now(
            JobKey::BackgroundLibraryRefreshSeries,
            JobTriggerSource::ScheduledInterval,
        )
        .await
        .expect("background series refresh should succeed");
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("load default admin");

    assert!(
        ctx.app.active_library_scan_sessions().await.is_empty(),
        "background refresh session should complete",
    );
    assert!(
        ctx.app
            .active_job_runs(&admin)
            .await
            .expect("load active job runs")
            .is_empty(),
        "terminal background job should no longer be active",
    );

    let refreshed_title = TitleRepository::get_by_id(&ctx.titles, &title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty(),
        "non-relinked additive refresh should not link files",
    );

    let workflow_store = WorkflowOperationStore::new(ctx.db.datastore());
    let runs = <WorkflowOperationStore as JobRunRepository>::list_job_runs(
        &workflow_store,
        Some(JobKey::BackgroundLibraryRefreshSeries),
        1,
    )
    .await
    .expect("list job runs");
    let run = runs.first().expect("background refresh run should exist");
    assert_eq!(run.status, JobRunStatus::Completed);
    let summary_json = run.summary_json.as_deref().expect("summary json");
    let summary: Value = serde_json::from_str(summary_json).expect("parse summary json");
    assert_eq!(summary["scanned"], 1);
    assert_eq!(summary["matched"], 0);
    assert_eq!(summary["skipped"], 1);
    assert_eq!(summary["unmatched"], 0);
}

#[tokio::test]
async fn scheduled_background_refresh_creates_one_job_run_per_library() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let default_root = tempfile::tempdir().expect("default movie root");
    set_media_path(
        &ctx,
        "movies.path",
        default_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let second_root = tempfile::tempdir().expect("second movie root");
    let now = Utc::now();
    LibraryRepository::create(
        &ctx.libraries,
        Library {
            id: "movie_kids_library".to_string(),
            facet: MediaFacet::Movie,
            name: "Kids".to_string(),
            slug: "kids".to_string(),
            is_default: false,
            roots: vec![],
            created_at: now,
            updated_at: now,
        },
        vec![LibraryRootDraft {
            path: second_root.path().to_string_lossy().to_string(),
            is_default: true,
        }],
    )
    .await
    .expect("create second movie library");

    ctx.app
        .run_scheduled_job_now(
            JobKey::BackgroundLibraryRefreshMovies,
            JobTriggerSource::ScheduledInterval,
        )
        .await
        .expect("scheduled refresh should run each library job");

    let workflow_store = WorkflowOperationStore::new(ctx.db.datastore());
    let runs = <WorkflowOperationStore as JobRunRepository>::list_job_runs(
        &workflow_store,
        Some(JobKey::BackgroundLibraryRefreshMovies),
        5,
    )
    .await
    .expect("list job runs");
    let operation_types = runs
        .iter()
        .map(|run| run.operation_type.as_str())
        .collect::<std::collections::HashSet<_>>();

    assert!(operation_types.contains("background_library_refresh_movies:movie_default_library"));
    assert!(operation_types.contains("background_library_refresh_movies:movie_kids_library"));
    assert_eq!(
        runs.iter()
            .filter(|run| run.job_key == JobKey::BackgroundLibraryRefreshMovies)
            .count(),
        2
    );
}

#[tokio::test]
async fn domain_events_omit_titleless_operational_events_for_library_viewer() {
    let ctx = TestContext::new().await;
    let user_id = Id::new().0;
    let viewer = UserRepository::create(
        &ctx.users,
        User {
            id: user_id.clone(),
            username: "movie-viewer".to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: Default::default(),
        },
    )
    .await
    .expect("create viewer");
    LibraryRepository::set_grants_for_user(
        &ctx.libraries,
        &user_id,
        vec![LibraryGrant {
            user_id: user_id.clone(),
            library_id: "movie_default_library".to_string(),
            permissions: LibraryPermissionMask::VIEW,
        }],
    )
    .await
    .expect("grant library view");
    let viewer = ctx
        .app
        .attach_user_authorization(viewer)
        .await
        .expect("attach authorization");

    ctx.app
        .emit_configuration_changed_event(
            None,
            "system",
            Some("system.secret".to_string()),
            ConfigurationChangeAction::Saved,
        )
        .await;
    ctx.app
        .emit_configuration_changed_event(
            None,
            "library",
            Some("movie_default_library".to_string()),
            ConfigurationChangeAction::Saved,
        )
        .await;

    let events = ctx
        .app
        .list_domain_events(
            &viewer,
            &DomainEventFilter {
                event_types: Some(vec![DomainEventType::ConfigurationChanged]),
                limit: 10,
                ..DomainEventFilter::default()
            },
        )
        .await
        .expect("list domain events");

    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0].payload,
        DomainEventPayload::ConfigurationChanged(data)
            if data.resource_type == "library"
                && data.resource_id.as_deref() == Some("movie_default_library")
    ));
}

#[tokio::test]
async fn manual_job_trigger_failure_is_persisted_and_broadcast() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    let workflow_store = WorkflowOperationStore::new(ctx.db.datastore());
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let mut rx = ctx
        .app
        .subscribe_job_run_events(&admin)
        .await
        .expect("subscribe to job events");

    let run = ctx
        .app
        .trigger_job(&admin, JobKey::BackgroundLibraryRefreshAnime)
        .await
        .expect("manual trigger should create the run");

    let terminal = timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("job event should be received");
            if event.id == run.id
                && event.status == JobRunStatus::Failed
                && event.error_text.as_deref().is_some_and(|value| {
                    value.contains("library path is not a directory: /data/anime")
                })
            {
                break event;
            }
        }
    })
    .await
    .expect("should observe failed job event");

    assert!(
        terminal
            .error_text
            .as_deref()
            .is_some_and(|value| value.contains("library path is not a directory: /data/anime")),
        "manual failed job event should surface the error",
    );

    let stored = timeout(Duration::from_secs(5), async {
        loop {
            if let Some(run) =
                <WorkflowOperationStore as JobRunRepository>::get_job_run(&workflow_store, &run.id)
                    .await
                    .expect("load stored run")
                && run.status == JobRunStatus::Failed
            {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("stored failed run should be persisted");
    assert_eq!(stored.status, JobRunStatus::Failed);
    assert!(
        stored
            .error_text
            .as_deref()
            .is_some_and(|value| value.contains("library path is not a directory: /data/anime")),
    );
}

#[tokio::test]
async fn automatic_backup_job_cannot_be_triggered_manually() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let error = ctx
        .app
        .trigger_job(&admin, JobKey::AutoBackup)
        .await
        .expect_err("automatic backup should reject manual triggering");

    assert!(
        error
            .to_string()
            .contains("Automatic Backup can only run on its configured schedule"),
    );
}

#[tokio::test]
async fn automatic_backup_job_skips_stale_enabled_config_without_key() {
    let ctx = TestContext::new().await;
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "backup.auto.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "backup.auto.daily_time_local".into(),
                data_type: "string".into(),
                default_value_json: "\"03:00\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: "backup.auto.key".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
        ])
        .await
        .expect("seed automatic backup settings");
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "backup.auto.enabled",
            None,
            "true",
            "integration_test",
            None,
        )
        .await
        .expect("enable automatic backup setting");

    ctx.app
        .run_scheduled_job_now(JobKey::AutoBackup, JobTriggerSource::ScheduledDaily)
        .await
        .expect("stale automatic backup config should skip without error");

    let backup_dir = ctx.app.default_backup_dir();
    assert!(
        !backup_dir.exists()
            || std::fs::read_dir(backup_dir)
                .expect("read backup dir")
                .next()
                .is_none(),
        "automatic backup should not create files without a key",
    );
}

#[tokio::test]
async fn health_check_job_persists_issue_details_in_summary_json() {
    let ctx = TestContext::new().await;
    let workflow_store = WorkflowOperationStore::new(ctx.db.datastore());

    ctx.app
        .run_scheduled_job_now(JobKey::HealthChecks, JobTriggerSource::ScheduledStartup)
        .await
        .expect("health checks should complete");

    let runs = <WorkflowOperationStore as JobRunRepository>::list_job_runs(
        &workflow_store,
        Some(JobKey::HealthChecks),
        1,
    )
    .await
    .expect("list health check runs");
    let run = runs.first().expect("health check run should exist");
    assert_eq!(run.status, JobRunStatus::Completed);

    let summary_json = run.summary_json.as_deref().expect("summary json");
    let summary: Value = serde_json::from_str(summary_json).expect("parse summary json");
    let checks = summary["checks"]
        .as_array()
        .expect("health check summary should include checks");

    assert!(
        !checks.is_empty(),
        "health check summary should include at least one check result",
    );
    assert_eq!(
        checks.len(),
        summary["total"]
            .as_u64()
            .expect("summary total should be numeric") as usize,
    );
    assert!(checks.iter().any(|check| {
        check["source"].is_string() && check["status"].is_string() && check["message"].is_string()
    }));
}

#[tokio::test]
async fn scheduled_job_failure_returns_err_and_persists_failed_run() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    let workflow_store = WorkflowOperationStore::new(ctx.db.datastore());

    let result = ctx
        .app
        .run_scheduled_job_now(
            JobKey::BackgroundLibraryRefreshMovies,
            JobTriggerSource::ScheduledStartup,
        )
        .await;
    assert!(
        result.is_err(),
        "scheduled failure should propagate to the caller"
    );

    let run = timeout(Duration::from_secs(5), async {
        loop {
            let runs = <WorkflowOperationStore as JobRunRepository>::list_job_runs(
                &workflow_store,
                Some(JobKey::BackgroundLibraryRefreshMovies),
                1,
            )
            .await
            .expect("list job runs");
            if let Some(run) = runs.first()
                && run.status == JobRunStatus::Failed
            {
                break run.clone();
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("scheduled failed run should be persisted");
    assert_eq!(run.status, JobRunStatus::Failed);
    assert!(
        run.error_text
            .as_deref()
            .is_some_and(|value| value.contains("library path is not a directory: /data/movies")),
    );
}
