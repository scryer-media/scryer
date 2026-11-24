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

pub(crate) fn run_rebaseline(ctx: &TaskContext, args: RebaselineArgs) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    runtime.block_on(async move { run_rebaseline_inner(ctx, args).await })
}

async fn run_rebaseline_inner(ctx: &TaskContext, args: RebaselineArgs) -> Result<()> {
    if args.through <= 0 {
        bail!("--through must be a positive migration version");
    }

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
        let postgres_dump = container.database_dump(&target_db, &target_pool).await?;
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
        let reference_dump = container
            .database_dump(&reference_db, &reference_pool)
            .await?;

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
        let verification_dump = container
            .database_dump(&verification_db, &verification_pool)
            .await?;

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
        // The image restarts the server once after its first-boot setup, and
        // the readiness probe can pass against the instance about to go away.
        let mut attempt = 0;
        let admin_pool = loop {
            match PgPoolOptions::new()
                .max_connections(1)
                .connect(&self.admin_database_url())
                .await
            {
                Err(_) if attempt < 10 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                result => break result,
            }
        }
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

impl DockerPostgresContainer {
    /// The schema followed by every row the migrations left behind, so a
    /// baseline carries the same seed data a full replay would.
    async fn database_dump(&self, database: &str, pool: &sqlx::PgPool) -> Result<String> {
        let mut dump = self.schema_dump(database)?;
        dump.push_str(&postgres_data_dump(pool).await?);
        Ok(dump)
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

/// Tables and views come out by name. Indexes and triggers come out in the
/// order the migrations created them: with no statistics SQLite breaks a tie
/// between two usable indexes by that order, and fires same-event triggers by
/// it, so a database built from the baseline must not reorder them.
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
          END,
          CASE WHEN type IN ('index', 'trigger') THEN rowid ELSE 0 END,
          name",
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

    if let Some(expression) = regenerated_seed_expression(BaselineEngine::Sqlite, table, column) {
        return Ok(expression.to_string());
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
            let library_id =
                if table == "library_roots" && column == "id" {
                    Some(row.try_get::<String, _>("library_id").context(
                        "failed to load library_roots.library_id during dump normalization",
                    )?)
                } else {
                    None
                };
            Ok(quote_sql_string(&normalize_dump_text_value(
                table,
                column,
                &value,
                library_id.as_deref(),
                normalization,
            )))
        }
    }
}

/// A seeded value its migration draws at random, so that no two installs
/// share it. The baseline carries the migration's own expression in place of
/// one drawn value: a fresh install still gets its own, and two replays dump
/// the same text.
fn regenerated_seed_expression(
    engine: BaselineEngine,
    table: &str,
    column: &str,
) -> Option<&'static str> {
    match (table, column) {
        ("discovery_affinity_revision", "revision") => Some(match engine {
            BaselineEngine::Sqlite => "lower(hex(randomblob(16)))",
            BaselineEngine::Postgres => "md5(random()::text || clock_timestamp()::text)",
        }),
        _ => None,
    }
}

/// `library_root_owner` is the row's `library_id`, present only for
/// `library_roots.id`, whose generated value is replaced by a stable one.
fn normalize_dump_text_value(
    table: &str,
    column: &str,
    value: &str,
    library_root_owner: Option<&str>,
    normalization: &DumpNormalization,
) -> String {
    if table == "library_roots"
        && column == "id"
        && let Some(library_id) = library_root_owner
    {
        return format!("canonical_root_for_{library_id}");
    }

    if column.ends_with("_at") && looks_like_utc_timestamp(value) {
        return CANONICAL_TIMESTAMP.to_string();
    }

    let Some(admin_user_id) = normalization.admin_user_id.as_deref() else {
        return value.to_string();
    };

    if value == admin_user_id
        && ((table == "users" && column == "id") || column.ends_with("user_id"))
    {
        CANONICAL_ADMIN_USER_ID.to_string()
    } else {
        value.to_string()
    }
}

struct PostgresColumn {
    name: String,
    /// Rendered without quotes: numbers and booleans.
    bare_literal: bool,
    /// A native timestamp. Its text form never matches the ISO shape the
    /// SQLite dump recognizes, and a seed row's clock reading means nothing.
    timestamp: bool,
}

/// Every row in the public schema as `INSERT` statements, normalized the same
/// way as the SQLite dump. Tables come out parents-first so the statements
/// replay under their foreign keys, then by name; rows by primary key.
async fn postgres_data_dump(pool: &sqlx::PgPool) -> Result<String> {
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT tablename::text
           FROM pg_tables
          WHERE schemaname = 'public'
            AND tablename NOT LIKE '\\_sqlx\\_%'
          ORDER BY tablename",
    )
    .fetch_all(pool)
    .await
    .context("failed to enumerate PostgreSQL tables for baseline dump")?;

    let dependencies = sqlx::query_as::<_, (String, String)>(
        "SELECT child.relname::text, parent.relname::text
           FROM pg_constraint
           JOIN pg_class child ON child.oid = pg_constraint.conrelid
           JOIN pg_class parent ON parent.oid = pg_constraint.confrelid
           JOIN pg_namespace ON pg_namespace.oid = child.relnamespace
          WHERE pg_constraint.contype = 'f'
            AND pg_namespace.nspname = 'public'
            AND child.oid <> parent.oid",
    )
    .fetch_all(pool)
    .await
    .context("failed to load PostgreSQL foreign keys for baseline dump")?;

    let admin_user_id = if tables.iter().any(|table| table == "users") {
        sqlx::query_scalar::<_, String>(
            "SELECT id::text FROM users WHERE username = 'admin' ORDER BY id LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .context("failed to load admin user id for PostgreSQL baseline normalization")?
    } else {
        None
    };
    let normalization = DumpNormalization { admin_user_id };

    let mut out = String::new();
    let mut seeded_tables = Vec::new();
    for table in parents_first(&tables, &dependencies)? {
        let columns = postgres_table_columns(pool, &table).await?;
        if columns.is_empty() {
            continue;
        }
        let order_by = postgres_order_columns(pool, &table, &columns).await?;
        let select_sql = format!(
            "SELECT {} FROM {} ORDER BY {}",
            columns
                .iter()
                .map(|column| format!("{}::text", quote_pg_ident(&column.name)))
                .collect::<Vec<_>>()
                .join(", "),
            quote_pg_ident(&table),
            order_by
                .iter()
                .map(|column| quote_pg_ident(column))
                .collect::<Vec<_>>()
                .join(", "),
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(&*select_sql))
            .fetch_all(pool)
            .await
            .with_context(|| format!("failed to dump rows from PostgreSQL table {table}"))?;
        if rows.is_empty() {
            continue;
        }
        seeded_tables.push(table.clone());

        let column_sql = columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let library_id_index = columns
            .iter()
            .position(|column| column.name == "library_id");
        for row in rows {
            let library_id = match library_id_index {
                Some(index) if table == "library_roots" => {
                    row.try_get::<Option<String>, _>(index)?
                }
                _ => None,
            };
            let mut values = Vec::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                let regenerated =
                    regenerated_seed_expression(BaselineEngine::Postgres, &table, &column.name);
                values.push(match row.try_get::<Option<String>, _>(index)? {
                    None => "NULL".to_string(),
                    Some(_) if regenerated.is_some() => regenerated.unwrap_or_default().to_string(),
                    Some(value) if column.bare_literal => value,
                    Some(_) if column.timestamp => quote_sql_string(CANONICAL_TIMESTAMP),
                    Some(value) => quote_sql_string(&normalize_dump_text_value(
                        &table,
                        &column.name,
                        &value,
                        library_id.as_deref(),
                        &normalization,
                    )),
                });
            }
            out.push_str(&format!(
                "INSERT INTO {table} ({column_sql}) VALUES ({});\n",
                values.join(", ")
            ));
        }
    }

    // A seeded row that took its key from a sequence leaves the restored
    // sequence behind it; the next insert would collide.
    let owned_sequences = sqlx::query_as::<_, (String, String, String)>(
        "SELECT sequence.relname::text, owner.relname::text, pg_attribute.attname::text
           FROM pg_class sequence
           JOIN pg_namespace ON pg_namespace.oid = sequence.relnamespace
           JOIN pg_depend ON pg_depend.objid = sequence.oid AND pg_depend.deptype = 'a'
           JOIN pg_class owner ON owner.oid = pg_depend.refobjid
           JOIN pg_attribute ON pg_attribute.attrelid = owner.oid
                            AND pg_attribute.attnum = pg_depend.refobjsubid
          WHERE sequence.relkind = 'S'
            AND pg_namespace.nspname = 'public'
          ORDER BY sequence.relname",
    )
    .fetch_all(pool)
    .await
    .context("failed to load PostgreSQL sequence ownership for baseline dump")?;
    for (sequence, table, column) in owned_sequences {
        if seeded_tables.contains(&table) {
            out.push_str(&format!(
                "SELECT pg_catalog.setval('{sequence}', (SELECT MAX({column}) FROM {table}));\n"
            ));
        }
    }

    Ok(out)
}

async fn postgres_table_columns(pool: &sqlx::PgPool, table: &str) -> Result<Vec<PostgresColumn>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT column_name::text, data_type::text
           FROM information_schema.columns
          WHERE table_schema = 'public'
            AND table_name = $1
            AND is_generated = 'NEVER'
          ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to load PostgreSQL columns for {table}"))?;

    Ok(rows
        .into_iter()
        .map(|(name, data_type)| PostgresColumn {
            name,
            bare_literal: matches!(
                data_type.as_str(),
                "boolean" | "smallint" | "integer" | "bigint" | "numeric"
            ),
            timestamp: data_type.starts_with("timestamp"),
        })
        .collect())
}

/// Primary-key columns, or every column for a table without one. The SQLite
/// dump's `library_roots` ordering is kept so both engines seed alike.
async fn postgres_order_columns(
    pool: &sqlx::PgPool,
    table: &str,
    columns: &[PostgresColumn],
) -> Result<Vec<String>> {
    if table == "library_roots" {
        return Ok(vec![
            "library_id".to_string(),
            "normalized_path".to_string(),
        ]);
    }
    let primary_key = sqlx::query_scalar::<_, String>(
        "SELECT pg_attribute.attname::text
           FROM pg_index
           JOIN pg_class ON pg_class.oid = pg_index.indrelid
           JOIN pg_namespace ON pg_namespace.oid = pg_class.relnamespace
           JOIN pg_attribute ON pg_attribute.attrelid = pg_class.oid
                            AND pg_attribute.attnum = ANY(pg_index.indkey)
          WHERE pg_index.indisprimary
            AND pg_namespace.nspname = 'public'
            AND pg_class.relname = $1
          ORDER BY array_position(pg_index.indkey::int2[], pg_attribute.attnum)",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to load PostgreSQL primary key for {table}"))?;
    if primary_key.is_empty() {
        Ok(columns.iter().map(|column| column.name.clone()).collect())
    } else {
        Ok(primary_key)
    }
}

/// `tables` reordered so every table follows the tables it references, ties
/// broken by name so the dump is stable.
fn parents_first(tables: &[String], dependencies: &[(String, String)]) -> Result<Vec<String>> {
    let mut remaining: Vec<&String> = tables.iter().collect();
    let mut ordered: Vec<String> = Vec::with_capacity(tables.len());
    while !remaining.is_empty() {
        let ready = remaining.iter().position(|table| {
            dependencies.iter().all(|(child, parent)| {
                child != *table || ordered.contains(parent) || !tables.contains(parent)
            })
        });
        let Some(index) = ready else {
            bail!(
                "PostgreSQL tables reference each other in a cycle: {}",
                remaining
                    .iter()
                    .map(|table| table.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        };
        ordered.push(remaining.remove(index).clone());
    }
    Ok(ordered)
}

fn looks_like_utc_timestamp(value: &str) -> bool {
    let suffix = value.get(19..).unwrap_or("invalid");
    let valid_suffix = suffix == "Z"
        || (value.as_bytes().get(10) == Some(&b' ') && suffix.is_empty())
        || suffix.strip_prefix('.').is_some_and(|fraction| {
            let digits = fraction.strip_suffix('Z').unwrap_or(fraction);
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        });
    value.len() >= 19
        && valid_suffix
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && matches!(value.as_bytes().get(10), Some(b'T' | b' '))
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

    #[tokio::test]
    async fn sqlite_current_baseline_matches_full_migration_replay() {
        let mut dumps = Vec::new();
        for enable_baselines in [false, true] {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap();
            scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
                &pool,
                None,
                enable_baselines,
            )
            .await
            .unwrap();
            assert!(
                !sqlite_table_exists(&pool, "title_search_spellfix")
                    .await
                    .unwrap()
            );
            dumps.push(canonical_database_dump(&pool).await.unwrap());
            pool.close().await;
        }
        assert_eq!(dumps[0], dumps[1]);
    }

    #[test]
    fn baseline_timestamps_cover_sqlite_default_and_fractional_formats() {
        for value in [
            "2026-09-21T13:54:49Z",
            "2026-09-21T13:54:49.500Z",
            "2026-09-21 13:54:49",
            "2026-09-21 13:54:49.500",
        ] {
            assert!(looks_like_utc_timestamp(value), "{value}");
        }
        for value in [
            "",
            "arbitrary text",
            "2026-09-21",
            "2026-09-21T13:54:49.fooZ",
        ] {
            assert!(!looks_like_utc_timestamp(value), "{value}");
        }
    }

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

    /// The SQLite baseline is held to a full migration replay above; seeding
    /// the same tables with the same number of rows ties the PostgreSQL one to
    /// it without a database server.
    #[test]
    fn checked_in_baselines_seed_the_same_rows_on_both_engines() {
        let seeded_rows = |relative: &str| {
            let sql = std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../crates/scryer/src/db")
                    .join(relative),
            )
            .expect("checked-in baseline should be readable");
            let mut counts = std::collections::BTreeMap::<String, usize>::new();
            for line in sql.lines() {
                if let Some(rest) = line.strip_prefix("INSERT INTO ") {
                    let table = rest.split(' ').next().unwrap_or_default().trim_matches('"');
                    *counts.entry(table.to_string()).or_default() += 1;
                }
            }
            counts
        };

        let sqlite = seeded_rows("baselines/0341_baseline.sql");
        assert!(!sqlite.is_empty());
        assert_eq!(sqlite, seeded_rows("postgres/baselines/0341_baseline.sql"));
    }

    #[test]
    fn parents_first_orders_tables_under_their_foreign_keys() {
        let tables = ["quality_profile_quality_tiers", "quality_profiles", "users"]
            .map(str::to_string)
            .to_vec();
        let dependencies = vec![(
            "quality_profile_quality_tiers".to_string(),
            "quality_profiles".to_string(),
        )];
        assert_eq!(
            parents_first(&tables, &dependencies).unwrap(),
            ["quality_profiles", "quality_profile_quality_tiers", "users"]
        );

        let cycle = vec![
            ("users".to_string(), "quality_profiles".to_string()),
            ("quality_profiles".to_string(), "users".to_string()),
        ];
        assert!(parents_first(&tables, &cycle).is_err());
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
