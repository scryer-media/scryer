//! Round-trips for the staged request metadata record: one row per subject,
//! replaced in place, and removed only once it is older than the cutoff.

use super::media_request_requesters::request_store;
use super::*;
use scryer_application::{MediaRequestRepository, StagedMediaRequestMetadata};

fn record(
    key: &str,
    payload: &str,
    staged_at: chrono::DateTime<Utc>,
) -> StagedMediaRequestMetadata {
    StagedMediaRequestMetadata {
        staging_key: key.to_string(),
        payload_json: payload.to_string(),
        staged_at,
    }
}

#[tokio::test]
async fn staging_a_subject_stores_one_record_and_restaging_replaces_it() {
    let (services, _db) = temp_services("scryer_media_request_staged_metadata").await;
    let store = request_store(&services);
    let first_at = Utc::now() - chrono::Duration::hours(30);

    assert_eq!(
        store.staged_metadata("movie|tvdb=1").await.expect("read"),
        None
    );

    store
        .stage_metadata(&record("movie|tvdb=1", r#"{"overview":"first"}"#, first_at))
        .await
        .expect("stage");
    let staged = store
        .staged_metadata("movie|tvdb=1")
        .await
        .expect("read")
        .expect("the record is staged");
    assert_eq!(staged.payload_json, r#"{"overview":"first"}"#);
    assert_eq!(staged.staged_at.timestamp(), first_at.timestamp());

    let second_at = Utc::now();
    store
        .stage_metadata(&record(
            "movie|tvdb=1",
            r#"{"overview":"second"}"#,
            second_at,
        ))
        .await
        .expect("stage again");
    let staged = store
        .staged_metadata("movie|tvdb=1")
        .await
        .expect("read")
        .expect("the record is staged");
    assert_eq!(staged.payload_json, r#"{"overview":"second"}"#);
    assert_eq!(staged.staged_at.timestamp(), second_at.timestamp());

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_request_staged_metadata")
        .fetch_one(services.pool())
        .await
        .expect("count");
    assert_eq!(
        rows, 1,
        "a subject keeps one record however often it is staged"
    );
    assert_eq!(
        store.staged_metadata("series|tvdb=1").await.expect("read"),
        None,
        "another subject has no record"
    );
}

#[tokio::test]
async fn cleanup_removes_only_records_older_than_the_cutoff_in_bounded_batches() {
    let (services, _db) = temp_services("scryer_media_request_staged_metadata").await;
    let store = request_store(&services);
    let now = Utc::now();
    for (key, days) in [("a", 30), ("b", 20), ("c", 10), ("d", 3), ("e", 0)] {
        store
            .stage_metadata(&record(key, "{}", now - chrono::Duration::days(days)))
            .await
            .expect("stage");
    }
    let cutoff = now - chrono::Duration::days(7);

    assert_eq!(
        store
            .delete_staged_metadata_before(cutoff, 2)
            .await
            .expect("first batch"),
        2,
        "a batch removes no more than its limit"
    );
    assert_eq!(
        store
            .delete_staged_metadata_before(cutoff, 2)
            .await
            .expect("second batch"),
        1
    );
    assert_eq!(
        store
            .delete_staged_metadata_before(cutoff, 2)
            .await
            .expect("empty batch"),
        0
    );

    for kept in ["d", "e"] {
        assert!(
            store.staged_metadata(kept).await.expect("read").is_some(),
            "{kept} is inside the retention window and stays"
        );
    }
    for removed in ["a", "b", "c"] {
        assert!(
            store
                .staged_metadata(removed)
                .await
                .expect("read")
                .is_none()
        );
    }
}
