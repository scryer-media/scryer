mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use common::TestContext;
use scryer_application::{AppError, TitleRepository};
use scryer_domain::{MediaFacet, Title};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Wire pending-release and wanted-item repos into the test AppUseCase.
fn app_with_pending(ctx: &TestContext) -> scryer_application::AppUseCase {
    let mut app = ctx.app.clone();
    app.services.pending_releases = Arc::new(ctx.db.clone());
    app.services.wanted_items = Arc::new(ctx.db.clone());
    app
}

/// Create a title so FK constraints are satisfied.
async fn seed_title(ctx: &TestContext, id: &str) {
    let title = Title {
        id: id.to_string(),
        name: "Test Title".to_string(),
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

/// Insert a wanted item directly via the repo and return its ID.
async fn seed_wanted_item(
    ctx: &TestContext,
    title_id: &str,
    status: &str,
) -> scryer_application::WantedItem {
    let item = scryer_application::WantedItem {
        id: scryer_domain::Id::new().0,
        title_id: title_id.to_string(),
        title_name: Some("Test Title".to_string()),
        episode_id: None,
        season_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: status.to_string(),
        grabbed_release: None,
        current_score: None,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    ctx.db.upsert_wanted_item(&item).await.expect("seed wanted");
    item
}

/// Insert a pending release directly via the repo.
async fn seed_pending_release(
    ctx: &TestContext,
    wanted_item_id: &str,
    title_id: &str,
    score: i32,
    delay_hours: i64,
    status: &str,
) -> scryer_application::PendingRelease {
    let now = Utc::now();
    let delay_until = now + Duration::hours(delay_hours);
    let pr = scryer_application::PendingRelease {
        id: scryer_domain::Id::new().0,
        wanted_item_id: wanted_item_id.to_string(),
        title_id: title_id.to_string(),
        release_title: format!("Test.Release.Score{score}.1080p.WEB-DL"),
        release_url: Some("https://example.com/nzb/123".to_string()),
        release_size_bytes: Some(1_500_000_000),
        release_score: score,
        scoring_log_json: None,
        indexer_source: Some("nzbgeek".to_string()),
        release_guid: Some(format!("guid-{}", scryer_domain::Id::new().0)),
        added_at: now.to_rfc3339(),
        delay_until: delay_until.to_rfc3339(),
        status: status.to_string(),
        grabbed_at: None,
    };
    ctx.db
        .insert_pending_release(&pr)
        .await
        .expect("seed pending release");
    pr
}

// ---------------------------------------------------------------------------
// list_pending_releases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_pending_releases_returns_only_waiting() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", "wanted").await;
    seed_pending_release(&ctx, &wi.id, "title-1", 500, 6, "waiting").await;
    seed_pending_release(&ctx, &wi.id, "title-1", 300, 6, "grabbed").await;
    seed_pending_release(&ctx, &wi.id, "title-1", 200, 6, "dismissed").await;

    let pending = app.list_pending_releases().await.expect("list");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].release_score, 500);
}

// ---------------------------------------------------------------------------
// dismiss_pending_release
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dismiss_sets_status_to_dismissed() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", "wanted").await;
    let pr = seed_pending_release(&ctx, &wi.id, "title-1", 500, 6, "waiting").await;

    let result = app.dismiss_pending_release(&pr.id).await.expect("dismiss");
    assert!(result);

    // Should no longer appear in waiting list
    let pending = app.list_pending_releases().await.unwrap();
    assert!(pending.is_empty());

    // Verify status in DB
    let fetched = ctx.db.get_pending_release(&pr.id).await.unwrap().unwrap();
    assert_eq!(fetched.status, "dismissed");
}

#[tokio::test]
async fn dismiss_nonexistent_returns_error() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    let err = app
        .dismiss_pending_release("nonexistent-id")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

#[tokio::test]
async fn dismiss_non_waiting_returns_error() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", "wanted").await;
    let pr = seed_pending_release(&ctx, &wi.id, "title-1", 500, 6, "grabbed").await;

    let err = app.dismiss_pending_release(&pr.id).await.unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

// ---------------------------------------------------------------------------
// force_grab_pending_release
// ---------------------------------------------------------------------------

#[tokio::test]
async fn force_grab_nonexistent_returns_error() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    let err = app
        .force_grab_pending_release("nonexistent-id")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

#[tokio::test]
async fn force_grab_non_waiting_returns_error() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", "wanted").await;
    let pr = seed_pending_release(&ctx, &wi.id, "title-1", 500, 6, "dismissed").await;

    let err = app.force_grab_pending_release(&pr.id).await.unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

// ---------------------------------------------------------------------------
// process_expired_pending_releases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn process_expired_skips_when_none_expired() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", "wanted").await;
    // delay_until is 6 hours from now — not expired
    seed_pending_release(&ctx, &wi.id, "title-1", 500, 6, "waiting").await;

    let count = app
        .process_expired_pending_releases()
        .await
        .expect("process");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn process_expired_marks_expired_when_wanted_item_gone() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    // Create pending release referencing a wanted item, then delete the wanted item
    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", "wanted").await;
    let pr = seed_pending_release(&ctx, &wi.id, "title-1", 500, -1, "waiting").await;
    // Delete the wanted item
    ctx.db
        .delete_wanted_items_for_title("title-1")
        .await
        .expect("delete wanted");

    let count = app
        .process_expired_pending_releases()
        .await
        .expect("process");
    assert_eq!(count, 0);

    // PR should be marked expired
    let fetched = ctx.db.get_pending_release(&pr.id).await.unwrap().unwrap();
    assert_eq!(fetched.status, "expired");
}

#[tokio::test]
async fn process_expired_supersedes_when_already_grabbed() {
    let ctx = TestContext::new().await;
    let app = app_with_pending(&ctx);

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", "grabbed").await;
    let pr = seed_pending_release(&ctx, &wi.id, "title-1", 500, -1, "waiting").await;

    let count = app
        .process_expired_pending_releases()
        .await
        .expect("process");
    assert_eq!(count, 0);

    // PR should be superseded (wanted item already grabbed)
    let fetched = ctx.db.get_pending_release(&pr.id).await.unwrap().unwrap();
    assert_eq!(fetched.status, "superseded");
}
