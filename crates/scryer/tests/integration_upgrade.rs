mod common;

use std::sync::Arc;

use common::TestContext;
use scryer_application::recycle_bin::RecycleBinConfig;
use scryer_application::upgrade::execute_upgrade;
use scryer_application::{ActivityKind, ActivitySeverity, InsertMediaFileInput, TitleRepository};
use scryer_domain::{MediaFacet, Title};
use scryer_infrastructure::FsFileImporter;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn app_with_real_fs(ctx: &TestContext) -> scryer_application::AppUseCase {
    let mut app = ctx.app.clone();
    app.services.media_files = Arc::new(ctx.db.clone());
    app.services.file_importer = Arc::new(FsFileImporter);
    app
}

async fn seed_title(ctx: &TestContext, id: &str) {
    let title = Title {
        id: id.to_string(),
        name: "Test Movie".to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        sort_title: None,
        slug: None,
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
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
    };
    ctx.db.create(title).await.expect("seed title");
}

fn make_recycle_config(base: &std::path::Path) -> RecycleBinConfig {
    RecycleBinConfig {
        enabled: true,
        base_path: base.to_path_buf(),
        retention_days: 7,
    }
}

/// Insert a media file record in the DB and create the physical file.
async fn seed_media_file(
    ctx: &TestContext,
    title_id: &str,
    file_path: &std::path::Path,
    size: i64,
    score: i32,
) -> scryer_application::TitleMediaFile {
    let input = InsertMediaFileInput {
        title_id: title_id.to_string(),
        file_path: file_path.to_string_lossy().to_string(),
        size_bytes: size,
        quality_label: Some("720p".to_string()),
        acquisition_score: Some(score),
        ..Default::default()
    };
    let file_id = ctx.db.insert_media_file(&input).await.expect("insert");
    let files = ctx.db.list_media_files_for_title(title_id).await.unwrap();
    files.into_iter().find(|f| f.id == file_id).unwrap()
}

fn last_upgrade_event(
    events: &[scryer_application::ActivityEvent],
) -> Option<&scryer_application::ActivityEvent> {
    events.iter().find(|e| e.kind == ActivityKind::FileUpgraded)
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_replaces_old_file_with_new() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    seed_title(&ctx, "title-1").await;

    // Set up directories
    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    // Create "old" file in media library
    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old video content 720p").expect("write old");

    // Create "new" higher-quality source file
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new video content 1080p better quality").expect("write new");

    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    // Seed old file in DB
    let existing = seed_media_file(&ctx, "title-1", &old_path, 22, 400).await;

    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL.x264");
    let recycle_config = make_recycle_config(recycle_dir.path());

    let outcome = execute_upgrade(
        &app,
        "Test Movie",
        "title-1",
        &existing,
        &new_source,
        &new_dest,
        &parsed,
        650,
        400,
        &recycle_config,
    )
    .await
    .expect("execute_upgrade");

    assert_eq!(outcome.old_score, 400);
    assert_eq!(outcome.new_score, 650);

    // New file should exist at destination
    assert!(new_dest.exists(), "new file should exist");

    // Old file should be gone from original location (recycled)
    assert!(!old_path.exists(), "old file should be recycled");

    // Recycle dir should contain the recycled file
    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path()).unwrap().collect();
    assert!(
        !recycle_entries.is_empty(),
        "recycle bin should have entries"
    );

    // DB should have the new file, not the old one
    let files = ctx.db.list_media_files_for_title("title-1").await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].acquisition_score, Some(650));

    // Activity event should be recorded
    let events = app.services.activity_stream.list(10, 0).await;
    let upgrade_event = last_upgrade_event(&events).expect("should have upgrade event");
    assert_eq!(upgrade_event.severity, ActivitySeverity::Success);
    assert!(upgrade_event.message.contains("400"));
    assert!(upgrade_event.message.contains("650"));
    assert!(upgrade_event.message.contains("Test Movie"));
}

// ---------------------------------------------------------------------------
// Rollback on import failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_restores_old_file_on_import_failure() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    seed_title(&ctx, "title-2").await;

    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");

    // Create old file
    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old video content").expect("write old");

    // Source file does NOT exist — this will cause import to fail
    let bad_source = std::path::PathBuf::from("/nonexistent/path/does/not/exist.mkv");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-2", &old_path, 17, 400).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let recycle_config = make_recycle_config(recycle_dir.path());

    let result = execute_upgrade(
        &app,
        "Test Movie",
        "title-2",
        &existing,
        &bad_source,
        &new_dest,
        &parsed,
        700,
        400,
        &recycle_config,
    )
    .await;

    // Should fail
    assert!(
        result.is_err(),
        "upgrade should fail when source is missing"
    );

    // Old file should be RESTORED (not lost)
    assert!(
        old_path.exists(),
        "old file should be restored after failed upgrade"
    );

    // Content should match original
    let content = std::fs::read_to_string(&old_path).unwrap();
    assert_eq!(content, "old video content");
}

// ---------------------------------------------------------------------------
// Disabled recycle bin (direct delete)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_with_disabled_recycle_bin() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    seed_title(&ctx, "title-3").await;

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new content 1080p better").expect("write new");

    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-3", &old_path, 11, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");

    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
    };

    let outcome = execute_upgrade(
        &app,
        "Test Movie",
        "title-3",
        &existing,
        &new_source,
        &new_dest,
        &parsed,
        600,
        300,
        &disabled_config,
    )
    .await
    .expect("execute_upgrade");

    assert_eq!(outcome.new_score, 600);

    // Old file should be deleted (not recycled)
    assert!(!old_path.exists(), "old file should be deleted");

    // New file should exist
    assert!(new_dest.exists(), "new file should exist");
}
