//! Store-level tests for the US7 merge engine, against a real sqlite database
//! with the full migration set replayed — so the cascades and the partial
//! unique indexes are exercised as they will behave in production, not against
//! a mock.

use super::*;

use scryer_application::location::merge::engine::plan_merge;
use scryer_application::location::merge::map::MergeBlockReason;
use scryer_application::location::merge::roles::RoleChangeReason;
use sqlx::sqlite::SqlitePoolOptions;

const SOURCE: &str = "title-source";
const DESTINATION: &str = "title-destination";

async fn test_store() -> (TitleMergeStore, StoreDatastore) {
    scryer_infrastructure_datastore::register_spellfix_auto_extension()
        .expect("spellfix extension should register before migrations");
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
        writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    };
    (TitleMergeStore::new(datastore.clone()), datastore)
}

async fn run(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "merge_test_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn scalar(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) -> i64 {
    SqlRuntime::fetch_optional(datastore.read_exec(), sql, &args)
        .await
        .expect("the count query should run")
        .expect("a count row")
        .i64("row_count")
        .expect("a count column")
}

async fn text(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) -> Option<String> {
    SqlRuntime::fetch_optional(datastore.read_exec(), sql, &args)
        .await
        .expect("the query should run")
        .map(|row| row.text("value").expect("a value column"))
}

async fn insert_title(datastore: &StoreDatastore, id: &str, library_id: &str, tags: &[&str]) {
    insert_title_with_facet(datastore, id, library_id, tags, "series").await;
}

async fn insert_title_with_facet(
    datastore: &StoreDatastore,
    id: &str,
    library_id: &str,
    tags: &[&str],
    facet: &str,
) {
    let tags = serde_json::to_string(&tags.iter().map(|t| t.to_string()).collect::<Vec<_>>())
        .expect("tags encode");
    run(
        datastore,
        // `root_folder_id` is non-null by trigger since migration 0136.
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags,
                             external_ids, created_at, library_id, root_folder_id)
         VALUES ({}, {}, {}, {}, 1, 'active', {}, '[]', '2026-01-01T00:00:00Z', {}, {})",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(format!("Title {id}")),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(facet.to_string()),
            SqlArg::Text(tags),
            SqlArg::Text(library_id.to_string()),
            SqlArg::Text(format!("root-{library_id}")),
        ],
    )
    .await;
}

async fn insert_collection(datastore: &StoreDatastore, id: &str, title_id: &str, season: &str) {
    run(
        datastore,
        "INSERT INTO collections (id, title_id, collection_type, collection_index, created_at)
         VALUES ({}, {}, 'season', {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(season.to_string()),
        ],
    )
    .await;
}

async fn insert_episode(
    datastore: &StoreDatastore,
    id: &str,
    title_id: &str,
    collection_id: Option<&str>,
    season: &str,
    number: &str,
) {
    run(
        datastore,
        "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number,
                               season_number, created_at)
         VALUES ({}, {}, {}, 'standard', {}, {}, '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::OptText(collection_id.map(str::to_string)),
            SqlArg::Text(number.to_string()),
            SqlArg::Text(season.to_string()),
        ],
    )
    .await;
}

async fn insert_media_file(datastore: &StoreDatastore, id: &str, title_id: &str, path: &str) {
    run(
        datastore,
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, role,
                                  created_at)
         VALUES ({}, {}, {}, 100, 'complete', 'primary', '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(path.to_string()),
        ],
    )
    .await;
}

async fn insert_file_episode(
    datastore: &StoreDatastore,
    file_id: &str,
    episode_id: &str,
    role: MergedMediaRole,
) {
    run(
        datastore,
        "INSERT INTO file_episode_map (file_id, episode_id, role, is_filler)
         VALUES ({}, {}, {}, 0)",
        vec![
            SqlArg::Text(file_id.to_string()),
            SqlArg::Text(episode_id.to_string()),
            SqlArg::Text(role.as_str().to_string()),
        ],
    )
    .await;
}

async fn insert_wanted_item(
    datastore: &StoreDatastore,
    id: &str,
    title_id: &str,
    episode_id: &str,
    status: &str,
) {
    run(
        datastore,
        "INSERT INTO wanted_items (id, title_id, episode_id, media_type, status, created_at,
                                   updated_at)
         VALUES ({}, {}, {}, 'episode', {}, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(episode_id.to_string()),
            SqlArg::Text(status.to_string()),
        ],
    )
    .await;
}

async fn insert_external_id(datastore: &StoreDatastore, id: &str, title_id: &str, library: &str) {
    run(
        datastore,
        "INSERT INTO title_external_ids (id, title_id, source, external_id, created_at, facet,
                                         library_id)
         VALUES ({}, {}, 'tvdb', '12345', '2026-01-01T00:00:00Z', 'series', {})",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(library.to_string()),
        ],
    )
    .await;
}

async fn insert_history_event(datastore: &StoreDatastore, id: &str, title_id: &str) {
    run(
        datastore,
        "INSERT INTO history_events (id, event_type, title_id, message, occurred_at, created_at)
         VALUES ({}, 'grabbed', {}, 'grabbed a release', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(title_id.to_string()),
        ],
    )
    .await;
}

async fn insert_release_grabbed_event(
    datastore: &StoreDatastore,
    event_id: &str,
    title_id: &str,
    episode_ids: &[&str],
) {
    let payload = serde_json::json!({
        "type": "release_grabbed",
        "data": {
            "title": {
                "title_name": "Title",
                "facet": "series",
                "external_ids": [],
                "poster_url": null,
                "year": null
            },
            "source_title": "Some.Release.S01E01",
            "source_hint": null,
            "source_provider": null,
            "download_id": "download-1",
            "episode_ids": episode_ids,
        }
    });
    let encoded = encode_domain_event_payload(&payload).expect("the payload should encode");
    run(
        datastore,
        "INSERT INTO domain_events (event_id, occurred_at, actor_kind, actor_display_name,
                                    title_id, schema_version, stream_kind, stream_id, event_type,
                                    payload_json)
         VALUES ({}, '2026-01-01T00:00:00Z', 'system', 'System', {}, 1, 'title', {},
                 'release_grabbed', {})",
        vec![
            SqlArg::Text(event_id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::OptBytes(Some(encoded)),
        ],
    )
    .await;
}

/// The episode ids inside a decoded payload, for the tests that assert on the
/// remap.
fn payload_episode_ids(payload: &Value) -> Vec<String> {
    payload
        .get("data")
        .and_then(|data| data.get("episode_ids"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A two-season series on both sides: seasons 1–2, one episode each, one file
/// per episode on each side.
async fn seed_two_season_series(datastore: &StoreDatastore) {
    insert_title(datastore, SOURCE, "library-a", &["rewatch"]).await;
    insert_title(datastore, DESTINATION, "library-b", &["4k"]).await;

    for (title, prefix) in [(SOURCE, "s"), (DESTINATION, "d")] {
        for season in ["1", "2"] {
            insert_collection(datastore, &format!("{prefix}-c{season}"), title, season).await;
            insert_episode(
                datastore,
                &format!("{prefix}-e{season}"),
                title,
                Some(&format!("{prefix}-c{season}")),
                season,
                "1",
            )
            .await;
            insert_media_file(
                datastore,
                &format!("{prefix}-f{season}"),
                title,
                &format!("/{prefix}/S{season}E01.mkv"),
            )
            .await;
            insert_file_episode(
                datastore,
                &format!("{prefix}-f{season}"),
                &format!("{prefix}-e{season}"),
                MergedMediaRole::Primary,
            )
            .await;
        }
    }
}

async fn plan_for(
    store: &TitleMergeStore,
) -> scryer_application::location::merge::engine::MergePlan {
    let snapshot = store
        .load_merge_snapshot(SOURCE, DESTINATION, None)
        .await
        .expect("the read phase should succeed");
    plan_merge(&snapshot)
}

async fn rows_referencing_source(datastore: &StoreDatastore, table: &str, column: &str) -> i64 {
    scalar(
        datastore,
        &format!("SELECT COUNT(*) AS row_count FROM {table} WHERE {column} = {{}}"),
        vec![SqlArg::Text(SOURCE.to_string())],
    )
    .await
}

#[tokio::test]
async fn a_two_season_series_carries_its_files_and_history_and_nothing_else() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    insert_external_id(&datastore, "xid-source", SOURCE, "library-a").await;
    insert_external_id(&datastore, "xid-destination", DESTINATION, "library-b").await;
    insert_release_grabbed_event(&datastore, "event-1", SOURCE, &["s-e1", "s-e2"]).await;
    insert_history_event(&datastore, "history-1", SOURCE).await;
    // A row the old per-table disposition list unioned. It retires with the
    // title now.
    insert_wanted_item(&datastore, "wanted-1", SOURCE, "s-e1", "wanted").await;

    let plan = plan_for(&store).await;
    assert!(!plan.is_blocked(), "blocked: {:?}", plan.blocked());
    // FR-071: the read phase reads the surviving title's name along with the
    // rest of its row, so the preview can name it instead of quoting its id.
    assert_eq!(
        plan.summary.destination_title_name.as_deref(),
        Some("Title title-destination")
    );
    assert_eq!(plan.summary.media_files_repointed, 2);
    // One `history_events` row plus one `domain_events` row.
    assert_eq!(plan.summary.history_rows_carried, 2);
    // The wanted item and the source external id.
    assert_eq!(plan.summary.source_records_dropped, 2);
    let map = plan.require_identity_map().expect("a complete map");
    assert_eq!(map.episode("s-e1"), Some("d-e1"));
    assert_eq!(map.episode("s-e2"), Some("d-e2"));
    assert_eq!(map.collection("s-c1"), Some("d-c1"));

    let outcome = store
        .execute_title_merge(&plan)
        .await
        .expect("the merge transaction should commit");
    assert_eq!(outcome.rows_affected.get("files:media_files"), Some(&2));
    assert_eq!(outcome.rows_affected.get("history:history_events"), Some(&1));
    assert_eq!(outcome.rows_affected.get("retire:titles"), Some(&1));

    // The source title is gone, and its files came with it.
    assert_eq!(rows_referencing_source(&datastore, "titles", "id").await, 0);
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM media_files WHERE title_id = {}",
            vec![SqlArg::Text(DESTINATION.to_string())],
        )
        .await,
        4
    );
    // Every incoming file is now mapped onto a destination episode.
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM file_episode_map
              WHERE file_id IN ('s-f1', 's-f2') AND episode_id IN ('d-e1', 'd-e2')",
            vec![],
        )
        .await,
        2
    );
    // Nothing points at the retired title in any of the tables the merge is
    // responsible for.
    for (table, column) in [
        ("history_events", "title_id"),
        ("domain_events", "title_id"),
        ("media_files", "title_id"),
        ("wanted_items", "title_id"),
    ] {
        assert_eq!(
            rows_referencing_source(&datastore, table, column).await,
            0,
            "{table}.{column} still references the retired title"
        );
    }
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM file_episode_map
              WHERE episode_id IN ('s-e1', 's-e2')",
            vec![],
        )
        .await,
        0
    );

    // FR-063: the destination keeps its own tags. Nothing unions.
    let tags = text(
        &datastore,
        "SELECT tags AS value FROM titles WHERE id = {}",
        vec![SqlArg::Text(DESTINATION.to_string())],
    )
    .await
    .expect("the destination title survives");
    assert_eq!(tags, r#"["4k"]"#);
}

#[tokio::test]
async fn a_movie_file_lands_as_additional_beside_a_destination_primary() {
    let (store, datastore) = test_store().await;
    insert_title_with_facet(&datastore, SOURCE, "library-a", &[], "movie").await;
    insert_title_with_facet(&datastore, DESTINATION, "library-b", &[], "movie").await;
    insert_media_file(&datastore, "s-movie", SOURCE, "/s/Film.mkv").await;
    insert_media_file(&datastore, "d-movie", DESTINATION, "/d/Film.mkv").await;

    let plan = plan_for(&store).await;
    assert!(!plan.is_blocked(), "blocked: {:?}", plan.blocked());
    assert_eq!(plan.summary.role_demotions, 1);
    assert_eq!(plan.summary.role_changes.len(), 1);
    assert_eq!(plan.summary.role_changes[0].file_id, "s-movie");
    assert_eq!(
        plan.summary.role_changes[0].reason,
        RoleChangeReason::DestinationPrimaryRetained
    );

    store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");

    assert_eq!(
        text(
            &datastore,
            "SELECT role AS value FROM media_files WHERE id = 's-movie'",
            vec![],
        )
        .await
        .as_deref(),
        Some("additional")
    );
    assert_eq!(
        text(
            &datastore,
            "SELECT role AS value FROM media_files WHERE id = 'd-movie'",
            vec![],
        )
        .await
        .as_deref(),
        Some("primary")
    );
}

#[tokio::test]
async fn a_movie_file_stays_primary_when_the_destination_has_no_file() {
    let (store, datastore) = test_store().await;
    insert_title_with_facet(&datastore, SOURCE, "library-a", &[], "movie").await;
    insert_title_with_facet(&datastore, DESTINATION, "library-b", &[], "movie").await;
    insert_media_file(&datastore, "s-movie", SOURCE, "/s/Film.mkv").await;

    let plan = plan_for(&store).await;
    assert!(plan.summary.role_changes.is_empty());
    store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");

    assert_eq!(
        text(
            &datastore,
            "SELECT role AS value FROM media_files WHERE id = 's-movie'",
            vec![],
        )
        .await
        .as_deref(),
        Some("primary")
    );
    assert_eq!(
        text(
            &datastore,
            "SELECT title_id AS value FROM media_files WHERE id = 's-movie'",
            vec![],
        )
        .await
        .as_deref(),
        Some(DESTINATION)
    );
}

/// FR-066: a slot the merge is carrying something onto has to map. A slot it is
/// carrying nothing onto retires with the title, so it never blocks.
#[tokio::test]
async fn an_unmapped_episode_blocks_only_when_it_carries_a_file() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    // A third source season the destination does not have, wanted by the
    // acquisition loop but carrying nothing the merge moves.
    insert_episode(&datastore, "s-e3", SOURCE, None, "3", "1").await;
    insert_wanted_item(&datastore, "wanted-orphan", SOURCE, "s-e3", "wanted").await;

    let plan = plan_for(&store).await;
    assert!(!plan.is_blocked(), "blocked: {:?}", plan.blocked());
    assert!(
        plan.require_identity_map()
            .expect("a map")
            .episode("s-e3")
            .is_none()
    );

    // Now put a file on it: the merge is carrying something onto a slot it
    // cannot place, so it refuses.
    insert_media_file(&datastore, "s-f3", SOURCE, "/s/S3E01.mkv").await;
    insert_file_episode(&datastore, "s-f3", "s-e3", MergedMediaRole::Primary).await;

    let plan = plan_for(&store).await;
    assert!(plan.is_blocked());
    assert!(plan.identity_map.is_none());
    assert!(
        plan.blocked()
            .iter()
            .all(|record| record.source_id == "s-e3"
                && record.reason == MergeBlockReason::UnmappedEpisode)
    );

    // The block costs no rollback: the execution refuses before it writes.
    let error = store
        .execute_title_merge(&plan)
        .await
        .expect_err("a blocked plan must not execute");
    assert!(error.to_string().contains("s-e3"), "{error}");
    assert_eq!(rows_referencing_source(&datastore, "titles", "id").await, 1);
}

#[tokio::test]
async fn an_incoming_primary_is_demoted_and_the_demotion_reaches_the_preview() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    // A second source file already additional for the same episode — the
    // partial unique index means the source side can only ever have one
    // primary, so this is the "incoming additionals stay additional" half of
    // FR-068.
    insert_media_file(&datastore, "s-f1b", SOURCE, "/s/S1E01.proper.mkv").await;
    insert_file_episode(&datastore, "s-f1b", "s-e1", MergedMediaRole::Additional).await;

    let plan = plan_for(&store).await;
    // Exactly one demotion for the contested slot: the incoming primary. The
    // additional is not a change, so it is not a preview line item.
    let season_one_changes: Vec<&_> = plan
        .summary
        .role_changes
        .iter()
        .filter(|change| change.destination_episode_id.as_deref() == Some("d-e1"))
        .collect();
    assert_eq!(season_one_changes.len(), 1);
    assert_eq!(season_one_changes[0].file_id, "s-f1");
    assert_eq!(
        season_one_changes[0].reason,
        RoleChangeReason::DestinationPrimaryRetained
    );
    assert_eq!(season_one_changes[0].previous_role, MergedMediaRole::Primary);
    assert_eq!(season_one_changes[0].new_role, MergedMediaRole::Additional);

    store
        .execute_title_merge(&plan)
        .await
        .expect("the demotion keeps the partial unique index satisfied");

    // FR-070: the destination file is still the one primary for the slot.
    assert_eq!(
        text(
            &datastore,
            "SELECT file_id AS value FROM file_episode_map
              WHERE episode_id = 'd-e1' AND role = 'primary'",
            vec![],
        )
        .await
        .as_deref(),
        Some("d-f1")
    );
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM file_episode_map
              WHERE episode_id = 'd-e1' AND role = 'additional'",
            vec![],
        )
        .await,
        2
    );
    // FR-069's other half: the season 2 file keeps primary, because that slot
    // had no destination primary conflict of its own.
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM file_episode_map
              WHERE episode_id = 'd-e2' AND role = 'primary'",
            vec![],
        )
        .await,
        1
    );
}

#[tokio::test]
async fn title_external_ids_are_deleted_rather_than_repointed() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    insert_external_id(&datastore, "xid-source", SOURCE, "library-a").await;
    insert_external_id(&datastore, "xid-destination", DESTINATION, "library-b").await;

    let plan = plan_for(&store).await;
    store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");

    // The destination's row stands; the source's is gone, never repointed.
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM title_external_ids WHERE id = 'xid-destination'",
            vec![],
        )
        .await,
        1
    );
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM title_external_ids WHERE id = 'xid-source'",
            vec![],
        )
        .await,
        0
    );

    // `idx_title_external_ids_library_lookup` is why the repoint is impossible:
    // a second (library_id, source, external_id) is rejected.
    let duplicate = SqlRuntime::execute_write(
        &datastore,
        "merge_test_duplicate_external_id",
        "INSERT INTO title_external_ids (id, title_id, source, external_id, created_at, facet,
                                         library_id)
         VALUES ('xid-duplicate', {}, 'tvdb', '12345', '2026-01-01T00:00:00Z', 'series',
                 'library-b')",
        vec![SqlArg::Text(DESTINATION.to_string())],
    )
    .await;
    assert!(
        duplicate.is_err(),
        "the library-scoped unique index should still reject a duplicate identity"
    );
}

#[tokio::test]
async fn history_keeps_its_title_and_its_episode_ids_are_remapped() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    insert_release_grabbed_event(&datastore, "event-1", SOURCE, &["s-e1", "s-e2"]).await;
    insert_release_grabbed_event(&datastore, "event-2", DESTINATION, &["d-e1"]).await;
    insert_history_event(&datastore, "history-1", SOURCE).await;

    let plan = plan_for(&store).await;
    let outcome = store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");
    assert_eq!(outcome.domain_event_payloads_rewritten, 1);

    // The column rewrite: `history_events.title_id`, and `domain_events`'
    // `title_id` and title `stream_id`.
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM domain_events
              WHERE title_id = {} OR (stream_kind = 'title' AND stream_id = {})",
            vec![
                SqlArg::Text(SOURCE.to_string()),
                SqlArg::Text(SOURCE.to_string()),
            ],
        )
        .await,
        0
    );
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM domain_events
              WHERE title_id = {} AND stream_kind = 'title' AND stream_id = {}",
            vec![
                SqlArg::Text(DESTINATION.to_string()),
                SqlArg::Text(DESTINATION.to_string()),
            ],
        )
        .await,
        2
    );
    // `history_events.title_id` is ON DELETE SET NULL, so a missed repoint would
    // read as an orphan rather than dangle.
    assert_eq!(
        text(
            &datastore,
            "SELECT title_id AS value FROM history_events WHERE id = 'history-1'",
            vec![],
        )
        .await
        .as_deref(),
        Some(DESTINATION)
    );

    // The payload rewrite: `$.data.episode_ids[]` now names destination
    // episodes, and the row still round-trips through the zstd codec.
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT event_id, payload_json FROM domain_events WHERE event_id = 'event-1'",
        &[],
    )
    .await
    .expect("the read should run")
    .expect("the event survives");
    let payload = decode_payload(&row)
        .expect("the payload should decode")
        .expect("the payload is not null");
    assert_eq!(
        payload_episode_ids(&payload),
        vec!["d-e1".to_string(), "d-e2".to_string()]
    );
    // The display snapshot is never touched.
    assert_eq!(
        payload["data"]["title"]["title_name"].as_str(),
        Some("Title")
    );
}

/// FR-086 for the merge: a plain move leaves the source title's acquisition rows
/// alone, but a merge retires the source through the delete path, which drops
/// them. So a live download refuses the merge instead.
#[tokio::test]
async fn a_queued_download_on_the_source_blocks_the_merge() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    run(
        &datastore,
        "INSERT INTO downloads (id, origin, created_at)
         VALUES ('submission-1', 'scryer_submission', '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await;
    run(
        &datastore,
        "INSERT INTO download_submissions (id, title_id, facet, download_client_type,
                                           download_client_item_id, tracked_state, submitted_at)
         VALUES ('submission-1', {}, 'series', 'sabnzbd', 'item-1', 'downloading',
                 '2026-01-01T00:00:00Z')",
        vec![SqlArg::Text(SOURCE.to_string())],
    )
    .await;

    let plan = plan_for(&store).await;
    assert!(plan.is_blocked());
    assert_eq!(
        plan.blocked()[0].reason,
        MergeBlockReason::ActiveAcquisitionWork
    );
    assert_eq!(plan.blocked()[0].source_id, "submission-1");

    // A finished download is not a claim.
    run(
        &datastore,
        "UPDATE download_submissions SET tracked_state = 'imported' WHERE id = 'submission-1'",
        vec![],
    )
    .await;
    assert!(!plan_for(&store).await.is_blocked());
}

/// A failure after the repoint and before the delete leaves the source title
/// whole: one transaction, all or nothing.
#[tokio::test]
async fn a_failure_after_the_repoint_leaves_the_source_intact() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    insert_history_event(&datastore, "history-1", SOURCE).await;

    let mut plan = plan_for(&store).await;
    assert!(!plan.is_blocked(), "blocked: {:?}", plan.blocked());
    // Force a failure between the repoint and the delete: a role row naming a
    // file that does not exist fails the `file_episode_map` foreign key after
    // `media_files` has already moved.
    plan.role_plan.rows.push(FileEpisodeRoleRow {
        file_id: "no-such-file".to_string(),
        episode_id: "d-e1".to_string(),
        role: MergedMediaRole::Additional,
        is_filler: false,
    });

    store
        .execute_title_merge(&plan)
        .await
        .expect_err("the transaction should abort");

    // Nothing moved: the source title, its files, and its history are all where
    // they were.
    assert_eq!(rows_referencing_source(&datastore, "titles", "id").await, 1);
    assert_eq!(
        rows_referencing_source(&datastore, "media_files", "title_id").await,
        2
    );
    assert_eq!(
        rows_referencing_source(&datastore, "history_events", "title_id").await,
        1
    );
}

#[tokio::test]
async fn a_second_operation_still_holding_the_source_title_blocks_the_merge() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    run(
        &datastore,
        "INSERT INTO location_operations (id, operation_type, execution_mode, state,
                                          plan_fingerprint, verification_depth, created_at,
                                          updated_at)
         VALUES ('op-other', 'cross_library_transfer', 'move_with_scryer', 'moving',
                 'fingerprint', 'quick', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await;
    run(
        &datastore,
        "INSERT INTO location_operation_owned_entities (operation_id, entity_type, entity_id)
         VALUES ('op-other', 'title', {})",
        vec![SqlArg::Text(SOURCE.to_string())],
    )
    .await;

    let snapshot = store
        .load_merge_snapshot(SOURCE, DESTINATION, None)
        .await
        .expect("the read phase should succeed");
    let plan = plan_merge(&snapshot);
    assert!(plan.is_blocked());
    assert_eq!(
        plan.blocked()[0].reason,
        MergeBlockReason::ResumableOperationHoldsSource
    );
    assert_eq!(plan.blocked()[0].source_id, "op-other");

    // The operation performing the merge legitimately owns the source title, so
    // it is excluded from its own check.
    let snapshot = store
        .load_merge_snapshot(SOURCE, DESTINATION, Some("op-other"))
        .await
        .expect("the read phase should succeed");
    assert!(!plan_merge(&snapshot).is_blocked());
}
