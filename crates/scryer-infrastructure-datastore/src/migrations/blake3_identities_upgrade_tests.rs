//! End-to-end upgrade coverage for migration 0192.
//!
//! The unit tests beside the hook exercise pure planning functions. These drive
//! the **real migration runner** over a database built at the pre-0192 state by
//! the real catalog, seeded with the kind of legacy rows an upgrading install
//! actually holds, and assert what survives.
//!
//! Each scenario builds its own in-memory database, so they are independent and
//! leave nothing behind.

use scryer_application::{HashDomain, blake3_identity_hex};
use sqlx::{Row, SqlitePool};

/// Version the catalog is replayed to before 0192 is applied.
const PRE_UPGRADE_VERSION: i64 = 191;

/// A plausible legacy SHA-256 digest. Content does not matter — only that it is
/// 64 hex characters and is not what BLAKE3 would produce.
const LEGACY_SHA256: &str = "9f2c4e1a7b3d5f80a1c2e3d4f5061728394a5b6c7d8e9f00112233445566778899";

async fn pre_upgrade_pool() -> SqlitePool {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix extension should register before the migration fixture");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite should open");
    crate::migrations::replay_source_catalog_for_fresh_install(
        &pool,
        Some(PRE_UPGRADE_VERSION),
        true,
    )
    .await
    .expect("pre-0192 migration fixture should apply");
    pool
}

async fn apply_upgrade(pool: &SqlitePool) {
    crate::migrations::run_migrations(pool, crate::MigrationMode::Apply)
        .await
        .expect("0192 upgrade should apply");
    let applied: i64 =
        sqlx::query_scalar("SELECT success FROM _sqlx_migrations WHERE version = 192")
            .fetch_one(pool)
            .await
            .expect("0192 ledger entry should exist");
    assert_eq!(applied, 1, "0192 must be recorded as successfully applied");
}

async fn seed_user(pool: &SqlitePool, id: &str, username: &str, password_hash: Option<&str>) {
    sqlx::query(
        "INSERT INTO users (id, username, status, password_hash, created_at, updated_at)
         VALUES (?1, ?2, 'active', ?3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .execute(pool)
    .await
    .expect("user should insert");
}

async fn user_row(pool: &SqlitePool, id: &str) -> (Option<String>, i64) {
    let row =
        sqlx::query("SELECT password_hash, password_change_required FROM users WHERE id = ?1")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("user row should load");
    (
        row.try_get::<Option<String>, _>("password_hash")
            .expect("password_hash column"),
        row.try_get::<i64, _>("password_change_required")
            .expect("password_change_required column"),
    )
}

async fn seed_coverage(pool: &SqlitePool, scope_key: &str) {
    sqlx::query(
        "INSERT INTO scope_indexer_coverage (scope_key, facet, indexer_id, fingerprint, searched_at)
         VALUES (?1, 'series', 'indexer-a', 'fp', '2026-01-01T00:00:00Z')",
    )
    .bind(scope_key)
    .execute(pool)
    .await
    .expect("coverage row should insert");
}

async fn surviving_scope_keys(pool: &SqlitePool) -> Vec<String> {
    let rows = sqlx::query("SELECT scope_key FROM scope_indexer_coverage ORDER BY scope_key")
        .fetch_all(pool)
        .await
        .expect("coverage rows should load");
    rows.iter()
        .map(|row| row.try_get::<String, _>("scope_key").expect("scope_key"))
        .collect()
}

async fn seed_unmatched(
    pool: &SqlitePool,
    id: &str,
    facet: &str,
    library_id: Option<&str>,
    item_path: &str,
) {
    sqlx::query(
        "INSERT INTO library_scan_unmatched_items
             (id, facet, library_id, scan_session_id, scan_root, item_path, display_name,
              query, reason_code, search_attempts_json, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'session-1', '/media', ?4, 'Display', 'query', 'no_match', '[]',
                 'pending', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(facet)
    .bind(library_id)
    .bind(item_path)
    .execute(pool)
    .await
    .expect("unmatched row should insert");
}

async fn unmatched_ids(pool: &SqlitePool) -> Vec<String> {
    let rows = sqlx::query("SELECT id FROM library_scan_unmatched_items ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("unmatched rows should load");
    rows.iter()
        .map(|row| row.try_get::<String, _>("id").expect("id"))
        .collect()
}

// ── WP0: v1 password retirement ─────────────────────────────────────────────

#[tokio::test]
async fn upgrade_clears_v1_passwords_and_leaves_every_other_account_alone() {
    let pool = pre_upgrade_pool().await;
    seed_user(
        &pool,
        "legacy-v1",
        "legacy_v1",
        Some(&format!(
            "v1$abcdef0123456789abcdef0123456789${LEGACY_SHA256}"
        )),
    )
    .await;
    let argon = "v2$$argon2id$v=19$m=19456,t=2,p=1$zyGbHzPhFQTT8+t6oz3ZNw$CtJ2dcsWSe1CCV4O30Gm9zPD/03F7MfEIMDvBvjc/ig";
    seed_user(&pool, "modern-v2", "modern_v2", Some(argon)).await;
    seed_user(&pool, "passwordless", "passwordless", None).await;

    apply_upgrade(&pool).await;

    // The v1 account fails closed with an explicit reset flag rather than
    // silently rejecting correct credentials.
    assert_eq!(user_row(&pool, "legacy-v1").await, (None, 1));
    // Argon2id and passwordless accounts are untouched, including their flag.
    assert_eq!(
        user_row(&pool, "modern-v2").await,
        (Some(argon.to_string()), 0)
    );
    assert_eq!(user_row(&pool, "passwordless").await, (None, 0));
}

/// Evidence for the open restore hazard, not an endorsement of it.
///
/// 0192 is recorded in the ledger, so a database restored from a pre-upgrade
/// backup after the upgrade brings `v1$` rows back and nothing clears them. The
/// account cannot authenticate, because v1 verification no longer exists.
#[tokio::test]
async fn a_v1_row_restored_after_the_upgrade_is_not_cleared() {
    let pool = pre_upgrade_pool().await;
    apply_upgrade(&pool).await;

    let restored = format!("v1$abcdef0123456789abcdef0123456789${LEGACY_SHA256}");
    seed_user(&pool, "restored-v1", "restored_v1", Some(&restored)).await;

    // Re-running the runner is a no-op: the ledger already has 192.
    crate::migrations::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("re-running migrations should succeed");

    assert_eq!(
        user_row(&pool, "restored-v1").await,
        (Some(restored), 0),
        "restore-after-upgrade leaves an unauthenticatable v1 row — tracked as an \
         open finding; change this assertion when a boot-time sweep lands"
    );
}

// ── WP6: convergence coverage sweep ─────────────────────────────────────────

#[tokio::test]
async fn upgrade_sweeps_only_legacy_set_scoped_coverage_keys() {
    let pool = pre_upgrade_pool().await;

    // Legacy set-scoped keys: the digest is embedded, so BLAKE3 orphans them.
    seed_coverage(&pool, &format!("episode_set:{LEGACY_SHA256}")).await;
    seed_coverage(&pool, &format!("series_pack_set:{LEGACY_SHA256}")).await;
    // Already-migrated keys carry the algorithm tag.
    seed_coverage(&pool, "episode_set:b3:aaaaaaaaaaaaaaaaaaaaaaaa").await;
    seed_coverage(&pool, "series_pack_set:b3:bbbbbbbbbbbbbbbbbbbbbbbb").await;
    // Every other scope-key shape is unchanged by the hash switch and must not
    // be touched — their fingerprints go stale and re-converge on their own.
    seed_coverage(&pool, "title:title-1").await;
    seed_coverage(&pool, "episode:episode-1").await;
    seed_coverage(&pool, "collection:collection-1").await;
    seed_coverage(&pool, "series_movie:link-1").await;
    seed_coverage(&pool, "series_pack_collection:collection-1").await;

    apply_upgrade(&pool).await;

    assert_eq!(
        surviving_scope_keys(&pool).await,
        vec![
            "collection:collection-1".to_string(),
            "episode:episode-1".to_string(),
            "episode_set:b3:aaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "series_movie:link-1".to_string(),
            "series_pack_collection:collection-1".to_string(),
            "series_pack_set:b3:bbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            "title:title-1".to_string(),
        ]
    );
}

// ── WP4: media-request identity re-key ──────────────────────────────────────

#[tokio::test]
async fn upgrade_rekeys_media_request_identity_fingerprints() {
    let pool = pre_upgrade_pool().await;
    seed_user(&pool, "requester", "requester", None).await;
    sqlx::query(
        "INSERT INTO libraries (id, facet, name, slug, created_at, updated_at)
         VALUES ('lib-1', 'movie', 'Fixture Movies', 'fixture-movies', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("library should insert");
    sqlx::query(
        "INSERT INTO media_requests
             (id, library_id, facet, status, identity_fingerprint, title, created_by_user_id,
              created_at, updated_at)
         VALUES ('req-1', 'lib-1', 'movie', 'pending', ?1, 'Some Film', 'requester',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(LEGACY_SHA256)
    .execute(&pool)
    .await
    .expect("media request should insert");
    for (source, external_id) in [("imdb", "tt0001"), ("tmdb", "42")] {
        sqlx::query(
            "INSERT INTO media_request_external_ids
                 (request_id, library_id, source, external_id, created_at)
             VALUES ('req-1', 'lib-1', ?1, ?2, '2026-01-01T00:00:00Z')",
        )
        .bind(source)
        .bind(external_id)
        .execute(&pool)
        .await
        .expect("external id should insert");
    }

    apply_upgrade(&pool).await;

    let fingerprint: String =
        sqlx::query_scalar("SELECT identity_fingerprint FROM media_requests WHERE id = 'req-1'")
            .fetch_one(&pool)
            .await
            .expect("fingerprint should load");
    // The loader reads `ORDER BY source, external_id`, so imdb precedes tmdb.
    assert_eq!(
        fingerprint,
        blake3_identity_hex(HashDomain::MediaRequestIdentity, "imdb:tt0001|tmdb:42"),
        "the backfill must reproduce exactly what the producer computes"
    );
    assert_ne!(fingerprint, LEGACY_SHA256);
}

// ── WP4: unmatched-item re-key, including the abort hazard ──────────────────

#[tokio::test]
async fn upgrade_rekeys_library_backed_unmatched_rows() {
    let pool = pre_upgrade_pool().await;
    seed_unmatched(
        &pool,
        "library_scan_unmatched:legacyaaaaaaaaaaaaaaaaaa",
        "movie",
        Some("lib-1"),
        "/media/a",
    )
    .await;

    apply_upgrade(&pool).await;

    let expected_digest =
        blake3_identity_hex(HashDomain::LibraryScanUnmatchedItem, "movie:lib-1:/media/a");
    assert_eq!(
        unmatched_ids(&pool).await,
        vec![format!("library_scan_unmatched:{}", &expected_digest[..24])]
    );
}

/// The defect this fixture exists for.
///
/// `library_id` is nullable (0104 added it without a backfill) and the unique
/// index is `(library_id, item_path)`, which SQLite treats as distinct per NULL.
/// Two such rows sharing an `item_path` both recompute to the same id, so an
/// unconditional re-key violates the primary key and aborts the whole upgrade.
#[tokio::test]
async fn duplicate_null_library_rows_do_not_abort_the_upgrade() {
    let pool = pre_upgrade_pool().await;
    seed_unmatched(
        &pool,
        "library_scan_unmatched:nulldupe0000000000000001",
        "movie",
        None,
        "/media/shared",
    )
    .await;
    seed_unmatched(
        &pool,
        "library_scan_unmatched:nulldupe0000000000000002",
        "movie",
        None,
        "/media/shared",
    )
    .await;

    // Would fail here before the guard: UNIQUE constraint failed.
    apply_upgrade(&pool).await;

    assert_eq!(
        unmatched_ids(&pool).await,
        vec![
            "library_scan_unmatched:nulldupe0000000000000001".to_string(),
            "library_scan_unmatched:nulldupe0000000000000002".to_string(),
        ],
        "pre-0104 rows keep their ids; the scan upsert re-keys them on next scan"
    );
}

// ── Whole-migration properties ──────────────────────────────────────────────

#[tokio::test]
async fn the_hook_is_idempotent_when_run_twice_over_the_same_database() {
    let pool = pre_upgrade_pool().await;
    seed_unmatched(
        &pool,
        "library_scan_unmatched:legacyaaaaaaaaaaaaaaaaaa",
        "movie",
        Some("lib-1"),
        "/media/a",
    )
    .await;
    apply_upgrade(&pool).await;
    let after_first = unmatched_ids(&pool).await;

    // Invoke the hook directly: the runner would skip it on a second pass, and
    // the property under test is the hook's own idempotence.
    let mut tx = pool.begin().await.expect("transaction should open");
    super::blake3_identities::backfill_blake3_identities_sqlite(&mut tx)
        .await
        .expect("second hook run should succeed");
    tx.commit().await.expect("commit should succeed");

    assert_eq!(unmatched_ids(&pool).await, after_first);
}

#[tokio::test]
async fn a_fresh_install_applies_the_whole_catalog_including_0192() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix extension should register");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite should open");

    crate::migrations::replay_source_catalog_for_fresh_install(&pool, None, true)
        .await
        .expect("fresh install should apply the whole catalog");

    // The seeded default admin has no password hash, so the v1 sweep is a no-op
    // and must not have flagged it for reset.
    let flagged: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE password_change_required = 1")
            .fetch_one(&pool)
            .await
            .expect("user count should load");
    assert_eq!(flagged, 0);
}

#[tokio::test]
async fn an_upgrade_with_nothing_legacy_to_migrate_still_applies_cleanly() {
    let pool = pre_upgrade_pool().await;
    apply_upgrade(&pool).await;
    assert!(surviving_scope_keys(&pool).await.is_empty());
    assert!(unmatched_ids(&pool).await.is_empty());
}

// ── PostgreSQL ──────────────────────────────────────────────────────────────
//
// Same scenarios against the other engine, because 0192 ships a separate
// PostgreSQL SQL file (`password_change_required` is BOOLEAN there, not the
// SQLite INTEGER) and the hook has a separate `$1`-placeholder implementation.
// Skips when `SCRYER_TEST_POSTGRES_URL` is unset, matching the convention in
// `postgres::services`. Each run works inside its own schema and drops it.

fn postgres_test_url() -> Option<String> {
    std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[tokio::test]
async fn postgres_upgrade_applies_and_migrates_legacy_identity_rows() {
    let Some(url) = postgres_test_url() else {
        eprintln!("skipping PostgreSQL 0192 upgrade test; SCRYER_TEST_POSTGRES_URL is not set");
        return;
    };
    let admin = sqlx::PgPool::connect(&url)
        .await
        .expect("postgres should connect");
    let schema = format!("blake3_upgrade_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("test schema should create");

    let scoped_url = if url.contains('?') {
        format!("{url}&options=-c%20search_path%3D{schema}")
    } else {
        format!("{url}?options=-c%20search_path%3D{schema}")
    };
    let pool = sqlx::PgPool::connect(&scoped_url)
        .await
        .expect("scoped postgres pool should connect");

    let outcome = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(PRE_UPGRADE_VERSION))
            .await
            .expect("pre-0192 postgres fixture should apply");

        let legacy_v1 = format!("v1$abcdef0123456789abcdef0123456789${LEGACY_SHA256}");
        sqlx::query(
            "INSERT INTO users (id, username, status, password_hash, created_at, updated_at)
             VALUES ($1, $2, 'active', $3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind("legacy-v1")
        .bind("legacy_v1")
        .bind(&legacy_v1)
        .execute(&pool)
        .await
        .expect("legacy user should insert");

        for scope_key in [
            format!("episode_set:{LEGACY_SHA256}"),
            "episode_set:b3:aaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "title:title-1".to_string(),
        ] {
            sqlx::query(
                "INSERT INTO scope_indexer_coverage
                     (scope_key, facet, indexer_id, fingerprint, searched_at)
                 VALUES ($1, 'series', 'indexer-a', 'fp', '2026-01-01T00:00:00Z')",
            )
            .bind(&scope_key)
            .execute(&pool)
            .await
            .expect("coverage row should insert");
        }

        // The NULL-library duplicate pair that aborts an unguarded re-key.
        for id in ["pg-null-dupe-1", "pg-null-dupe-2"] {
            sqlx::query(
                "INSERT INTO library_scan_unmatched_items
                     (id, facet, library_id, scan_session_id, scan_root, item_path, display_name,
                      query, reason_code, search_attempts_json, status, created_at, updated_at)
                 VALUES ($1, 'movie', NULL, 'session-1', '/media', '/media/shared', 'Display',
                         'query', 'no_match', '[]', 'pending',
                         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .expect("unmatched row should insert");
        }

        // Upgrade through the real service bootstrap, the same path production
        // takes on boot, rather than reaching into the migration runner.
        let services = crate::postgres::PostgresServices::new_with_mode(
            &scoped_url,
            crate::MigrationMode::Apply,
        )
        .await
        .expect("0192 postgres upgrade should apply");
        drop(services);

        // BOOLEAN column, not the SQLite INTEGER — this is what the separate
        // PostgreSQL SQL file exists for.
        let row = sqlx::query(
            "SELECT password_hash, password_change_required FROM users WHERE id = 'legacy-v1'",
        )
        .fetch_one(&pool)
        .await
        .expect("legacy user should load");
        assert_eq!(
            row.try_get::<Option<String>, _>("password_hash")
                .expect("password_hash"),
            None
        );
        assert!(
            row.try_get::<bool, _>("password_change_required")
                .expect("password_change_required"),
            "the retired v1 account must be flagged for reset"
        );

        let mut survivors: Vec<String> =
            sqlx::query("SELECT scope_key FROM scope_indexer_coverage ORDER BY scope_key")
                .fetch_all(&pool)
                .await
                .expect("coverage should load")
                .iter()
                .map(|row| row.try_get::<String, _>("scope_key").expect("scope_key"))
                .collect();
        survivors.sort();
        assert_eq!(
            survivors,
            vec![
                "episode_set:b3:aaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                "title:title-1".to_string(),
            ]
        );

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM library_scan_unmatched_items")
                .fetch_one(&pool)
                .await
                .expect("unmatched count should load");
        assert_eq!(remaining, 2, "the NULL-library pair survives untouched");
    }
    .await;

    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("test schema should drop");
    outcome
}
