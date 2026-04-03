#![recursion_limit = "256"]

mod common;

use chrono::Utc;
use serde_json::Value;
use tokio::time::{Duration, timeout};

use common::TestContext;
use scryer_application::{
    JobKey, JobRunRepository, JobRunStatus, JobTriggerSource, SettingsRepository, TitleRepository,
};
use scryer_domain::{ExternalId, Id, MediaFacet, Title};
use scryer_infrastructure::{SettingDefinitionSeed, SqliteServices};

async fn seed_media_path_settings(ctx: &TestContext) {
    ctx.db
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
    <SqliteServices as SettingsRepository>::upsert_setting_json(
        &ctx.db,
        "media",
        key_name,
        None,
        serde_json::to_string(value).expect("serialize setting value"),
        "integration_test",
        None,
    )
    .await
    .expect("upsert setting");
}

#[tokio::test]
async fn background_series_refresh_skips_non_relinked_titles_and_completes_job_run() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let title = ctx
        .db
        .create(Title {
            id: Id::new().0,
            name: "Pending Series".to_string(),
            facet: MediaFacet::Series,
            monitored: false,
            tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".to_string(),
                value: "345679".to_string(),
            }],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: Some("Pending hydration title".to_string()),
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: Some("Pending Series".to_string()),
            slug: Some("pending-series".to_string()),
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
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
        })
        .await
        .expect("create pending title");

    let media_root = tempfile::tempdir().expect("media root tempdir");
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

    set_media_path(
        &ctx,
        "series.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    ctx.app
        .run_scheduled_job_now(
            JobKey::BackgroundLibraryRefreshSeries,
            JobTriggerSource::ScheduledInterval,
        )
        .await
        .expect("background series refresh should succeed");

    assert!(
        ctx.app
            .services
            .library_scan_tracker
            .list_active()
            .await
            .is_empty(),
        "background refresh session should complete",
    );
    assert!(
        ctx.app
            .services
            .job_run_tracker
            .list_active()
            .await
            .is_empty(),
        "terminal background job should no longer be active",
    );

    let refreshed_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        ctx.db
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty(),
        "non-relinked additive refresh should not link files",
    );

    let runs = <SqliteServices as JobRunRepository>::list_job_runs(
        &ctx.db,
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
async fn manual_job_trigger_failure_is_persisted_and_broadcast() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let mut rx = ctx
        .app
        .subscribe_job_run_events(&admin)
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
            if let Some(run) = <SqliteServices as JobRunRepository>::get_job_run(&ctx.db, &run.id)
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
async fn scheduled_job_failure_returns_err_and_persists_failed_run() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

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
            let runs = <SqliteServices as JobRunRepository>::list_job_runs(
                &ctx.db,
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
