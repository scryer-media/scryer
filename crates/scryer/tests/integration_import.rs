mod common;

use std::sync::Arc;

use common::TestContext;
use scryer_application::{import_completed_download, TitleRepository};
use scryer_domain::{CompletedDownload, ImportDecision, ImportSkipReason, MediaFacet, Title};
use scryer_infrastructure::FsFileImporter;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an AppUseCase with a real SQLite import repository and filesystem
/// file importer so that tests can exercise the full import pipeline.
fn app_with_real_imports(ctx: &TestContext) -> scryer_application::AppUseCase {
    let mut app = ctx.app.clone();
    app.services.imports = Arc::new(ctx.db.clone());
    app.services.file_importer = Arc::new(FsFileImporter);
    app.services.media_files = Arc::new(ctx.db.clone());
    app
}

/// Build a minimal CompletedDownload with scryer-origin parameters.
fn scryer_completed(
    item_id: &str,
    dest_dir: &str,
    title_id: &str,
    facet_id: &str,
) -> CompletedDownload {
    CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: item_id.to_string(),
        name: format!("Test.Download.{item_id}"),
        dest_dir: dest_dir.to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![
            ("*scryer_title_id".to_string(), title_id.to_string()),
            ("*scryer_facet".to_string(), facet_id.to_string()),
        ],
    }
}

/// Add a minimal movie Title to the DB, tagging `media_root` so import
/// uses it as the destination library folder without needing settings.
async fn add_movie_title(
    ctx: &TestContext,
    id: &str,
    name: &str,
    media_root: &str,
) -> Title {
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        // The root-folder tag overrides the settings lookup.
        tags: vec![format!("scryer:root-folder:{}", media_root)],
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
    ctx.db.create(title).await.expect("add movie title")
}

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

/// Directly mark a download as "completed" in the import repository, then
/// attempt to import the same download again.  The second call should be
/// short-circuited as AlreadyImported without re-running the pipeline.
#[tokio::test]
async fn import_deduplicates_completed_imports() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    // Seed a completed import record for (nzbget, "dl-dedup").
    let import_id = app
        .services
        .imports
        .queue_import_request(
            "nzbget".to_string(),
            "dl-dedup".to_string(),
            "movie_download".to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue_import_request");
    app.services
        .imports
        .update_import_status(&import_id, "completed", None)
        .await
        .expect("update_import_status");

    // Now attempt to import the same download — dedup should fire immediately.
    let completed = CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: "dl-dedup".to_string(),
        name: "Already.Imported.Movie".to_string(),
        dest_dir: "/tmp/wherever".to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![("*scryer_title_id".to_string(), "any-id".to_string())],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Skipped);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::AlreadyImported));
}

// ---------------------------------------------------------------------------
// Title matching
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_returns_unmatched_when_title_not_found() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let completed = CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: "dl-no-title".to_string(),
        name: "Unknown.Movie.2024".to_string(),
        dest_dir: "/tmp/wherever".to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![
            ("*scryer_title_id".to_string(), "nonexistent-id".to_string()),
        ],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Unmatched);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::UnresolvedIdentity));
}

// ---------------------------------------------------------------------------
// Video file detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_fails_when_no_video_files_in_dest_dir() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    // Source dir exists but contains only a text file — no video files.
    let source_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(source_dir.path().join("readme.txt"), b"no video here").expect("write");

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let title = add_movie_title(
        &ctx,
        "title-no-video",
        "No Video Movie",
        dest_dir.path().to_str().unwrap(),
    )
    .await;

    let completed =
        scryer_completed("dl-no-video", source_dir.path().to_str().unwrap(), &title.id, "movie");

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Failed);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
}

// ---------------------------------------------------------------------------
// Happy path: movie import
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_movie_succeeds_and_copies_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    // Source: a temp dir containing a plausible movie .mkv file.
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mkv = source_dir
        .path()
        .join("Test.Movie.2024.1080p.WEB-DL.H264.mkv");
    std::fs::write(&mkv, b"fake video content").expect("write mkv");

    // Destination: a different temp dir used as the media library root.
    let dest_root = tempfile::tempdir().expect("dest tempdir");

    let title = add_movie_title(
        &ctx,
        "title-movie-1",
        "Test Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed =
        scryer_completed("dl-movie-1", source_dir.path().to_str().unwrap(), &title.id, "movie");

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Imported, "expected Imported");
    assert!(result.dest_path.is_some(), "dest_path should be set after import");

    // The imported file must physically exist.
    let dest_path = result.dest_path.unwrap();
    assert!(
        std::path::Path::new(&dest_path).exists(),
        "imported file should exist at {dest_path}"
    );
}

// ---------------------------------------------------------------------------
// Dedup after a real successful import
// ---------------------------------------------------------------------------

/// Run a complete movie import, then confirm that a second attempt with the
/// same download_client_item_id is immediately short-circuited.
#[tokio::test]
async fn import_movie_second_attempt_is_deduped() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("source tempdir");
    std::fs::write(
        source_dir.path().join("Movie.2024.1080p.mkv"),
        b"fake video",
    )
    .expect("write mkv");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-dedup-2",
        "Dedup Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed =
        scryer_completed("dl-dedup-2", source_dir.path().to_str().unwrap(), &title.id, "movie");

    // First import — should succeed.
    let first = import_completed_download(&app, &user, &completed)
        .await
        .expect("first import");
    assert_eq!(first.decision, ImportDecision::Imported);

    // Second import — same download_client_item_id → AlreadyImported.
    let second = import_completed_download(&app, &user, &completed)
        .await
        .expect("second import");
    assert_eq!(second.decision, ImportDecision::Skipped);
    assert_eq!(second.skip_reason, Some(ImportSkipReason::AlreadyImported));
}
