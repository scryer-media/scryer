use scryer_application::{AppError, AppResult};
use scryer_infrastructure_sql::domain_event_payload::{
    compact_legacy_domain_event_json, derive_domain_event_projections, encode_domain_event_payload,
};
use serde_json::Value;
use sqlx::Row;

const BATCH_SIZE: i64 = 500;
const RELEASE_EXPLANATION_FORMAT_V1: u8 = 1;
const RELEASE_EXPLANATION_MAX_BYTES: usize = 64 * 1024;
const RELEASE_EXPLANATION_DICTIONARY_V1: &[u8] = include_bytes!(
    "../../../scryer-infrastructure-library/src/media/libraries/state_store/release_decision_explanation_v1.dict"
);

pub async fn compact_event_storage_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let mut last_sequence = i64::MIN;
    loop {
        let rows = sqlx::query(
            "SELECT sequence, event_id, occurred_at, actor_user_id, title_id, facet,
                    correlation_id, causation_id, schema_version, stream_kind, stream_id,
                    event_type, payload_json, actor_kind, actor_display_name
               FROM domain_events_legacy_0197
              WHERE sequence > ?1
              ORDER BY sequence
              LIMIT ?2",
        )
        .bind(last_sequence)
        .bind(BATCH_SIZE)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_error)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let sequence: i64 = row.try_get("sequence").map_err(repo_error)?;
            let event_id: String = row.try_get("event_id").map_err(repo_error)?;
            let event_type: String = row.try_get("event_type").map_err(repo_error)?;
            let legacy: String = row.try_get("payload_json").map_err(repo_error)?;
            let value = compact_legacy_domain_event_json(legacy.as_bytes()).map_err(|error| {
                AppError::Repository(format!(
                    "failed to decode legacy domain event {event_id} ({event_type}): {error}"
                ))
            })?;
            let encoded = encode_domain_event_payload(&value).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode domain event {event_id} ({event_type}): {error}"
                ))
            })?;
            let projections = derive_domain_event_projections(&event_type, &value);
            sqlx::query(
                "INSERT INTO domain_events (
                    sequence, event_id, occurred_at, actor_user_id, title_id, facet,
                    correlation_id, causation_id, schema_version, stream_kind, stream_id,
                    event_type, payload_json, actor_kind, actor_display_name, import_status,
                    media_file_delete_reason, download_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            )
            .bind(sequence)
            .bind(&event_id)
            .bind(row.try_get::<String, _>("occurred_at").map_err(repo_error)?)
            .bind(row.try_get::<Option<String>, _>("actor_user_id").map_err(repo_error)?)
            .bind(row.try_get::<Option<String>, _>("title_id").map_err(repo_error)?)
            .bind(row.try_get::<Option<String>, _>("facet").map_err(repo_error)?)
            .bind(row.try_get::<Option<String>, _>("correlation_id").map_err(repo_error)?)
            .bind(row.try_get::<Option<String>, _>("causation_id").map_err(repo_error)?)
            .bind(row.try_get::<i64, _>("schema_version").map_err(repo_error)?)
            .bind(row.try_get::<String, _>("stream_kind").map_err(repo_error)?)
            .bind(row.try_get::<Option<String>, _>("stream_id").map_err(repo_error)?)
            .bind(&event_type)
            .bind(encoded)
            .bind(row.try_get::<String, _>("actor_kind").map_err(repo_error)?)
            .bind(row.try_get::<String, _>("actor_display_name").map_err(repo_error)?)
            .bind(projections.import_status)
            .bind(projections.media_file_delete_reason)
            .bind(projections.download_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_error)?;
            last_sequence = sequence;
        }
    }

    let mut last_id = String::new();
    loop {
        let rows = sqlx::query(
            "SELECT id, wanted_item_id, title_id, release_title, release_url,
                    release_size_bytes, decision_code, candidate_score, current_score,
                    score_delta, explanation_json, created_at
               FROM release_decisions_legacy_0197
              WHERE id > ?1
              ORDER BY id
              LIMIT ?2",
        )
        .bind(&last_id)
        .bind(BATCH_SIZE)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_error)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id: String = row.try_get("id").map_err(repo_error)?;
            let explanation = row
                .try_get::<Option<String>, _>("explanation_json")
                .map_err(repo_error)?
                .as_deref()
                .and_then(|value| encode_release_explanation_or_null(&id, value.as_bytes()));
            sqlx::query(
                "INSERT INTO release_decisions (
                    id, wanted_item_id, title_id, release_title, release_url,
                    release_size_bytes, decision_code, candidate_score, current_score,
                    score_delta, explanation_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .bind(&id)
            .bind(
                row.try_get::<String, _>("wanted_item_id")
                    .map_err(repo_error)?,
            )
            .bind(row.try_get::<String, _>("title_id").map_err(repo_error)?)
            .bind(
                row.try_get::<String, _>("release_title")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<String>, _>("release_url")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<i64>, _>("release_size_bytes")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<String, _>("decision_code")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<i64, _>("candidate_score")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<i64>, _>("current_score")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<i64>, _>("score_delta")
                    .map_err(repo_error)?,
            )
            .bind(explanation)
            .bind(row.try_get::<String, _>("created_at").map_err(repo_error)?)
            .execute(&mut **tx)
            .await
            .map_err(repo_error)?;
            last_id = id;
        }
    }
    Ok(())
}

pub async fn compact_event_storage_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let mut last_sequence = i64::MIN;
    loop {
        let rows = sqlx::query(
            "SELECT sequence, event_id, event_type, payload_json
               FROM domain_events
              WHERE sequence > $1
              ORDER BY sequence
              LIMIT $2",
        )
        .bind(last_sequence)
        .bind(BATCH_SIZE)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_error)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let sequence: i64 = row.try_get("sequence").map_err(repo_error)?;
            let event_id: String = row.try_get("event_id").map_err(repo_error)?;
            let event_type: String = row.try_get("event_type").map_err(repo_error)?;
            let legacy: Vec<u8> = row.try_get("payload_json").map_err(repo_error)?;
            let value = compact_legacy_domain_event_json(&legacy).map_err(|error| {
                AppError::Repository(format!(
                    "failed to decode legacy domain event {event_id} ({event_type}): {error}"
                ))
            })?;
            let encoded = encode_domain_event_payload(&value).map_err(|error| {
                AppError::Repository(format!(
                    "failed to encode domain event {event_id} ({event_type}): {error}"
                ))
            })?;
            let projections = derive_domain_event_projections(&event_type, &value);
            sqlx::query(
                "UPDATE domain_events
                    SET payload_json = $1, import_status = $2,
                        media_file_delete_reason = $3, download_id = $4
                  WHERE sequence = $5",
            )
            .bind(encoded)
            .bind(projections.import_status)
            .bind(projections.media_file_delete_reason)
            .bind(projections.download_id)
            .bind(sequence)
            .execute(&mut **tx)
            .await
            .map_err(repo_error)?;
            last_sequence = sequence;
        }
    }

    let mut last_id = String::new();
    loop {
        let rows = sqlx::query(
            "SELECT id, explanation_json
               FROM release_decisions
              WHERE id > $1
              ORDER BY id
              LIMIT $2",
        )
        .bind(&last_id)
        .bind(BATCH_SIZE)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_error)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id: String = row.try_get("id").map_err(repo_error)?;
            let explanation = row
                .try_get::<Option<Vec<u8>>, _>("explanation_json")
                .map_err(repo_error)?
                .as_deref()
                .and_then(|value| encode_release_explanation_or_null(&id, value));
            sqlx::query("UPDATE release_decisions SET explanation_json = $1 WHERE id = $2")
                .bind(explanation)
                .bind(&id)
                .execute(&mut **tx)
                .await
                .map_err(repo_error)?;
            last_id = id;
        }
    }
    Ok(())
}

fn encode_release_explanation(legacy: &[u8]) -> Result<Vec<u8>, String> {
    let value: Value = serde_json::from_slice(legacy)
        .map_err(|error| format!("invalid legacy explanation JSON: {error}"))?;
    let compact = serde_json::to_vec(&value)
        .map_err(|error| format!("failed to compact explanation JSON: {error}"))?;
    if compact.len() > RELEASE_EXPLANATION_MAX_BYTES {
        return Err(format!(
            "explanation expands to {} bytes, exceeding the {}-byte limit",
            compact.len(),
            RELEASE_EXPLANATION_MAX_BYTES
        ));
    }
    let mut compressor =
        zstd::bulk::Compressor::with_dictionary(3, RELEASE_EXPLANATION_DICTIONARY_V1)
            .map_err(|error| format!("failed to initialize zstd: {error}"))?;
    let compressed = compressor
        .compress(&compact)
        .map_err(|error| format!("failed to compress explanation JSON: {error}"))?;
    let mut encoded = Vec::with_capacity(compressed.len() + 1);
    encoded.push(RELEASE_EXPLANATION_FORMAT_V1);
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
}

fn encode_release_explanation_or_null(id: &str, legacy: &[u8]) -> Option<Vec<u8>> {
    match encode_release_explanation(legacy) {
        Ok(encoded) => Some(encoded),
        Err(error) => {
            tracing::warn!(
                decision_id = id,
                error = %error,
                "discarding invalid release-decision explanation during event-storage migration"
            );
            None
        }
    }
}

fn repo_error(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use sqlx::{Row, sqlite::SqlitePoolOptions};

    use super::*;

    #[tokio::test]
    async fn sqlite_migration_keeps_decisions_and_nulls_invalid_explanations() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE domain_events_legacy_0197 (
                sequence INTEGER PRIMARY KEY, event_id TEXT NOT NULL, occurred_at TEXT NOT NULL,
                actor_user_id TEXT, title_id TEXT, facet TEXT, correlation_id TEXT,
                causation_id TEXT, schema_version INTEGER NOT NULL, stream_kind TEXT NOT NULL,
                stream_id TEXT, event_type TEXT NOT NULL, payload_json TEXT NOT NULL,
                actor_kind TEXT NOT NULL, actor_display_name TEXT NOT NULL
             );
             CREATE TABLE domain_events (
                sequence INTEGER PRIMARY KEY, event_id TEXT NOT NULL, occurred_at TEXT NOT NULL,
                actor_user_id TEXT, title_id TEXT, facet TEXT, correlation_id TEXT,
                causation_id TEXT, schema_version INTEGER NOT NULL, stream_kind TEXT NOT NULL,
                stream_id TEXT, event_type TEXT NOT NULL, payload_json BLOB NOT NULL,
                actor_kind TEXT NOT NULL, actor_display_name TEXT NOT NULL,
                import_status TEXT, media_file_delete_reason TEXT, download_id TEXT
             );
             CREATE TABLE release_decisions_legacy_0197 (
                id TEXT PRIMARY KEY, wanted_item_id TEXT NOT NULL, title_id TEXT NOT NULL,
                release_title TEXT NOT NULL, release_url TEXT, release_size_bytes INTEGER,
                decision_code TEXT NOT NULL, candidate_score INTEGER NOT NULL,
                current_score INTEGER, score_delta INTEGER, explanation_json TEXT,
                created_at TEXT NOT NULL
             );
             CREATE TABLE release_decisions (
                id TEXT PRIMARY KEY, wanted_item_id TEXT NOT NULL, title_id TEXT NOT NULL,
                release_title TEXT NOT NULL, release_url TEXT, release_size_bytes INTEGER,
                decision_code TEXT NOT NULL, candidate_score INTEGER NOT NULL,
                current_score INTEGER, score_delta INTEGER, explanation_json BLOB,
                created_at TEXT NOT NULL
             );",
        )
        .execute(&pool)
        .await
        .unwrap();

        let oversized = serde_json::to_string(&"x".repeat(RELEASE_EXPLANATION_MAX_BYTES))
            .expect("oversized explanation should serialize");
        for (id, explanation) in [
            ("a-valid", "{\"eligible\":true}".to_string()),
            ("b-invalid", "not-json".to_string()),
            ("c-oversized", oversized),
        ] {
            sqlx::query(
                "INSERT INTO release_decisions_legacy_0197 (
                    id, wanted_item_id, title_id, release_title, decision_code,
                    candidate_score, explanation_json, created_at
                 ) VALUES (?1, 'wanted-1', 'title-1', 'release', 'eligible', 1, ?2,
                           '2026-08-29T00:00:00Z')",
            )
            .bind(id)
            .bind(explanation)
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut tx = pool.begin().await.unwrap();
        compact_event_storage_sqlite(&mut tx).await.unwrap();
        let rows = sqlx::query("SELECT id, explanation_json FROM release_decisions ORDER BY id")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        let valid: Option<Vec<u8>> = rows[0].try_get("explanation_json").unwrap();
        assert_eq!(
            valid.as_deref().and_then(|value| value.first()).copied(),
            Some(1)
        );
        for row in &rows[1..] {
            assert_eq!(
                row.try_get::<Option<Vec<u8>>, _>("explanation_json")
                    .unwrap(),
                None
            );
        }
    }
}
