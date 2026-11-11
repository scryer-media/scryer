use sqlx::sqlite::SqlitePoolOptions;

const SQLITE: &str = include_str!("../../../scryer/src/db/migrations/0336_query_audit_indexes.sql");
const POSTGRES: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0336_query_audit_indexes.sql");

const ADDED: [&str; 21] = [
    "idx_discovery_items_discovery_title",
    "idx_discovery_items_tombstoned_by_run",
    "idx_discovery_section_items_item",
    "idx_domain_events_event_type_occurred_at",
    "idx_download_identity_states_reason_canonical",
    "idx_history_events_actor_user",
    "idx_imports_canonical_download_id",
    "idx_imports_source_identity",
    "idx_imports_type_status_updated_at",
    "idx_indexer_search_runs_session",
    "idx_maintenance_evaluation_runs_started",
    "idx_media_files_title_attention",
    "idx_media_requests_fingerprint_created",
    "idx_media_requests_requester_created",
    "idx_pending_releases_title_added",
    "idx_release_decisions_title_created_at",
    "idx_request_rule_decisions_evaluated",
    "idx_request_rule_decisions_outcome_evaluated",
    "idx_titles_library_catalog_order_asc",
    "idx_titles_library_catalog_order_desc",
    "idx_wanted_items_status_updated_at",
];

/// Indexes that already answer what a dropped one did; they must survive.
const KEPT: [&str; 4] = [
    "idx_download_identity_states_canonical_latest",
    "idx_history_events_title_time",
    "idx_history_events_type_time",
    "idx_media_files_title_path",
];

const DROPPED: [&str; 20] = [
    "idx_discovery_item_rank_components_item",
    "idx_discovery_items_run",
    "idx_discovery_title_source_tags_title",
    "idx_discovery_titles_key_language",
    "idx_domain_events_occurred_at",
    "idx_download_identity_states_canonical_download_id",
    "idx_external_subtitle_probe_cache_media_file",
    "idx_history_title_time",
    "idx_history_type_time",
    "idx_indexer_search_learning_title",
    "idx_indexer_search_run_sources_run",
    "idx_maintenance_action_steps_candidate",
    "idx_manual_import_selection_candidates_selection",
    "idx_media_files_title",
    "idx_monitor_selections_owner",
    "idx_subtitle_blocklist_media_file",
    "idx_title_image_variants_image_variant",
    "idx_title_images_title_kind",
    "idx_title_search_terms_title_id",
    "idx_wanted_items_title",
];

/// The catalog page for one library, in the title order the store builds.
fn catalog_page_plan(direction: &str) -> String {
    format!(
        "EXPLAIN QUERY PLAN SELECT titles.id FROM titles \
         WHERE library_id IN ('library-emberfall') AND facet = 'movie' \
         ORDER BY catalog_sort_key COLLATE BINARY {direction}, \
                  catalog_name_tie_key COLLATE BINARY {direction}, \
                  CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year {direction}, \
                  titles.id COLLATE BINARY {direction} \
         LIMIT 50 OFFSET 100"
    )
}

/// The import store's identity lookup with `terms` identities in one chunk.
fn identity_lookup_plan(terms: usize) -> String {
    let clauses = (0..terms)
        .map(|index| {
            format!(
                "(COALESCE(source_client_id, '') = 'client-1' AND source_system = 'nzbget' \
                 AND source_ref = 'job-{index}')"
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("EXPLAIN QUERY PLAN SELECT id FROM imports WHERE {clauses} ORDER BY updated_at DESC")
}

async fn query_plan(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_all(pool)
        .await
        .unwrap_or_else(|error| panic!("query plan should load for {sql}: {error}"));
    rows.into_iter().map(|row| row.3).collect()
}

async fn assert_plan_uses(pool: &sqlx::SqlitePool, state: &str, sql: &str, index: &str) {
    let plan = query_plan(pool, sql).await;
    assert!(
        plan.iter().any(|detail| detail.contains(index)),
        "{state}: expected {index} in the plan of {sql}: {plan:?}"
    );
}

async fn assert_plan_uses_without_sort(
    pool: &sqlx::SqlitePool,
    state: &str,
    sql: &str,
    index: &str,
) {
    let plan = query_plan(pool, sql).await;
    assert!(
        plan.iter().any(|detail| detail.contains(index)),
        "{state}: expected {index} in the plan of {sql}: {plan:?}"
    );
    assert!(
        !plan.iter().any(|detail| detail.contains("TEMP B-TREE")),
        "{state}: {index} should return rows already in order for {sql}: {plan:?}"
    );
}

async fn assert_sqlite_schema(pool: &sqlx::SqlitePool) {
    for name in ADDED.into_iter().chain(KEPT) {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "{name} should exist");
    }
    for name in DROPPED {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = ?")
            .bind(name)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{name} should be gone");
    }
}

/// Plans that must not depend on whether `ANALYZE` has run.
async fn assert_sqlite_plans(pool: &sqlx::SqlitePool, state: &str) {
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN DELETE FROM domain_events \
         WHERE occurred_at <= '2026-03-01T00:00:00Z' AND event_type IN ('type-1', 'type-2')",
        "idx_domain_events_event_type_occurred_at",
    )
    .await;
    for terms in [1, 200] {
        assert_plan_uses(
            pool,
            state,
            &identity_lookup_plan(terms),
            "idx_imports_source_identity",
        )
        .await;
    }
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM imports \
         WHERE (canonical_download_id = 'download-7') \
            OR (COALESCE(source_client_id, '') = 'client-1' AND source_system = 'nzbget' \
                AND source_ref = 'job-7')",
        "idx_imports_canonical_download_id",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM imports \
         WHERE import_type = 'manual_import' AND status = 'completed' \
           AND updated_at >= '2026-01-01T00:00:00Z' \
         ORDER BY updated_at DESC LIMIT 50",
        "idx_imports_type_status_updated_at",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT w.id FROM wanted_items w \
         LEFT JOIN titles t ON t.id = w.title_id \
         WHERE 1=1 AND w.status IN ('grabbed') \
         ORDER BY w.updated_at DESC LIMIT 50 OFFSET 0",
        "idx_wanted_items_status_updated_at",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM release_decisions \
         WHERE title_id = 'title-7' ORDER BY created_at DESC LIMIT 20 OFFSET 0",
        "idx_release_decisions_title_created_at",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM pending_releases \
         WHERE title_id = 'title-7' ORDER BY added_at DESC",
        "idx_pending_releases_title_added",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN DELETE FROM pending_releases WHERE title_id = 'title-7'",
        "idx_pending_releases_title_added",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT created_at FROM media_requests \
         WHERE created_by_user_id = 'user-3' AND id <> 'request-9' \
         ORDER BY created_at DESC, id DESC LIMIT 1",
        "idx_media_requests_requester_created",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM media_requests \
         WHERE identity_fingerprint = 'fingerprint-3' ORDER BY created_at DESC, id DESC",
        "idx_media_requests_fingerprint_created",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        &catalog_page_plan("ASC"),
        "idx_titles_library_catalog_order_asc",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        &catalog_page_plan("DESC"),
        "idx_titles_library_catalog_order_desc",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT titles.id FROM titles \
         WHERE library_id IN ('library-emberfall') \
           AND EXISTS (SELECT 1 FROM media_files catalog_attention \
                        WHERE catalog_attention.title_id = titles.id \
                          AND catalog_attention.role = 'primary' \
                          AND catalog_attention.scan_status IN ('scan_failed', 'review_required') \
                          AND (instr(catalog_attention.file_path, '/.recycle/') = 0 \
                               AND instr(catalog_attention.file_path, '/.staged/') = 0))",
        "idx_media_files_title_attention",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT detail FROM download_identity_states \
         WHERE reason = 'retry' AND canonical_download_id > 'download-7' \
         ORDER BY canonical_download_id LIMIT 100",
        "idx_download_identity_states_reason_canonical",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN UPDATE indexer_search_runs SET completion_state = 'complete' \
         WHERE search_session_id = 'session-7'",
        "idx_indexer_search_runs_session",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM request_rule_decisions \
         ORDER BY evaluated_at DESC, id DESC LIMIT 50",
        "idx_request_rule_decisions_evaluated",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM request_rule_decisions \
         WHERE effective_outcome = 'denied' ORDER BY evaluated_at DESC, id DESC LIMIT 50",
        "idx_request_rule_decisions_outcome_evaluated",
    )
    .await;
    assert_plan_uses_without_sort(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM maintenance_evaluation_runs \
         ORDER BY started_at DESC, id ASC LIMIT 50",
        "idx_maintenance_evaluation_runs_started",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN UPDATE history_events SET actor_user_id = NULL \
         WHERE actor_user_id = 'user-3'",
        "idx_history_events_actor_user",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT run_id FROM discovery_section_items WHERE item_id = 'item-7'",
        "idx_discovery_section_items_item",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN SELECT id FROM discovery_items WHERE discovery_title_id IN ('title-7')",
        "idx_discovery_items_discovery_title",
    )
    .await;
    assert_plan_uses(
        pool,
        state,
        "EXPLAIN QUERY PLAN UPDATE discovery_items SET tombstoned_by_run_id = NULL \
         WHERE tombstoned_by_run_id = 'run-7'",
        "idx_discovery_items_tombstoned_by_run",
    )
    .await;
}

/// Synthetic rows with the skew a real library has, so `ANALYZE` records
/// statistics the planner then has to weigh against the new indexes.
async fn seed_and_analyze(pool: &sqlx::SqlitePool) {
    sqlx::raw_sql(
        "PRAGMA foreign_keys = OFF;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
         INSERT INTO titles (id, name, facet, created_at, root_folder_id, library_id,
                             catalog_sort_key, catalog_name_tie_key, year)
         SELECT 'title-' || i,
                CASE i % 3 WHEN 0 THEN 'Emberfall ' WHEN 1 THEN 'Cinderwake ' ELSE 'Ashgrove ' END || i,
                'movie', '2026-01-01T00:00:00Z', 'root-1',
                CASE WHEN i % 10 < 8 THEN 'library-emberfall'
                     WHEN i % 10 = 8 THEN 'library-cinderwake' ELSE 'library-ashgrove' END,
                printf('emberfall %05d', i), printf('emberfall %05d', i),
                CASE WHEN i % 10 = 0 THEN NULL ELSE 1990 + i % 30 END
           FROM n;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 6000)
         INSERT INTO media_files (id, title_id, file_path, size_bytes, created_at, role, scan_status)
         SELECT 'file-' || i, 'title-' || ((i + 1) / 2), '/library/Emberfall/' || i || '.mkv', 1,
                '2026-01-01T00:00:00Z', CASE WHEN i % 2 = 0 THEN 'primary' ELSE 'additional' END,
                CASE WHEN i % 300 = 0 THEN 'scan_failed' ELSE 'scanned' END
           FROM n;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 4000)
         INSERT INTO domain_events (event_id, occurred_at, schema_version, stream_kind,
                                    event_type, payload_json)
         SELECT 'event-' || i, printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28), 1,
                'title', 'type-' || (i % 10), '{}'
           FROM n;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 4000)
         INSERT INTO imports (id, source_system, source_ref, import_type, payload_json,
                              created_at, updated_at, status, source_client_id,
                              canonical_download_id)
         SELECT 'import-' || i, 'nzbget', 'job-' || i,
                CASE WHEN i % 10 = 0 THEN 'manual_import' ELSE 'download' END, '{}',
                printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
                printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
                CASE WHEN i % 20 = 0 THEN 'pending' ELSE 'completed' END,
                CASE WHEN i % 2 = 0 THEN 'client-1' ELSE NULL END, 'download-' || i
           FROM n;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
         INSERT INTO wanted_items (id, title_id, media_type, created_at, updated_at, status)
         SELECT 'wanted-' || i, 'title-' || i, 'movie', '2026-01-01T00:00:00Z',
                printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
                CASE WHEN i % 50 = 0 THEN 'grabbed' ELSE 'wanted' END
           FROM n;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 4000)
         INSERT INTO release_decisions (id, wanted_item_id, title_id, release_title,
                                        decision_code, candidate_score, created_at)
         SELECT 'decision-' || i, 'wanted-' || (1 + i % 3000), 'title-' || (1 + i % 3000),
                'Cinderwake.' || i, 'accepted', 1,
                printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28)
           FROM n;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000)
         INSERT INTO pending_releases (id, wanted_item_id, title_id, release_title, release_score,
                                       added_at, delay_until, status, release_identity)
         SELECT 'pending-' || i, 'wanted-' || (1 + i % 500), 'title-' || (1 + i % 500),
                'Ashgrove.' || i, 1, printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
                '2026-12-31T00:00:00Z', 'waiting', 'identity-' || i
           FROM n;
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000)
         INSERT INTO media_requests (id, library_id, facet, status, identity_fingerprint, title,
                                     created_by_user_id, created_at, updated_at)
         SELECT 'request-' || i, 'library-emberfall', 'movie', 'approved',
                'fingerprint-' || (i % 300), 'Ashgrove ' || i, 'user-' || (i % 20),
                printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
                printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28)
           FROM n;
         PRAGMA foreign_keys = ON;
         ANALYZE;",
    )
    .execute(pool)
    .await
    .expect("synthetic rows and statistics should load");
    let analyzed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_stat1 WHERE idx = 'idx_imports_source_identity'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        analyzed, 1,
        "ANALYZE should record statistics for the new indexes"
    );
}

async fn memory_pool() -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap()
}

#[tokio::test]
async fn query_audit_indexes_apply_on_fresh_install_with_and_without_statistics() {
    assert_eq!(
        SQLITE.replace("COLLATE BINARY", "COLLATE \"C\"").replace(
            "imports(COALESCE(source_client_id, ''), source_system",
            "imports((COALESCE(source_client_id, '')), source_system"
        ),
        POSTGRES,
        "both engines create the same indexes, differing only in collation spelling \
         and expression parentheses"
    );
    let pool = memory_pool().await;
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("fresh migration catalog should apply");
    assert_sqlite_schema(&pool).await;
    assert_sqlite_plans(&pool, "without statistics").await;
    seed_and_analyze(&pool).await;
    assert_sqlite_plans(&pool, "after ANALYZE").await;
}

#[tokio::test]
async fn query_audit_indexes_upgrade_from_the_previous_catalog() {
    let pool = memory_pool().await;
    super::replay_source_catalog_for_fresh_install(&pool, Some(335), true)
        .await
        .expect("pre-upgrade catalog should apply");
    for name in ADDED {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = ?")
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{name} must be introduced after 335");
    }
    for name in DROPPED {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = ?")
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "{name} exists before the upgrade");
    }
    sqlx::raw_sql(
        "INSERT INTO history_events (id, event_type, actor_user_id, title_id, message,
                                     occurred_at, created_at)
         VALUES ('history-1', 'title_added', NULL, NULL, 'Emberfall added',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade must retain the published migration checksums");
    assert_sqlite_schema(&pool).await;
    assert_sqlite_plans(&pool, "upgraded").await;
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("later startup must be idempotent");
    assert_sqlite_schema(&pool).await;
    let messages: Vec<String> = sqlx::query_scalar("SELECT message FROM history_events")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        messages,
        ["Emberfall added"],
        "dropping indexes keeps every row"
    );
}

#[tokio::test]
async fn query_audit_indexes_postgres_preserve_rows_and_apply_idempotently() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    // Temporary tables plus a rolled-back transaction keep this fixture
    // isolated: the indexes the migration drops are the temporary ones below.
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "CREATE TEMP TABLE domain_events (
            sequence BIGINT PRIMARY KEY, event_type TEXT NOT NULL,
            occurred_at TIMESTAMPTZ NOT NULL);
         CREATE INDEX idx_domain_events_occurred_at ON domain_events (occurred_at DESC);
         CREATE TEMP TABLE imports (
            id TEXT PRIMARY KEY, source_client_id TEXT, source_system TEXT NOT NULL,
            source_ref TEXT NOT NULL, import_type TEXT NOT NULL, status TEXT NOT NULL,
            canonical_download_id TEXT, updated_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE wanted_items (
            id TEXT PRIMARY KEY, title_id TEXT NOT NULL, episode_id TEXT,
            status TEXT NOT NULL, updated_at TIMESTAMPTZ NOT NULL,
            UNIQUE (title_id, episode_id));
         CREATE INDEX idx_wanted_items_title ON wanted_items (title_id);
         CREATE TEMP TABLE download_identity_states (
            id TEXT PRIMARY KEY, canonical_download_id TEXT NOT NULL, reason TEXT,
            updated_at TIMESTAMPTZ NOT NULL);
         CREATE INDEX idx_download_identity_states_canonical_download_id
            ON download_identity_states (canonical_download_id);
         CREATE INDEX idx_download_identity_states_canonical_latest
            ON download_identity_states (canonical_download_id, updated_at DESC, id DESC);
         CREATE TEMP TABLE release_decisions (
            id TEXT PRIMARY KEY, title_id TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE pending_releases (
            id TEXT PRIMARY KEY, title_id TEXT NOT NULL, added_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE media_requests (
            id TEXT PRIMARY KEY, created_by_user_id TEXT NOT NULL,
            identity_fingerprint TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE history_events (
            id TEXT PRIMARY KEY, event_type TEXT NOT NULL, actor_user_id TEXT, title_id TEXT,
            message TEXT NOT NULL, occurred_at TIMESTAMPTZ NOT NULL);
         CREATE INDEX idx_history_title_time ON history_events (title_id, occurred_at DESC);
         CREATE INDEX idx_history_type_time ON history_events (event_type, occurred_at DESC);
         CREATE INDEX idx_history_events_title_time ON history_events (title_id, occurred_at DESC);
         CREATE INDEX idx_history_events_type_time ON history_events (event_type, occurred_at DESC);
         CREATE TEMP TABLE titles (
            id TEXT PRIMARY KEY, library_id TEXT, facet TEXT NOT NULL,
            catalog_sort_key TEXT NOT NULL DEFAULT '', catalog_name_tie_key TEXT NOT NULL DEFAULT '',
            year INTEGER);
         CREATE TEMP TABLE media_files (
            id TEXT PRIMARY KEY, title_id TEXT NOT NULL, file_path TEXT NOT NULL UNIQUE,
            role TEXT NOT NULL DEFAULT 'primary', scan_status TEXT NOT NULL DEFAULT 'pending');
         CREATE INDEX idx_media_files_title ON media_files (title_id);
         CREATE TEMP TABLE indexer_search_runs (
            id TEXT PRIMARY KEY, search_session_id TEXT, completion_state TEXT);
         CREATE TEMP TABLE request_rule_decisions (
            id TEXT PRIMARY KEY, effective_outcome TEXT NOT NULL,
            evaluated_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE maintenance_evaluation_runs (
            id TEXT PRIMARY KEY, started_at TIMESTAMPTZ NOT NULL);
         CREATE TEMP TABLE discovery_titles (
            id TEXT PRIMARY KEY, target_key_norm TEXT NOT NULL, language TEXT NOT NULL,
            UNIQUE (target_key_norm, language));
         CREATE INDEX idx_discovery_titles_key_language
            ON discovery_titles (target_key_norm, language);
         CREATE TEMP TABLE discovery_items (
            id TEXT PRIMARY KEY, run_id TEXT NOT NULL, section_id TEXT,
            sort_index INTEGER NOT NULL DEFAULT 0, discovery_title_id TEXT NOT NULL,
            tombstoned_by_run_id TEXT);
         CREATE INDEX idx_discovery_items_run ON discovery_items (run_id);
         CREATE INDEX idx_discovery_items_run_section
            ON discovery_items (run_id, section_id, sort_index);
         CREATE TEMP TABLE discovery_section_items (
            run_id TEXT NOT NULL, section_id TEXT NOT NULL, item_id TEXT NOT NULL,
            PRIMARY KEY (run_id, section_id, item_id));
         CREATE TEMP TABLE discovery_item_rank_components (
            item_id TEXT NOT NULL, component_index INTEGER NOT NULL,
            UNIQUE (item_id, component_index));
         CREATE INDEX idx_discovery_item_rank_components_item
            ON discovery_item_rank_components (item_id, component_index);
         CREATE TEMP TABLE discovery_title_source_tags (
            discovery_title_id TEXT NOT NULL, sort_index INTEGER NOT NULL,
            category TEXT NOT NULL, name TEXT NOT NULL,
            UNIQUE (discovery_title_id, sort_index, category, name));
         CREATE INDEX idx_discovery_title_source_tags_title
            ON discovery_title_source_tags (discovery_title_id, sort_index);
         CREATE TEMP TABLE title_search_terms (
            term_id BIGINT PRIMARY KEY, title_id TEXT NOT NULL, term_kind TEXT NOT NULL,
            literal_term TEXT NOT NULL, language_tag TEXT);
         CREATE UNIQUE INDEX idx_title_search_terms_title_kind_language
            ON title_search_terms (title_id, term_kind, literal_term, COALESCE(language_tag, ''));
         CREATE INDEX idx_title_search_terms_title_id ON title_search_terms (title_id);
         CREATE TEMP TABLE indexer_search_run_candidate_sources (
            run_id TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY (run_id, source_id));
         CREATE INDEX idx_indexer_search_run_sources_run
            ON indexer_search_run_candidate_sources (run_id);
         CREATE TEMP TABLE indexer_search_learning (
            indexer_id TEXT NOT NULL, title_id TEXT NOT NULL, facet TEXT NOT NULL,
            strategy_key TEXT NOT NULL, PRIMARY KEY (indexer_id, title_id, facet, strategy_key));
         CREATE INDEX idx_indexer_search_learning_title
            ON indexer_search_learning (indexer_id, title_id, facet);
         CREATE TEMP TABLE external_subtitle_probe_cache (
            media_file_id TEXT NOT NULL, file_path TEXT NOT NULL,
            PRIMARY KEY (media_file_id, file_path));
         CREATE INDEX idx_external_subtitle_probe_cache_media_file
            ON external_subtitle_probe_cache (media_file_id);
         CREATE TEMP TABLE subtitle_blocklist (
            id TEXT PRIMARY KEY, media_file_id TEXT NOT NULL, provider TEXT NOT NULL,
            provider_file_id TEXT NOT NULL, UNIQUE (media_file_id, provider, provider_file_id));
         CREATE INDEX idx_subtitle_blocklist_media_file ON subtitle_blocklist (media_file_id);
         CREATE TEMP TABLE manual_import_selection_candidates (
            id TEXT PRIMARY KEY, selection_id TEXT NOT NULL, canonical_path TEXT NOT NULL,
            UNIQUE (selection_id, canonical_path));
         CREATE INDEX idx_manual_import_selection_candidates_selection
            ON manual_import_selection_candidates (selection_id);
         CREATE TEMP TABLE maintenance_action_steps (
            candidate_id TEXT NOT NULL, match_generation INTEGER NOT NULL,
            revision_number INTEGER NOT NULL, step_id TEXT NOT NULL,
            PRIMARY KEY (candidate_id, match_generation, revision_number, step_id));
         CREATE INDEX idx_maintenance_action_steps_candidate
            ON maintenance_action_steps (candidate_id, match_generation, revision_number, step_id);
         CREATE TEMP TABLE monitor_selections (
            owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL, entry_kind TEXT NOT NULL,
            entry_key TEXT NOT NULL, PRIMARY KEY (owner_kind, owner_id, entry_kind, entry_key));
         CREATE INDEX idx_monitor_selections_owner ON monitor_selections (owner_kind, owner_id);
         CREATE TEMP TABLE title_images (
            id TEXT PRIMARY KEY, title_id TEXT NOT NULL, kind TEXT NOT NULL,
            UNIQUE (title_id, kind));
         CREATE INDEX idx_title_images_title_kind ON title_images (title_id, kind);
         CREATE TEMP TABLE title_image_variants (
            id TEXT PRIMARY KEY, title_image_id TEXT NOT NULL, variant_key TEXT NOT NULL,
            UNIQUE (title_image_id, variant_key));
         CREATE INDEX idx_title_image_variants_image_variant
            ON title_image_variants (title_image_id, variant_key);
         INSERT INTO history_events VALUES
            ('history-1', 'title_added', NULL, NULL, 'Emberfall added', '2026-01-01');
         INSERT INTO titles VALUES
            ('title-1', 'library-emberfall', 'movie', 'cinderwake', 'cinderwake', NULL),
            ('title-2', 'library-emberfall', 'movie', 'ashgrove', 'ashgrove', 2001);",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(POSTGRES).execute(&mut *tx).await.unwrap();
    }
    let present: Vec<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes
         WHERE schemaname = (SELECT nspname FROM pg_namespace WHERE oid = pg_my_temp_schema())
           AND indexname LIKE 'idx_%'
         ORDER BY indexname",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    for name in ADDED {
        assert!(
            present.iter().any(|index| index == name),
            "{name} missing: {present:?}"
        );
    }
    for name in DROPPED {
        assert!(
            !present.iter().any(|index| index == name),
            "{name} kept: {present:?}"
        );
    }
    let messages: Vec<String> = sqlx::query_scalar("SELECT message FROM history_events")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(messages, ["Emberfall added"]);
    let titles: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM titles WHERE library_id IN ('library-emberfall')
         ORDER BY catalog_sort_key COLLATE \"C\" ASC, catalog_name_tie_key COLLATE \"C\" ASC,
                  CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year ASC, titles.id COLLATE \"C\" ASC",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(titles, ["title-2", "title-1"]);

    // With sequential scans and sorts priced out, each statement shows the
    // index it can use: column order, collation, expression and predicate match.
    sqlx::raw_sql("SET LOCAL enable_seqscan = off; SET LOCAL enable_sort = off;")
        .execute(&mut *tx)
        .await
        .unwrap();
    for (sql, index) in [
        (
            "EXPLAIN DELETE FROM domain_events
             WHERE occurred_at <= '2026-03-01' AND event_type IN ('type-1', 'type-2')",
            "idx_domain_events_event_type_occurred_at",
        ),
        (
            "EXPLAIN SELECT id FROM imports
             WHERE (COALESCE(source_client_id, '') = 'client-1' AND source_system = 'nzbget'
                    AND source_ref = 'job-7')
             ORDER BY updated_at DESC",
            "idx_imports_source_identity",
        ),
        (
            "EXPLAIN SELECT id FROM titles WHERE library_id IN ('library-emberfall')
             ORDER BY catalog_sort_key COLLATE \"C\" ASC, catalog_name_tie_key COLLATE \"C\" ASC,
                      CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year ASC,
                      titles.id COLLATE \"C\" ASC LIMIT 50",
            "idx_titles_library_catalog_order_asc",
        ),
        (
            "EXPLAIN SELECT id FROM titles WHERE library_id IN ('library-emberfall')
             ORDER BY catalog_sort_key COLLATE \"C\" DESC, catalog_name_tie_key COLLATE \"C\" DESC,
                      CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year DESC,
                      titles.id COLLATE \"C\" DESC LIMIT 50",
            "idx_titles_library_catalog_order_desc",
        ),
        (
            "EXPLAIN SELECT 1 FROM media_files catalog_attention
             WHERE catalog_attention.title_id = 'title-1'
               AND catalog_attention.role = 'primary'
               AND catalog_attention.scan_status IN ('scan_failed', 'review_required')",
            "idx_media_files_title_attention",
        ),
        (
            "EXPLAIN SELECT id FROM pending_releases WHERE title_id = 'title-1'
             ORDER BY added_at DESC",
            "idx_pending_releases_title_added",
        ),
        (
            "EXPLAIN SELECT id FROM maintenance_evaluation_runs
             ORDER BY started_at DESC, id ASC LIMIT 50",
            "idx_maintenance_evaluation_runs_started",
        ),
        (
            "EXPLAIN SELECT id FROM request_rule_decisions WHERE effective_outcome = 'denied'
             ORDER BY evaluated_at DESC, id DESC LIMIT 50",
            "idx_request_rule_decisions_outcome_evaluated",
        ),
        (
            "EXPLAIN UPDATE history_events SET actor_user_id = NULL WHERE actor_user_id = 'user-3'",
            "idx_history_events_actor_user",
        ),
    ] {
        let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert!(
            plan.iter().any(|line| line.contains(index)),
            "expected {index} in the plan of {sql}: {plan:?}"
        );
    }
    tx.rollback().await.unwrap();
}
