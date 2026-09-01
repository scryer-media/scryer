//! Store-level tests for the US7 merge engine, against a real sqlite database
//! with the full migration set replayed — so the cascades, the partial unique
//! indexes, and the FR-067 gate are exercised as they will behave in
//! production, not against a mock.

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
    let tags = serde_json::to_string(&tags.iter().map(|t| t.to_string()).collect::<Vec<_>>())
        .expect("tags encode");
    run(
        datastore,
        // `root_folder_id` is non-null by trigger since migration 0136.
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags,
                             external_ids, created_at, library_id, root_folder_id)
         VALUES ({}, {}, {}, 'series', 1, 'active', {}, '[]', '2026-01-01T00:00:00Z', {}, {})",
        vec![
            SqlArg::Text(id.to_string()),
            SqlArg::Text(format!("Title {id}")),
            SqlArg::Text(id.to_string()),
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
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, created_at)
         VALUES ({}, {}, {}, 100, 'complete', '2026-01-01T00:00:00Z')",
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

/// A two-season series on both sides: seasons 1–2, one episode each, one file
/// per episode on each side.
async fn seed_two_season_series(datastore: &StoreDatastore) {
    insert_title(datastore, SOURCE, "library-a", &["rewatch"]).await;
    insert_title(datastore, DESTINATION, "library-b", &["4k"]).await;

    for (title, prefix) in [(SOURCE, "s"), (DESTINATION, "d")] {
        for season in ["1", "2"] {
            insert_collection(
                datastore,
                &format!("{prefix}-c{season}"),
                title,
                season,
            )
            .await;
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
        .expect("the Group 0 read should succeed");
    plan_merge(&snapshot)
}

#[tokio::test]
async fn a_two_season_series_merges_files_history_and_tags_into_the_destination() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    insert_external_id(&datastore, "xid-source", SOURCE, "library-a").await;
    insert_external_id(&datastore, "xid-destination", DESTINATION, "library-b").await;
    insert_release_grabbed_event(&datastore, "event-1", SOURCE, &["s-e1", "s-e2"]).await;

    let plan = plan_for(&store).await;
    assert!(!plan.is_blocked(), "blocked: {:?}", plan.blocked());
    let map = plan.require_identity_map().expect("a complete map");
    assert_eq!(map.episode("s-e1"), Some("d-e1"));
    assert_eq!(map.episode("s-e2"), Some("d-e2"));
    assert_eq!(map.collection("s-c1"), Some("d-c1"));

    let outcome = store
        .execute_title_merge(&plan)
        .await
        .expect("the merge transaction should commit");
    assert_eq!(outcome.rows_affected.get("1:media_files"), Some(&2));
    assert_eq!(outcome.rows_affected.get("5:titles"), Some(&1));

    // FR-067: the source title is gone, and its files came with it.
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM titles WHERE id = {}",
            vec![SqlArg::Text(SOURCE.to_string())],
        )
        .await,
        0
    );
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
    // OQ9: the free-form tag unions onto the destination's array.
    let tags = text(
        &datastore,
        "SELECT tags AS value FROM titles WHERE id = {}",
        vec![SqlArg::Text(DESTINATION.to_string())],
    )
    .await
    .expect("the destination title survives");
    assert_eq!(tags, r#"["4k","rewatch"]"#);
}

#[tokio::test]
async fn an_unmappable_episode_blocks_the_plan_and_names_the_referencing_table() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    // A third source season the destination does not have, wanted by the
    // acquisition loop.
    insert_episode(&datastore, "s-e3", SOURCE, None, "3", "1").await;
    insert_wanted_item(&datastore, "wanted-orphan", SOURCE, "s-e3", "wanted").await;

    let plan = plan_for(&store).await;
    assert!(plan.is_blocked());
    assert!(plan.identity_map.is_none());
    let tables: Vec<&str> = plan
        .blocked()
        .iter()
        .map(|record| record.table.as_str())
        .collect();
    assert!(tables.contains(&"episodes"));
    assert!(tables.contains(&"wanted_items"));
    assert!(
        plan.blocked()
            .iter()
            .all(|record| record.source_id == "s-e3"
                && record.reason == MergeBlockReason::UnmappedEpisode)
    );

    // FR-066's block costs no rollback: the execution refuses before it writes.
    let error = store
        .execute_title_merge(&plan)
        .await
        .expect_err("a blocked plan must not execute");
    assert!(error.to_string().contains("s-e3"), "{error}");
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM titles WHERE id = {}",
            vec![SqlArg::Text(SOURCE.to_string())],
        )
        .await,
        1
    );
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
    // Exactly one demotion: the incoming primary. The additional is not a
    // change, so it is not a preview line item.
    let season_one_changes: Vec<&_> = plan
        .summary
        .role_changes
        .iter()
        .filter(|change| change.destination_episode_id == "d-e1")
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
async fn title_external_ids_cascade_rather_than_repoint_and_the_unique_index_survives() {
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

    // `idx_title_external_ids_library_lookup` is the one unique index that
    // survived migrations 0079/0104/0105, and it is why the repoint is
    // impossible: a second (library_id, source, external_id) is rejected.
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
async fn reserved_tags_are_destination_wins_and_the_conflict_is_previewed() {
    let (store, datastore) = test_store().await;
    insert_title(
        &datastore,
        SOURCE,
        "library-a",
        &["scryer:quality-profile:profile-source", "anime"],
    )
    .await;
    insert_title(
        &datastore,
        DESTINATION,
        "library-b",
        &["scryer:quality-profile:profile-destination", "Anime", "4k"],
    )
    .await;

    let plan = plan_for(&store).await;
    assert_eq!(plan.summary.reserved_tag_conflicts.len(), 1);
    assert_eq!(
        plan.summary.reserved_tag_conflicts[0].prefix,
        "scryer:quality-profile:"
    );
    // "anime" already exists on the destination in a different case, so nothing
    // is added.
    assert!(plan.summary.free_form_tags_added.is_empty());

    store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");

    let tags = text(
        &datastore,
        "SELECT tags AS value FROM titles WHERE id = {}",
        vec![SqlArg::Text(DESTINATION.to_string())],
    )
    .await
    .expect("the destination title survives");
    assert_eq!(
        tags,
        r#"["scryer:quality-profile:profile-destination","Anime","4k"]"#
    );
    // Exactly one quality-profile tag, so `find_map(strip_prefix(..))` cannot
    // resolve FR-063's setting by array order.
    assert_eq!(tags.matches("scryer:quality-profile:").count(), 1);
}

#[tokio::test]
async fn wanted_items_keep_the_destination_row_and_carry_the_source_only_one() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    // Both sides want season 1; only the source wants season 2.
    insert_wanted_item(&datastore, "wanted-source-1", SOURCE, "s-e1", "grabbed").await;
    insert_wanted_item(&datastore, "wanted-destination-1", DESTINATION, "d-e1", "wanted").await;
    insert_wanted_item(&datastore, "wanted-source-2", SOURCE, "s-e2", "wanted").await;

    let plan = plan_for(&store).await;
    store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");

    // Destination-wins on UNIQUE(title_id, episode_id): the source's colliding
    // row is dropped, not adopted.
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM wanted_items WHERE id = 'wanted-source-1'",
            vec![],
        )
        .await,
        0
    );
    assert_eq!(
        text(
            &datastore,
            "SELECT status AS value FROM wanted_items WHERE episode_id = 'd-e1'",
            vec![],
        )
        .await
        .as_deref(),
        Some("wanted")
    );
    // The source-only row carries, remapped onto the destination episode.
    assert_eq!(
        text(
            &datastore,
            "SELECT episode_id AS value FROM wanted_items WHERE id = 'wanted-source-2'",
            vec![],
        )
        .await
        .as_deref(),
        Some("d-e2")
    );
    assert_eq!(
        text(
            &datastore,
            "SELECT title_id AS value FROM wanted_items WHERE id = 'wanted-source-2'",
            vec![],
        )
        .await
        .as_deref(),
        Some(DESTINATION)
    );
}

#[tokio::test]
async fn domain_events_keep_their_title_and_their_episode_ids_are_remapped() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    insert_release_grabbed_event(&datastore, "event-1", SOURCE, &["s-e1", "s-e2"]).await;
    // An event type outside the OQ8 list: its columns are rewritten, its
    // payload is not decompressed at all.
    insert_release_grabbed_event(&datastore, "event-2", DESTINATION, &["d-e1"]).await;

    let plan = plan_for(&store).await;
    let outcome = store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");
    assert_eq!(outcome.domain_event_payloads_rewritten, 1);

    // The column rewrite: both `title_id` and the title `stream_id`.
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

#[tokio::test]
async fn the_fr067_gate_aborts_the_transaction_when_a_no_fk_row_is_left_behind() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    // A third source episode carrying a `subtitle_downloads` row.
    // `subtitle_downloads.episode_id` has no foreign key, so nothing in the
    // database would ever complain about it dangling.
    insert_episode(&datastore, "s-e3", SOURCE, None, "3", "1").await;
    run(
        &datastore,
        "INSERT INTO subtitle_downloads (id, media_file_id, title_id, episode_id, language,
                                         provider, file_path, downloaded_at)
         VALUES ('subtitle-1', 's-f1', {}, 's-e3', 'en', 'opensubtitles', '/s/S3E01.en.srt',
                 '2026-01-01T00:00:00Z')",
        vec![SqlArg::Text(SOURCE.to_string())],
    )
    .await;

    // Group 0 would block on `s-e3`. This test is about what happens when a
    // reference sweep misses one, so the plan is built from a snapshot that
    // does not know the episode exists — exactly the failure the gate is for.
    let mut snapshot = store
        .load_merge_snapshot(SOURCE, DESTINATION, None)
        .await
        .expect("the Group 0 read should succeed");
    snapshot
        .source_episodes
        .retain(|episode| episode.id != "s-e3");
    snapshot.episode_references.remove("subtitle_downloads");
    let plan = plan_merge(&snapshot);
    assert!(!plan.is_blocked(), "blocked: {:?}", plan.blocked());

    let error = store
        .execute_title_merge(&plan)
        .await
        .expect_err("the FR-067 gate should refuse the delete");
    let message = error.to_string();
    assert!(message.contains("FR-067 gate"), "{message}");
    assert!(message.contains("subtitle_downloads.episode_id"), "{message}");

    // The whole transaction rolled back: the source title, its files, and its
    // episodes are all still where they were.
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM titles WHERE id = {}",
            vec![SqlArg::Text(SOURCE.to_string())],
        )
        .await,
        1
    );
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM media_files WHERE title_id = {}",
            vec![SqlArg::Text(SOURCE.to_string())],
        )
        .await,
        2
    );
}

#[tokio::test]
async fn the_merging_operations_own_claims_collapse_onto_the_surviving_title() {
    let (store, datastore) = test_store().await;
    seed_two_season_series(&datastore).await;
    run(
        &datastore,
        "INSERT INTO location_operations (id, operation_type, execution_mode, state,
                                          plan_fingerprint, verification_depth, created_at,
                                          updated_at)
         VALUES ('op-self', 'cross_library_transfer', 'move_with_scryer', 'moving',
                 'fingerprint', 'quick', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await;
    // The operation running the merge holds both titles, which is the common
    // case: `idx_location_operation_owned_entities_active` is unique on
    // `(entity_type, entity_id)` across every operation, so the source claim
    // cannot simply be repointed.
    for title in [SOURCE, DESTINATION] {
        run(
            &datastore,
            "INSERT INTO location_operation_owned_entities (operation_id, entity_type, entity_id)
             VALUES ('op-self', 'title', {})",
            vec![SqlArg::Text(title.to_string())],
        )
        .await;
    }

    let snapshot = store
        .load_merge_snapshot(SOURCE, DESTINATION, Some("op-self"))
        .await
        .expect("the Group 0 read should succeed");
    let plan = plan_merge(&snapshot);
    assert!(!plan.is_blocked(), "blocked: {:?}", plan.blocked());
    store
        .execute_title_merge(&plan)
        .await
        .expect("the merge should commit");

    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM location_operation_owned_entities
              WHERE entity_type = 'title' AND entity_id = {}",
            vec![SqlArg::Text(SOURCE.to_string())],
        )
        .await,
        0
    );
    assert_eq!(
        scalar(
            &datastore,
            "SELECT COUNT(*) AS row_count FROM location_operation_owned_entities
              WHERE entity_type = 'title' AND entity_id = {}",
            vec![SqlArg::Text(DESTINATION.to_string())],
        )
        .await,
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

    // OQ7(a): the second operation is a hard block.
    let snapshot = store
        .load_merge_snapshot(SOURCE, DESTINATION, None)
        .await
        .expect("the Group 0 read should succeed");
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
        .expect("the Group 0 read should succeed");
    assert!(!plan_merge(&snapshot).is_blocked());
}
