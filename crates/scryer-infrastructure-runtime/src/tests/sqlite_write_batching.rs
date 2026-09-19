use super::*;

/// Counts commit records in a WAL file. Every frame carries a 24-byte header
/// whose bytes 4..8 hold the database size in pages after that frame; the field
/// is non-zero only on the last frame of a committed transaction, so the count
/// of non-zero values is exactly the number of commits still in the WAL.
fn wal_commit_count(db: &std::path::Path) -> usize {
    let wal_path = db.with_extension("db-wal");
    let Ok(bytes) = std::fs::read(&wal_path) else {
        return 0;
    };
    if bytes.len() < 32 {
        return 0;
    }
    let page_size = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if page_size == 0 {
        return 0;
    }
    let frame_size = 24 + page_size;
    let mut commits = 0;
    let mut offset = 32;
    while offset + frame_size <= bytes.len() {
        let db_size_after_commit = u32::from_be_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        if db_size_after_commit != 0 {
            commits += 1;
        }
        offset += frame_size;
    }
    commits
}

async fn truncate_wal(pool: &sqlx::SqlitePool) {
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .expect("wal should truncate");
}

fn scan_progress_event(session_id: &str, sequence_hint: i64) -> NewDomainEvent {
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: scryer_domain::DomainEventActorKind::System,
        actor_user_id: None,
        actor_display_name: "System".to_string(),
        title_id: None,
        facet: Some(MediaFacet::Series),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::LibraryScan {
            session_id: session_id.to_string(),
        },
        payload: DomainEventPayload::LibraryScanProgressed(
            scryer_domain::LibraryScanProgressedEventData {
                session_id: session_id.to_string(),
                status: "running".to_string(),
                found_titles: sequence_hint,
                title_match_completed: sequence_hint,
                title_match_total_known: true,
                titles_completed: sequence_hint,
                titles_total: Some(sequence_hint),
                files_completed: sequence_hint,
                files_total: Some(sequence_hint),
                warning_message: None,
            },
        ),
    }
}

/// The scan's per-file progress flush appends its delta and progress events
/// together. One `append_many` must cost one commit, where the equivalent
/// sequence of single appends costs one commit each.
#[tokio::test]
async fn batched_domain_event_append_costs_one_commit() {
    let (services, db) = temp_services("scryer_write_batching").await;
    let domain_events = DomainEventStore::new(services.datastore());
    let session_id = Id::new().0;

    truncate_wal(services.pool()).await;
    domain_events
        .append_many(vec![
            scan_progress_event(&session_id, 1),
            scan_progress_event(&session_id, 2),
            scan_progress_event(&session_id, 3),
        ])
        .await
        .expect("batched append should succeed");
    let batched_commits = wal_commit_count(&db);

    truncate_wal(services.pool()).await;
    for hint in 4..7 {
        domain_events
            .append(scan_progress_event(&session_id, hint))
            .await
            .expect("single append should succeed");
    }
    let unbatched_commits = wal_commit_count(&db);

    assert_eq!(
        batched_commits, 1,
        "three batched events should commit once"
    );
    assert_eq!(
        unbatched_commits, 3,
        "three separate appends should commit three times"
    );
}

#[tokio::test]
async fn sqlite_wal_autocheckpoint_defaults_to_the_configured_page_budget() {
    let (services, _db) = temp_services("scryer_wal_autocheckpoint").await;
    let pages: i64 = sqlx::query_scalar("PRAGMA wal_autocheckpoint")
        .fetch_one(services.pool())
        .await
        .expect("pragma should read");
    assert_eq!(pages, 16_384);
}

#[tokio::test]
async fn sqlite_wal_autocheckpoint_honors_the_env_override() {
    // nextest runs every test in its own process, so this env write is local.
    unsafe { std::env::set_var("SCRYER_SQLITE_WAL_AUTOCHECKPOINT_PAGES", "4096") };
    let (services, _db) = temp_services("scryer_wal_autocheckpoint_env").await;
    let pages: i64 = sqlx::query_scalar("PRAGMA wal_autocheckpoint")
        .fetch_one(services.pool())
        .await
        .expect("pragma should read");
    unsafe { std::env::remove_var("SCRYER_SQLITE_WAL_AUTOCHECKPOINT_PAGES") };
    assert_eq!(pages, 4096);
}
