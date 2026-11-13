//! The batched playback-item read against a real sqlite database.
//!
//! The calendar asks for the playback items of every entity in its window,
//! which can be thousands. The batched read must return, for any list size and
//! whether it runs as one statement or several, exactly the rows the
//! per-entity read returns, ordered by kind, entity and connection.

use std::sync::{Arc, RwLock};

use chrono::{TimeZone, Utc};
use scryer_application::MediaServerConnectionRepository;
use scryer_domain::{MediaServerPlaybackEntityKind, MediaServerPlaybackItem};
use sqlx::sqlite::SqlitePoolOptions;

use super::{MediaServerConnectionStore, playback_items_sql, playback_kind_predicate};
use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

const CONNECTIONS: [&str; 3] = ["connection-a", "connection-b", "connection-c"];
/// More episodes than one split statement binds, so the split path crosses a
/// chunk boundary inside one kind.
const EPISODES: usize = 1_000;
const TITLES: usize = 400;

async fn store() -> MediaServerConnectionStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .expect("fresh migrations should apply");
    let datastore = StoreDatastore::Sqlite {
        pool,
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    };
    for connection_id in CONNECTIONS {
        SqlRuntime::execute_write(
            &datastore,
            "playback_items_fixture",
            "INSERT INTO media_server_connections
                (id, provider, display_name, base_url, created_at, updated_at)
             VALUES ({}, 'jellyfin', {}, 'http://media.example.test', {}, {})",
            vec![
                SqlArg::Text(connection_id.to_string()),
                SqlArg::Text(connection_id.to_string()),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
        .expect("seed connection");
    }
    MediaServerConnectionStore::new(datastore, Arc::new(RwLock::new(None)))
}

/// Synthetic entities: episodes then titles, ids zero-padded.
fn entities() -> Vec<(MediaServerPlaybackEntityKind, String)> {
    (0..EPISODES)
        .map(|index| {
            (
                MediaServerPlaybackEntityKind::Episode,
                format!("episode-{index:05}"),
            )
        })
        .chain((0..TITLES).map(|index| {
            (
                MediaServerPlaybackEntityKind::Title,
                format!("title-{index:05}"),
            )
        }))
        .collect()
}

/// Every seventh entity is unmapped; the rest are mapped on one to three
/// connections, so the connection order inside an entity matters.
async fn seed(store: &MediaServerConnectionStore) {
    let seen = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    for (position, connection_id) in CONNECTIONS.iter().enumerate() {
        let items = entities()
            .into_iter()
            .enumerate()
            .filter(|(index, _)| index % 7 != 0 && index % 3 >= position)
            .map(
                |(index, (entity_kind, entity_id))| MediaServerPlaybackItem {
                    connection_id: (*connection_id).to_string(),
                    entity_kind,
                    entity_id,
                    provider_item_id: format!("{connection_id}-item-{index}"),
                    last_seen_at: seen,
                },
            )
            .collect::<Vec<_>>();
        store
            .upsert_playback_items_for_connection(connection_id, items)
            .await
            .expect("seed playback items");
    }
}

/// The per-entity read, entity by entity in kind and id byte order.
async fn per_entity(
    store: &MediaServerConnectionStore,
    request: &[(MediaServerPlaybackEntityKind, String)],
) -> Vec<MediaServerPlaybackItem> {
    let mut distinct = request
        .iter()
        .map(|(kind, id)| (kind.as_str(), *kind, id.clone()))
        .collect::<Vec<_>>();
    distinct.sort_by(|left, right| (left.0, &left.2).cmp(&(right.0, &right.2)));
    distinct.dedup_by(|left, right| left.0 == right.0 && left.2 == right.2);
    let mut items = Vec::new();
    for (_, kind, id) in distinct {
        items.extend(
            store
                .list_playback_items_for_entity(kind, &id)
                .await
                .expect("per-entity read"),
        );
    }
    items
}

#[tokio::test]
async fn a_large_entity_list_matches_the_per_entity_read_in_one_statement_or_split() {
    let store = store().await;
    seed(&store).await;

    // Scrambled, with repeats and ids nothing is mapped to.
    let mut request = entities();
    request.reverse();
    request.rotate_left(TITLES / 2);
    request.extend(entities().into_iter().step_by(11));
    request.push((
        MediaServerPlaybackEntityKind::Episode,
        "episode-unknown".to_string(),
    ));
    request.push((
        MediaServerPlaybackEntityKind::Title,
        "title-unknown".to_string(),
    ));
    assert!(request.len() > 1_000);

    let expected = per_entity(&store, &request).await;
    assert!(expected.len() > EPISODES + TITLES);
    let mut keys = expected
        .iter()
        .map(|item| {
            (
                item.entity_kind.as_str(),
                item.entity_id.as_str(),
                item.connection_id.as_str(),
            )
        })
        .collect::<Vec<_>>();
    let unsorted = keys.clone();
    keys.sort();
    assert_eq!(
        keys, unsorted,
        "expected rows are in kind, entity, connection order"
    );

    let one = store
        .list_playback_items_for_entities(&request)
        .await
        .expect("one statement should run");
    assert_eq!(one, expected);

    // One bind per statement forces every kind through split statements, and
    // the episodes across more than one chunk.
    let _lowered = crate::media::id_list_statements::lower_max_binds_for_tests(1);
    let split = store
        .list_playback_items_for_entities(&request)
        .await
        .expect("split statements should run");
    assert_eq!(split, expected);
}

#[tokio::test]
async fn the_batched_read_is_served_by_the_entity_index() {
    let store = store().await;
    let predicates = [playback_kind_predicate(3), playback_kind_predicate(2)].join(" OR ");
    let sql = format!("EXPLAIN QUERY PLAN {}", playback_items_sql(&predicates));
    let args = ["episode", "e1", "e2", "e3", "title", "t1", "t2"]
        .into_iter()
        .map(|value| SqlArg::Text(value.to_string()))
        .collect::<Vec<_>>();
    let plan = SqlRuntime::fetch_all(store.datastore.read_exec(), &sql, &args)
        .await
        .expect("query plan")
        .iter()
        .map(|row| row.text("detail").expect("plan detail"))
        .collect::<Vec<_>>();
    assert!(
        plan.iter()
            .any(|detail| detail.contains("idx_media_server_playback_items_entity")),
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|detail| detail.starts_with("SCAN media_server_playback_items")),
        "{plan:?}"
    );
}
