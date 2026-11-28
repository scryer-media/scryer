#![recursion_limit = "256"]

//! SQLite page-cache size benchmark at library scale.
//!
//! Two ignored tests. `seed_sqlite_cache_bench_fixture` builds a file-backed
//! database at `BENCH_DB` through the real migrations: 25,000 movies and
//! 5,000 series (300,000 episodes), about 250,000 media files with their
//! episode maps, wanted rows for every missing scope, 500,000 release
//! decisions and 1,000,000 history events. `sqlite_cache_bench_run` opens a
//! copy of it, sets `PRAGMA cache_size` on every pooled connection to
//! `BENCH_CACHE_KIB`, and times a fixed workload: catalog page with episode
//! progress, title overview with files, wanted walk derivation, history page,
//! collection progress, title search and a 24-episode season import burst.
//! Each operation runs once cold (after `PRAGMA shrink_memory` on every
//! connection) and then warm, alone or as 16 concurrent readers depending on
//! `BENCH_MODE` (`single` or `concurrent`). Results are JSON lines appended to
//! `BENCH_OUT`.
//!
//! Run a fresh process per cache size and mode, on a fresh copy of the
//! fixture each time; the burst writes into the copy. Set
//! `SCRYER_SQLITE_MAX_CONNECTIONS=1` for `single`. Every name is synthetic.

// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

mod common;

use std::ffi::{c_int, c_void};
use std::time::{Duration, Instant};

use common::TestContext;
use scryer_application::{
    AcquisitionScopeState, AcquisitionScopeStateRepository, AcquisitionScopeStatus,
    DomainEventRepository, InsertMediaFileInput, MediaFileRepository, MediaFileRole,
    ReleaseDecision, ReleaseDecisionExplanation,
};
use scryer_domain::{
    DomainEventActorKind, DomainEventPayload, DomainEventStream, DomainExternalIds,
    ImportCompletedEventData, MediaFacet, NewDomainEvent, TitleContextSnapshot,
};
use scryer_infrastructure_library::media::libraries::state_store::WantedStore;
use scryer_infrastructure_workflow::workflow::stores::domain_event_store::DomainEventStore;
use serde_json::{Value, json};

const MOVIES: i64 = 25_000;
const MOVIE_MISSING_EVERY: i64 = 10;
const SERIES: i64 = 5_000;
const SEASONS: i64 = 5;
const EPISODES_PER_SEASON: i64 = 12;
const EPISODE_MISSING_EVERY: i64 = 4;
const HISTORY_EVENTS: usize = 1_000_000;
const RELEASE_DECISIONS: usize = 500_000;
const BURST_SEASONS: i64 = 20;
const BURST_EPISODES: i64 = 24;
const BURST_SERIES_ID: &str = "synthetic-burst-series";
const READERS: usize = 16;
const PAGE: i64 = 60;

// sqlite3.h, SQLite 3.54.1 as bundled by libsqlite3-sys 0.40.0.
const SQLITE_STATUS_MEMORY_USED: c_int = 0;
const SQLITE_STATUS_PAGECACHE_OVERFLOW: c_int = 2;
const SQLITE_DBSTATUS_CACHE_USED: c_int = 1;
const SQLITE_DBSTATUS_CACHE_HIT: c_int = 7;
const SQLITE_DBSTATUS_CACHE_MISS: c_int = 8;
const SQLITE_DBSTATUS_CACHE_WRITE: c_int = 9;
const SQLITE_DBSTATUS_CACHE_SPILL: c_int = 12;

unsafe extern "C" {
    fn sqlite3_db_status(
        db: *mut c_void,
        op: c_int,
        current: *mut c_int,
        highwater: *mut c_int,
        reset: c_int,
    ) -> c_int;
    fn sqlite3_status64(op: c_int, current: *mut i64, highwater: *mut i64, reset: c_int) -> c_int;
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

async fn run_sql(conn: &mut sqlx::SqliteConnection, sql: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&mut *conn)
        .await
        .unwrap_or_else(|error| panic!("{error}: {sql}"));
}

async fn library_and_root(conn: &mut sqlx::SqliteConnection, facet: &str) -> (String, String) {
    let row: (String,) = sqlx::query_as("SELECT id FROM libraries WHERE facet = ? LIMIT 1")
        .bind(facet)
        .fetch_one(&mut *conn)
        .await
        .expect("seeded library for facet");
    let root_id = format!("synthetic-root-{facet}");
    let path = format!("/synthetic/{facet}");
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&root_id)
    .bind(&row.0)
    .bind(&path)
    .bind(&path)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await
    .expect("insert synthetic root");
    (row.0, root_id)
}

fn counter(limit: i64) -> String {
    format!("WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < {limit} - 1)")
}

async fn seed_catalog(ctx: &TestContext) {
    let mut conn = ctx.db.pool.acquire().await.expect("seed connection");
    let (movie_library, movie_root) = library_and_root(&mut conn, "movie").await;
    let (series_library, series_root) = library_and_root(&mut conn, "series").await;
    let created = (chrono::Utc::now() - chrono::Duration::days(400)).to_rfc3339();
    run_sql(&mut conn, "BEGIN").await;
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO titles (id, name, name_normalized, facet, monitored, created_at, slug, year, \
             first_aired, digital_release_date, metadata_fetched_at, min_availability, library_id, root_folder_id) \
             {c} SELECT printf('synthetic-movie-%06d', i), \
             printf('Synthetic Fixture %05d (%d)', i, 1990 + i % 30), \
             lower(printf('Synthetic Fixture %05d (%d)', i, 1990 + i % 30)), 'movie', 1, '{created}', \
             printf('synthetic-fixture-%05d', i), 1990 + i % 30, printf('%d-06-01', 1990 + i % 30), \
             printf('%d-09-01', 1990 + i % 30), '{created}', 'released', '{movie_library}', '{movie_root}' FROM n",
            c = counter(MOVIES)
        ),
    )
    .await;
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, created_at, \
             video_width, video_height, role) \
             {c} SELECT printf('synthetic-movie-file-%06d', i), printf('synthetic-movie-%06d', i), \
             printf('/synthetic/movie/synthetic-movie-%06d/synthetic-movie-%06d.mkv', i, i), 8000000000, \
             'scanned', '{created}', CASE WHEN i % 5 = 1 THEN 1280 ELSE 1920 END, \
             CASE WHEN i % 5 = 1 THEN 720 ELSE 1080 END, 'primary' FROM n WHERE i % {MOVIE_MISSING_EVERY} <> 0",
            c = counter(MOVIES)
        ),
    )
    .await;
    let series_title = |id: &str, name: &str, year: &str| {
        format!(
            "INSERT INTO titles (id, name, name_normalized, facet, monitored, created_at, slug, year, \
             first_aired, metadata_fetched_at, library_id, root_folder_id) \
             {c} SELECT {id}, {name}, lower({name}), 'series', 1, '{created}', lower(replace({name}, ' ', '-')), \
             {year}, printf('%d-01-01', {year}), '{created}', '{series_library}', '{series_root}' FROM n",
            c = counter(SERIES)
        )
    };
    run_sql(
        &mut conn,
        &series_title(
            "printf('synthetic-series-%05d', i)",
            "printf('Synthetic Fixture Series %05d (%d)', i, 1995 + i % 25)",
            "1995 + i % 25",
        ),
    )
    .await;
    let episodes = SERIES * SEASONS * EPISODES_PER_SEASON;
    let per_series = SEASONS * EPISODES_PER_SEASON;
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO collections (id, title_id, collection_type, collection_index, created_at, monitored) \
             {c} SELECT printf('synthetic-series-%05d-season-%d', i / {SEASONS}, i % {SEASONS} + 1), \
             printf('synthetic-series-%05d', i / {SEASONS}), 'season', CAST(i % {SEASONS} + 1 AS TEXT), \
             '{created}', 1 FROM n",
            c = counter(SERIES * SEASONS)
        ),
    )
    .await;
    // i -> series i / per_series, season (i / EPISODES_PER_SEASON) % SEASONS + 1,
    // episode i % EPISODES_PER_SEASON + 1.
    let episode_id = format!(
        "printf('synthetic-series-%05d-s%02de%02d', i / {per_series}, (i / {EPISODES_PER_SEASON}) % {SEASONS} + 1, i % {EPISODES_PER_SEASON} + 1)"
    );
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number, \
             season_number, title, air_date, created_at, monitored) \
             {c} SELECT {episode_id}, printf('synthetic-series-%05d', i / {per_series}), \
             printf('synthetic-series-%05d-season-%d', i / {per_series}, (i / {EPISODES_PER_SEASON}) % {SEASONS} + 1), \
             'standard', CAST(i % {EPISODES_PER_SEASON} + 1 AS TEXT), CAST((i / {EPISODES_PER_SEASON}) % {SEASONS} + 1 AS TEXT), \
             printf('Synthetic Episode %02d', i % {EPISODES_PER_SEASON} + 1), \
             printf('%04d-%02d-%02d', 1995 + (i / {per_series}) % 25 + (i / {EPISODES_PER_SEASON}) % {SEASONS}, \
               1 + (i % {EPISODES_PER_SEASON}), 1 + (i % 28)), '{created}', 1 FROM n",
            c = counter(episodes)
        ),
    )
    .await;
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, created_at, \
             video_width, video_height, role) \
             {c} SELECT {episode_id} || '-file', printf('synthetic-series-%05d', i / {per_series}), \
             '/synthetic/series/' || {episode_id} || '.mkv', 1500000000, 'scanned', '{created}', \
             CASE WHEN i % 5 = 1 THEN 1280 ELSE 1920 END, CASE WHEN i % 5 = 1 THEN 720 ELSE 1080 END, \
             'primary' FROM n WHERE i % {EPISODE_MISSING_EVERY} <> 0",
            c = counter(episodes)
        ),
    )
    .await;
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO file_episode_map (file_id, episode_id, role) \
             {c} SELECT {episode_id} || '-file', {episode_id}, 'primary' FROM n \
             WHERE i % {EPISODE_MISSING_EVERY} <> 0",
            c = counter(episodes)
        ),
    )
    .await;
    // The burst series: seasons of 24 episodes with no files yet, imported
    // one season per burst.
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO titles (id, name, name_normalized, facet, monitored, created_at, slug, year, \
             first_aired, metadata_fetched_at, library_id, root_folder_id) VALUES \
             ('{BURST_SERIES_ID}', 'Synthetic Burst Series (2020)', 'synthetic burst series (2020)', 'series', 1, \
             '{created}', 'synthetic-burst-series', 2020, '2020-01-01', '{created}', '{series_library}', '{series_root}')"
        ),
    )
    .await;
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO collections (id, title_id, collection_type, collection_index, created_at, monitored) \
             {c} SELECT printf('{BURST_SERIES_ID}-season-%d', i + 1), '{BURST_SERIES_ID}', 'season', \
             CAST(i + 1 AS TEXT), '{created}', 1 FROM n",
            c = counter(BURST_SEASONS)
        ),
    )
    .await;
    run_sql(
        &mut conn,
        &format!(
            "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number, \
             season_number, title, air_date, created_at, monitored) \
             {c} SELECT printf('{BURST_SERIES_ID}-s%02de%02d', i / {BURST_EPISODES} + 1, i % {BURST_EPISODES} + 1), \
             '{BURST_SERIES_ID}', printf('{BURST_SERIES_ID}-season-%d', i / {BURST_EPISODES} + 1), 'standard', \
             CAST(i % {BURST_EPISODES} + 1 AS TEXT), CAST(i / {BURST_EPISODES} + 1 AS TEXT), \
             printf('Synthetic Burst Episode %02d', i % {BURST_EPISODES} + 1), \
             printf('%04d-%02d-%02d', 2000 + i / {BURST_EPISODES}, 1 + i % 12, 1 + i % 28), '{created}', 1 FROM n",
            c = counter(BURST_SEASONS * BURST_EPISODES)
        ),
    )
    .await;
    run_sql(&mut conn, "COMMIT").await;
}

fn scope_state(
    title_id: String,
    facet: &str,
    episode: Option<(String, String, String, String)>,
) -> AcquisitionScopeState {
    let now = chrono::Utc::now().to_rfc3339();
    let (episode_id, collection_id, season, number) = match episode {
        Some((e, c, s, n)) => (Some(e), Some(c), Some(s), Some(n)),
        None => (None, None, None, None),
    };
    AcquisitionScopeState {
        id: scryer_domain::Id::new().0,
        title_id,
        title_name: None,
        title_slug: None,
        title_facet: Some(facet.to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        media_type: if episode_id.is_some() {
            "episode".to_string()
        } else {
            "movie".to_string()
        },
        episode_id,
        collection_id,
        series_movie_link_id: None,
        season_number: season,
        episode_number: number,
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now,
    }
}

async fn seed_wanted(ctx: &TestContext) -> Vec<(String, String)> {
    let store = WantedStore::new(ctx.db.datastore());
    let movies: Vec<(String,)> = sqlx::query_as(
        "SELECT t.id FROM titles t WHERE t.facet = 'movie' \
         AND NOT EXISTS (SELECT 1 FROM media_files f WHERE f.title_id = t.id) ORDER BY t.id",
    )
    .fetch_all(&ctx.db.pool)
    .await
    .expect("missing movies");
    let episodes: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT e.id, e.title_id, e.collection_id, e.season_number, e.episode_number FROM episodes e \
         WHERE NOT EXISTS (SELECT 1 FROM file_episode_map m WHERE m.episode_id = e.id) ORDER BY e.id",
    )
    .fetch_all(&ctx.db.pool)
    .await
    .expect("missing episodes");
    let mut wanted = Vec::with_capacity(movies.len() + episodes.len());
    for (title_id,) in movies {
        let item = scope_state(title_id.clone(), "movie", None);
        store
            .upsert_acquisition_scope_state(&item)
            .await
            .expect("wanted movie");
        wanted.push((item.id, title_id));
    }
    for (episode_id, title_id, collection_id, season, number) in episodes {
        let item = scope_state(
            title_id.clone(),
            "series",
            Some((episode_id, collection_id, season, number)),
        );
        store
            .upsert_acquisition_scope_state(&item)
            .await
            .expect("wanted episode");
        wanted.push((item.id, title_id));
    }
    wanted
}

async fn seed_release_decisions(ctx: &TestContext, wanted: &[(String, String)]) {
    let store = WantedStore::new(ctx.db.datastore());
    let base = chrono::Utc::now();
    let mut batch = Vec::with_capacity(5_000);
    for index in 0..RELEASE_DECISIONS {
        let (wanted_id, title_id) = &wanted[index % wanted.len()];
        let explanation = (index % 10 == 0).then(|| {
            ReleaseDecisionExplanation::from_value(
                &json!({ "auto_decision": { "decided_by": "quality_tier" } }),
            )
        });
        batch.push(ReleaseDecision {
            id: scryer_domain::Id::new().0,
            wanted_item_id: wanted_id.clone(),
            title_id: title_id.clone(),
            release_title: format!("Synthetic.Fixture.Release.{index:07}.1080p.WEB-DL-SYN"),
            release_url: None,
            release_size_bytes: Some(4_000_000_000),
            decision_code: if index % 3 == 0 { "accepted" } else { "rejected" }.to_string(),
            candidate_score: 100 + (index % 50) as i32,
            current_score: None,
            score_delta: None,
            explanation_json: explanation,
            created_at: (base - chrono::Duration::seconds(index as i64 * 60)).to_rfc3339(),
        });
        if batch.len() == 5_000 {
            for result in store.insert_release_decisions(std::mem::take(&mut batch)).await {
                result.expect("release decision");
            }
        }
    }
    for result in store.insert_release_decisions(batch).await {
        result.expect("release decision");
    }
}

fn import_event(
    title_id: &str,
    facet: MediaFacet,
    occurred_at: chrono::DateTime<chrono::Utc>,
    episode_ids: Vec<String>,
    path: String,
) -> NewDomainEvent {
    NewDomainEvent {
        event_id: scryer_domain::Id::new().0,
        occurred_at,
        actor_kind: DomainEventActorKind::System,
        actor_user_id: None,
        actor_display_name: "System".to_string(),
        title_id: Some(title_id.to_string()),
        facet: Some(facet.clone()),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Title {
            title_id: title_id.to_string(),
        },
        payload: DomainEventPayload::ImportCompleted(ImportCompletedEventData {
            title: TitleContextSnapshot {
                title_name: format!("Synthetic Fixture {title_id}"),
                facet,
                external_ids: DomainExternalIds::default(),
                poster_url: None,
                year: Some(2001),
            },
            media_updates: Vec::new(),
            imported_count: 1,
            import_id: None,
            source_system: None,
            source_ref: None,
            source_title: Some("Synthetic.Fixture.Release.1080p.WEB-DL-SYN".to_string()),
            source_path: None,
            dest_path: Some(path),
            quality: Some("1080p".to_string()),
            episode_ids,
            size_bytes: Some(1_500_000_000),
            upgrade: false,
            file_placements: Vec::new(),
        }),
    }
}

async fn seed_history(ctx: &TestContext) {
    let store = DomainEventStore::new(ctx.db.datastore());
    let base = chrono::Utc::now() - chrono::Duration::days(1);
    let mut batch = Vec::with_capacity(5_000);
    for index in 0..HISTORY_EVENTS {
        let occurred_at = base - chrono::Duration::seconds(index as i64 * 30);
        let event = if index % 2 == 0 {
            let movie = (index as i64 / 2) % MOVIES;
            let id = format!("synthetic-movie-{movie:06}");
            let path = format!("/synthetic/movie/{id}/{id}.mkv");
            import_event(&id, MediaFacet::Movie, occurred_at, Vec::new(), path)
        } else {
            let series = (index as i64 / 2) % SERIES;
            let id = format!("synthetic-series-{series:05}");
            let episode = format!("{id}-s01e01");
            let path = format!("/synthetic/series/{episode}.mkv");
            import_event(&id, MediaFacet::Series, occurred_at, vec![episode], path)
        };
        batch.push(event);
        if batch.len() == 5_000 {
            store
                .append_many(std::mem::take(&mut batch))
                .await
                .expect("history batch");
        }
    }
    store.append_many(batch).await.expect("history batch");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "library-scale fixture build; run explicitly"]
async fn seed_sqlite_cache_bench_fixture() {
    let path = env("BENCH_DB");
    let ctx = TestContext::new_with_database_path(&path).await;
    let started = Instant::now();
    seed_catalog(&ctx).await;
    println!("catalog seeded in {:?}", started.elapsed());
    let wanted = seed_wanted(&ctx).await;
    println!("{} wanted rows in {:?}", wanted.len(), started.elapsed());
    seed_release_decisions(&ctx, &wanted).await;
    println!("release decisions in {:?}", started.elapsed());
    seed_history(&ctx).await;
    println!("history in {:?}", started.elapsed());
    scryer_infrastructure_datastore::queries::title_search::rebuild_title_search_projection(
        &ctx.db.pool,
    )
    .await
    .expect("title search projection");
    let mut conn = ctx.db.pool.acquire().await.expect("connection");
    run_sql(&mut conn, "ANALYZE").await;
    run_sql(&mut conn, "PRAGMA wal_checkpoint(TRUNCATE)").await;
    drop(conn);
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT 'titles', COUNT(*) FROM titles UNION ALL SELECT 'episodes', COUNT(*) FROM episodes \
         UNION ALL SELECT 'media_files', COUNT(*) FROM media_files \
         UNION ALL SELECT 'file_episode_map', COUNT(*) FROM file_episode_map \
         UNION ALL SELECT 'domain_events', COUNT(*) FROM domain_events \
         UNION ALL SELECT 'release_decisions', COUNT(*) FROM release_decisions",
    )
    .fetch_all(&ctx.db.pool)
    .await
    .expect("counts");
    println!("seeded in {:?}: {counts:?}", started.elapsed());
    ctx.db.pool.close().await;
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Op {
    CatalogPage,
    TitleOverview,
    WantedWalk,
    History,
    CollectionProgress,
    TitleSearch,
    ImportBurst,
}

impl Op {
    const ALL: [Op; 7] = [
        Op::CatalogPage,
        Op::TitleOverview,
        Op::WantedWalk,
        Op::History,
        Op::CollectionProgress,
        Op::TitleSearch,
        Op::ImportBurst,
    ];

    fn name(self) -> &'static str {
        match self {
            Op::CatalogPage => "catalog_page",
            Op::TitleOverview => "title_overview",
            Op::WantedWalk => "wanted_walk",
            Op::History => "history_page",
            Op::CollectionProgress => "collection_progress",
            Op::TitleSearch => "title_search",
            Op::ImportBurst => "import_burst_24",
        }
    }

    /// Background-shaped operations run once at a time; with concurrent
    /// readers they run beside fifteen readers instead of sixteen copies.
    fn is_singleton(self) -> bool {
        matches!(self, Op::WantedWalk | Op::ImportBurst)
    }
}

const CATALOG_QUERY: &str = "query($offset: Int) { titles(facet: SERIES, sort: {key: TITLE, direction: ASC}, \
    limit: 60, offset: $offset) { totalCount hasMore items { id name year posterUrl monitored sizeBytes \
    qualityTier episodesOwned episodesTotal episodesOnDisk episodesExpected } } }";
const OVERVIEW_QUERY: &str = "query($id: ID!) { title(id: $id) { id name facet monitored sizeBytes \
    collections { id collectionIndex label monitored episodesOwned episodesTotal episodesOnDisk \
    episodesExpected episodes { id episodeNumber seasonNumber title } } \
    mediaFiles { id episodeId filePath sizeBytes role qualityLabel scanStatus videoCodec videoWidth \
    videoHeight audioCodec } } }";
const HISTORY_QUERY: &str = "query($offset: Int) { titleHistory(filter: {limit: 50, offset: $offset}) { \
    items { id titleId titleName facet episodeId eventType } } }";
const SEARCH_QUERY: &str = "query($query: String) { titles(query: $query, limit: 20) { items { id name year facet } } }";

struct Bench {
    ctx: TestContext,
    burst_season: std::sync::atomic::AtomicI64,
}

impl Bench {
    async fn graphql(&self, query: &str, variables: Value) {
        let body = self
            .ctx
            .graphql_json_with_request_loaders(query, variables)
            .await;
        if let Some(errors) = body.get("errors") {
            panic!("graphql errors: {errors}");
        }
    }

    async fn run(&self, op: Op, reader: usize) {
        let reader = reader as i64;
        match op {
            Op::CatalogPage => {
                self.graphql(CATALOG_QUERY, json!({ "offset": reader * PAGE }))
                    .await
            }
            Op::TitleOverview => {
                let series = (reader * 311) % SERIES;
                self.graphql(
                    OVERVIEW_QUERY,
                    json!({ "id": format!("synthetic-series-{series:05}") }),
                )
                .await
            }
            Op::History => {
                self.graphql(HISTORY_QUERY, json!({ "offset": reader * 50 }))
                    .await
            }
            Op::TitleSearch => {
                let series = (reader * 313) % SERIES;
                self.graphql(
                    SEARCH_QUERY,
                    json!({ "query": format!("Synthetic Fixture Series {series:05}") }),
                )
                .await
            }
            Op::CollectionProgress => {
                let ids = (0..PAGE)
                    .map(|offset| {
                        format!("synthetic-series-{:05}", (reader * PAGE + offset) % SERIES)
                    })
                    .collect::<Vec<_>>();
                let rows = self
                    .ctx
                    .media_files
                    .list_collection_episode_progress_summaries(
                        &ids,
                        chrono::Utc::now().date_naive(),
                    )
                    .await
                    .expect("collection progress");
                assert!(!rows.is_empty());
            }
            Op::WantedWalk => {
                self.ctx
                    .app
                    .profile_acquisition_target_derivation()
                    .await
                    .expect("walk derivation");
            }
            Op::ImportBurst => self.import_burst().await,
        }
    }

    /// The writes an import of one 24-episode season makes: a media file
    /// per episode, its episode map and roles, the import events, and the
    /// wanted rows completed.
    async fn import_burst(&self) {
        let season = self
            .burst_season
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        assert!(season <= BURST_SEASONS, "burst seasons exhausted");
        let mut episode_ids = Vec::new();
        let mut events = Vec::new();
        for episode in 1..=BURST_EPISODES {
            let episode_id = format!("{BURST_SERIES_ID}-s{season:02}e{episode:02}");
            let path = format!("/synthetic/series/{episode_id}.mkv");
            let file_id = self
                .ctx
                .media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: BURST_SERIES_ID.to_string(),
                    file_path: path.clone(),
                    size_bytes: 1_500_000_000,
                    role: MediaFileRole::default(),
                    quality_label: Some("1080p".to_string()),
                    scene_name: Some(format!("Synthetic.Burst.S{season:02}E{episode:02}.1080p")),
                    ..InsertMediaFileInput::default()
                })
                .await
                .expect("insert media file");
            self.ctx
                .link_primary_file_to_episode(BURST_SERIES_ID, &file_id, &episode_id)
                .await
                .expect("link file");
            events.push(import_event(
                BURST_SERIES_ID,
                MediaFacet::Series,
                chrono::Utc::now(),
                vec![episode_id.clone()],
                path,
            ));
            episode_ids.push(episode_id);
        }
        DomainEventStore::new(self.ctx.db.datastore())
            .append_many(events)
            .await
            .expect("import events");
        WantedStore::new(self.ctx.db.datastore())
            .complete_acquisition_scopes_for_episodes(BURST_SERIES_ID, &episode_ids, None, true)
            .await
            .expect("complete wanted");
    }
}

/// Every connection the pool can hold, checked out at once.
async fn all_connections(
    pool: &sqlx::SqlitePool,
) -> Vec<sqlx::pool::PoolConnection<sqlx::Sqlite>> {
    let max = pool.options().get_max_connections() as usize;
    let mut held = Vec::with_capacity(max);
    for _ in 0..max {
        held.push(pool.acquire().await.expect("acquire connection"));
    }
    held
}

#[derive(Default, Clone, Copy)]
struct CacheCounters {
    used: i64,
    hit: i64,
    miss: i64,
    write: i64,
    spill: i64,
}

async fn db_status(pool: &sqlx::SqlitePool, reset: bool) -> (CacheCounters, Vec<i64>) {
    let mut held = all_connections(pool).await;
    let mut total = CacheCounters::default();
    let mut per_connection = Vec::new();
    for conn in &mut held {
        let mut handle = conn.lock_handle().await.expect("lock handle");
        let raw = handle.as_raw_handle().as_ptr().cast::<c_void>();
        let read = |op: c_int| {
            let (mut current, mut highwater) = (0, 0);
            // SAFETY: the handle is locked and open for the duration of the call.
            let rc = unsafe {
                sqlite3_db_status(raw, op, &mut current, &mut highwater, c_int::from(reset))
            };
            assert_eq!(rc, 0, "sqlite3_db_status({op})");
            i64::from(current)
        };
        let used = read(SQLITE_DBSTATUS_CACHE_USED);
        total.used += used;
        total.hit += read(SQLITE_DBSTATUS_CACHE_HIT);
        total.miss += read(SQLITE_DBSTATUS_CACHE_MISS);
        total.write += read(SQLITE_DBSTATUS_CACHE_WRITE);
        total.spill += read(SQLITE_DBSTATUS_CACHE_SPILL);
        per_connection.push(used);
    }
    (total, per_connection)
}

async fn on_every_connection(pool: &sqlx::SqlitePool, sql: &str) {
    let mut held = all_connections(pool).await;
    for conn in &mut held {
        run_sql(conn, sql).await;
    }
}

fn sqlite_memory(op: c_int, reset: bool) -> (i64, i64) {
    let (mut current, mut highwater) = (0i64, 0i64);
    // SAFETY: a process-wide status read with valid out-pointers.
    let rc = unsafe { sqlite3_status64(op, &mut current, &mut highwater, c_int::from(reset)) };
    assert_eq!(rc, 0);
    (current, highwater)
}

fn peak_rss_bytes() -> i64 {
    // SAFETY: getrusage fills the provided struct.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    // macOS reports bytes.
    usage.ru_maxrss
}

fn current_rss_bytes() -> i64 {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<i64>()
        .unwrap_or(0)
        * 1024
}

fn ms(values: &[Duration]) -> Vec<f64> {
    values
        .iter()
        .map(|value| (value.as_secs_f64() * 1_000_000.0).round() / 1000.0)
        .collect()
}

async fn timed(bench: &Bench, op: Op, reader: usize) -> Duration {
    let started = Instant::now();
    bench.run(op, reader).await;
    started.elapsed()
}

/// One round: the op alone, sixteen copies of it, or (for a singleton op)
/// the op once beside fifteen readers looping over the read mix.
async fn round(bench: &std::sync::Arc<Bench>, op: Op, concurrent: bool) -> Vec<Duration> {
    if !concurrent {
        return vec![timed(bench, op, 0).await];
    }
    if op.is_singleton() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut readers = tokio::task::JoinSet::new();
        for reader in 1..READERS {
            let bench = bench.clone();
            let stop = stop.clone();
            readers.spawn(async move {
                let mix = [Op::CatalogPage, Op::TitleOverview, Op::History, Op::TitleSearch];
                let mut turn = reader;
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    bench.run(mix[turn % mix.len()], reader).await;
                    turn += 1;
                }
            });
        }
        let elapsed = timed(bench, op, 0).await;
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        while let Some(joined) = readers.join_next().await {
            joined.expect("reader");
        }
        return vec![elapsed];
    }
    let mut set = tokio::task::JoinSet::new();
    for reader in 0..READERS {
        let bench = bench.clone();
        set.spawn(async move { timed(&bench, op, reader).await });
    }
    let mut out = Vec::with_capacity(READERS);
    while let Some(joined) = set.join_next().await {
        out.push(joined.expect("reader"));
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "library-scale measurement; run explicitly"]
async fn sqlite_cache_bench_run() {
    let path = env("BENCH_DB");
    let kib: i64 = env("BENCH_CACHE_KIB").parse().expect("BENCH_CACHE_KIB");
    let mode = env("BENCH_MODE");
    let concurrent = match mode.as_str() {
        "single" => false,
        "concurrent" => true,
        other => panic!("unknown BENCH_MODE {other}"),
    };
    let out_path = env("BENCH_OUT");
    let ctx = TestContext::new_with_database_path(&path).await;
    let pool = ctx.db.pool.clone();
    let connections = pool.options().get_max_connections();

    on_every_connection(&pool, &format!("PRAGMA cache_size = -{kib}")).await;
    {
        let mut held = all_connections(&pool).await;
        for conn in &mut held {
            let (size,): (i64,) = sqlx::query_as("PRAGMA cache_size")
                .fetch_one(&mut **conn)
                .await
                .expect("cache_size");
            assert_eq!(size, -kib, "every pooled connection carries the bench cache size");
        }
    }
    on_every_connection(&pool, "PRAGMA shrink_memory").await;
    let rss_after_open = current_rss_bytes();
    sqlite_memory(SQLITE_STATUS_MEMORY_USED, true);
    sqlite_memory(SQLITE_STATUS_PAGECACHE_OVERFLOW, true);

    let bench = std::sync::Arc::new(Bench {
        ctx,
        burst_season: std::sync::atomic::AtomicI64::new(0),
    });
    let mut lines = Vec::new();
    for op in Op::ALL {
        let warm_rounds = match (op, concurrent) {
            (Op::WantedWalk, _) => 5,
            (Op::ImportBurst, false) => 9,
            (Op::ImportBurst, true) => 6,
            (_, false) => 20,
            (_, true) => 5,
        };
        on_every_connection(&pool, "PRAGMA shrink_memory").await;
        db_status(&pool, true).await;
        let cold = round(&bench, op, concurrent).await;
        let (cold_counters, _) = db_status(&pool, true).await;
        let mut warm = Vec::new();
        for _ in 0..warm_rounds {
            warm.extend(round(&bench, op, concurrent).await);
        }
        let (warm_counters, per_connection_used) = db_status(&pool, true).await;
        let line = json!({
            "kib": kib,
            "mode": mode,
            "connections": connections,
            "op": op.name(),
            "cold_ms": ms(&cold),
            "warm_ms": ms(&warm),
            "cold_hit": cold_counters.hit,
            "cold_miss": cold_counters.miss,
            "cold_spill": cold_counters.spill,
            "warm_hit": warm_counters.hit,
            "warm_miss": warm_counters.miss,
            "warm_spill": warm_counters.spill,
            "warm_write": warm_counters.write,
            "cache_used_bytes": warm_counters.used,
            "cache_used_max_connection_bytes": per_connection_used.iter().max().copied().unwrap_or(0),
        });
        println!("{line}");
        lines.push(line.to_string());
    }
    let (memory_now, memory_peak) = sqlite_memory(SQLITE_STATUS_MEMORY_USED, false);
    let (pagecache_now, pagecache_peak) = sqlite_memory(SQLITE_STATUS_PAGECACHE_OVERFLOW, false);
    let summary = json!({
        "kib": kib,
        "mode": mode,
        "connections": connections,
        "op": "summary",
        "peak_rss_bytes": peak_rss_bytes(),
        "rss_after_open_bytes": rss_after_open,
        "rss_end_bytes": current_rss_bytes(),
        "sqlite_memory_used_bytes": memory_now,
        "sqlite_memory_peak_bytes": memory_peak,
        "sqlite_pagecache_bytes": pagecache_now,
        "sqlite_pagecache_peak_bytes": pagecache_peak,
    });
    println!("{summary}");
    lines.push(summary.to_string());
    let mut text = lines.join("\n");
    text.push('\n');
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out_path)
        .expect("open BENCH_OUT")
        .write_all(text.as_bytes())
        .expect("write BENCH_OUT");
}
