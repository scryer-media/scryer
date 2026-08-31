#![allow(dead_code)]

use crate::migration_hook_ids;
use blake3::Hasher as Blake3Hasher;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha384};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Sha384,
    Blake3,
}

impl ChecksumAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha384 => "sha384",
            Self::Blake3 => "blake3",
        }
    }

    pub fn digest(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha384 => Sha384::digest(bytes).to_vec(),
            Self::Blake3 => {
                let mut hasher = Blake3Hasher::new();
                hasher.update(bytes);
                hasher.finalize().as_bytes().to_vec()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationInstallKind {
    FreshInstall,
    Upgrade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepScope {
    #[default]
    All,
    UpgradeOnly,
    NewInstallOnly,
}

impl StepScope {
    pub fn applies_to(self, install_kind: MigrationInstallKind) -> bool {
        matches!(
            (self, install_kind),
            (Self::All, _)
                | (Self::UpgradeOnly, MigrationInstallKind::Upgrade)
                | (Self::NewInstallOnly, MigrationInstallKind::FreshInstall)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineScope {
    All,
    Sqlite,
    Postgres,
}

impl EngineScope {
    pub fn applies_to(self, engine: EngineScope) -> bool {
        matches!(self, Self::All) || self == engine
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMigrationManifest {
    #[serde(default = "default_manifest_version")]
    pub format_version: u32,
    pub legacy_sql: LegacySqlBlock,
    #[serde(default, rename = "migration")]
    pub migrations: Vec<SourceExplicitMigration>,
    #[serde(default, rename = "baseline")]
    pub baselines: Vec<SourceBaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySqlBlock {
    pub path: String,
    pub through_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceExplicitMigration {
    pub version: i64,
    pub description: String,
    #[serde(default = "default_explicit_checksum_algorithm")]
    pub checksum_algo: ChecksumAlgorithm,
    #[serde(default)]
    pub steps: Vec<SourceMigrationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceMigrationStep {
    Sql {
        file: String,
        #[serde(default)]
        engine: Option<EngineScope>,
        #[serde(default)]
        scope: StepScope,
    },
    Rust {
        hook_id: String,
        #[serde(default)]
        engine: Option<EngineScope>,
        #[serde(default)]
        scope: StepScope,
    },
}

impl SourceMigrationStep {
    fn resolved_engine(&self) -> EngineScope {
        match self {
            Self::Sql { engine, .. } | Self::Rust { engine, .. } => {
                engine.unwrap_or(EngineScope::All)
            }
        }
    }

    fn explicit_engine(&self) -> Option<EngineScope> {
        match self {
            Self::Sql { engine, .. } | Self::Rust { engine, .. } => *engine,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceBaselineEntry {
    pub through_version: i64,
    pub file: String,
    pub engine: EngineScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledMigrationBundle {
    pub catalog: CompiledMigrationCatalog,
    pub payload_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledMigrationCatalog {
    pub format_version: u32,
    pub migrations: Vec<CompiledMigration>,
    pub baselines: Vec<CompiledBaseline>,
}

impl CompiledMigrationCatalog {
    pub fn max_version(&self) -> i64 {
        self.migrations
            .last()
            .map(|migration| migration.version)
            .unwrap_or(0)
    }

    pub fn find_migration(&self, version: i64) -> Option<&CompiledMigration> {
        self.migrations
            .iter()
            .find(|migration| migration.version == version)
    }

    pub fn latest_baseline_at_or_below(
        &self,
        version: i64,
        engine: EngineScope,
    ) -> Option<&CompiledBaseline> {
        self.baselines
            .iter()
            .filter(|baseline| baseline.through_version <= version)
            .filter(|baseline| baseline.engine.applies_to(engine))
            .max_by_key(|baseline| baseline.through_version)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledMigration {
    pub version: i64,
    pub description: String,
    pub key: String,
    pub filename: String,
    pub checksum_algo: ChecksumAlgorithm,
    pub checksum: Vec<u8>,
    pub steps: Vec<CompiledMigrationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompiledMigrationStep {
    Sql {
        file: String,
        engine: EngineScope,
        scope: StepScope,
        payload: PayloadSlice,
    },
    Rust {
        hook_id: String,
        engine: EngineScope,
        scope: StepScope,
    },
}

impl CompiledMigrationStep {
    pub fn scope(&self) -> StepScope {
        match self {
            Self::Sql { scope, .. } | Self::Rust { scope, .. } => *scope,
        }
    }

    pub fn engine(&self) -> EngineScope {
        match self {
            Self::Sql { engine, .. } | Self::Rust { engine, .. } => *engine,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledBaseline {
    pub through_version: i64,
    pub file: String,
    pub engine: EngineScope,
    pub payload: PayloadSlice,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PayloadSlice {
    pub start: u64,
    pub len: u64,
}

impl PayloadSlice {
    pub fn bytes<'a>(&self, payload_bytes: &'a [u8]) -> Result<&'a [u8], String> {
        let start = usize::try_from(self.start).map_err(|_| "payload start out of range")?;
        let len = usize::try_from(self.len).map_err(|_| "payload length out of range")?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| "payload slice overflow".to_string())?;
        payload_bytes
            .get(start..end)
            .ok_or_else(|| "payload slice outside bundle".to_string())
    }

    pub fn text<'a>(&self, payload_bytes: &'a [u8]) -> Result<&'a str, String> {
        std::str::from_utf8(self.bytes(payload_bytes)?)
            .map_err(|error| format!("payload is not valid UTF-8: {error}"))
    }
}

#[derive(Debug, Serialize)]
struct CanonicalMigration {
    version: i64,
    description: String,
    steps: Vec<CanonicalStep>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CanonicalStep {
    Sql {
        #[serde(skip_serializing_if = "Option::is_none")]
        engine: Option<EngineScope>,
        scope: StepScope,
        sql: String,
    },
    Rust {
        #[serde(skip_serializing_if = "Option::is_none")]
        engine: Option<EngineScope>,
        scope: StepScope,
        hook_id: String,
    },
}

fn default_manifest_version() -> u32 {
    DEFAULT_MANIFEST_VERSION
}

fn default_explicit_checksum_algorithm() -> ChecksumAlgorithm {
    ChecksumAlgorithm::Blake3
}

pub fn source_manifest_path(db_root: &Path) -> PathBuf {
    db_root.join("migration_manifest.toml")
}

pub fn load_source_manifest(db_root: &Path) -> Result<SourceMigrationManifest, String> {
    let path = source_manifest_path(db_root);
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    toml::from_str(&contents)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

pub fn write_source_manifest(
    db_root: &Path,
    manifest: &SourceMigrationManifest,
) -> Result<(), String> {
    let path = source_manifest_path(db_root);
    let contents = toml::to_string_pretty(manifest)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    fs::write(&path, format!("{contents}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn compile_source_bundle(db_root: &Path) -> Result<CompiledMigrationBundle, String> {
    let manifest = load_source_manifest(db_root)?;
    if manifest.format_version != DEFAULT_MANIFEST_VERSION {
        return Err(format!(
            "unsupported migration manifest version {}",
            manifest.format_version
        ));
    }

    let mut payload_bytes = Vec::new();
    let mut migrations =
        compile_legacy_migrations(db_root, &manifest.legacy_sql, &mut payload_bytes)?;
    let legacy_through_version = manifest.legacy_sql.through_version;

    let mut explicit = manifest.migrations.clone();
    explicit.sort_by_key(|migration| migration.version);

    for (expected_version, migration) in (legacy_through_version + 1..).zip(explicit) {
        if migration.version != expected_version {
            return Err(format!(
                "explicit migration versions must be contiguous starting at {expected_version:04}; found {:04}",
                migration.version
            ));
        }

        migrations.push(compile_explicit_migration(
            db_root,
            &migration,
            &mut payload_bytes,
        )?);
    }

    validate_contiguous_versions(&migrations)?;

    let mut baselines = Vec::new();
    let mut baseline_versions = std::collections::HashSet::new();
    for baseline in manifest.baselines {
        if !baseline_versions.insert((baseline.through_version, baseline.engine)) {
            return Err(format!(
                "duplicate baseline entry for version {:04} and engine {:?}",
                baseline.through_version, baseline.engine
            ));
        }
        if migrations
            .iter()
            .all(|migration| migration.version != baseline.through_version)
        {
            return Err(format!(
                "baseline {:04} does not match any known migration version",
                baseline.through_version
            ));
        }
        let path = db_root.join(&baseline.file);
        let sql = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let payload = push_payload(sql.as_bytes(), &mut payload_bytes);
        baselines.push(CompiledBaseline {
            through_version: baseline.through_version,
            file: baseline.file,
            engine: baseline.engine,
            payload,
        });
    }
    baselines.sort_by_key(|baseline| (baseline.through_version, baseline.file.clone()));

    Ok(CompiledMigrationBundle {
        catalog: CompiledMigrationCatalog {
            format_version: manifest.format_version,
            migrations,
            baselines,
        },
        payload_bytes,
    })
}

pub fn encode_catalog(catalog: &CompiledMigrationCatalog) -> Result<Vec<u8>, String> {
    serde_json::to_vec(catalog)
        .map_err(|error| format!("failed to serialize migration catalog: {error}"))
}

pub fn decode_catalog(bytes: &[u8]) -> Result<CompiledMigrationCatalog, String> {
    serde_json::from_slice(bytes)
        .map_err(|error| format!("failed to decode migration catalog: {error}"))
}

pub fn checksum_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|value| format!("{value:02x}")).collect()
}

pub fn migration_key_from_version_and_desc(version: i64, description: &str) -> String {
    format!("{version:04}_{}", description.replace(' ', "_"))
}

fn compile_legacy_migrations(
    db_root: &Path,
    legacy: &LegacySqlBlock,
    payload_bytes: &mut Vec<u8>,
) -> Result<Vec<CompiledMigration>, String> {
    let migrations_dir = db_root.join(&legacy.path);
    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&migrations_dir)
        .map_err(|error| format!("failed to read {}: {error}", migrations_dir.display()))?;

    for entry in read_dir {
        let entry = entry
            .map_err(|error| format!("failed to read {}: {error}", migrations_dir.display()))?;
        let path = entry.path();
        if path.extension().is_none_or(|value| value != "sql") {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy().to_string();
        let (version, description, key) = parse_legacy_filename(&file_name)?;
        if version > legacy.through_version {
            continue;
        }

        let sql = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let payload = push_payload(sql.as_bytes(), payload_bytes);
        entries.push(CompiledMigration {
            version,
            description,
            key,
            filename: file_name,
            checksum_algo: ChecksumAlgorithm::Sha384,
            checksum: ChecksumAlgorithm::Sha384.digest(sql.as_bytes()),
            steps: vec![CompiledMigrationStep::Sql {
                file: normalize_relative_path(db_root, &path),
                engine: EngineScope::Sqlite,
                scope: StepScope::All,
                payload,
            }],
        });
    }

    entries.sort_by_key(|migration| migration.version);
    if entries.len() != legacy.through_version as usize {
        return Err(format!(
            "legacy migration directory {} does not contain a contiguous 0001..{:04} prefix",
            migrations_dir.display(),
            legacy.through_version
        ));
    }

    validate_contiguous_versions(&entries)?;
    Ok(entries)
}

fn compile_explicit_migration(
    db_root: &Path,
    migration: &SourceExplicitMigration,
    payload_bytes: &mut Vec<u8>,
) -> Result<CompiledMigration, String> {
    if migration.steps.is_empty() {
        return Err(format!("migration {:04} has no steps", migration.version));
    }

    let mut compiled_steps = Vec::with_capacity(migration.steps.len());
    let mut canonical_steps = Vec::with_capacity(migration.steps.len());

    for step in &migration.steps {
        match step {
            SourceMigrationStep::Sql { file, scope, .. } => {
                let engine = step.resolved_engine();
                let path = db_root.join(file);
                let sql = fs::read_to_string(&path)
                    .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
                let payload = push_payload(sql.as_bytes(), payload_bytes);
                compiled_steps.push(CompiledMigrationStep::Sql {
                    file: file.clone(),
                    engine,
                    scope: *scope,
                    payload,
                });
                canonical_steps.push(CanonicalStep::Sql {
                    engine: step.explicit_engine(),
                    scope: *scope,
                    sql,
                });
            }
            SourceMigrationStep::Rust { hook_id, scope, .. } => {
                let engine = step.resolved_engine();
                migration_hook_ids::validate_migration_hook_id(hook_id)?;
                compiled_steps.push(CompiledMigrationStep::Rust {
                    hook_id: hook_id.clone(),
                    engine,
                    scope: *scope,
                });
                canonical_steps.push(CanonicalStep::Rust {
                    engine: step.explicit_engine(),
                    scope: *scope,
                    hook_id: hook_id.clone(),
                });
            }
        }
    }

    let canonical = CanonicalMigration {
        version: migration.version,
        description: migration.description.clone(),
        steps: canonical_steps,
    };
    let canonical_bytes = serde_json::to_vec(&canonical).map_err(|error| {
        format!(
            "failed to serialize canonical migration {:04}: {error}",
            migration.version
        )
    })?;

    let key = migration_key_from_version_and_desc(migration.version, &migration.description);
    let filename = infer_explicit_filename(migration, &key);

    Ok(CompiledMigration {
        version: migration.version,
        description: migration.description.clone(),
        key,
        filename,
        checksum_algo: migration.checksum_algo,
        checksum: migration.checksum_algo.digest(&canonical_bytes),
        steps: compiled_steps,
    })
}

fn infer_explicit_filename(migration: &SourceExplicitMigration, key: &str) -> String {
    if migration.steps.len() == 1
        && let SourceMigrationStep::Sql { file, .. } = &migration.steps[0]
        && let Some(name) = Path::new(file).file_name().and_then(|value| value.to_str())
    {
        return name.to_string();
    }

    format!("{key}.migration")
}

fn parse_legacy_filename(file_name: &str) -> Result<(i64, String, String), String> {
    let stem = file_name
        .strip_suffix(".sql")
        .ok_or_else(|| format!("legacy migration {file_name} must end with .sql"))?;
    let (version, rest) = stem.split_once('_').ok_or_else(|| {
        format!("legacy migration {file_name} must be named NNNN_description.sql")
    })?;
    let version = version
        .parse::<i64>()
        .map_err(|error| format!("invalid migration version in {file_name}: {error}"))?;
    let description = rest.replace('_', " ");
    Ok((version, description, stem.to_string()))
}

fn normalize_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn push_payload(bytes: &[u8], payload_bytes: &mut Vec<u8>) -> PayloadSlice {
    let start = payload_bytes.len() as u64;
    payload_bytes.extend_from_slice(bytes);
    PayloadSlice {
        start,
        len: bytes.len() as u64,
    }
}

fn validate_contiguous_versions(migrations: &[CompiledMigration]) -> Result<(), String> {
    for (index, migration) in migrations.iter().enumerate() {
        let expected = index as i64 + 1;
        if migration.version != expected {
            return Err(format!(
                "migration versions must be contiguous from 0001; expected {expected:04}, found {:04}",
                migration.version
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_db_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scryer/src/db")
    }

    fn source_postgres_0140_baseline_sql() -> String {
        fs::read_to_string(source_db_root().join("postgres/baselines/0140_baseline.sql"))
            .expect("read PostgreSQL 0140 baseline")
    }

    #[test]
    fn source_bundle_registers_latest_migration_and_engine_baselines() {
        let bundle =
            compile_source_bundle(&source_db_root()).expect("compile source migration bundle");
        assert!(
            bundle.catalog.find_migration(198).is_some(),
            "migration 0198 must be registered in migration_manifest.toml"
        );
        assert!(
            bundle
                .catalog
                .latest_baseline_at_or_below(140, EngineScope::Sqlite)
                .is_some_and(|baseline| baseline.file == "baselines/0140_baseline.sql"),
            "SQLite should register the latest manifest-owned baseline"
        );
        assert!(
            bundle
                .catalog
                .latest_baseline_at_or_below(140, EngineScope::Postgres)
                .is_some_and(|baseline| baseline.file == "postgres/baselines/0140_baseline.sql"),
            "PostgreSQL should register the latest manifest-owned baseline"
        );
        assert!(
            bundle
                .catalog
                .latest_baseline_at_or_below(198, EngineScope::Sqlite)
                .is_some_and(|baseline| baseline.file == "baselines/0198_baseline.sql"),
            "SQLite should register the 0198 baseline"
        );
        assert!(
            bundle
                .catalog
                .latest_baseline_at_or_below(198, EngineScope::Postgres)
                .is_some_and(|baseline| baseline.file == "postgres/baselines/0198_baseline.sql"),
            "PostgreSQL should register the 0198 baseline"
        );
    }

    #[test]
    fn postgres_0140_baseline_keeps_expected_index_coverage() {
        let sql = source_postgres_0140_baseline_sql();
        let index_statement_count = sql
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("CREATE INDEX ") || trimmed.starts_with("CREATE UNIQUE INDEX ")
            })
            .count();
        assert_eq!(
            index_statement_count, 190,
            "PostgreSQL 0140 baseline should preserve the audited index set"
        );

        for index_name in [
            "idx_titles_facet_normalized_slug",
            "idx_pending_releases_wanted",
            "idx_domain_events_stream_sequence",
            "idx_download_queue_commands_source",
            "idx_external_subtitle_probe_cache_file_path",
            "idx_history_events_title_time",
            "idx_notification_subscriptions_target_scope",
            "idx_release_download_attempts_outcome_attempted",
            "idx_subtitle_provider_configs_provider_type",
            "idx_wanted_items_next_search",
            "idx_workflow_operations_job_key_status",
        ] {
            assert!(
                sql.contains(index_name),
                "expected PostgreSQL 0140 baseline to include {index_name}"
            );
        }
    }

    #[test]
    fn postgres_0198_keeps_sqlite_builtin_seed_parity_without_rewriting_the_baseline() {
        let postgres = fs::read_to_string(
            source_db_root().join("postgres/migrations/0198_seed_canonical_defaults.sql"),
        )
        .expect("PostgreSQL 0198 seed migration should be readable");
        let sqlite = fs::read_to_string(source_db_root().join("baselines/0140_baseline.sql"))
            .expect("SQLite 0140 baseline should be readable");

        for seed in [
            "anime_default_library",
            "movie_default_library",
            "series_default_library",
            "canonical_root_for_anime_default_library",
            "canonical_root_for_movie_default_library",
            "canonical_root_for_series_default_library",
            "('1080p', '1080P', 0, '1970-01-01T00:00:00Z')",
            "('1080p', '720P', 1, '1970-01-01T00:00:00Z')",
            "('4k', '1080P', 1, '1970-01-01T00:00:00Z')",
            "('4k', '2160P', 0, '1970-01-01T00:00:00Z')",
            "('4k', '720P', 2, '1970-01-01T00:00:00Z')",
            "('1080p', '1080P', 'system'",
            "('4k', '4K', 'system'",
            "00000000000000000000000000000001",
        ] {
            assert!(sqlite.contains(seed), "SQLite baseline missing seed {seed}");
            assert!(
                postgres.contains(seed),
                "PostgreSQL baseline missing seed {seed}"
            );
        }
        for guard in [
            "INNER JOIN libraries parent",
            "parent.id = roots.library_id",
            "parent.facet = roots.facet",
            "parent.slug = roots.slug",
            "ON CONFLICT DO NOTHING",
        ] {
            assert!(
                postgres.contains(guard),
                "PostgreSQL 0198 must guard canonical roots with `{guard}`"
            );
        }
    }

    #[test]
    fn released_0_18_21_sql_assets_are_immutable() {
        let root = source_db_root();
        let mut paths = vec![
            root.join("baselines/0140_baseline.sql"),
            root.join("postgres/baselines/0140_baseline.sql"),
        ];
        for relative_dir in ["migrations", "postgres/migrations"] {
            for entry in fs::read_dir(root.join(relative_dir)).expect("migration directory") {
                let path = entry.expect("migration entry").path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let version = name
                    .get(..4)
                    .and_then(|version| version.parse::<u32>().ok());
                if version.is_some_and(|version| version <= 173) {
                    paths.push(path);
                }
            }
        }
        paths.sort_by_key(|path| {
            path.strip_prefix(&root)
                .expect("asset should be under db root")
                .to_string_lossy()
                .replace('\\', "/")
        });

        let mut hasher = Blake3Hasher::new();
        for path in paths {
            let relative = path
                .strip_prefix(&root)
                .expect("asset should be under db root")
                .to_string_lossy()
                .replace('\\', "/");
            hasher.update(relative.as_bytes());
            hasher.update(&[0]);
            hasher.update(&fs::read(&path).expect("released SQL asset should be readable"));
            hasher.update(&[0]);
        }
        assert_eq!(
            hasher.finalize().to_hex().as_str(),
            "57b821be60b4e6cab89d76ad2961d05288cca5bd19e8314c2083eb4f66a43f58"
        );
    }

    #[test]
    fn migration_0197_converges_binary_event_storage_without_the_stream_index() {
        let root = source_db_root();
        let sqlite = fs::read_to_string(root.join("migrations/0197_compact_event_storage_pre.sql"))
            .expect("SQLite 0197 pre-migration should be readable");
        let postgres =
            fs::read_to_string(root.join("postgres/migrations/0197_compact_event_storage.sql"))
                .expect("PostgreSQL 0197 migration should be readable");

        assert!(sqlite.contains("payload_json BLOB NOT NULL"));
        assert!(sqlite.contains("explanation_json BLOB"));
        assert!(postgres.contains("ALTER COLUMN payload_json TYPE bytea"));
        assert!(postgres.contains("ALTER COLUMN explanation_json TYPE bytea"));
        for sql in [&sqlite, &postgres] {
            assert!(sql.contains("DROP INDEX IF EXISTS idx_domain_events_stream_sequence"));
            assert!(sql.contains("import_status"));
            assert!(sql.contains("media_file_delete_reason"));
            assert!(sql.contains("download_id"));
        }
    }

    #[test]
    fn migration_0154_declares_mapping_and_legacy_indexer_columns_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 154"));

        for relative_path in [
            "migrations/0154_indexer_download_client_mapping.sql",
            "postgres/migrations/0154_indexer_download_client_mapping.sql",
        ] {
            let sql = fs::read_to_string(db_root.join(relative_path))
                .expect("0154 migration should be readable");
            assert!(sql.contains("download_client_id"));
            assert!(sql.contains("idx_indexers_download_client_id"));
            assert!(sql.contains("idx_pending_releases_indexer_id"));
            assert!(sql.contains("ON DELETE SET NULL"));
            assert!(sql.contains("pending_releases"));
            assert!(sql.contains("indexer_id"));
            assert!(sql.contains("indexer_source"));
            assert!(sql.contains("COUNT(*)") && sql.contains("= 1"));
        }

        let sqlite_sql =
            fs::read_to_string(db_root.join("migrations/0154_indexer_download_client_mapping.sql"))
                .expect("SQLite 0154 migration should be readable");
        let pending_release_sql = sqlite_sql
            .split("ALTER TABLE pending_releases")
            .nth(1)
            .expect("pending release migration section");
        assert!(!pending_release_sql.contains("REFERENCES download_clients"));
    }

    #[test]
    fn migration_0155_widens_external_account_provider_constraint_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 155"));

        let sqlite_sql = fs::read_to_string(db_root.join("migrations/0155_emby_first_class.sql"))
            .expect("SQLite 0155 migration should be readable");
        assert!(sqlite_sql.contains("CHECK (provider IN ('plex', 'jellyfin', 'emby'))"));
        assert!(sqlite_sql.contains("FROM user_external_accounts"));
        assert!(sqlite_sql.contains("DROP TABLE user_external_accounts"));
        for index in [
            "idx_user_external_accounts_pending_username",
            "idx_user_external_accounts_provider_identity",
            "idx_user_external_accounts_user_provider_connection",
            "idx_user_external_accounts_user_status",
        ] {
            assert!(
                sqlite_sql.contains(index),
                "SQLite 0155 must restore {index}"
            );
        }

        let postgres_sql =
            fs::read_to_string(db_root.join("postgres/migrations/0155_emby_first_class.sql"))
                .expect("PostgreSQL 0155 migration should be readable");
        assert!(postgres_sql.contains("DROP CONSTRAINT user_external_accounts_provider_check"));
        assert!(postgres_sql.contains("ADD CONSTRAINT user_external_accounts_provider_check"));
        assert!(postgres_sql.contains("CHECK (provider IN ('plex', 'jellyfin', 'emby'))"));
    }

    #[test]
    fn migration_0162_adds_unmatched_size_bytes_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 162"));
        assert!(manifest.contains("migrations/0162_library_scan_unmatched_size_bytes.sql"));
        assert!(
            manifest.contains("postgres/migrations/0162_library_scan_unmatched_size_bytes.sql")
        );

        let sqlite_sql = fs::read_to_string(
            db_root.join("migrations/0162_library_scan_unmatched_size_bytes.sql"),
        )
        .expect("SQLite 0162 migration should be readable");
        assert!(sqlite_sql.contains("ALTER TABLE library_scan_unmatched_items"));
        assert!(sqlite_sql.contains("ADD COLUMN size_bytes INTEGER"));
        // The column must stay nullable: rows recorded before this migration
        // have no size, and readers fall back to a filesystem stat.
        assert!(!sqlite_sql.contains("NOT NULL"));

        let postgres_sql = fs::read_to_string(
            db_root.join("postgres/migrations/0162_library_scan_unmatched_size_bytes.sql"),
        )
        .expect("PostgreSQL 0162 migration should be readable");
        assert!(postgres_sql.contains("ALTER TABLE library_scan_unmatched_items"));
        assert!(postgres_sql.contains("ADD COLUMN size_bytes bigint"));
        assert!(!postgres_sql.contains("NOT NULL"));

        let bundle = compile_source_bundle(&db_root).expect("compile source migration bundle");
        assert!(
            bundle.catalog.find_migration(161).is_some(),
            "migration 0161 must be registered in migration_manifest.toml"
        );
    }

    #[test]
    fn postgres_0140_baseline_keeps_title_aware_unmatched_items_schema() {
        let sql = source_postgres_0140_baseline_sql();
        assert!(
            sql.contains("title_id text"),
            "PostgreSQL 0140 baseline must include library_scan_unmatched_items.title_id"
        );
        assert!(
            sql.contains("idx_library_scan_unmatched_items_facet_title_status_updated"),
            "PostgreSQL 0140 baseline must preserve the title-aware unmatched-items index"
        );
    }

    #[test]
    fn postgres_0140_baseline_keeps_runtime_title_image_columns() {
        let sql = source_postgres_0140_baseline_sql();
        for expected in [
            "poster_local_path text",
            "background_local_path text",
            "CREATE TABLE title_images",
            "CREATE TABLE title_image_variants",
        ] {
            assert!(
                sql.contains(expected),
                "expected PostgreSQL 0140 baseline to include {expected}"
            );
        }
    }

    #[test]
    fn explicit_migrations_after_postgres_baseline_treat_both_engines() {
        let bundle =
            compile_source_bundle(&source_db_root()).expect("compile source migration bundle");
        for migration in bundle
            .catalog
            .migrations
            .iter()
            .filter(|migration| migration.version > 140)
            .filter(|migration| {
                migration
                    .steps
                    .iter()
                    .any(|step| step.engine().applies_to(EngineScope::Postgres))
            })
        {
            let treats_sqlite = migration
                .steps
                .iter()
                .any(|step| step.engine().applies_to(EngineScope::Sqlite));
            let treats_postgres = migration
                .steps
                .iter()
                .any(|step| step.engine().applies_to(EngineScope::Postgres));
            assert!(
                treats_sqlite && treats_postgres,
                "migration {} must explicitly treat both sqlite and postgres after the 0140 postgres baseline",
                migration.key
            );
        }
    }

    #[test]
    fn migration_0177_requeues_legacy_movie_hydration_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 177"));

        let sqlite_sql =
            fs::read_to_string(db_root.join("migrations/0177_movie_smg_identity_backfill.sql"))
                .expect("SQLite 0177 migration should be readable");
        assert!(sqlite_sql.contains("facet = 'movie'"));
        assert!(sqlite_sql.contains("('tmdb', 'imdb')"));
        assert!(sqlite_sql.contains("= 'tvdb'"));
        assert!(sqlite_sql.contains("metadata_hydration_attempt_count = 0"));

        let postgres_sql = fs::read_to_string(
            db_root.join("postgres/migrations/0177_movie_smg_identity_backfill.sql"),
        )
        .expect("PostgreSQL 0177 migration should be readable");
        assert!(postgres_sql.contains("NOW()"));
        assert!(postgres_sql.contains("jsonb_array_elements"));
        assert!(postgres_sql.contains("('tmdb', 'imdb')"));
        assert!(postgres_sql.contains("= 'tvdb'"));
    }

    #[test]
    fn migration_0200_queues_supported_movie_identities_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 200"));

        for relative_path in [
            "migrations/0200_movie_supported_identity_hydration.sql",
            "postgres/migrations/0200_movie_supported_identity_hydration.sql",
        ] {
            let sql = fs::read_to_string(db_root.join(relative_path))
                .expect("0200 migration should be readable");
            assert!(sql.contains("facet = 'movie'"));
            assert!(sql.contains("('smg', 'tvdb', 'tmdb', 'imdb')"));
            assert!(sql.contains("metadata_fetched_at IS NULL"));
            assert!(sql.contains("metadata_hydration_next_attempt_at IS NULL"));
            assert!(sql.contains("metadata_hydration_attempt_count = 0"));
        }
    }

    #[test]
    fn migration_0183_adds_manual_import_canonical_identity_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 183"));
        assert!(manifest.contains("migrations/0183_manual_import_selection_durable_identity.sql"));
        assert!(
            manifest
                .contains("postgres/migrations/0183_manual_import_selection_durable_identity.sql")
        );

        for relative_path in [
            "migrations/0183_manual_import_selection_durable_identity.sql",
            "postgres/migrations/0183_manual_import_selection_durable_identity.sql",
        ] {
            let sql = fs::read_to_string(db_root.join(relative_path))
                .expect("0183 migration should be readable");
            assert!(sql.contains("ALTER TABLE manual_import_selections"));
            assert!(sql.contains("ADD COLUMN canonical_download_id"));
            assert!(sql.contains("idx_manual_import_selections_canonical_download"));
        }
    }

    #[test]
    fn migration_0186_drops_the_legacy_token_requirement_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 186"));
        assert!(manifest.contains("migrations/0186_download_identity_states_token_optional.sql"));
        assert!(
            manifest
                .contains("postgres/migrations/0186_download_identity_states_token_optional.sql")
        );

        let sqlite_sql = fs::read_to_string(
            db_root.join("migrations/0186_download_identity_states_token_optional.sql"),
        )
        .expect("SQLite 0186 migration should be readable");
        // The rebuilt table keeps the canonical column mandatory and the legacy
        // token optional, and picks up the downloads(id) foreign key the other
        // canonical dependents already carry.
        assert!(sqlite_sql.contains("canonical_download_id TEXT NOT NULL"));
        assert!(!sqlite_sql.contains("CHECK (download_id IS NOT NULL)"));
        assert!(
            sqlite_sql.contains("FOREIGN KEY (canonical_download_id) REFERENCES downloads(id)")
        );
        assert!(sqlite_sql.contains("idx_download_identity_states_canonical_download_id"));

        let postgres_sql = fs::read_to_string(
            db_root.join("postgres/migrations/0186_download_identity_states_token_optional.sql"),
        )
        .expect("PostgreSQL 0186 migration should be readable");
        assert!(
            postgres_sql
                .contains("DROP CONSTRAINT IF EXISTS download_identity_states_download_id_check")
        );
        assert!(
            postgres_sql
                .contains("ADD CONSTRAINT download_identity_states_canonical_download_id_fkey")
        );
        assert!(postgres_sql.contains("idx_download_identity_states_canonical_download_id"));
    }

    #[tokio::test]
    async fn migration_0188_classifies_only_unreasoned_legacy_import_blocks() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 188"));

        let sqlite_sql = fs::read_to_string(
            db_root.join("migrations/0188_unverified_already_imported_blocks.sql"),
        )
        .expect("SQLite 0188 migration should be readable");
        let postgres_sql = fs::read_to_string(
            db_root.join("postgres/migrations/0188_unverified_already_imported_blocks.sql"),
        )
        .expect("PostgreSQL 0188 migration should be readable");
        assert!(postgres_sql.contains("btrim(reason)"));
        assert!(postgres_sql.contains("unverified_already_imported"));

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open SQLite database");
        sqlx::raw_sql(
            "CREATE TABLE download_identity_states (
                id TEXT PRIMARY KEY,
                tracked_state TEXT NOT NULL,
                reason TEXT,
                detail TEXT
            );",
        )
        .execute(&pool)
        .await
        .expect("create download identity states table");

        for (id, state, reason, detail) in [
            (
                "legacy-null",
                "import_blocked",
                None,
                Some(" Import blocked: ALREADY_IMPORTED "),
            ),
            (
                "legacy-blank",
                "import_blocked",
                Some("   "),
                Some("Import blocked: already_imported"),
            ),
            (
                "post-import",
                "import_blocked",
                Some("import_blocked_after_import"),
                Some("Import blocked: already_imported"),
            ),
            (
                "other-state",
                "imported",
                None,
                Some("Import blocked: already_imported"),
            ),
            (
                "other-detail",
                "import_blocked",
                None,
                Some("manual review"),
            ),
        ] {
            sqlx::query(
                "INSERT INTO download_identity_states (id, tracked_state, reason, detail)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(id)
            .bind(state)
            .bind(reason)
            .bind(detail)
            .execute(&pool)
            .await
            .expect("seed migration row");
        }

        sqlx::raw_sql(sqlx::AssertSqlSafe(sqlite_sql))
            .execute(&pool)
            .await
            .expect("run 0188 migration");

        for id in ["legacy-null", "legacy-blank"] {
            let reason: Option<String> =
                sqlx::query_scalar("SELECT reason FROM download_identity_states WHERE id = ?")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .expect("read migrated reason");
            assert_eq!(reason.as_deref(), Some("unverified_already_imported"));
        }

        for (id, expected_reason) in [
            ("post-import", Some("import_blocked_after_import")),
            ("other-state", None),
            ("other-detail", None),
        ] {
            let reason: Option<String> =
                sqlx::query_scalar("SELECT reason FROM download_identity_states WHERE id = ?")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .expect("read untouched reason");
            assert_eq!(reason.as_deref(), expected_reason);
        }
    }

    #[test]
    fn migration_0195_creates_application_migration_ledgers_for_both_engines() {
        let db_root = source_db_root();
        let manifest = fs::read_to_string(db_root.join("migration_manifest.toml"))
            .expect("migration manifest should be readable");
        assert!(manifest.contains("version = 195"));

        for relative_path in [
            "migrations/0195_application_migrations.sql",
            "postgres/migrations/0195_application_migrations.sql",
        ] {
            assert!(manifest.contains(relative_path));
            let sql = fs::read_to_string(db_root.join(relative_path))
                .expect("0195 migration should be readable");
            assert!(sql.contains("CREATE TABLE application_migrations"));
            assert!(sql.contains("migration_id TEXT PRIMARY KEY"));
        }
    }

    #[test]
    fn source_manifest_defaults_missing_step_engine_to_all() {
        let manifest = r#"
format_version = 1

[legacy_sql]
path = "migrations"
through_version = 0

[[migration]]
version = 1
description = "missing engine"

[[migration.steps]]
kind = "sql"
file = "0001_missing_engine.sql"
"#;
        let parsed = toml::from_str::<SourceMigrationManifest>(manifest)
            .expect("manifest without step engine should default to all");
        match &parsed.migrations[0].steps[0] {
            SourceMigrationStep::Sql { engine, scope, .. } => {
                assert_eq!(*engine, None);
                assert_eq!(*scope, StepScope::All);
                assert_eq!(
                    parsed.migrations[0].steps[0].resolved_engine(),
                    EngineScope::All
                );
            }
            other => panic!("expected sql step, got {other:?}"),
        }
    }
}
