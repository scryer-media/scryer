use anyhow::{Context, Result, anyhow, bail};
use sqlx::{Row, TypeInfo, ValueRef, postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use std::collections::HashSet;
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use xtask_support::{TaskContext, require_command, run_capture, run_status};

use crate::RebaselineArgs;

const CANONICAL_ADMIN_USER_ID: &str = "00000000000000000000000000000001";
const CANONICAL_TIMESTAMP: &str = "1970-01-01T00:00:00Z";
const POSTGRES_BUILTIN_BASELINE_SEED_MIN_VERSION: i64 = 140;
const POSTGRES_BUILTIN_BASELINE_SEED_SQL: &str = r#"INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) VALUES ('anime_default_library', 'anime', 'Anime', 'anime', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) VALUES ('movie_default_library', 'movie', 'Movies', 'movies', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) VALUES ('series_default_library', 'series', 'Series', 'series', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at) VALUES ('canonical_root_for_anime_default_library', 'anime_default_library', '/data/anime', '/data/anime', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at) VALUES ('canonical_root_for_movie_default_library', 'movie_default_library', '/data/movies', '/data/movies', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at) VALUES ('canonical_root_for_series_default_library', 'series_default_library', '/data/series', '/data/series', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO quality_profiles (id, name, scope, scope_id, archival_quality, allow_unknown_quality, atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades, created_at, prefer_dual_audio, required_audio_languages, scoring_config) VALUES ('1080p', '1080P', 'system', NULL, '1080P', false, true, true, true, true, false, true, '1970-01-01T00:00:00Z', false, '[]', '{}');
INSERT INTO quality_profiles (id, name, scope, scope_id, archival_quality, allow_unknown_quality, atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades, created_at, prefer_dual_audio, required_audio_languages, scoring_config) VALUES ('4k', '4K', 'system', NULL, '2160P', false, true, true, true, true, false, true, '1970-01-01T00:00:00Z', false, '[]', '{}');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('1080p', '1080P', 0, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('1080p', '720P', 1, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('4k', '1080P', 1, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('4k', '2160P', 0, '1970-01-01T00:00:00Z');
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES ('4k', '720P', 2, '1970-01-01T00:00:00Z');
INSERT INTO users (id, username, display_name, status, password_hash, passkey_public_key, locale, created_at, updated_at, last_login_at, account_kind, auth_session_version) VALUES ('00000000000000000000000000000001', 'admin', NULL, 'active', NULL, NULL, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', NULL, 'local', NULL);
"#;

pub(crate) fn run_rebaseline(ctx: &TaskContext, args: RebaselineArgs) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    runtime.block_on(async move { run_rebaseline_inner(ctx, args).await })
}

async fn run_rebaseline_inner(ctx: &TaskContext, args: RebaselineArgs) -> Result<()> {
    if args.through <= 0 {
        bail!("--through must be a positive migration version");
    }

    scryer_infrastructure_datastore::register_spellfix_auto_extension()
        .map_err(|error| anyhow!(error.to_string()))?;

    let db_root = ctx.path("crates/scryer/src/db");
    let sqlite_baseline_relative = baseline_relative(args.through, BaselineEngine::Sqlite);
    let sqlite_baseline_path = db_root.join(&sqlite_baseline_relative);
    let postgres_baseline_relative = baseline_relative(args.through, BaselineEngine::Postgres);
    let postgres_baseline_path = db_root.join(&postgres_baseline_relative);

    let mut manifest =
        scryer_infrastructure_datastore::migration_assets::load_source_manifest(&db_root)
            .map_err(|error| anyhow!(error))?;
    let sqlite_entry_present =
        manifest_has_baseline_entry(&manifest, args.through, BaselineEngine::Sqlite);
    let postgres_entry_present =
        manifest_has_baseline_entry(&manifest, args.through, BaselineEngine::Postgres);

    let should_write_sqlite = args.force || !sqlite_baseline_path.exists();
    let should_write_postgres = args.force || !postgres_baseline_path.exists();
    let sqlite_changed = should_write_sqlite || !sqlite_entry_present;
    let postgres_changed = should_write_postgres || !postgres_entry_present;
    if !sqlite_changed && !postgres_changed {
        bail!(
            "SQLite and PostgreSQL baselines through {:04} already exist and are already registered; pass --force to regenerate them",
            args.through
        );
    }

    let source_bundle =
        scryer_infrastructure_datastore::migrations::load_source_migration_catalog()
            .map_err(|error| anyhow!(error.to_string()))?;
    if source_bundle.catalog.find_migration(args.through).is_none() {
        bail!(
            "migration {:04} does not exist in the source catalog",
            args.through
        );
    }
    let mut generation_catalog = source_bundle.catalog.clone();
    if args.force {
        generation_catalog
            .baselines
            .retain(|baseline| baseline.through_version != args.through);
    }
    if postgres_changed
        && generation_catalog
            .latest_baseline_at_or_below(
                args.through,
                scryer_infrastructure_datastore::migration_assets::EngineScope::Postgres,
            )
            .is_none()
    {
        bail!(
            "cannot generate PostgreSQL baseline through {:04}: no PostgreSQL baseline exists at or below that version",
            args.through
        );
    }

    let mut generated_paths = Vec::new();
    if should_write_sqlite {
        let reference_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .context("failed to open reference in-memory sqlite database")?;
        scryer_infrastructure_datastore::migrations::replay_catalog_into_fresh_db(
            &reference_pool,
            &generation_catalog,
            &source_bundle.payload_bytes,
            Some(args.through),
            false,
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        let reference_dump = canonical_database_dump(&reference_pool).await?;
        write_baseline_file(&sqlite_baseline_path, &reference_dump)?;
        generated_paths.push(sqlite_baseline_path.clone());
    }

    let docker = if postgres_changed {
        Some(DockerPostgresContainer::start(ctx, args.through)?)
    } else {
        None
    };

    if should_write_postgres {
        let container = docker
            .as_ref()
            .expect("PostgreSQL container is present when PostgreSQL work is required");
        let target_db = format!("rebaseline_target_{}", unique_token(args.through));
        let target_pool = container.create_database_pool(&target_db).await?;
        scryer_infrastructure_datastore::postgres::replay_catalog_into_fresh_db(
            &target_pool,
            &generation_catalog,
            &source_bundle.payload_bytes,
            Some(args.through),
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        let postgres_dump = append_postgres_builtin_baseline_seeds(
            args.through,
            container.schema_dump(&target_db)?,
        );
        write_baseline_file(&postgres_baseline_path, &postgres_dump)?;
        generated_paths.push(postgres_baseline_path.clone());
    }

    let mut manifest_changed = false;
    manifest_changed |= upsert_baseline_entry(
        &mut manifest,
        args.through,
        &sqlite_baseline_relative,
        BaselineEngine::Sqlite,
    );
    manifest_changed |= upsert_baseline_entry(
        &mut manifest,
        args.through,
        &postgres_baseline_relative,
        BaselineEngine::Postgres,
    );
    if manifest_changed {
        scryer_infrastructure_datastore::migration_assets::write_source_manifest(
            &db_root, &manifest,
        )
        .map_err(|error| anyhow!(error))?;
    }

    let updated_bundle =
        scryer_infrastructure_datastore::migrations::load_source_migration_catalog()
            .map_err(|error| anyhow!(error.to_string()))?;

    let reference_head_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .context("failed to open reference full-replay in-memory sqlite database")?;
    scryer_infrastructure_datastore::migrations::replay_catalog_into_fresh_db(
        &reference_head_pool,
        &generation_catalog,
        &source_bundle.payload_bytes,
        None,
        false,
    )
    .await
    .map_err(|error| anyhow!(error.to_string()))?;
    let reference_head_dump = canonical_database_dump(&reference_head_pool).await?;

    let verification_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .context("failed to open verification in-memory sqlite database")?;
    scryer_infrastructure_datastore::migrations::replay_catalog_into_fresh_db(
        &verification_pool,
        &updated_bundle.catalog,
        &updated_bundle.payload_bytes,
        None,
        true,
    )
    .await
    .map_err(|error| anyhow!(error.to_string()))?;
    let verification_dump = canonical_database_dump(&verification_pool).await?;

    if reference_head_dump != verification_dump {
        let debug_dir = ctx.path("tmp/rebaseline-debug");
        std::fs::create_dir_all(&debug_dir)
            .with_context(|| format!("failed to create {}", debug_dir.display()))?;
        let reference_path = debug_dir.join(format!("{:04}_reference_head.sql", args.through));
        let verification_path =
            debug_dir.join(format!("{:04}_verification_head.sql", args.through));
        std::fs::write(&reference_path, reference_head_dump.as_bytes()).with_context(|| {
            format!(
                "failed to write debug reference dump {}",
                reference_path.display()
            )
        })?;
        std::fs::write(&verification_path, verification_dump.as_bytes()).with_context(|| {
            format!(
                "failed to write debug verification dump {}",
                verification_path.display()
            )
        })?;
        bail!(
            "baseline replay verification failed for version {:04}; wrote {} and {}",
            args.through,
            reference_path.display(),
            verification_path.display()
        );
    }

    if postgres_changed {
        let container = docker
            .as_ref()
            .expect("PostgreSQL container is present when PostgreSQL work is required");
        let reference_db = format!("rebaseline_reference_{}", unique_token(args.through));
        let reference_pool = container.create_database_pool(&reference_db).await?;
        scryer_infrastructure_datastore::postgres::replay_catalog_into_fresh_db(
            &reference_pool,
            &generation_catalog,
            &source_bundle.payload_bytes,
            None,
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        let reference_dump = container.schema_dump(&reference_db)?;

        let verification_db = format!("rebaseline_verification_{}", unique_token(args.through));
        let verification_pool = container.create_database_pool(&verification_db).await?;
        scryer_infrastructure_datastore::postgres::replay_catalog_into_fresh_db(
            &verification_pool,
            &updated_bundle.catalog,
            &updated_bundle.payload_bytes,
            None,
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        let verification_dump = container.schema_dump(&verification_db)?;

        if reference_dump != verification_dump {
            let debug_dir = ctx.path("tmp/rebaseline-debug");
            std::fs::create_dir_all(&debug_dir)
                .with_context(|| format!("failed to create {}", debug_dir.display()))?;
            let reference_path =
                debug_dir.join(format!("{:04}_postgres_reference_head.sql", args.through));
            let verification_path = debug_dir.join(format!(
                "{:04}_postgres_verification_head.sql",
                args.through
            ));
            std::fs::write(&reference_path, reference_dump.as_bytes()).with_context(|| {
                format!(
                    "failed to write debug PostgreSQL reference dump {}",
                    reference_path.display()
                )
            })?;
            std::fs::write(&verification_path, verification_dump.as_bytes()).with_context(
                || {
                    format!(
                        "failed to write debug PostgreSQL verification dump {}",
                        verification_path.display()
                    )
                },
            )?;
            bail!(
                "PostgreSQL baseline replay verification failed for version {:04}; wrote {} and {}",
                args.through,
                reference_path.display(),
                verification_path.display()
            );
        }
    }

    println!(
        "updated baseline sources through {:04}: {}",
        args.through,
        generated_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaselineEngine {
    Sqlite,
    Postgres,
}

impl BaselineEngine {
    fn scope(self) -> scryer_infrastructure_datastore::migration_assets::EngineScope {
        match self {
            Self::Sqlite => scryer_infrastructure_datastore::migration_assets::EngineScope::Sqlite,
            Self::Postgres => {
                scryer_infrastructure_datastore::migration_assets::EngineScope::Postgres
            }
        }
    }

    fn relative_path(self, through_version: i64) -> String {
        match self {
            Self::Sqlite => format!("baselines/{through_version:04}_baseline.sql"),
            Self::Postgres => {
                format!("postgres/baselines/{through_version:04}_baseline.sql")
            }
        }
    }
}

fn baseline_relative(through_version: i64, engine: BaselineEngine) -> String {
    engine.relative_path(through_version)
}

fn manifest_has_baseline_entry(
    manifest: &scryer_infrastructure_datastore::migration_assets::SourceMigrationManifest,
    through_version: i64,
    engine: BaselineEngine,
) -> bool {
    manifest
        .baselines
        .iter()
        .any(|entry| entry.through_version == through_version && entry.engine == engine.scope())
}

fn upsert_baseline_entry(
    manifest: &mut scryer_infrastructure_datastore::migration_assets::SourceMigrationManifest,
    through_version: i64,
    file: &str,
    engine: BaselineEngine,
) -> bool {
    let desired_engine = engine.scope();
    let desired_file = file.to_string();
    if let Some(entry) = manifest
        .baselines
        .iter_mut()
        .find(|entry| entry.through_version == through_version && entry.engine == desired_engine)
    {
        if entry.file == desired_file {
            return false;
        }
        entry.file = desired_file;
    } else {
        manifest.baselines.push(
            scryer_infrastructure_datastore::migration_assets::SourceBaselineEntry {
                through_version,
                file: desired_file,
                engine: desired_engine,
            },
        );
    }

    manifest.baselines.sort_by(|left, right| {
        baseline_sort_key(left.through_version, left.engine, &left.file).cmp(&baseline_sort_key(
            right.through_version,
            right.engine,
            &right.file,
        ))
    });
    true
}

fn baseline_sort_key(
    through_version: i64,
    engine: scryer_infrastructure_datastore::migration_assets::EngineScope,
    file: &str,
) -> (i64, u8, &str) {
    (through_version, engine_sort_key(engine), file)
}

fn engine_sort_key(engine: scryer_infrastructure_datastore::migration_assets::EngineScope) -> u8 {
    match engine {
        scryer_infrastructure_datastore::migration_assets::EngineScope::All => 0,
        scryer_infrastructure_datastore::migration_assets::EngineScope::Sqlite => 1,
        scryer_infrastructure_datastore::migration_assets::EngineScope::Postgres => 2,
    }
}

fn write_baseline_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, contents.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

struct DockerPostgresContainer {
    name: String,
    port: u16,
}

impl DockerPostgresContainer {
    fn start(ctx: &TaskContext, through_version: i64) -> Result<Self> {
        require_command("docker")?;

        let port = reserve_local_port()?;
        let name = format!("scryer-rebaseline-pg-{}", unique_token(through_version));
        let mut command = ctx.command("docker");
        command.args([
            "run",
            "-d",
            "--rm",
            "--name",
            &name,
            "-e",
            "POSTGRES_PASSWORD=postgres",
            "-p",
            &format!("127.0.0.1:{port}:5432"),
            "postgres:18-alpine",
        ]);
        run_capture(&mut command).with_context(|| {
            format!(
                "failed to start Docker PostgreSQL container for {:04}",
                through_version
            )
        })?;

        let container = Self { name, port };
        container.wait_until_ready(ctx)?;
        Ok(container)
    }

    fn database_url(&self, database: &str) -> String {
        format!(
            "postgres://postgres:postgres@127.0.0.1:{}/{}",
            self.port, database
        )
    }

    fn admin_database_url(&self) -> String {
        self.database_url("postgres")
    }

    fn wait_until_ready(&self, ctx: &TaskContext) -> Result<()> {
        for _ in 0..40 {
            let mut command = ctx.command("docker");
            command.args(["exec", &self.name, "pg_isready", "-U", "postgres"]);
            match run_status(&mut command) {
                Ok(status) if status.success() => return Ok(()),
                Ok(_) | Err(_) => std::thread::sleep(Duration::from_millis(500)),
            }
        }

        bail!(
            "Docker PostgreSQL container {} did not become ready on port {}",
            self.name,
            self.port
        );
    }

    async fn create_database_pool(&self, database: &str) -> Result<sqlx::PgPool> {
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.admin_database_url())
            .await
            .with_context(|| {
                format!(
                    "failed to connect to Docker PostgreSQL admin database at {}",
                    self.admin_database_url()
                )
            })?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {}",
            quote_pg_ident(database)
        )))
        .execute(&admin_pool)
        .await
        .with_context(|| format!("failed to create Docker PostgreSQL database {database}"))?;
        admin_pool.close().await;

        PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.database_url(database))
            .await
            .with_context(|| {
                format!(
                    "failed to connect to Docker PostgreSQL database {} at {}",
                    database,
                    self.database_url(database)
                )
            })
    }

    fn schema_dump(&self, database: &str) -> Result<String> {
        let mut command = Command::new("docker");
        command.args([
            "exec",
            &self.name,
            "pg_dump",
            "--schema-only",
            "--no-owner",
            "--no-privileges",
            "--schema=public",
            "--exclude-table=_sqlx_migrations",
            "-U",
            "postgres",
            "-d",
            database,
        ]);
        let dump = run_capture(&mut command)
            .with_context(|| format!("failed to dump PostgreSQL schema for database {database}"))?;
        Ok(normalize_postgres_schema_dump(&dump))
    }
}

impl Drop for DockerPostgresContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .output();
    }
}

fn reserve_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .context("failed to reserve a local port for Docker PostgreSQL")?;
    let port = listener
        .local_addr()
        .context("failed to read reserved Docker PostgreSQL port")?
        .port();
    drop(listener);
    Ok(port)
}

fn unique_token(through_version: i64) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{through_version:04}-{}-{millis:x}", std::process::id())
}

fn normalize_postgres_schema_dump(raw: &str) -> String {
    let mut out = String::new();
    let mut previous_blank = true;

    for line in raw.lines() {
        let trimmed = line.trim();
        if should_skip_postgres_dump_line(trimmed) {
            continue;
        }

        let normalized = line.replace("public.", "");
        let normalized = normalized.trim_end();
        if normalized.trim().is_empty() {
            if !previous_blank {
                out.push('\n');
                previous_blank = true;
            }
            continue;
        }

        out.push_str(normalized);
        out.push('\n');
        previous_blank = false;
    }

    out
}

fn append_postgres_builtin_baseline_seeds(through_version: i64, mut dump: String) -> String {
    if through_version < POSTGRES_BUILTIN_BASELINE_SEED_MIN_VERSION {
        return dump;
    }
    if !dump.ends_with('\n') {
        dump.push('\n');
    }
    dump.push_str(POSTGRES_BUILTIN_BASELINE_SEED_SQL);
    dump
}

fn should_skip_postgres_dump_line(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("--")
        || line.starts_with("\\restrict ")
        || line.starts_with("\\unrestrict ")
        || line.starts_with("SET ")
        || line.starts_with("SELECT pg_catalog.set_config(")
        || line == "CREATE SCHEMA public;"
        || line.starts_with("ALTER SCHEMA public ")
        || line.starts_with("COMMENT ON SCHEMA public ")
}

fn quote_pg_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[derive(Debug, Clone)]
struct TableColumn {
    cid: i64,
    name: String,
    pk: i64,
}

#[derive(Debug, Default, Clone)]
struct DumpNormalization {
    admin_user_id: Option<String>,
}

async fn canonical_database_dump(pool: &sqlx::SqlitePool) -> Result<String> {
    let mut out = String::new();
    let virtual_tables = virtual_table_names(pool).await?;
    let normalization = build_dump_normalization(pool).await?;
    let schema_rows = sqlx::query(
        "SELECT type, name, sql
           FROM sqlite_master
          WHERE sql IS NOT NULL
            AND name NOT LIKE 'sqlite_%'
            AND name NOT LIKE '_sqlx_%'
          ORDER BY CASE type
              WHEN 'table' THEN 1
              WHEN 'index' THEN 2
              WHEN 'trigger' THEN 3
              WHEN 'view' THEN 4
              ELSE 5
          END, name",
    )
    .fetch_all(pool)
    .await
    .context("failed to query sqlite_master")?;

    for row in schema_rows {
        let name: String = row.try_get("name")?;
        if is_virtual_shadow_table(&virtual_tables, &name) {
            continue;
        }
        let sql: String = row.try_get("sql")?;
        out.push_str(sql.trim());
        out.push_str(";\n");
    }

    let tables = sqlx::query_scalar::<_, String>(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
            AND sql IS NOT NULL
            AND name NOT LIKE 'sqlite_%'
            AND name NOT LIKE '_sqlx_%'
          ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("failed to enumerate tables for baseline dump")?;

    for table in tables {
        if is_virtual_shadow_table(&virtual_tables, &table) {
            continue;
        }
        let columns = table_columns(pool, &table).await?;
        if columns.is_empty() {
            continue;
        }

        let select_sql = format!(
            "SELECT * FROM {}{}",
            quote_ident(&table),
            build_order_clause(&table, &columns)
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(&*select_sql))
            .fetch_all(pool)
            .await
            .with_context(|| format!("failed to dump rows from {table}"))?;

        if rows.is_empty() {
            continue;
        }

        let column_sql = columns
            .iter()
            .map(|column| quote_ident(&column.name))
            .collect::<Vec<_>>()
            .join(", ");

        for row in rows {
            let mut values = Vec::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                values.push(sql_literal(
                    &row,
                    index,
                    &table,
                    &column.name,
                    &normalization,
                )?);
            }
            out.push_str(&format!(
                "INSERT INTO {} ({column_sql}) VALUES ({});\n",
                quote_ident(&table),
                values.join(", ")
            ));
        }
    }

    Ok(out)
}

async fn build_dump_normalization(pool: &sqlx::SqlitePool) -> Result<DumpNormalization> {
    if !sqlite_table_exists(pool, "users").await? {
        return Ok(DumpNormalization::default());
    }

    let admin_user_id = sqlx::query_scalar::<_, String>(
        "SELECT id
           FROM users
          WHERE username = 'admin'
          ORDER BY id
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("failed to load admin user id for baseline normalization")?;

    Ok(DumpNormalization { admin_user_id })
}

async fn sqlite_table_exists(pool: &sqlx::SqlitePool, table_name: &str) -> Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM sqlite_master
          WHERE type = 'table'
            AND name = ?1",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to probe sqlite table {table_name}"))?;

    Ok(count > 0)
}

async fn virtual_table_names(pool: &sqlx::SqlitePool) -> Result<HashSet<String>> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
            AND sql LIKE 'CREATE VIRTUAL TABLE %'
          ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("failed to enumerate sqlite virtual tables")?;

    Ok(names.into_iter().collect())
}

fn is_virtual_shadow_table(virtual_tables: &HashSet<String>, table_name: &str) -> bool {
    virtual_tables.iter().any(|virtual_table| {
        table_name != virtual_table
            && table_name
                .strip_prefix(virtual_table)
                .is_some_and(|suffix| suffix.starts_with('_'))
    })
}

async fn table_columns(pool: &sqlx::SqlitePool, table: &str) -> Result<Vec<TableColumn>> {
    let pragma_sql = format!("PRAGMA table_info({})", quote_sql_string(table));
    let rows = sqlx::query(sqlx::AssertSqlSafe(&*pragma_sql))
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to load table info for {table}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TableColumn {
            cid: row.try_get("cid")?,
            name: row.try_get("name")?,
            pk: row.try_get("pk")?,
        });
    }

    out.sort_by_key(|column| column.cid);
    Ok(out)
}

fn build_order_clause(table: &str, columns: &[TableColumn]) -> String {
    if table == "library_roots" {
        return format!(
            " ORDER BY {}, {}",
            quote_ident("library_id"),
            quote_ident("normalized_path")
        );
    }

    let mut ordered = columns
        .iter()
        .filter(|column| column.pk > 0)
        .collect::<Vec<_>>();
    if !ordered.is_empty() {
        ordered.sort_by_key(|column| column.pk);
    } else {
        ordered = columns.iter().collect();
    }

    let clause = ordered
        .into_iter()
        .map(|column| quote_ident(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    if clause.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {clause}")
    }
}

fn sql_literal(
    row: &sqlx::sqlite::SqliteRow,
    index: usize,
    table: &str,
    column: &str,
    normalization: &DumpNormalization,
) -> Result<String> {
    let raw = row.try_get_raw(index)?;
    if raw.is_null() {
        return Ok("NULL".to_string());
    }

    match raw.type_info().name() {
        "INTEGER" | "BOOLEAN" => Ok(row.try_get::<i64, _>(index)?.to_string()),
        "REAL" => {
            let value = row.try_get::<f64, _>(index)?;
            if !value.is_finite() {
                bail!("non-finite REAL values are not supported in baseline dumps");
            }
            Ok(value.to_string())
        }
        "BLOB" => {
            let value = row.try_get::<Vec<u8>, _>(index)?;
            Ok(format!(
                "X'{}'",
                value
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ))
        }
        _ => {
            let value = row.try_get::<String, _>(index)?;
            Ok(quote_sql_string(&normalize_dump_text_value(
                row,
                table,
                column,
                &value,
                normalization,
            )?))
        }
    }
}

fn normalize_dump_text_value(
    row: &sqlx::sqlite::SqliteRow,
    table: &str,
    column: &str,
    value: &str,
    normalization: &DumpNormalization,
) -> Result<String> {
    if table == "library_roots" && column == "id" {
        let library_id = row
            .try_get::<String, _>("library_id")
            .context("failed to load library_roots.library_id during dump normalization")?;
        return Ok(format!("canonical_root_for_{library_id}"));
    }

    if column.ends_with("_at") && looks_like_utc_timestamp(value) {
        return Ok(CANONICAL_TIMESTAMP.to_string());
    }

    let Some(admin_user_id) = normalization.admin_user_id.as_deref() else {
        return Ok(value.to_string());
    };

    if value == admin_user_id
        && ((table == "users" && column == "id") || column.ends_with("user_id"))
    {
        Ok(CANONICAL_ADMIN_USER_ID.to_string())
    } else {
        Ok(value.to_string())
    }
}

fn looks_like_utc_timestamp(value: &str) -> bool {
    value.len() == 20
        && value.ends_with('Z')
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_infrastructure_datastore::migration_assets::{
        EngineScope, LegacySqlBlock, SourceMigrationManifest,
    };

    #[test]
    fn normalize_postgres_schema_dump_strips_runtime_noise() {
        let dump = r#"
--
-- PostgreSQL database dump
--

\restrict abc123
SET statement_timeout = 0;
SET search_path = public, pg_catalog;
SELECT pg_catalog.set_config('search_path', '', false);

CREATE SCHEMA public;

CREATE TABLE public.download_jobs (
    id uuid NOT NULL
);

ALTER TABLE ONLY public.download_jobs
    ADD CONSTRAINT download_jobs_pkey PRIMARY KEY (id);

\unrestrict abc123
"#;

        assert_eq!(
            normalize_postgres_schema_dump(dump),
            "CREATE TABLE download_jobs (\n    id uuid NOT NULL\n);\nALTER TABLE ONLY download_jobs\n    ADD CONSTRAINT download_jobs_pkey PRIMARY KEY (id);\n"
        );
    }

    #[test]
    fn postgres_baseline_generation_appends_builtin_seed_data() {
        let generated =
            append_postgres_builtin_baseline_seeds(140, "CREATE TABLE users ();\n".into());
        assert!(generated.ends_with(POSTGRES_BUILTIN_BASELINE_SEED_SQL));
        assert_eq!(generated.matches("INSERT INTO ").count(), 14);
        assert_eq!(
            append_postgres_builtin_baseline_seeds(139, "CREATE TABLE users ();\n".into()),
            "CREATE TABLE users ();\n"
        );

        let checked_in = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../crates/scryer/src/db/postgres/baselines/0198_baseline.sql"),
        )
        .expect("active PostgreSQL baseline should be readable");
        assert!(checked_in.ends_with(POSTGRES_BUILTIN_BASELINE_SEED_SQL));
    }

    #[test]
    fn upsert_baseline_entry_preserves_other_engine_entries() {
        let mut manifest = SourceMigrationManifest {
            format_version: 1,
            legacy_sql: LegacySqlBlock {
                path: "migrations".to_string(),
                through_version: 100,
            },
            migrations: Vec::new(),
            baselines: vec![
                scryer_infrastructure_datastore::migration_assets::SourceBaselineEntry {
                    through_version: 114,
                    file: "baselines/0114_baseline.sql".to_string(),
                    engine: EngineScope::Sqlite,
                },
            ],
        };

        assert!(upsert_baseline_entry(
            &mut manifest,
            114,
            "postgres/baselines/0114_baseline.sql",
            BaselineEngine::Postgres,
        ));
        assert!(manifest_has_baseline_entry(
            &manifest,
            114,
            BaselineEngine::Sqlite
        ));
        assert!(manifest_has_baseline_entry(
            &manifest,
            114,
            BaselineEngine::Postgres
        ));
        assert_eq!(manifest.baselines.len(), 2);
    }

    #[tokio::test]
    async fn sqlite_0130_migrates_duplicate_legacy_provider_ids() {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create sqlite pool");
        let mut conn = pool.acquire().await.expect("acquire sqlite connection");

        let setup = r#"
            CREATE TABLE titles (
                id TEXT PRIMARY KEY NOT NULL
            );
            CREATE TABLE collections (
                id TEXT PRIMARY KEY NOT NULL,
                title_id TEXT NOT NULL,
                collection_type TEXT NOT NULL,
                collection_index TEXT NOT NULL,
                label TEXT,
                narrative_order TEXT,
                monitored INTEGER,
                ordered_path TEXT,
                interstitial_name TEXT,
                interstitial_sort_title TEXT,
                interstitial_slug TEXT,
                interstitial_year INTEGER,
                interstitial_overview TEXT,
                interstitial_poster_url TEXT,
                interstitial_language TEXT,
                interstitial_runtime_minutes INTEGER,
                interstitial_content_status TEXT,
                interstitial_genres_json TEXT,
                interstitial_studio TEXT,
                interstitial_digital_release_date TEXT,
                interstitial_imdb_id TEXT,
                interstitial_tvdb_id TEXT,
                interstitial_movie_tmdb_id TEXT,
                interstitial_movie_mal_id TEXT,
                interstitial_movie_anidb_id TEXT,
                interstitial_placement TEXT,
                interstitial_association_confidence TEXT,
                interstitial_continuity_status TEXT,
                interstitial_movie_form TEXT,
                interstitial_confidence TEXT,
                interstitial_signal_summary TEXT,
                interstitial_season_episode TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT
            );
            CREATE TABLE episodes (
                id TEXT PRIMARY KEY NOT NULL,
                title_id TEXT NOT NULL,
                collection_id TEXT,
                season_number TEXT,
                episode_number TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE media_files (
                id TEXT PRIMARY KEY NOT NULL,
                title_id TEXT NOT NULL,
                file_path TEXT NOT NULL
            );
            CREATE TABLE file_episode_map (
                file_id TEXT NOT NULL,
                episode_id TEXT NOT NULL,
                PRIMARY KEY (file_id, episode_id)
            );
            CREATE TABLE wanted_items (
                id TEXT PRIMARY KEY NOT NULL,
                title_id TEXT NOT NULL,
                episode_id TEXT,
                collection_id TEXT,
                media_type TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX idx_wanted_items_movie_unique
                ON wanted_items(title_id)
                WHERE episode_id IS NULL AND collection_id IS NULL;
            CREATE TABLE download_submissions (
                id TEXT PRIMARY KEY NOT NULL,
                collection_id TEXT
            );
            CREATE TABLE workflow_operations (
                id TEXT PRIMARY KEY NOT NULL,
                collection_id TEXT
            );

            INSERT INTO titles (id) VALUES ('title-1');
            INSERT INTO collections (
                id, title_id, collection_type, collection_index, label, narrative_order,
                monitored, ordered_path, interstitial_name, interstitial_sort_title,
                interstitial_slug, interstitial_year, interstitial_overview,
                interstitial_poster_url, interstitial_language, interstitial_runtime_minutes,
                interstitial_content_status, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_imdb_id, interstitial_tvdb_id,
                interstitial_movie_tmdb_id, interstitial_movie_mal_id,
                interstitial_movie_anidb_id, interstitial_placement,
                interstitial_association_confidence, interstitial_continuity_status,
                interstitial_movie_form, interstitial_confidence, interstitial_signal_summary,
                interstitial_season_episode, created_at, updated_at
            ) VALUES
                (
                    'legacy-collection-1', 'title-1', 'interstitial', '1.5', 'Bridge Movie A',
                    '1.5', 1, '/media/bridge-a.mkv', 'Bridge Movie', 'Bridge Movie',
                    'bridge-movie', 2024, 'overview', 'poster', 'eng', 95, 'released',
                    '[]', 'Studio', '2024-01-01', 'tt1234567', 'movie-tvdb-1', NULL,
                    NULL, NULL, 'after season 1', 'high', 'canonical', 'movie', 'high',
                    'fixture', 'S00E01', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
                ),
                (
                    'legacy-collection-2', 'title-1', 'interstitial', '2.5', 'Bridge Movie B',
                    '2.5', 1, '/media/bridge-b.mkv', 'Bridge Movie', 'Bridge Movie',
                    'bridge-movie', 2024, 'overview', 'poster', 'eng', 95, 'released',
                    '[]', 'Studio', '2024-01-01', 'tt1234567', 'movie-tvdb-1', NULL,
                    NULL, NULL, 'after season 2', 'high', 'canonical', 'movie', 'high',
                    'fixture', 'S00E02', '2024-01-02T00:00:00Z', '2024-01-02T00:00:00Z'
                );
            INSERT INTO media_files (id, title_id, file_path)
            VALUES
                ('file-1', 'title-1', '/media/bridge-a.mkv'),
                ('file-2', 'title-1', '/media/bridge-b.mkv');
        "#;

        for statement in split_sql_statements(setup) {
            sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
                .execute(&mut *conn)
                .await
                .unwrap_or_else(|error| panic!("failed setup statement {statement}: {error}"));
        }

        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask-migrations has a repository parent");
        let migration_sql = std::fs::read_to_string(
            repo_root.join("crates/scryer/src/db/migrations/0130_series_movie_links.sql"),
        )
        .expect("read sqlite 0130 migration");
        for statement in split_sql_statements(&migration_sql) {
            sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
                .execute(&mut *conn)
                .await
                .unwrap_or_else(|error| panic!("failed migration statement {statement}: {error}"));
        }

        let link_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM series_movie_links")
            .fetch_one(&mut *conn)
            .await
            .expect("count links");
        let movie_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM movie_entities")
            .fetch_one(&mut *conn)
            .await
            .expect("count movies");
        let distinct_movie_count: i64 =
            sqlx::query_scalar("SELECT COUNT(DISTINCT movie_entity_id) FROM series_movie_links")
                .fetch_one(&mut *conn)
                .await
                .expect("count linked movies");
        let file_link_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM file_series_movie_link_map")
                .fetch_one(&mut *conn)
                .await
                .expect("count file links");

        assert_eq!(link_count, 2);
        assert_eq!(movie_count, 1);
        assert_eq!(distinct_movie_count, 1);
        assert_eq!(file_link_count, 2);
    }

    fn split_sql_statements(sql: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut quote = None;
        for (idx, ch) in sql.char_indices() {
            if let Some(active_quote) = quote {
                if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                ';' => {
                    let statement = sql[start..idx].trim();
                    if !statement.is_empty() {
                        out.push(statement.to_string());
                    }
                    start = idx + 1;
                }
                _ => {}
            }
        }
        let statement = sql[start..].trim();
        if !statement.is_empty() {
            out.push(statement.to_string());
        }
        out
    }
}
