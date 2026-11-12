#![recursion_limit = "256"]

//! Bulk title deletion measurement at catalog scale.
//!
//! Seeds a synthetic SQLite catalog on disk: 10,000 movies with one media
//! file each, 2,000 series with two seasons of ten episodes (fifteen media
//! files and five wanted items each), history rows and a large domain-event
//! table, with the notification dispatcher running as the server runs it.
//! Then times one title deletion job over every series with files kept on
//! disk, and reads the write-ahead log to count the write transactions and
//! pages the job committed.
//!
//! The measurements are ignored by default: they are not a regression gate.
//! `title_delete_job_matches_one_title_at_a_time` is, and always runs. Run the
//! measurements with
//! `cargo nextest run -p scryer --test integration_title_delete_bench
//! --run-ignored only --no-capture`. Every name is synthetic.

mod common;

use common::TestContext;
use scryer_application::{DeleteTitlesJobItem, DeleteTitlesJobRequest};

const MOVIES: usize = 10_000;
const SERIES: usize = 2_000;
const SEASONS: usize = 2;
const EPISODES_PER_SEASON: usize = 10;
const DOMAIN_EVENTS: usize = 830_000;

async fn library_root(ctx: &TestContext, facet: &str, path: &str) -> (String, String) {
    let row: (String,) = sqlx::query_as("SELECT id FROM libraries WHERE facet = ? LIMIT 1")
        .bind(facet)
        .fetch_one(&ctx.db.pool)
        .await
        .expect("seeded library");
    let root_id = format!("synthetic-root-{facet}");
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&root_id)
    .bind(&row.0)
    .bind(path)
    .bind(path)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db.pool)
    .await
    .expect("insert synthetic root");
    (row.0, root_id)
}

async fn insert_title(
    tx: &mut sqlx::SqliteConnection,
    id: &str,
    name: &str,
    facet: &str,
    library: &str,
    root: &str,
    created: &str,
) {
    sqlx::query(
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, created_at, slug, year, \
         library_id, root_folder_id) VALUES (?, ?, ?, ?, 1, ?, ?, 2001, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(name.to_lowercase())
    .bind(facet)
    .bind(created)
    .bind(name.to_lowercase().replace(' ', "-"))
    .bind(library)
    .bind(root)
    .execute(&mut *tx)
    .await
    .expect("insert title");
    for event in 0..3 {
        sqlx::query(
            "INSERT INTO history_events (id, event_type, actor_user_id, title_id, message, occurred_at, source, created_at, metadata_json) \
             VALUES (?, 'test', NULL, ?, 'synthetic history', ?, NULL, ?, NULL)",
        )
        .bind(format!("{id}-history-{event}"))
        .bind(id)
        .bind(created)
        .bind(created)
        .execute(&mut *tx)
        .await
        .expect("insert history");
    }
}

async fn insert_media_file(
    tx: &mut sqlx::SqliteConnection,
    file_id: &str,
    title_id: &str,
    path: &str,
    created: &str,
) {
    sqlx::query(
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, created_at, role) \
         VALUES (?, ?, ?, 1000000, 'scanned', ?, 'primary')",
    )
    .bind(file_id)
    .bind(title_id)
    .bind(path)
    .bind(created)
    .execute(&mut *tx)
    .await
    .expect("insert media file");
}

/// Seeds the catalog and returns the series ids, which the job deletes.
async fn seed(ctx: &TestContext, root: &std::path::Path) -> Vec<String> {
    let movie_root_path = root.join("movies").to_string_lossy().to_string();
    let series_root_path = root.join("series").to_string_lossy().to_string();
    let (movie_library, movie_root) = library_root(ctx, "movie", &movie_root_path).await;
    let (series_library, series_root) = library_root(ctx, "series", &series_root_path).await;
    let created = chrono::Utc::now().to_rfc3339();
    let mut tx = ctx.db.pool.begin().await.expect("begin seed");

    for index in 0..MOVIES {
        let id = format!("bench-movie-{index:05}");
        let name = format!("Synthetic Fixture Movie {index:05}");
        insert_title(
            &mut tx,
            &id,
            &name,
            "movie",
            &movie_library,
            &movie_root,
            &created,
        )
        .await;
        insert_media_file(
            &mut tx,
            &format!("{id}-file"),
            &id,
            &format!("{movie_root_path}/{name}/{name}.mkv"),
            &created,
        )
        .await;
    }

    let mut series = Vec::new();
    for index in 0..SERIES {
        let id = format!("bench-series-{index:05}");
        let name = format!("Synthetic Fixture Series {index:05}");
        insert_title(
            &mut tx,
            &id,
            &name,
            "series",
            &series_library,
            &series_root,
            &created,
        )
        .await;
        for season in 1..=SEASONS {
            let collection_id = format!("{id}-season-{season}");
            sqlx::query(
                "INSERT INTO collections (id, title_id, collection_type, collection_index, created_at, monitored) \
                 VALUES (?, ?, 'season', ?, ?, 1)",
            )
            .bind(&collection_id)
            .bind(&id)
            .bind(season.to_string())
            .bind(&created)
            .execute(&mut *tx)
            .await
            .expect("insert season");
            for episode in 1..=EPISODES_PER_SEASON {
                let episode_id = format!("{id}-s{season:02}e{episode:02}");
                sqlx::query(
                    "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number, \
                     season_number, title, air_date, created_at, monitored) \
                     VALUES (?, ?, ?, 'standard', ?, ?, ?, '2001-01-01', ?, 1)",
                )
                .bind(&episode_id)
                .bind(&id)
                .bind(&collection_id)
                .bind(episode.to_string())
                .bind(season.to_string())
                .bind(format!("Synthetic Episode {episode:02}"))
                .bind(&created)
                .execute(&mut *tx)
                .await
                .expect("insert episode");
                if episode % 4 == 0 {
                    sqlx::query(
                        "INSERT INTO wanted_items (id, title_id, episode_id, media_type, status, created_at, updated_at) \
                         VALUES (?, ?, ?, 'episode', 'wanted', ?, ?)",
                    )
                    .bind(format!("{episode_id}-wanted"))
                    .bind(&id)
                    .bind(&episode_id)
                    .bind(&created)
                    .bind(&created)
                    .execute(&mut *tx)
                    .await
                    .expect("insert wanted item");
                    continue;
                }
                let file_id = format!("{episode_id}-file");
                insert_media_file(
                    &mut tx,
                    &file_id,
                    &id,
                    &format!("{series_root_path}/{name}/{episode_id}.mkv"),
                    &created,
                )
                .await;
                sqlx::query(
                    "INSERT INTO file_episode_map (file_id, episode_id, role) VALUES (?, ?, 'primary')",
                )
                .bind(&file_id)
                .bind(&episode_id)
                .execute(&mut *tx)
                .await
                .expect("map episode file");
            }
        }
        series.push(id);
    }

    // A long event history: most events name a title, spread over every
    // title in the catalog, as a library that has been running a while.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < {last}) \
         INSERT INTO domain_events (event_id, occurred_at, title_id, facet, schema_version, \
           stream_kind, stream_id, event_type, payload_json) \
         SELECT 'bench-event-' || i, ?, \
           CASE WHEN i % 5 = 0 THEN NULL \
                WHEN i % {titles} < {series} THEN printf('bench-series-%05d', i % {titles}) \
                ELSE printf('bench-movie-%05d', (i % {titles}) - {series}) END, \
           NULL, 1, 'title', NULL, \
           CASE i % 4 WHEN 0 THEN 'title_added' WHEN 1 THEN 'import_requested' \
                WHEN 2 THEN 'acquisition_search_completed' ELSE 'library_scan_progressed' END, \
           '{{}}' FROM n",
        last = DOMAIN_EVENTS - 1,
        titles = MOVIES + SERIES,
        series = SERIES,
    )))
    .bind(&created)
    .execute(&mut *tx)
    .await
    .expect("insert domain events");

    tx.commit().await.expect("commit seed");
    sqlx::query("ANALYZE")
        .execute(&ctx.db.pool)
        .await
        .expect("analyze");
    series
}

async fn count(ctx: &TestContext, sql: &'static str) -> i64 {
    let row: (i64,) = sqlx::query_as(sql)
        .fetch_one(&ctx.db.pool)
        .await
        .expect("count");
    row.0
}

/// Valid frames and commit frames in a write-ahead log that has not been
/// checkpointed. Each commit frame ends one write transaction.
fn wal_frames(path: &std::path::Path) -> (u64, u64) {
    let Ok(bytes) = std::fs::read(path) else {
        return (0, 0);
    };
    if bytes.len() < 32 {
        return (0, 0);
    }
    let page_size = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let salt = &bytes[16..24];
    let (mut frames, mut commits) = (0, 0);
    let mut offset = 32;
    while offset + 24 + page_size <= bytes.len() {
        let header = &bytes[offset..offset + 24];
        if &header[8..16] != salt {
            break;
        }
        frames += 1;
        if header[4..8] != [0, 0, 0, 0] {
            commits += 1;
        }
        offset += 24 + page_size;
    }
    (frames, commits)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement; run explicitly"]
async fn bulk_title_delete_at_scale() {
    run(false, false).await;
}

/// The same job with checkpoints off, so the write-ahead log holds every
/// page the job wrote and can be counted.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement; run explicitly"]
async fn bulk_title_delete_at_scale_write_counts() {
    run(true, false).await;
}

/// The same titles deleted one at a time through the single-title delete,
/// the cost the job paid per title before it deleted in chunks.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement; run explicitly"]
async fn one_at_a_time_title_delete_at_scale() {
    run(false, true).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement; run explicitly"]
async fn one_at_a_time_title_delete_at_scale_write_counts() {
    run(true, true).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement; run explicitly"]
async fn probe_unindexed_foreign_keys() {
    let ctx = TestContext::new().await;
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT m.name, f.\"table\", f.\"from\", f.on_delete \
         FROM sqlite_master m, pragma_foreign_key_list(m.name) f \
         WHERE m.type = 'table' AND NOT EXISTS ( \
           SELECT 1 FROM pragma_index_list(m.name) il, pragma_index_info(il.name) ii \
           WHERE ii.seqno = 0 AND ii.name = f.\"from\") \
         AND NOT EXISTS (SELECT 1 FROM pragma_table_info(m.name) ti WHERE ti.pk = 1 AND ti.name = f.\"from\") \
         ORDER BY f.\"table\", m.name",
    )
    .fetch_all(&ctx.db.pool)
    .await
    .expect("probe");
    for (child, parent, column, on_delete) in rows {
        eprintln!("UNINDEXED-FK {child}.{column} -> {parent} on_delete={on_delete}");
    }
}

/// Print a measurement, and append it to the file `TITLE_DELETE_BENCH_OUT`
/// names when it is set, since a test runner may not show test output.
fn report(line: String) {
    eprintln!("{line}");
    if let Some(path) = std::env::var_os("TITLE_DELETE_BENCH_OUT") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// The query plan of every statement a chunked title delete issues, by
/// title id, on the migrated schema. A `SCAN` of a table that grows is a
/// statement without an index.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement; run explicitly"]
async fn title_delete_statement_plans() {
    let ctx = TestContext::new().await;
    sqlx::query("ANALYZE")
        .execute(&ctx.db.pool)
        .await
        .expect("analyze");
    const STATEMENTS: &[&str] = &[
        "DELETE FROM manual_import_selection_candidates WHERE selection_id IN (SELECT id FROM manual_import_selections WHERE title_id IN (?, ?))",
        "DELETE FROM manual_import_selections WHERE title_id IN (?, ?)",
        "DELETE FROM pending_releases WHERE title_id IN (?, ?)",
        "DELETE FROM wanted_items WHERE title_id IN (?, ?)",
        "DELETE FROM download_submission_episode_links WHERE download_id IN (SELECT id FROM download_submissions WHERE title_id IN (?, ?))",
        "DELETE FROM download_submissions WHERE title_id IN (?, ?)",
        "DELETE FROM blocklist WHERE title_id IN (?, ?)",
        "DELETE FROM library_probe_signatures WHERE title_id IN (?, ?)",
        "DELETE FROM library_scan_unmatched_items WHERE title_id IN (?, ?)",
        "DELETE FROM indexer_search_learning WHERE title_id IN (?, ?)",
        "DELETE FROM monitor_selections WHERE owner_kind = ? AND owner_id IN (?, ?)",
        "DELETE FROM titles WHERE id IN (?, ?)",
        "UPDATE lifecycle_claims SET state = ?, released_reason = ?, updated_at = ? WHERE title_id IN (?, ?) AND state IN (?, ?)",
        "SELECT entity_id, operation_id FROM location_operation_owned_entities WHERE entity_type = ? AND entity_id IN (?, ?) AND released_at IS NULL",
    ];
    for statement in STATEMENTS {
        let explained = format!("EXPLAIN QUERY PLAN {statement}");
        let mut query =
            sqlx::query_as::<_, (i64, i64, i64, String)>(sqlx::AssertSqlSafe(&*explained));
        for _ in 0..statement.matches('?').count() {
            query = query.bind("synthetic");
        }
        match query.fetch_all(&ctx.db.pool).await {
            Ok(rows) => {
                let plan = rows
                    .into_iter()
                    .map(|row| row.3)
                    .collect::<Vec<_>>()
                    .join(" | ");
                report(format!("PLAN {statement}\n     -> {plan}"));
            }
            Err(error) => report(format!("PLAN {statement}\n     -> error: {error}")),
        }
    }
}

/// Every growing table a title delete clears by title id is searched by an
/// index rather than read whole.
#[tokio::test(flavor = "multi_thread")]
async fn title_delete_statements_search_by_title_index() {
    let ctx = TestContext::new().await;
    sqlx::query("ANALYZE")
        .execute(&ctx.db.pool)
        .await
        .expect("analyze");
    for (statement, index) in [
        (
            "DELETE FROM manual_import_selections WHERE title_id IN (?, ?)",
            "idx_manual_import_selections_title_id",
        ),
        (
            "DELETE FROM library_scan_unmatched_items WHERE title_id IN (?, ?)",
            "idx_library_scan_unmatched_items_title_id",
        ),
        (
            "DELETE FROM indexer_search_learning WHERE title_id IN (?, ?)",
            "idx_indexer_search_learning_title_id",
        ),
    ] {
        let explained = format!("EXPLAIN QUERY PLAN {statement}");
        let rows = sqlx::query_as::<_, (i64, i64, i64, String)>(sqlx::AssertSqlSafe(&*explained))
            .bind("synthetic-a")
            .bind("synthetic-b")
            .fetch_all(&ctx.db.pool)
            .await
            .expect("plan");
        let plan = rows
            .into_iter()
            .map(|row| row.3)
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(plan.contains(index), "{statement}: {plan}");
        assert!(!plan.contains("SCAN "), "{statement}: {plan}");
    }
}

async fn run(count_writes: bool, one_at_a_time: bool) {
    if count_writes {
        // SAFETY: set before any connection opens, in this test's own process.
        unsafe { std::env::set_var("SCRYER_SQLITE_WAL_AUTOCHECKPOINT_PAGES", "0") };
    }
    let root = tempfile::tempdir().expect("root");
    let db = tempfile::tempdir().expect("db dir");
    let db_path = db.path().join("bench.db");
    let ctx = TestContext::new_with_database_path(&db_path.to_string_lossy()).await;
    let seed_started = std::time::Instant::now();
    let series = seed(&ctx, root.path()).await;
    let seeded = seed_started.elapsed();

    let cancel = tokio_util::sync::CancellationToken::new();
    let dispatcher = tokio::spawn(scryer_application::start_notification_dispatcher(
        ctx.app.clone(),
        cancel.clone(),
    ));

    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default admin");
    let before_titles = count(&ctx, "SELECT COUNT(*) FROM titles").await;
    let wal_path = db.path().join("bench.db-wal");
    let (frames_before, commits_before) = wal_frames(&wal_path);

    let started = std::time::Instant::now();
    let mut admitted = std::time::Duration::ZERO;
    if one_at_a_time {
        for title_id in &series {
            ctx.app
                .delete_title(&admin, title_id, false, None)
                .await
                .expect("delete title");
        }
    } else {
        let accepted = ctx
            .app
            .start_delete_titles_job(
                &admin,
                DeleteTitlesJobRequest {
                    items: series
                        .iter()
                        .map(|title_id| DeleteTitlesJobItem {
                            title_id: title_id.clone(),
                            preview_fingerprint: None,
                        })
                        .collect(),
                    delete_files_on_disk: false,
                    typed_confirmation: None,
                },
            )
            .await
            .expect("start delete job");
        admitted = started.elapsed();
        let run_id = accepted.job_run.id.clone();
        common::wait_until("title deletion job completes", || {
            let admin = &admin;
            let run_id = run_id.clone();
            let ctx = &ctx;
            async move {
                ctx.app
                    .list_recent_job_runs(admin, 20)
                    .await
                    .expect("list job runs")
                    .into_iter()
                    .any(|run| run.id == run_id && run.completed_at.is_some())
            }
        })
        .await;
    }
    let total = started.elapsed();
    let (frames_after, commits_after) = wal_frames(&wal_path);

    cancel.cancel();
    let _ = dispatcher.await;

    let after_titles = count(&ctx, "SELECT COUNT(*) FROM titles").await;
    assert_eq!(before_titles - after_titles, SERIES as i64);
    let per_title = |value: u64| value as f64 / SERIES as f64;
    report(format!(
        "BENCH [scale{}]: seeded {MOVIES} movies, {SERIES} series ({} episodes each), {DOMAIN_EVENTS} domain events in {:.1} s; deleted {SERIES} series in {:.1} ms (admission {:.1} ms), {:.2} ms per title",
        if one_at_a_time {
            ", one at a time"
        } else {
            ", job"
        },
        SEASONS * EPISODES_PER_SEASON,
        seeded.as_secs_f64(),
        total.as_secs_f64() * 1000.0,
        admitted.as_secs_f64() * 1000.0,
        total.as_secs_f64() * 1000.0 / SERIES as f64,
    ));
    if count_writes {
        let commits = commits_after - commits_before;
        let frames = frames_after - frames_before;
        report(format!(
            "BENCH [writes]: {commits} write transactions ({:.2} per title), {frames} pages written ({:.1} per title)",
            per_title(commits),
            per_title(frames),
        ));
    }
}

/// Rows a title delete removes or keeps, seeded for one title.
async fn seed_title_records(ctx: &TestContext, title_id: &str, now: &str) {
    let statements: [(&str, Vec<String>); 10] = [
        (
            "INSERT INTO manual_import_selections (id, actor_user_id, title_id, source_system, source_ref, created_at, updated_at) VALUES (?, 'synthetic-user', ?, 'synthetic', ?, ?, ?)",
            vec![
                format!("{title_id}-sel"),
                title_id.into(),
                format!("{title_id}-ref"),
                now.into(),
                now.into(),
            ],
        ),
        (
            "INSERT INTO manual_import_selection_candidates (id, selection_id, canonical_path, created_at) VALUES (?, ?, ?, ?)",
            vec![
                format!("{title_id}-cand"),
                format!("{title_id}-sel"),
                format!("/synthetic/{title_id}.mkv"),
                now.into(),
            ],
        ),
        (
            "INSERT INTO pending_releases (id, wanted_item_id, title_id, release_title, release_score, added_at, delay_until) VALUES (?, ?, ?, 'Synthetic.Release', 1, ?, ?)",
            vec![
                format!("{title_id}-pending"),
                format!("{title_id}-wanted"),
                title_id.into(),
                now.into(),
                now.into(),
            ],
        ),
        (
            "INSERT INTO wanted_items (id, title_id, media_type, status, created_at, updated_at) VALUES (?, ?, 'movie', 'wanted', ?, ?)",
            vec![
                format!("{title_id}-wanted"),
                title_id.into(),
                now.into(),
                now.into(),
            ],
        ),
        (
            "INSERT INTO blocklist (id, title_id, release_name, created_at) VALUES (?, ?, 'Synthetic.Blocked', ?)",
            vec![format!("{title_id}-block"), title_id.into(), now.into()],
        ),
        (
            "INSERT INTO library_probe_signatures (title_id, path, created_at, updated_at) VALUES (?, ?, ?, ?)",
            vec![
                title_id.into(),
                format!("/synthetic/{title_id}"),
                now.into(),
                now.into(),
            ],
        ),
        (
            "INSERT INTO library_scan_unmatched_items (id, facet, scan_session_id, scan_root, item_path, display_name, query, reason_code, created_at, updated_at, title_id) VALUES (?, 'movie', 'synthetic-scan', '/synthetic', ?, 'Synthetic', 'synthetic', 'synthetic', ?, ?, ?)",
            vec![
                format!("{title_id}-unmatched"),
                format!("/synthetic/{title_id}/item.mkv"),
                now.into(),
                now.into(),
                title_id.into(),
            ],
        ),
        (
            "INSERT INTO indexer_search_learning (indexer_id, title_id, facet, strategy_key) VALUES ('synthetic-indexer', ?, 'movie', 'synthetic')",
            vec![title_id.into()],
        ),
        (
            "INSERT INTO monitor_selections (owner_kind, owner_id, entry_kind, entry_key, created_at, updated_at) VALUES ('title', ?, 'season', '1', ?, ?)",
            vec![title_id.into(), now.into(), now.into()],
        ),
        (
            "INSERT INTO lifecycle_claims (id, title_id, producer, state) VALUES (?, ?, 'synthetic', 'active')",
            vec![format!("{title_id}-claim"), title_id.into()],
        ),
    ];
    for (sql, binds) in statements {
        let mut query = sqlx::query(sql);
        for bind in binds {
            query = query.bind(bind);
        }
        query
            .execute(&ctx.db.pool)
            .await
            .unwrap_or_else(|error| panic!("seed {sql}: {error}"));
    }
}

/// Every row of the tables a title delete touches, as text, sorted, with
/// the columns that hold the time of the delete left out.
async fn table_snapshot(ctx: &TestContext) -> Vec<String> {
    const TABLES: &[(&str, &str)] = &[
        ("titles", "id"),
        ("collections", "id"),
        ("episodes", "id"),
        ("media_files", "id"),
        ("file_episode_map", "file_id || episode_id"),
        ("wanted_items", "id"),
        ("history_events", "id"),
        ("manual_import_selections", "id"),
        ("manual_import_selection_candidates", "id"),
        ("pending_releases", "id"),
        ("blocklist", "id"),
        ("library_probe_signatures", "title_id"),
        ("library_scan_unmatched_items", "id"),
        ("indexer_search_learning", "title_id"),
        ("monitor_selections", "owner_id"),
        (
            "lifecycle_claims",
            "id || ':' || state || ':' || coalesce(released_reason, '')",
        ),
        ("download_submissions", "id"),
        ("title_search_terms", "title_id"),
        ("title_recycle_purges", "title_id || ':' || library_id"),
    ];
    let mut rows = Vec::new();
    for (table, key) in TABLES {
        let found: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT CAST({key} AS TEXT) FROM {table}"
        )))
        .fetch_all(&ctx.db.pool)
        .await
        .unwrap_or_else(|error| panic!("snapshot {table}: {error}"));
        rows.extend(found.into_iter().map(|(key,)| format!("{table}:{key}")));
    }
    rows.sort();
    rows
}

/// The title-deleted events, in order, with the time they happened left out.
async fn title_deleted_events(ctx: &TestContext) -> Vec<(String, String)> {
    let rows: Vec<(Option<String>, Vec<u8>)> = sqlx::query_as(
        "SELECT title_id, CAST(payload_json AS BLOB) FROM domain_events \
         WHERE event_type = 'title_deleted' ORDER BY sequence",
    )
    .fetch_all(&ctx.db.pool)
    .await
    .expect("title-deleted events");
    rows.into_iter()
        .map(|(title_id, payload)| {
            (
                title_id.unwrap_or_default(),
                String::from_utf8_lossy(&payload).to_string(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn title_delete_job_matches_one_title_at_a_time() {
    async fn scenario(
        bulk: bool,
    ) -> (
        Vec<(String, Option<String>)>,
        Vec<String>,
        Vec<(String, String)>,
    ) {
        let root = tempfile::tempdir().expect("root");
        let ctx = TestContext::new().await;
        let movie_root = root.path().join("movies").to_string_lossy().to_string();
        let series_root = root.path().join("series").to_string_lossy().to_string();
        let (movie_library, movie_root_id) = library_root(&ctx, "movie", &movie_root).await;
        let (series_library, series_root_id) = library_root(&ctx, "series", &series_root).await;
        let now = chrono::Utc::now().to_rfc3339();
        let mut ids = Vec::new();
        {
            let mut tx = ctx.db.pool.begin().await.expect("begin");
            for index in 0..80 {
                let (id, facet, library, root) = if index % 2 == 0 {
                    (
                        format!("cmp-movie-{index:03}"),
                        "movie",
                        &movie_library,
                        &movie_root_id,
                    )
                } else {
                    (
                        format!("cmp-series-{index:03}"),
                        "series",
                        &series_library,
                        &series_root_id,
                    )
                };
                let name = format!("Synthetic Compare {index:03}");
                insert_title(&mut tx, &id, &name, facet, library, root, &now).await;
                insert_media_file(
                    &mut tx,
                    &format!("{id}-file"),
                    &id,
                    &format!("/synthetic/{id}.mkv"),
                    &now,
                )
                .await;
                ids.push(id);
            }
            tx.commit().await.expect("commit");
        }
        for id in &ids {
            seed_title_records(&ctx, id, &now).await;
        }
        // One title with a recorded download takes the title-by-title cleanup.
        sqlx::query("INSERT INTO downloads (id, origin, created_at) VALUES ('00000000-0000-4000-8000-0000000c0de0', 'scryer_submission', ?)")
            .bind(&now)
            .execute(&ctx.db.pool)
            .await
            .expect("seed download");
        sqlx::query(
            "INSERT INTO download_submissions (id, title_id, facet, download_client_type, download_client_item_id, source_title, submitted_at) \
             VALUES ('00000000-0000-4000-8000-0000000c0de0', ?, 'movie', 'synthetic', 'synthetic-item', 'Synthetic.Release', ?)",
        )
        .bind(&ids[4])
        .bind(&now)
        .execute(&ctx.db.pool)
        .await
        .expect("seed download submission");
        // The last ten titles are kept.
        let deleted = ids[..70].to_vec();
        let admin = ctx.app.find_or_create_default_user().await.expect("admin");

        let mut results = Vec::new();
        if bulk {
            let accepted = ctx
                .app
                .start_delete_titles_job(
                    &admin,
                    DeleteTitlesJobRequest {
                        items: deleted
                            .iter()
                            .map(|title_id| DeleteTitlesJobItem {
                                title_id: title_id.clone(),
                                preview_fingerprint: None,
                            })
                            .collect(),
                        delete_files_on_disk: false,
                        typed_confirmation: None,
                    },
                )
                .await
                .expect("start delete job");
            let run_id = accepted.job_run.id.clone();
            common::wait_until("title deletion job completes", || {
                let admin = &admin;
                let run_id = run_id.clone();
                let ctx = &ctx;
                async move {
                    ctx.app
                        .list_recent_job_runs(admin, 20)
                        .await
                        .expect("list job runs")
                        .into_iter()
                        .any(|run| run.id == run_id && run.completed_at.is_some())
                }
            })
            .await;
            let summary = ctx
                .app
                .list_recent_job_runs(&admin, 20)
                .await
                .expect("list job runs")
                .into_iter()
                .find(|run| run.id == run_id)
                .and_then(|run| run.summary_json);
            let summary: serde_json::Value =
                serde_json::from_str(summary.as_deref().expect("summary")).expect("summary json");
            let failures = summary["failures"].as_array().cloned().unwrap_or_default();
            for id in &deleted {
                let reason = failures
                    .iter()
                    .find(|failure| failure["titleId"] == id.as_str())
                    .map(|failure| failure["reason"].as_str().unwrap_or("").to_string());
                results.push((id.clone(), reason));
            }
        } else {
            for id in &deleted {
                let reason = ctx
                    .app
                    .delete_title(&admin, id, false, None)
                    .await
                    .err()
                    .map(|error| error.reason().unwrap_or("").to_string());
                results.push((id.clone(), reason));
            }
        }
        (
            results,
            table_snapshot(&ctx).await,
            title_deleted_events(&ctx).await,
        )
    }

    let (bulk_results, bulk_rows, bulk_events) = scenario(true).await;
    let (single_results, single_rows, single_events) = scenario(false).await;
    assert!(
        bulk_results.iter().all(|(_, reason)| reason.is_none()),
        "{bulk_results:?}"
    );
    assert_eq!(bulk_results, single_results);
    assert_eq!(bulk_rows, single_rows);
    assert_eq!(bulk_events.len(), 70);
    assert_eq!(bulk_events, single_events);
    // Kept titles keep every row.
    assert!(
        bulk_rows
            .iter()
            .any(|row| row == "lifecycle_claims:cmp-series-079-claim:active:")
    );
    assert!(
        bulk_rows
            .iter()
            .any(|row| row == "lifecycle_claims:cmp-movie-000-claim:released:title_deleted")
    );
}

const LIBRARY_DELETE_TITLES: usize = 150;
const EMBERFALL_LIBRARY: &str = "emberfall-library";

/// Seeds a non-default movie library with `LIBRARY_DELETE_TITLES` titles,
/// their records, a pending import and a library-scoped setting, plus ten
/// titles in the default movie library that the delete keeps.
async fn seed_library_for_delete(ctx: &TestContext, root: &std::path::Path) {
    let now = chrono::Utc::now().to_rfc3339();
    let library_path = root.join("emberfall").to_string_lossy().to_string();
    let kept_path = root.join("ashgrove").to_string_lossy().to_string();
    // The default movie library, picked before the deleted library exists.
    let (kept_library, kept_root) = library_root(ctx, "movie", &kept_path).await;
    sqlx::query(
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) \
         VALUES (?, 'movie', 'Emberfall Movies', 'emberfall-movies', 0, ?, ?)",
    )
    .bind(EMBERFALL_LIBRARY)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db.pool)
    .await
    .expect("insert library");
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at) \
         VALUES ('emberfall-root', ?, ?, ?, 1, ?, ?)",
    )
    .bind(EMBERFALL_LIBRARY)
    .bind(&library_path)
    .bind(&library_path)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db.pool)
    .await
    .expect("insert library root");
    let mut ids = Vec::new();
    {
        let mut tx = ctx.db.pool.begin().await.expect("begin");
        for index in 0..LIBRARY_DELETE_TITLES + 10 {
            let (id, library, root_id) = if index < LIBRARY_DELETE_TITLES {
                (
                    format!("emberfall-{index:03}"),
                    EMBERFALL_LIBRARY,
                    "emberfall-root",
                )
            } else {
                (
                    format!("ashgrove-{index:03}"),
                    kept_library.as_str(),
                    kept_root.as_str(),
                )
            };
            let name = format!("Cinderwake {index:03}");
            insert_title(&mut tx, &id, &name, "movie", library, root_id, &now).await;
            insert_media_file(
                &mut tx,
                &format!("{id}-file"),
                &id,
                &format!("/synthetic/{id}.mkv"),
                &now,
            )
            .await;
            ids.push(id);
        }
        tx.commit().await.expect("commit");
    }
    for id in &ids {
        seed_title_records(ctx, id, &now).await;
    }
    sqlx::query(
        "INSERT INTO library_scan_unmatched_items (id, facet, scan_session_id, scan_root, item_path, display_name, query, reason_code, created_at, updated_at, library_id) \
         VALUES ('emberfall-pending', 'movie', 'synthetic-scan', ?, ?, 'Emberfall Pending', 'emberfall pending', 'synthetic', ?, ?, ?)",
    )
    .bind(&library_path)
    .bind(format!("{library_path}/Emberfall.Pending.mkv"))
    .bind(&now)
    .bind(&now)
    .bind(EMBERFALL_LIBRARY)
    .execute(&ctx.db.pool)
    .await
    .expect("insert pending import");
    sqlx::query(
        "INSERT INTO settings_values (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at) \
         SELECT 'emberfall-setting', id, scope, ?, '\"emberfall\"', 'user', NULL, ?, ? FROM settings_definitions ORDER BY id LIMIT 1",
    )
    .bind(EMBERFALL_LIBRARY)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db.pool)
    .await
    .expect("insert library setting");
    // Titles with a recorded download retire it title by title, one in each
    // of the job's chunks but the last.
    for (index, title_id) in ["emberfall-004", "emberfall-070"].iter().enumerate() {
        let download_id = format!("00000000-0000-4000-8000-0000000e0de{index}");
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at) VALUES (?, 'scryer_submission', ?)",
        )
        .bind(&download_id)
        .bind(&now)
        .execute(&ctx.db.pool)
        .await
        .expect("seed download");
        sqlx::query(
            "INSERT INTO download_submissions (id, title_id, facet, download_client_type, download_client_item_id, source_title, submitted_at) \
             VALUES (?, ?, 'movie', 'synthetic', ?, 'Synthetic.Release', ?)",
        )
        .bind(&download_id)
        .bind(title_id)
        .bind(format!("synthetic-item-{index}"))
        .bind(&now)
        .execute(&ctx.db.pool)
        .await
        .expect("seed download submission");
    }
    // Earlier events of deleted and kept titles.
    for title_id in ["emberfall-000", "emberfall-149", "ashgrove-150"] {
        sqlx::query(
            "INSERT INTO domain_events (event_id, occurred_at, title_id, facet, schema_version, \
               stream_kind, stream_id, event_type, payload_json) \
             VALUES (?, ?, ?, 'movie', 1, 'title', NULL, 'title_added', '{}')",
        )
        .bind(format!("{title_id}-added"))
        .bind(&now)
        .bind(title_id)
        .execute(&ctx.db.pool)
        .await
        .expect("seed domain event");
    }
}

/// The rows a library delete touches beyond a title delete's.
async fn library_snapshot(ctx: &TestContext) -> Vec<String> {
    const TABLES: &[(&str, &str)] = &[
        ("libraries", "id"),
        ("library_roots", "id"),
        ("settings_values", "id || ':' || coalesce(scope_id, '')"),
        (
            "library_scan_unmatched_items",
            "id || ':' || coalesce(library_id, '')",
        ),
    ];
    let mut rows = table_snapshot(ctx).await;
    for (table, key) in TABLES {
        let found: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT CAST({key} AS TEXT) FROM {table}"
        )))
        .fetch_all(&ctx.db.pool)
        .await
        .unwrap_or_else(|error| panic!("snapshot {table}: {error}"));
        rows.extend(found.into_iter().map(|(key,)| format!("{table}:{key}")));
    }
    rows.sort();
    rows
}

/// Every domain event but a job run's, in order: type, title and payload.
async fn non_job_events(ctx: &TestContext) -> Vec<(String, String, String)> {
    let rows: Vec<(String, Option<String>, Vec<u8>)> = sqlx::query_as(
        "SELECT event_type, title_id, CAST(payload_json AS BLOB) FROM domain_events \
         WHERE event_type NOT LIKE 'job_run_%' ORDER BY sequence",
    )
    .fetch_all(&ctx.db.pool)
    .await
    .expect("domain events");
    rows.into_iter()
        .map(|(event_type, title_id, payload)| {
            (
                event_type,
                title_id.unwrap_or_default(),
                String::from_utf8_lossy(&payload).to_string(),
            )
        })
        .collect()
}

async fn job_events(ctx: &TestContext) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM domain_events WHERE event_type LIKE 'job_run_%' ORDER BY sequence",
    )
    .fetch_all(&ctx.db.pool)
    .await
    .expect("job events");
    rows.into_iter().map(|(event_type,)| event_type).collect()
}

struct LibraryDeleteOutcome {
    rows: Vec<String>,
    events: Vec<(String, String, String)>,
    job_events: Vec<String>,
    commits: u64,
    summary: Option<serde_json::Value>,
    progress: Option<serde_json::Value>,
}

async fn library_delete_scenario(as_job: bool) -> LibraryDeleteOutcome {
    let root = tempfile::tempdir().expect("root");
    let db = tempfile::tempdir().expect("db dir");
    let db_path = db.path().join("library-delete.db");
    let ctx = TestContext::new_with_database_path(&db_path.to_string_lossy()).await;
    seed_library_for_delete(&ctx, root.path()).await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default admin");
    let wal_path = db.path().join("library-delete.db-wal");
    let (_, commits_before) = wal_frames(&wal_path);

    let (mut summary, mut progress) = (None, None);
    if as_job {
        let accepted = ctx
            .app
            .start_delete_library_job(&admin, EMBERFALL_LIBRARY)
            .await
            .expect("start library deletion job");
        let run_id = accepted.job_run.id.clone();
        common::wait_until("library deletion job completes", || {
            let admin = &admin;
            let run_id = run_id.clone();
            let ctx = &ctx;
            async move {
                ctx.app
                    .list_recent_job_runs(admin, 20)
                    .await
                    .expect("list job runs")
                    .into_iter()
                    .any(|run| run.id == run_id && run.completed_at.is_some())
            }
        })
        .await;
        let run = ctx
            .app
            .list_recent_job_runs(&admin, 20)
            .await
            .expect("list job runs")
            .into_iter()
            .find(|run| run.id == run_id)
            .expect("finished run");
        assert_eq!(run.status, scryer_application::JobRunStatus::Completed);
        summary = run
            .summary_json
            .as_deref()
            .map(|json| serde_json::from_str(json).expect("summary json"));
        progress = run
            .progress_json
            .as_deref()
            .map(|json| serde_json::from_str(json).expect("progress json"));
    } else {
        let deleted = ctx
            .app
            .delete_library(&admin, EMBERFALL_LIBRARY)
            .await
            .expect("delete library");
        assert!(deleted);
    }
    let (_, commits_after) = wal_frames(&wal_path);

    LibraryDeleteOutcome {
        rows: library_snapshot(&ctx).await,
        events: non_job_events(&ctx).await,
        job_events: job_events(&ctx).await,
        commits: commits_after - commits_before,
        summary,
        progress,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn library_delete_job_matches_the_request_delete() {
    // SAFETY: set before any connection opens, in this test's own process.
    unsafe { std::env::set_var("SCRYER_SQLITE_WAL_AUTOCHECKPOINT_PAGES", "0") };
    let request = library_delete_scenario(false).await;
    let job = library_delete_scenario(true).await;

    assert_eq!(job.rows, request.rows);
    assert_eq!(job.events, request.events);
    assert!(request.job_events.is_empty(), "{:?}", request.job_events);
    assert_eq!(job.job_events, ["job_run_started", "job_run_completed"]);
    // The library and everything scoped to it is gone; the kept library's
    // titles keep every row.
    assert!(
        !job.rows
            .iter()
            // A deleted title's lifecycle claim is kept, released.
            .any(|row| row.contains("emberfall-") && !row.ends_with(":released:title_deleted")),
        "{:?}",
        job.rows
            .iter()
            .filter(|row| row.contains("emberfall-"))
            .collect::<Vec<_>>()
    );
    assert!(job.rows.iter().any(|row| row == "titles:ashgrove-150"));
    assert_eq!(
        job.events
            .iter()
            .filter(|(event_type, _, _)| event_type == "title_deleted")
            .count(),
        LIBRARY_DELETE_TITLES
    );
    let summary = job.summary.expect("summary");
    assert_eq!(summary["deleted"], true);
    assert_eq!(summary["titleCount"], LIBRARY_DELETE_TITLES);
    let progress = job.progress.expect("progress");
    assert_eq!(progress["processed"], LIBRARY_DELETE_TITLES);
    assert_eq!(progress["total"], LIBRARY_DELETE_TITLES);

    // Only the kept title's earlier event survives beside the title-deleted
    // events.
    assert_eq!(
        job.events
            .iter()
            .filter(|(event_type, _, _)| event_type == "title_added")
            .map(|(_, title_id, _)| title_id.as_str())
            .collect::<Vec<_>>(),
        ["ashgrove-150"]
    );
    // Title-deleted events follow the library's title order.
    let deleted_order = job
        .events
        .iter()
        .filter(|(event_type, _, _)| event_type == "title_deleted")
        .map(|(_, title_id, _)| title_id.clone())
        .collect::<Vec<_>>();
    let mut sorted = deleted_order.clone();
    sorted.sort();
    assert_eq!(deleted_order, sorted);

    // The request deletes title by title; the job deletes in chunks of 64,
    // with one progress write after each chunk, plus its run, the started
    // and finished events, and the finished run.
    let progress_writes = LIBRARY_DELETE_TITLES.div_ceil(64) as u64;
    report(format!(
        "LIBRARY DELETE [writes]: request {} write transactions, job {} ({progress_writes} progress writes) for {LIBRARY_DELETE_TITLES} titles",
        request.commits, job.commits,
    ));
    assert_eq!(request.commits, 1359);
    assert_eq!(job.commits, LIBRARY_DELETE_JOB_COMMITS);
}

/// Write transactions of the library deletion job for
/// [`LIBRARY_DELETE_TITLES`] titles, two with a recorded download. Deleting
/// title by title, with a progress write every 64 titles of each of two
/// passes, the job made 1369: the request's 1359 and its own 10.
const LIBRARY_DELETE_JOB_COMMITS: u64 = 40;

#[tokio::test(flavor = "multi_thread")]
async fn library_delete_job_refuses_like_the_request_delete() {
    let root = tempfile::tempdir().expect("root");
    let ctx = TestContext::new().await;
    seed_library_for_delete(&ctx, root.path()).await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default admin");
    let default_movies =
        scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie);
    for library_id in [default_movies.as_str(), "ashgrove-missing-library"] {
        let in_request = ctx
            .app
            .delete_library(&admin, library_id)
            .await
            .expect_err("the request delete refuses");
        let as_job = ctx
            .app
            .start_delete_library_job(&admin, library_id)
            .await
            .expect_err("the job refuses");
        assert_eq!(as_job.reason(), in_request.reason(), "{library_id}");
        assert!(as_job.reason().is_some(), "{library_id}: {as_job:?}");
        assert_eq!(as_job.to_string(), in_request.to_string(), "{library_id}");
    }
    assert!(job_events(&ctx).await.is_empty());
    let rows = library_snapshot(&ctx).await;
    assert!(rows.iter().any(|row| row == "libraries:emberfall-library"));
}
