#![recursion_limit = "256"]

//! Discovery read-path measurement harness.
//!
//! Seeds an isolated, file-backed SQLite database with a synthetic catalog
//! (titles, external ids, canonical tags, series-movie links) and a Discovery
//! pool whose size is set independently of the catalog (a context run with
//! submitted subjects and personalized items, a public run with sections),
//! then drives the four Discovery endpoints through the real GraphQL router.
//! Every SQL statement the request issues is observed through sqlx's
//! `sqlx::query` tracing events, so the harness reports statement counts and
//! decoded rows without touching production code.
//!
//! `discovery_read_path_measurements` prints the table and the
//! `EXPLAIN QUERY PLAN` of every statement; it is ignored by default:
//!
//! ```text
//! cargo nextest run --locked -p scryer --test integration_discovery_perf \
//!     --run-ignored only --no-capture
//! ```
//!
//! `SCRYER_DISCOVERY_PERF_SIZES` overrides the catalog sizes
//! (comma-separated, default `1000,10000,100000`),
//! `SCRYER_DISCOVERY_PERF_POOL` the context pool size (default 3000),
//! `SCRYER_DISCOVERY_PERF_MIX` the movie/series/anime percentages (default
//! `60/30/10`), and `SCRYER_DISCOVERY_PERF_EPISODES=1` seeds seasons and 20
//! to 200 episodes per series and anime title. The episodic shape is
//! `SCRYER_DISCOVERY_PERF_SIZES=10000 SCRYER_DISCOVERY_PERF_MIX=50/50/0
//! SCRYER_DISCOVERY_PERF_EPISODES=1`: 5,000 movies beside 5,000 series.
//! `SCRYER_DISCOVERY_PERF_REPORT=<path>` also writes the table, the grouped
//! statements per endpoint and the plans to a file.
//!
//! Besides the Discovery endpoints it measures the movie and series catalog
//! pages and a series detail read with and without seasons and episodes
//! selected, so movie and series numbers are reported separately.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use common::TestContext;
use scryer_application::testing::AppUseCaseTestExt;
use scryer_application::{
    DiscoveryItemLibraryProvenanceRecord, DiscoveryItemRecord, DiscoveryRepository,
    DiscoverySectionRecord, DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord,
    DiscoverySyncStateRecord,
};
use scryer_domain::CanonicalMediaTag;
use scryer_infrastructure_metadata::discovery::store::DiscoveryStore;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context as LayerContext, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

const CONTEXT_RUN_ID: &str = "perf-context-run";
const PUBLIC_RUN_ID: &str = "perf-public-run";
const PUBLIC_SECTION_COUNT: usize = 6;
const PUBLIC_ITEMS_PER_SECTION: usize = 100;
const GENRES: [&str; 8] = [
    "Drama", "Comedy", "Thriller", "Fantasy", "Horror", "Crime", "Romance", "Mystery",
];

// ---------------------------------------------------------------------------
// SQL observation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ObservedStatement {
    pub sql: String,
    pub rows: u64,
    /// Server-side execution time sqlx measured for the statement.
    pub elapsed_secs: f64,
}

#[derive(Clone, Default)]
pub struct SqlProbe {
    statements: Arc<Mutex<Vec<ObservedStatement>>>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl SqlProbe {
    /// SQLite statements run on sqlx's connection worker threads, so a
    /// thread-local subscriber never sees them: the probe is the process-wide
    /// default, installed once for this test binary.
    pub fn global() -> Self {
        static PROBE: OnceLock<SqlProbe> = OnceLock::new();
        PROBE
            .get_or_init(|| {
                let probe = SqlProbe::default();
                tracing::subscriber::set_global_default(Registry::default().with(probe.clone()))
                    .expect("install the SQL probe");
                probe
            })
            .clone()
    }

    pub fn take(&self) -> Vec<ObservedStatement> {
        self.errors.lock().expect("probe lock").clear();
        std::mem::take(&mut *self.statements.lock().expect("probe lock"))
    }

    /// Error events logged since the last [`SqlProbe::take`], so a failed
    /// endpoint reports the server-side cause, not only its error id.
    pub fn errors(&self) -> Vec<String> {
        self.errors.lock().expect("probe lock").clone()
    }
}

#[derive(Default)]
struct StatementVisitor {
    summary: String,
    statement: String,
    rows: u64,
    elapsed_secs: f64,
}

impl Visit for StatementVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "rows_returned" {
            self.rows = value;
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "elapsed_secs" {
            self.elapsed_secs = value;
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "summary" => self.summary = value.to_string(),
            "db.statement" => self.statement = value.to_string(),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "summary" if self.summary.is_empty() => {
                self.summary = format!("{value:?}").trim_matches('"').to_string();
            }
            "db.statement" if self.statement.is_empty() => {
                self.statement = format!("{value:?}");
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct ErrorVisitor(String);

impl Visit for ErrorVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for SqlProbe {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
        if event.metadata().target() != "sqlx::query" {
            if *event.metadata().level() == tracing::Level::ERROR {
                let mut fields = ErrorVisitor::default();
                event.record(&mut fields);
                self.errors.lock().expect("probe lock").push(format!(
                    "{}: {}",
                    event.metadata().target(),
                    fields.0
                ));
            }
            return;
        }
        let mut visitor = StatementVisitor::default();
        event.record(&mut visitor);
        let sql = if visitor.statement.trim().is_empty() {
            visitor.summary
        } else {
            visitor.statement.trim().to_string()
        };
        self.statements
            .lock()
            .expect("probe lock")
            .push(ObservedStatement {
                sql,
                rows: visitor.rows,
                elapsed_secs: visitor.elapsed_secs,
            });
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct FixtureShape {
    pub catalog_titles: usize,
    pub context_pool: usize,
    /// Percent of the catalog that is movies, series and anime; sums to 100.
    pub mix: FacetMix,
    /// Seed seasons and 20 to 200 episodes for every series and anime title.
    pub episodes: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FacetMix {
    pub movie: usize,
    pub series: usize,
    pub anime: usize,
}

impl FacetMix {
    pub const DEFAULT: FacetMix = FacetMix {
        movie: 60,
        series: 30,
        anime: 10,
    };

    fn label(self) -> String {
        format!("{}/{}/{}", self.movie, self.series, self.anime)
    }
}

impl FixtureShape {
    /// Facet of catalog title `index`. Multiplying by 37 (coprime with 100)
    /// permutes each block of 100 indexes, so the facets interleave while
    /// every block holds the exact mix.
    fn facet(&self, index: usize) -> &'static str {
        let slot = (index * 37) % 100;
        if slot < self.mix.movie {
            "movie"
        } else if slot < self.mix.movie + self.mix.series {
            "series"
        } else {
            "anime"
        }
    }
}

/// Episodes seeded for an episodic catalog title: 20 to 200, in seasons of
/// ten.
fn episode_count(index: usize) -> usize {
    20 + (index * 53) % 181
}

pub struct SeededDiscovery {
    pub ctx: TestContext,
    pub _dir: tempfile::TempDir,
    pub probe: SqlProbe,
    pub ids: EndpointIds,
}

/// Ids the endpoint requests name.
#[derive(Clone, Debug)]
pub struct EndpointIds {
    pub movie_library_id: String,
    pub series_library_id: String,
    /// The series title with the most seeded episodes, or any series title
    /// when episodes are not seeded.
    pub series_title_id: Option<String>,
}

/// The external value a catalog title carries for its primary provider.
fn catalog_external_value(index: usize) -> usize {
    1_000_000 + index
}

fn genre_tags(index: usize) -> Vec<CanonicalMediaTag> {
    [
        GENRES[index % GENRES.len()],
        GENRES[(index / 3) % GENRES.len()],
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>()
    .into_iter()
    .map(|label| CanonicalMediaTag {
        affinity_signals: Vec::new(),
        key: format!("canonical:genre:{}", label.to_ascii_lowercase()),
        category: "genre".to_string(),
        name: label.to_string(),
        confidence: Some(1.0),
        sources: vec!["tmdb".to_string()],
        source_tag_keys: Vec::new(),
        is_adult: false,
        is_spoiler: false,
    })
    .collect()
}

async fn run_sql(pool: &sqlx::SqlitePool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(pool)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn insert_rows(pool: &sqlx::SqlitePool, prefix: &str, rows: Vec<String>) {
    for chunk in rows.chunks(400) {
        run_sql(pool, &format!("{prefix} VALUES {}", chunk.join(", "))).await;
    }
}

async fn library_ids_by_facet(pool: &sqlx::SqlitePool) -> BTreeMap<String, String> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT facet, id FROM libraries ORDER BY is_default DESC, id ASC")
            .fetch_all(pool)
            .await
            .expect("libraries");
    let mut by_facet = BTreeMap::new();
    for (facet, id) in rows {
        by_facet.entry(facet).or_insert(id);
    }
    for facet in ["movie", "series", "anime"] {
        if !by_facet.contains_key(facet) {
            let id = format!("perf-library-{facet}");
            run_sql(
                pool,
                &format!(
                    "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
                     VALUES ('{id}', '{facet}', 'Perf {facet}', 'perf-{facet}', 0,
                             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
                ),
            )
            .await;
            by_facet.insert(facet.to_string(), id);
        }
    }
    by_facet
}

/// Bulk-loads the synthetic catalog straight into the tables the Discovery
/// read path consults: titles, their external-id projection, canonical tags,
/// and series-movie links for anime titles.
async fn seed_catalog(
    pool: &sqlx::SqlitePool,
    libraries: &BTreeMap<String, String>,
    shape: FixtureShape,
) -> Option<String> {
    let size = shape.catalog_titles;
    let mut titles = Vec::with_capacity(size);
    let mut collections = Vec::new();
    let mut episodes = Vec::new();
    let mut series_title: Option<(usize, String)> = None;
    let mut external_ids = Vec::with_capacity(size * 2);
    let mut tags = Vec::with_capacity(size * 2);
    let mut movie_entities = Vec::new();
    let mut movie_links = Vec::new();
    for index in 0..size {
        let facet = shape.facet(index);
        let library_id = &libraries[facet];
        let id = format!("perf-title-{index}");
        if facet != "movie" {
            let count = if shape.episodes {
                episode_count(index)
            } else {
                0
            };
            if series_title.as_ref().is_none_or(|(best, _)| count > *best) {
                series_title = Some((count, id.clone()));
            }
            for episode in 0..count {
                let season = episode / 10 + 1;
                if episode % 10 == 0 {
                    collections.push(format!(
                        "('{id}-s{season}', '{id}', 'season', '{season}', 'Season {season}',
                          '2026-01-01T00:00:00Z')"
                    ));
                }
                let number = episode % 10 + 1;
                episodes.push(format!(
                    "('{id}-e{episode}', '{id}', '{id}-s{season}', 'standard', '{number}',
                      '{season}', 'Perf Episode {episode}', '2026-01-01T00:00:00Z')"
                ));
            }
        }
        let value = catalog_external_value(index);
        let imdb = format!("tt{:08}", value);
        titles.push(format!(
            "('{id}', 'Perf Title {index}', 'perf title {index}', '{facet}', 1, 'active', '[]', '[]',
              '2026-01-01T00:00:00Z', '{library_id}', '{imdb}', 'perf-root-{facet}')"
        ));
        let mut ids = vec![("imdb", "", imdb.clone())];
        match facet {
            "movie" => ids.push(("tmdb", "movie", value.to_string())),
            "series" => {
                ids.push(("tvdb", "series", value.to_string()));
                ids.push(("tmdb", "tv", (value + 5_000_000).to_string()));
            }
            _ => {
                ids.push(("anilist", "anime", value.to_string()));
                ids.push(("tvdb", "series", value.to_string()));
            }
        }
        for (source, kind, external) in ids {
            let key = if kind.is_empty() {
                format!("{source}:{external}")
            } else {
                format!("{source}:{kind}:{external}")
            };
            external_ids.push(format!(
                "('{id}-{source}-{kind}', '{id}', '{library_id}', '{facet}', '{source}', '{kind}',
                  '{external}', '{key}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
            ));
        }
        for (sort_index, tag) in genre_tags(index).into_iter().enumerate() {
            tags.push(format!(
                "('{id}', '{}', 'genre', '{}', 1.0, 0, 0, {sort_index})",
                tag.key, tag.name
            ));
        }
        if facet == "anime" && index % 20 == 9 {
            let movie_id = format!("perf-movie-entity-{index}");
            movie_entities.push(format!(
                "('{movie_id}', 'Perf Linked Feature {index}', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                value + 7_000_000
            ));
            movie_links.push(format!("('{id}-link', '{id}', '{movie_id}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"));
        }
    }
    insert_rows(
        pool,
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags,
                             external_ids, created_at, library_id, imdb_id, root_folder_id)",
        titles,
    )
    .await;
    insert_rows(
        pool,
        "INSERT INTO title_external_ids (id, title_id, library_id, facet, source, external_kind,
                                         external_id, external_key, created_at, updated_at)",
        external_ids,
    )
    .await;
    insert_rows(
        pool,
        "INSERT INTO title_metadata_tags (title_id, tag_key, category, name, confidence,
                                          is_adult, is_spoiler, sort_index)",
        tags,
    )
    .await;
    insert_rows(
        pool,
        "INSERT INTO movie_entities (id, title, tmdb_id, created_at, updated_at)",
        movie_entities,
    )
    .await;
    insert_rows(
        pool,
        "INSERT INTO series_movie_links (id, series_title_id, movie_entity_id, created_at, updated_at)",
        movie_links,
    )
    .await;
    insert_rows(
        pool,
        "INSERT INTO collections (id, title_id, collection_type, collection_index, label, created_at)",
        collections,
    )
    .await;
    insert_rows(
        pool,
        "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number,
                               season_number, title, created_at)",
        episodes,
    )
    .await;
    series_title.map(|(_, id)| id)
}

fn sync_run(id: &str, kind: &str) -> DiscoverySyncRunRecord {
    let now = Utc::now();
    DiscoverySyncRunRecord {
        id: id.to_string(),
        kind: kind.to_string(),
        status: "complete".to_string(),
        trigger_source: "scheduled_interval".to_string(),
        region: "US".to_string(),
        language: "eng".to_string(),
        subject_count: 1,
        subject_fingerprint: Some(format!("{id}-fingerprint")),
        previous_subject_fingerprint: None,
        base_generation_id: None,
        changed_subject_count: 0,
        affected_target_count: 0,
        smg_request_id: None,
        smg_status: Some("complete".to_string()),
        discovery_index_watermark: None,
        page_count: None,
        item_count: Some(0),
        facet_count: Some(0),
        acknowledged_at: None,
        error_text: None,
        started_at: Some(now),
        completed_at: Some(now),
        created_at: now,
        updated_at: now,
    }
}

#[allow(clippy::too_many_arguments)]
fn pool_item(
    run_id: &str,
    source_run_kind: &str,
    index: usize,
    target_key: String,
    target_kind: &str,
    section_id: Option<String>,
    provenance: Vec<DiscoveryItemLibraryProvenanceRecord>,
    matched_subject_keys: Vec<String>,
) -> DiscoveryItemRecord {
    let now = Utc::now();
    let display_title = format!("Perf Pool {run_id} {index}");
    DiscoveryItemRecord {
        id: format!("{run_id}:item:{index}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(run_id.to_string()),
        source_run_kind: source_run_kind.to_string(),
        section_id,
        sort_index: index as i32,
        target_key,
        target_kind: target_kind.to_string(),
        resolved: !index.is_multiple_of(7),
        resolved_title_id: None,
        display_title: display_title.clone(),
        original_title: None,
        sort_title: Some(display_title),
        year: Some(1980 + (index % 45) as i32),
        poster_path: None,
        poster_url: Some(format!(
            "https://example.invalid/poster-{run_id}-{index}.jpg"
        )),
        background_url: index
            .is_multiple_of(3)
            .then(|| format!("https://example.invalid/backdrop-{run_id}-{index}.jpg")),
        overview: Some(format!("Synthetic overview {index}")),
        content_type: Some(target_kind.to_string()),
        canonical_tags: genre_tags(index),
        affinity_signals: Vec::new(),
        is_adult: false,
        content_ratings: Vec::new(),
        rating: Some(5.0 + (index % 50) as f64 / 10.0),
        rating_sources: vec!["tmdb".to_string()],
        external_ratings: Vec::new(),
        external_ids: Vec::new(),
        status_tags: Vec::new(),
        source_tags: Vec::new(),
        sources: vec!["tmdb".to_string()],
        best_source: Some("tmdb".to_string()),
        relation_types: Vec::new(),
        relation_subtypes: Vec::new(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: Some(1 + (index % 4) as i32),
        edge_count: Some(1),
        relation_count: Some(1),
        source_subject_count: Some(matched_subject_keys.len() as i32),
        rank_score: Some(100.0 - (index % 1000) as f64 / 10.0),
        recommendation_score: (source_run_kind == "context_snapshot")
            .then(|| 1000.0 - index as f64 / 10.0),
        base_rank: Some(index as f64),
        matched_subject_count: matched_subject_keys.len() as i32,
        matched_subject_titles: Vec::new(),
        matched_subject_keys,
        library_provenance: provenance,
        tmdb_collection_id: None,
        tmdb_collection_name: None,
        owned_in_input: false,
        studio_slug: Some(format!("studio-{}", index % 12)),
        person_ids: Vec::new(),
        facet_terms: Vec::new(),
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        created_at: now,
        updated_at: now,
    }
}

/// Target key for pool item `index`. Every fifth item names a catalog title
/// when the catalog is big enough, so ownership exclusion has real work;
/// the rest name titles the catalog does not hold.
fn pool_target(index: usize, shape: FixtureShape) -> (String, &'static str) {
    let facet = shape.facet(index);
    if index.is_multiple_of(5) && index < shape.catalog_titles {
        let value = catalog_external_value(index);
        return match facet {
            "movie" => (format!("tmdb:movie:{value}"), "movie"),
            "series" => (format!("tvdb:series:{value}"), "series"),
            _ => (format!("anilist:anime:{value}"), "anime"),
        };
    }
    let value = 50_000_000 + index;
    match facet {
        "movie" => (format!("tmdb:movie:{value}"), "movie"),
        "series" => (format!("tvdb:series:{value}"), "series"),
        _ => (format!("anilist:anime:{value}"), "anime"),
    }
}

async fn seed_discovery(
    store: &DiscoveryStore,
    libraries: &BTreeMap<String, String>,
    shape: FixtureShape,
) {
    for (run, kind) in [
        (CONTEXT_RUN_ID, "context_snapshot"),
        (PUBLIC_RUN_ID, "public_feed"),
    ] {
        store
            .upsert_discovery_sync_run(&sync_run(run, kind))
            .await
            .expect("run should upsert");
    }

    let subjects = (0..shape.catalog_titles)
        .map(|index| {
            let facet = shape.facet(index);
            DiscoverySubmittedSubjectRecord {
                run_id: CONTEXT_RUN_ID.to_string(),
                subject_key: format!("subject:{index}"),
                title_id: Some(format!("perf-title-{index}")),
                library_id: Some(libraries[facet].clone()),
                library_facet: Some(facet.to_string()),
                title_kind: Some(facet.to_string()),
                display_title: Some(format!("Perf Title {index}")),
                external_ids_json: "[]".to_string(),
                raw_subject_json: "{}".to_string(),
            }
        })
        .collect::<Vec<_>>();
    store
        .replace_discovery_submitted_subjects(CONTEXT_RUN_ID, &subjects)
        .await
        .expect("subjects should store");

    let context_items = (0..shape.context_pool)
        .map(|index| {
            let (target_key, kind) = pool_target(index, shape);
            let subject = index % shape.catalog_titles.max(1);
            let facet = shape.facet(subject);
            pool_item(
                CONTEXT_RUN_ID,
                "context_snapshot",
                index,
                target_key,
                kind,
                None,
                vec![DiscoveryItemLibraryProvenanceRecord {
                    subject_key: format!("subject:{subject}"),
                    title_id: Some(format!("perf-title-{subject}")),
                    library_id: Some(libraries[facet].clone()),
                }],
                vec![format!("subject:{subject}")],
            )
        })
        .collect::<Vec<_>>();
    store
        .replace_discovery_items(CONTEXT_RUN_ID, &context_items)
        .await
        .expect("context items should store");

    let now = Utc::now();
    let sections = (0..PUBLIC_SECTION_COUNT)
        .map(|index| DiscoverySectionRecord {
            id: format!("{PUBLIC_RUN_ID}:section:{index}"),
            run_id: PUBLIC_RUN_ID.to_string(),
            section_id: format!("perf_public_{index}"),
            section_type: if index == 0 { "TRENDING" } else { "POPULAR" }.to_string(),
            surface: "public".to_string(),
            title: format!("Perf Public {index}"),
            sort_index: index as i32,
            created_at: now,
            updated_at: now,
        })
        .collect::<Vec<_>>();
    store
        .replace_discovery_sections(PUBLIC_RUN_ID, &sections)
        .await
        .expect("sections should store");
    let public_items = (0..PUBLIC_SECTION_COUNT * PUBLIC_ITEMS_PER_SECTION)
        .map(|index| {
            // Public items overlap the context pool every third item so
            // cross-shelf deduplication has duplicates to fold.
            let pool_index = if index % 3 == 0 {
                index
            } else {
                90_000 + index
            };
            let (target_key, kind) = pool_target(pool_index, shape);
            pool_item(
                PUBLIC_RUN_ID,
                "public_feed",
                index,
                target_key,
                kind,
                Some(format!("perf_public_{}", index / PUBLIC_ITEMS_PER_SECTION)),
                Vec::new(),
                Vec::new(),
            )
        })
        .collect::<Vec<_>>();
    store
        .replace_discovery_items(PUBLIC_RUN_ID, &public_items)
        .await
        .expect("public items should store");

    let state = DiscoverySyncStateRecord {
        last_success_generation_id: Some(CONTEXT_RUN_ID.to_string()),
        last_public_feed_generation_id: Some(PUBLIC_RUN_ID.to_string()),
        last_context_snapshot_completed_at: Some(now),
        last_public_feed_completed_at: Some(now),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    };
    store
        .upsert_discovery_sync_state(&state)
        .await
        .expect("state should store");
}

/// Builds a file-backed context, loads the fixture and wires the real
/// Discovery store in place of the null repository.
pub async fn seeded_discovery(shape: FixtureShape) -> SeededDiscovery {
    let dir = tempfile::Builder::new()
        .prefix("scryer-discovery-perf-")
        .tempdir()
        .expect("fixture directory");
    let database = dir.path().join("discovery-perf.db");
    let mut ctx = TestContext::new_with_database_path(&database.to_string_lossy()).await;
    let pool = ctx.db.pool.clone();
    let libraries = library_ids_by_facet(&pool).await;
    let series_title_id = seed_catalog(&pool, &libraries, shape).await;
    let store = DiscoveryStore::new(ctx.db.datastore());
    seed_discovery(&store, &libraries, shape).await;
    run_sql(&pool, "ANALYZE").await;
    let discovery = Arc::new(DiscoveryStore::new(ctx.db.datastore()));
    let app = ctx
        .app
        .with_test_overrides(|builder| builder.with_discovery_store(discovery));
    ctx.schema = scryer_interface::build_schema(app.clone(), ctx.auth_runtime.clone());
    ctx.app = app;
    SeededDiscovery {
        ctx,
        _dir: dir,
        probe: SqlProbe::global(),
        ids: EndpointIds {
            movie_library_id: libraries["movie"].clone(),
            series_library_id: libraries["series"].clone(),
            series_title_id,
        },
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

const ITEM_FIELDS: &str = "id targetKey targetKind resolved resolvedTitleId displayTitle
    originalTitle sortTitle year posterUrl backgroundUrl overview contentType isAdult
    canonicalTags { key category name confidence sources sourceTagKeys isAdult isSpoiler }
    rating externalIds { source kind id key } statusTags sourceTags sources bestSource
    relationTypes relationSubtypes sourceCount edgeCount relationCount sourceSubjectCount
    rankScore matchedSubjectTitles matchedSubjectCount tmdbCollectionId tmdbCollectionName
    ownedInInput facetTerms contextTerms studioSlug personIds";

const CARD_FIELDS: &str = "id targetKey targetKind displayTitle originalTitle sortTitle year posterUrl contentType isAdult ownedInInput";

const HERO_FIELDS: &str = "id targetKey targetKind displayTitle originalTitle sortTitle year
    posterUrl contentType isAdult ownedInInput backgroundUrl overview rating ratingSources
    externalRatings { source value score normalized votes url }
    genreTags { key category name confidence sources sourceTagKeys isAdult isSpoiler }
    matchedSubjectCount";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Endpoint {
    HomeFull,
    HomeCards,
    CatalogSidebar,
    CatalogSidebarSeries,
    SectionPage,
    FilterOptions,
    MovieCatalogPage,
    SeriesCatalogPage,
    SeriesDetailShallow,
    SeriesDetailEpisodes,
}

impl Endpoint {
    pub const ALL: [Endpoint; 10] = [
        Endpoint::HomeFull,
        Endpoint::HomeCards,
        Endpoint::CatalogSidebar,
        Endpoint::CatalogSidebarSeries,
        Endpoint::SectionPage,
        Endpoint::FilterOptions,
        Endpoint::MovieCatalogPage,
        Endpoint::SeriesCatalogPage,
        Endpoint::SeriesDetailShallow,
        Endpoint::SeriesDetailEpisodes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Endpoint::HomeFull => "home (full projection)",
            Endpoint::HomeCards => "home (cards)",
            Endpoint::CatalogSidebar => "catalog sidebar (movie)",
            Endpoint::CatalogSidebarSeries => "catalog sidebar (series)",
            Endpoint::MovieCatalogPage => "movie catalog page",
            Endpoint::SeriesCatalogPage => "series catalog page",
            Endpoint::SeriesDetailShallow => "series detail (id, name)",
            Endpoint::SeriesDetailEpisodes => "series detail (seasons, episodes)",
            Endpoint::SectionPage => "section page (browse_all)",
            Endpoint::FilterOptions => "filter options",
        }
    }

    pub fn request(self, ids: &EndpointIds) -> (String, Value) {
        let sidebar = format!(
            "query($input: CatalogDiscoveryInput!) {{ catalogDiscovery(input: $input) {{
                canViewPersonalized groups {{ id kind surface labelValue labelKey totalCount
                items {{ {ITEM_FIELDS} }} }} }} }}"
        );
        let catalog_page = "query($facet: MediaFacetValue, $sort: TitleCatalogSortInput) {
            titles(facet: $facet, sort: $sort, limit: 60) { totalCount hasMore
            items { id name year episodesOwned episodesTotal } } }";
        let series_title_id = ids.series_title_id.clone().unwrap_or_default();
        let section = |fields: &str| {
            format!("sectionId sectionType title labelKey surface totalCount items {{ {fields} }}")
        };
        match self {
            Endpoint::HomeFull => (
                format!(
                    "query($input: DiscoveryHomeInput) {{ discoveryHome(input: $input) {{
                        canViewPersonalized status {{ pendingContextChangeCount }}
                        heroItem {{ {ITEM_FIELDS} }}
                        publicSections {{ {s} }} personalizedSections {{ {s} }}
                        completeCollection {{ {s} }}
                        facets {{ name value smgCount localCount }}
                        browseLabels {{ family labelKey sectionId title itemCount }} }} }}",
                    s = section(ITEM_FIELDS)
                ),
                json!({"input": {}}),
            ),
            Endpoint::HomeCards => (
                format!(
                    "query($input: DiscoveryHomeInput) {{ discoveryHomeCards(input: $input) {{
                        canViewPersonalized status {{ pendingContextChangeCount }}
                        heroItem {{ {HERO_FIELDS} }}
                        publicSections {{ {s} }} personalizedSections {{ {s} }}
                        completeCollection {{ {s} }}
                        browseLabels {{ family labelKey sectionId title itemCount }} }} }}",
                    s = section(CARD_FIELDS)
                ),
                json!({"input": {}}),
            ),
            Endpoint::CatalogSidebar => (
                sidebar,
                json!({"input": {"facet": "MOVIE", "libraryIds": [ids.movie_library_id]}}),
            ),
            Endpoint::CatalogSidebarSeries => (
                sidebar,
                json!({"input": {"facet": "SERIES", "libraryIds": [ids.series_library_id]}}),
            ),
            Endpoint::MovieCatalogPage => (
                catalog_page.to_string(),
                json!({"facet": "MOVIE", "sort": {"key": "TITLE", "direction": "ASC"}}),
            ),
            Endpoint::SeriesCatalogPage => (
                catalog_page.to_string(),
                json!({"facet": "SERIES", "sort": {"key": "TITLE", "direction": "ASC"}}),
            ),
            Endpoint::SeriesDetailShallow => (
                "query($id: ID!) { title(id: $id) { id name facet } }".to_string(),
                json!({"id": series_title_id}),
            ),
            Endpoint::SeriesDetailEpisodes => (
                "query($id: ID!) { title(id: $id) { id name facet
                    collections { id collectionIndex label
                    episodes { id episodeNumber seasonNumber title } } } }"
                    .to_string(),
                json!({"id": series_title_id}),
            ),
            Endpoint::SectionPage => (
                format!(
                    "query($input: DiscoveryItemsInput) {{ discoveryItems(input: $input) {{
                        totalCount canViewPersonalized section {{ {s} generationIds }} }} }}",
                    s = section(CARD_FIELDS)
                ),
                json!({"input": {"sectionId": "browse_all", "limit": 48}}),
            ),
            Endpoint::FilterOptions => (
                "query($input: DiscoveryHomeFilterOptionsInput) {
                    discoveryHomeFilterOptions(input: $input) {
                        genres { key name } themes { key name } studioSlugs } }"
                    .to_string(),
                json!({"input": {}}),
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EndpointSample {
    pub elapsed: Duration,
    pub statements: Vec<ObservedStatement>,
    pub response_bytes: usize,
    pub body: Value,
    /// The server-side cause when the endpoint answered with GraphQL errors.
    /// The harness records a failing endpoint instead of stopping, because a
    /// size that breaks an endpoint is itself a measurement.
    pub error: Option<String>,
}

impl EndpointSample {
    pub fn query_count(&self) -> usize {
        self.statements.len()
    }

    /// Statements that read (`SELECT` or `WITH`), leaving out the
    /// request's authorization bookkeeping writes and first-sighting image
    /// proxy registrations.
    pub fn read_count(&self) -> usize {
        self.statements
            .iter()
            .filter(|statement| {
                let sql = statement.sql.trim_start().to_ascii_lowercase();
                sql.starts_with("select") || sql.starts_with("with")
            })
            .count()
    }

    pub fn rows(&self) -> u64 {
        self.statements.iter().map(|statement| statement.rows).sum()
    }
}

pub async fn call(seeded: &SeededDiscovery, endpoint: Endpoint) -> EndpointSample {
    let (query, variables) = endpoint.request(&seeded.ids);
    seeded.probe.take();
    let started = Instant::now();
    let body = seeded
        .ctx
        .graphql_json_with_request_loaders(&query, variables)
        .await;
    let elapsed = started.elapsed();
    let server_errors = seeded.probe.errors();
    let statements = seeded.probe.take();
    let error = body.get("errors").map(|errors| {
        if server_errors.is_empty() {
            errors.to_string()
        } else {
            server_errors.join("; ")
        }
    });
    EndpointSample {
        elapsed,
        statements,
        response_bytes: serde_json::to_vec(&body).expect("serialize").len(),
        body,
        error,
    }
}

#[derive(Clone, Debug)]
pub struct EndpointMeasurement {
    pub cold: EndpointSample,
    pub warm_median: Duration,
}

pub async fn measure(
    seeded: &SeededDiscovery,
    endpoint: Endpoint,
    warm_rounds: usize,
) -> EndpointMeasurement {
    let cold = call(seeded, endpoint).await;
    if cold.error.is_some() {
        return EndpointMeasurement {
            cold,
            warm_median: Duration::ZERO,
        };
    }
    let mut warm = Vec::with_capacity(warm_rounds);
    for _ in 0..warm_rounds {
        warm.push(call(seeded, endpoint).await.elapsed);
    }
    warm.sort();
    EndpointMeasurement {
        cold,
        warm_median: warm.get(warm.len() / 2).copied().unwrap_or_default(),
    }
}

async fn explain(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
    let placeholders = sql.matches('?').count();
    let explained = format!("EXPLAIN QUERY PLAN {sql}");
    let mut query = sqlx::query_as::<_, (i64, i64, i64, String)>(sqlx::AssertSqlSafe(&*explained));
    for _ in 0..placeholders {
        query = query.bind(Option::<String>::None);
    }
    match query.fetch_all(pool).await {
        Ok(rows) => rows.into_iter().map(|(_, _, _, detail)| detail).collect(),
        Err(error) => vec![format!("<explain failed: {error}>")],
    }
}

fn env_sizes() -> Vec<usize> {
    std::env::var("SCRYER_DISCOVERY_PERF_SIZES")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse().ok())
                .collect()
        })
        .filter(|sizes: &Vec<usize>| !sizes.is_empty())
        .unwrap_or_else(|| vec![1_000, 10_000, 100_000])
}

fn env_pool() -> usize {
    std::env::var("SCRYER_DISCOVERY_PERF_POOL")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(3_000)
}

/// A statement's first 160 characters on one line, the grouping key for
/// the per-endpoint statement listing.
fn statement_head(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

/// `SCRYER_DISCOVERY_PERF_MIX` as `movie/series/anime` percentages.
fn env_mix() -> FacetMix {
    std::env::var("SCRYER_DISCOVERY_PERF_MIX")
        .ok()
        .and_then(|value| {
            let parts = value
                .split('/')
                .map(|part| part.trim().parse::<usize>().ok())
                .collect::<Option<Vec<_>>>()?;
            match parts.as_slice() {
                [movie, series, anime] if movie + series + anime == 100 => Some(FacetMix {
                    movie: *movie,
                    series: *series,
                    anime: *anime,
                }),
                _ => None,
            }
        })
        .unwrap_or(FacetMix::DEFAULT)
}

fn env_episodes() -> bool {
    std::env::var("SCRYER_DISCOVERY_PERF_EPISODES").is_ok_and(|value| value.trim() == "1")
}

fn ms(duration: Duration) -> String {
    format!("{:.1}", duration.as_secs_f64() * 1000.0)
}

#[tokio::test]
#[ignore = "measurement harness, run on demand with --run-ignored only --no-capture"]
async fn discovery_read_path_measurements() {
    let pool_size = env_pool();
    let mut report = String::new();
    report.push_str(
        "| titles | mix m/s/a | episodes | pool | endpoint | cold ms | warm ms | queries | reads | rows | bytes |\n|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    let mut plans = BTreeMap::<String, Vec<String>>::new();
    let mut listing = String::new();
    let mut failures = String::new();
    for size in env_sizes() {
        let seed_started = Instant::now();
        let shape = FixtureShape {
            catalog_titles: size,
            context_pool: pool_size,
            mix: env_mix(),
            episodes: env_episodes(),
        };
        let seeded = seeded_discovery(shape).await;
        let episodes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM episodes")
            .fetch_one(&seeded.ctx.db.pool)
            .await
            .expect("episode count");
        let row_head = format!(
            "| {size} | {} | {episodes} | {pool_size}",
            shape.mix.label()
        );
        println!(
            "seeded {size} titles / pool {pool_size} in {} ms",
            ms(seed_started.elapsed())
        );
        for endpoint in Endpoint::ALL {
            let measurement = measure(&seeded, endpoint, 5).await;
            if let Some(error) = &measurement.cold.error {
                failures.push_str(&format!("{size} titles, {}: {error}\n", endpoint.label()));
                report.push_str(&format!(
                    "{row_head} | {} | error | error | {} | {} | {} | {} |\n",
                    endpoint.label(),
                    measurement.cold.query_count(),
                    measurement.cold.read_count(),
                    measurement.cold.rows(),
                    measurement.cold.response_bytes,
                ));
                continue;
            }
            report.push_str(&format!(
                "{row_head} | {} | {} | {} | {} | {} | {} | {} |\n",
                endpoint.label(),
                ms(measurement.cold.elapsed),
                ms(measurement.warm_median),
                measurement.cold.query_count(),
                measurement.cold.read_count(),
                measurement.cold.rows(),
                measurement.cold.response_bytes,
            ));
            for statement in &measurement.cold.statements {
                if !plans.contains_key(&statement.sql) {
                    let plan = explain(&seeded.ctx.db.pool, &statement.sql).await;
                    plans.insert(statement.sql.clone(), plan);
                }
            }
            listing.push_str(&format!("\n### {} at {size} titles\n", endpoint.label()));
            let mut grouped = BTreeMap::<String, (usize, u64, f64)>::new();
            for statement in &measurement.cold.statements {
                let entry = grouped.entry(statement_head(&statement.sql)).or_default();
                entry.0 += 1;
                entry.1 += statement.rows;
                entry.2 += statement.elapsed_secs;
            }
            for (head, (count, rows, secs)) in grouped {
                listing.push_str(&format!(
                    "{count:>5}x rows={rows:>7} ms={:>8.1} {head}\n",
                    secs * 1000.0
                ));
            }
        }
    }
    println!("\n{report}");
    if !failures.is_empty() {
        println!("--- FAILURES ---\n{failures}");
    }
    println!("--- EXPLAIN QUERY PLAN ---");
    for (sql, plan) in &plans {
        println!(
            "\n{}",
            sql.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(240)
                .collect::<String>()
        );
        for line in plan {
            println!("    {line}");
        }
    }
    if let Ok(path) = std::env::var("SCRYER_DISCOVERY_PERF_REPORT") {
        let mut full = report.clone();
        full.push_str("\n\nFAILURES\n");
        full.push_str(&failures);
        full.push_str("\n\nSTATEMENTS (cold call, grouped)\n");
        full.push_str(&listing);
        full.push_str("\n\nEXPLAIN QUERY PLAN\n");
        for (sql, plan) in &plans {
            full.push_str(&format!("\n{sql}\n"));
            for line in plan {
                full.push_str(&format!("    {line}\n"));
            }
        }
        std::fs::write(path, full).expect("write report");
    }
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// The Discovery endpoints whose cost must follow the pool, not the catalog.
const POOL_BOUNDED_ENDPOINTS: [Endpoint; 5] = [
    Endpoint::HomeFull,
    Endpoint::HomeCards,
    Endpoint::CatalogSidebar,
    Endpoint::CatalogSidebarSeries,
    Endpoint::SectionPage,
];

/// The library affinity read: the one read that summarizes the whole
/// readable catalog, and the one the affinity cache keeps off later calls.
fn is_affinity_read(sql: &str) -> bool {
    sql.trim_start()
        .starts_with("WITH scoped AS (SELECT id, facet FROM titles")
}

/// The full title projection the catalog list reads. Discovery must never
/// hydrate titles to decide ownership.
fn is_full_title_hydration(sql: &str) -> bool {
    let sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    sql.starts_with("SELECT id, library_id, name, facet, monitored, tags, external_ids")
}

fn bind_count(sql: &str) -> usize {
    sql.matches('?').count()
}

fn guard_shape(catalog_titles: usize) -> FixtureShape {
    FixtureShape {
        catalog_titles,
        context_pool: 300,
        mix: FacetMix::DEFAULT,
        episodes: false,
    }
}

#[tokio::test]
async fn discovery_reads_stay_pool_bounded_as_the_catalog_grows() {
    let mut samples = BTreeMap::<Endpoint, Vec<EndpointSample>>::new();
    for size in [1_000, 10_000] {
        let seeded = seeded_discovery(guard_shape(size)).await;
        for endpoint in POOL_BOUNDED_ENDPOINTS {
            let sample = call(&seeded, endpoint).await;
            assert!(
                sample.error.is_none(),
                "{} at {size} titles failed: {:?}",
                endpoint.label(),
                sample.error
            );
            for statement in &sample.statements {
                assert!(
                    !is_full_title_hydration(&statement.sql),
                    "{} at {size} titles hydrated full titles: {}",
                    endpoint.label(),
                    statement_head(&statement.sql)
                );
                // The pool (300 here) may bind pool-sized lists; a list that
                // follows the catalog would pass half of the 10k titles.
                assert!(
                    bind_count(&statement.sql) < 10_000 / 2,
                    "{} at {size} titles bound a catalog-sized list ({} binds): {}",
                    endpoint.label(),
                    bind_count(&statement.sql),
                    statement_head(&statement.sql)
                );
            }
            samples.entry(endpoint).or_default().push(sample);
        }
    }
    for (endpoint, sizes) in samples {
        let [small, large] = &sizes[..] else {
            panic!("two sizes per endpoint");
        };
        // Reads may not grow with the catalog: a per-title loop would.
        assert_eq!(
            small.read_count(),
            large.read_count(),
            "{} issued more reads at 10k titles than at 1k",
            endpoint.label()
        );
        let widest = |sample: &EndpointSample| {
            sample
                .statements
                .iter()
                .map(|statement| bind_count(&statement.sql))
                .max()
                .unwrap_or_default()
        };
        assert!(
            widest(large) <= widest(small),
            "{} bound more values at 10k titles ({}) than at 1k ({})",
            endpoint.label(),
            widest(large),
            widest(small)
        );
    }
}

async fn affinity_revision(pool: &sqlx::SqlitePool) -> String {
    sqlx::query_scalar("SELECT revision FROM discovery_affinity_revision WHERE id = 1")
        .fetch_one(pool)
        .await
        .expect("a migrated database keeps an affinity revision")
}

async fn affinity_reads(seeded: &SeededDiscovery) -> usize {
    let sample = call(seeded, Endpoint::HomeCards).await;
    assert!(sample.error.is_none(), "home failed: {:?}", sample.error);
    sample
        .statements
        .iter()
        .filter(|statement| is_affinity_read(&statement.sql))
        .count()
}

#[tokio::test]
async fn the_affinity_profile_is_reused_until_one_of_its_inputs_changes() {
    let seeded = seeded_discovery(guard_shape(1_000)).await;
    let pool = &seeded.ctx.db.pool;
    assert_eq!(
        affinity_reads(&seeded).await,
        1,
        "the first home call fills"
    );
    assert_eq!(
        affinity_reads(&seeded).await,
        0,
        "an unchanged catalog reuses it"
    );
    // The sidebar asks for the movie library alone: another scope, another fill.
    let sidebar = call(&seeded, Endpoint::CatalogSidebar).await;
    assert!(
        sidebar.error.is_none(),
        "sidebar failed: {:?}",
        sidebar.error
    );
    assert!(
        sidebar
            .statements
            .iter()
            .any(|statement| is_affinity_read(&statement.sql)),
        "a different library scope never reuses another scope's profile"
    );

    let changes = [
        (
            "canonical tag update",
            "UPDATE title_metadata_tags SET name = 'Retitled Genre'
             WHERE title_id = 'perf-title-0'",
        ),
        (
            "library reassignment",
            // perf-title-2 is a series title, so this moves a movie into
            // the series library.
            "UPDATE titles SET library_id = (SELECT library_id FROM titles
                 WHERE id = 'perf-title-2') WHERE id = 'perf-title-0'",
        ),
        (
            "ownership change",
            "DELETE FROM titles WHERE id = 'perf-title-4'",
        ),
        (
            "submitted subject change",
            "DELETE FROM discovery_submitted_subjects WHERE title_id = 'perf-title-3'",
        ),
    ];
    // Each change moves the revision the profile was built from. Rebuilds are
    // rate limited, so inside the rebuild window the kept profile is still
    // served without another whole-catalog read; the cache's own tests cover
    // the rebuild once the window has passed.
    for (label, sql) in changes {
        let before = affinity_revision(pool).await;
        run_sql(pool, sql).await;
        assert_ne!(
            affinity_revision(pool).await,
            before,
            "{label} must change the affinity revision"
        );
        assert_eq!(
            affinity_reads(&seeded).await,
            0,
            "{label}: the kept profile is served inside the rebuild window"
        );
    }

    // A new generation is a new key even when no catalog row changed.
    run_sql(
        pool,
        &format!(
            "INSERT INTO discovery_sync_runs (id, kind, status, trigger_source, region, language)
             SELECT 'perf-context-run-2', kind, status, trigger_source, region, language
             FROM discovery_sync_runs WHERE id = '{CONTEXT_RUN_ID}'"
        ),
    )
    .await;
    run_sql(
        pool,
        "UPDATE discovery_sync_state SET last_success_generation_id = 'perf-context-run-2'",
    )
    .await;
    assert_eq!(
        affinity_reads(&seeded).await,
        1,
        "a generation change must refill"
    );
}
