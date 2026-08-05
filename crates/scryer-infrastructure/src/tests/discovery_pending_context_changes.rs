//! Regression coverage for replaying the domain-event log into
//! `discovery_pending_context_changes`.
//!
//! Sqlite declares `title_id TEXT REFERENCES titles(id) ON DELETE SET NULL` on
//! that table, but the rows are built by replaying *historical* domain events —
//! so the id being replayed routinely names a title that has since been deleted.
//! Binding it directly raised `FOREIGN KEY constraint failed` (sqlite extended
//! code 787), which aborted the whole discovery sync. Because the catch-up
//! watermark only advances on success, the next run replayed the same event and
//! failed identically, freezing discovery indefinitely.

use super::*;
use scryer_application::{DiscoveryPendingContextChangeRecord, DiscoveryRepository};

fn pending_change(id: &str, title_id: Option<&str>) -> DiscoveryPendingContextChangeRecord {
    let now = Utc::now();
    DiscoveryPendingContextChangeRecord {
        id: format!("default:title:{id}"),
        scope_key: "default".to_string(),
        subject_key: Some("tvdb:81189".to_string()),
        previous_subject_key: None,
        change_type: "removed".to_string(),
        title_id: title_id.map(str::to_string),
        previous_title_id: None,
        library_facet: Some("series".to_string()),
        raw_subject_json: Some("{\"subjectKey\":\"tvdb:81189\"}".to_string()),
        raw_previous_subject_json: None,
        first_seen_sequence: Some(911_654),
        last_seen_sequence: Some(911_654),
        first_seen_at: now,
        last_seen_at: now,
    }
}

/// Replaying a `title_deleted` event must persist, not abort the sync.
///
/// This is the exact production shape: the last successful discovery generation
/// predates the deletion, so the catch-up replays a `title_deleted` event whose
/// title row is already gone.
#[tokio::test]
async fn pending_context_change_persists_after_its_title_is_deleted() {
    let (services, db) = temp_services("scryer_discovery_pending_change_deleted_title").await;
    let catalog = title_store(&services);
    let discovery = discovery_store(&services);

    let title = make_test_title("title-deleted-then-replayed", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    TitleRepository::delete(&catalog, &title.id)
        .await
        .expect("title should delete");

    let change = pending_change(&title.id, Some(&title.id));
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("replaying a deleted title must not fail the discovery sync");

    let stored = discovery
        .get_pending_discovery_context_change(&change.id)
        .await
        .expect("pending change should read back")
        .expect("pending change should exist");

    // The dangling reference is dropped, which is exactly what the declared
    // `ON DELETE SET NULL` would have left behind had the delete landed a moment
    // later. Everything that identifies the change survives.
    assert_eq!(stored.title_id, None);
    assert_eq!(stored.id, change.id);
    assert_eq!(stored.change_type, "removed");
    assert_eq!(stored.subject_key, change.subject_key);
    assert_eq!(stored.library_facet, change.library_facet);
    assert_eq!(stored.last_seen_sequence, change.last_seen_sequence);

    let _ = std::fs::remove_file(db);
}

/// A title that never existed (pruned rows, restored backups) behaves the same,
/// and re-running the upsert is idempotent — sqlite busy-retries re-run the
/// whole write closure, so the second pass must not diverge from the first.
#[tokio::test]
async fn pending_context_change_with_unknown_title_is_idempotent() {
    let (services, db) = temp_services("scryer_discovery_pending_change_unknown_title").await;
    let discovery = discovery_store(&services);

    let change = pending_change(
        "2b55f5ae-368d-4325-bfb9-b906078fc9ab",
        Some("2b55f5ae-368d-4325-bfb9-b906078fc9ab"),
    );
    for _ in 0..2 {
        discovery
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("unknown title id must resolve to NULL rather than violating the FK");
    }

    let stored = discovery
        .get_pending_discovery_context_change(&change.id)
        .await
        .expect("pending change should read back")
        .expect("pending change should exist");
    assert_eq!(stored.title_id, None);

    let listed = discovery
        .list_all_pending_discovery_context_changes("default")
        .await
        .expect("pending changes should list");
    assert_eq!(listed.len(), 1, "re-upserting must not duplicate the row");

    let _ = std::fs::remove_file(db);
}

/// The guard must not blanket-null the column: a live title still round-trips,
/// so the foreign key keeps its meaning for references that are actually live.
#[tokio::test]
async fn pending_context_change_keeps_live_title_reference() {
    let (services, db) = temp_services("scryer_discovery_pending_change_live_title").await;
    let catalog = title_store(&services);
    let discovery = discovery_store(&services);

    let title = make_test_title("title-still-present", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let change = pending_change(&title.id, Some(&title.id));
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("live title reference should persist");

    let stored = discovery
        .get_pending_discovery_context_change(&change.id)
        .await
        .expect("pending change should read back")
        .expect("pending change should exist");
    assert_eq!(stored.title_id, Some(title.id.clone()));

    let _ = std::fs::remove_file(db);
}
