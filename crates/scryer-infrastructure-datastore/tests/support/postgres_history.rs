use scryer_application::{AppError, AppResult};
use sqlx::PgPool;

use datastore::migration_assets::{CompiledBaseline, EngineScope, PayloadSlice};

// Exact archived bytes from commit df249815c52750e75f0f8a0ef41817e16c5f1704,
// crates/scryer/src/db/postgres/baselines/0140_baseline.sql.
// Git blob: ac9b51f880bae94ebc0a8b46b49e0e896639fb48.
// SHA-256: 2ff46cd9dd7f9af06d573e6df7e79ffb82dfb976fde0a40b322efe5f77967e80.
// This snapshot is test data, never a registered production baseline.
const ARCHIVED_BASELINE: &[u8] = include_bytes!("../fixtures/postgres_0140_baseline.sql");

/// Builds a historical fixture through the real migration runner. The current
/// catalog supplies every migration/checksum/hook; only this test's baseline
/// payload is added. Normal service startup then exercises the real upgrade path.
pub(crate) async fn replay_historical_postgres(
    pool: &PgPool,
    through_version: i64,
) -> AppResult<()> {
    if !(140..254).contains(&through_version) {
        return Err(AppError::Repository(format!(
            "historical PostgreSQL fixture target {through_version} must be between 140 and 253"
        )));
    }
    let mut bundle = datastore::migrations::load_source_migration_catalog()?;
    let payload = PayloadSlice {
        start: u64::try_from(bundle.payload_bytes.len()).expect("test payload offset fits u64"),
        len: u64::try_from(ARCHIVED_BASELINE.len()).expect("test baseline length fits u64"),
    };
    bundle.payload_bytes.extend_from_slice(ARCHIVED_BASELINE);
    bundle.catalog.baselines.push(CompiledBaseline {
        through_version: 140,
        file: "test-fixtures/postgres_0140_baseline.sql".to_string(),
        engine: EngineScope::Postgres,
        payload,
    });
    datastore::postgres::replay_catalog_into_fresh_db(
        pool,
        &bundle.catalog,
        &bundle.payload_bytes,
        Some(through_version),
    )
    .await
}
