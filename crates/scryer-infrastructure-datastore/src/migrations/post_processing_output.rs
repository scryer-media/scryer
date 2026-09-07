//! Migration 0210: post-processing script output tails become zstd frames.
//!
//! SQLite rebuilds the table (the `_pre` SQL renamed the old one aside), so
//! this hook copies every legacy row into the new table while compressing the
//! tails. PostgreSQL retyped the columns in place, so this hook rewrites the
//! populated tails of each row.

use scryer_application::{AppError, AppResult};
use scryer_infrastructure_sql::script_output::encode_script_output_tail;
use sqlx::Row;

const BATCH_SIZE: i64 = 500;

pub async fn compress_post_processing_output_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let mut last_id = String::new();
    loop {
        let rows = sqlx::query(
            "SELECT id, script_id, script_name, title_id, title_name, facet, file_path,
                    status, exit_code, stdout_tail, stderr_tail, duration_ms,
                    env_payload_json, started_at, completed_at
               FROM post_processing_script_runs_legacy_0210
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
            let stdout_tail = encode_legacy_tail(
                &id,
                "stdout_tail",
                row.try_get::<Option<String>, _>("stdout_tail")
                    .map_err(repo_error)?,
            )?;
            let stderr_tail = encode_legacy_tail(
                &id,
                "stderr_tail",
                row.try_get::<Option<String>, _>("stderr_tail")
                    .map_err(repo_error)?,
            )?;
            sqlx::query(
                "INSERT INTO post_processing_script_runs (
                    id, script_id, script_name, title_id, title_name, facet, file_path,
                    status, exit_code, stdout_tail, stderr_tail, duration_ms,
                    env_payload_json, started_at, completed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )
            .bind(&id)
            .bind(row.try_get::<String, _>("script_id").map_err(repo_error)?)
            .bind(
                row.try_get::<String, _>("script_name")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<String>, _>("title_id")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<String>, _>("title_name")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<String>, _>("facet")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<String>, _>("file_path")
                    .map_err(repo_error)?,
            )
            .bind(row.try_get::<String, _>("status").map_err(repo_error)?)
            .bind(
                row.try_get::<Option<i64>, _>("exit_code")
                    .map_err(repo_error)?,
            )
            .bind(stdout_tail)
            .bind(stderr_tail)
            .bind(
                row.try_get::<Option<i64>, _>("duration_ms")
                    .map_err(repo_error)?,
            )
            .bind(
                row.try_get::<Option<String>, _>("env_payload_json")
                    .map_err(repo_error)?,
            )
            .bind(row.try_get::<String, _>("started_at").map_err(repo_error)?)
            .bind(
                row.try_get::<Option<String>, _>("completed_at")
                    .map_err(repo_error)?,
            )
            .execute(&mut **tx)
            .await
            .map_err(repo_error)?;
            last_id = id;
        }
    }
    Ok(())
}

pub async fn compress_post_processing_output_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let mut last_id = String::new();
    loop {
        let rows = sqlx::query(
            "SELECT id, stdout_tail, stderr_tail
               FROM post_processing_script_runs
              WHERE id > $1
                AND (stdout_tail IS NOT NULL OR stderr_tail IS NOT NULL)
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
            // The SQL step retyped the columns with the legacy text bytes intact.
            let stdout_tail = encode_legacy_tail(
                &id,
                "stdout_tail",
                row.try_get::<Option<Vec<u8>>, _>("stdout_tail")
                    .map_err(repo_error)?
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            )?;
            let stderr_tail = encode_legacy_tail(
                &id,
                "stderr_tail",
                row.try_get::<Option<Vec<u8>>, _>("stderr_tail")
                    .map_err(repo_error)?
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            )?;
            sqlx::query(
                "UPDATE post_processing_script_runs
                    SET stdout_tail = $1, stderr_tail = $2
                  WHERE id = $3",
            )
            .bind(stdout_tail)
            .bind(stderr_tail)
            .bind(&id)
            .execute(&mut **tx)
            .await
            .map_err(repo_error)?;
            last_id = id;
        }
    }
    Ok(())
}

fn encode_legacy_tail(
    run_id: &str,
    column: &str,
    text: Option<String>,
) -> AppResult<Option<Vec<u8>>> {
    text.as_deref()
        .map(|text| {
            encode_script_output_tail(text).map_err(|error| {
                AppError::Repository(format!(
                    "failed to compress {column} of post-processing run {run_id}: {error}"
                ))
            })
        })
        .transpose()
}

fn repo_error(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use scryer_infrastructure_sql::script_output::decode_script_output_tail;
    use sqlx::{Row, sqlite::SqlitePoolOptions};

    use super::*;

    #[tokio::test]
    async fn sqlite_migration_copies_rows_and_compresses_tails() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE post_processing_script_runs_legacy_0210 (
                id TEXT PRIMARY KEY, script_id TEXT NOT NULL, script_name TEXT NOT NULL,
                title_id TEXT, title_name TEXT, facet TEXT, file_path TEXT,
                status TEXT NOT NULL, exit_code INTEGER, stdout_tail TEXT, stderr_tail TEXT,
                duration_ms INTEGER, env_payload_json TEXT, started_at TEXT NOT NULL,
                completed_at TEXT
             );
             CREATE TABLE post_processing_script_runs (
                id TEXT PRIMARY KEY, script_id TEXT NOT NULL, script_name TEXT NOT NULL,
                title_id TEXT, title_name TEXT, facet TEXT, file_path TEXT,
                status TEXT NOT NULL, exit_code INTEGER, stdout_tail BLOB, stderr_tail BLOB,
                duration_ms INTEGER, env_payload_json TEXT, started_at TEXT NOT NULL,
                completed_at TEXT
             );",
        )
        .execute(&pool)
        .await
        .unwrap();

        let long_output = "script line\n".repeat(400);
        for (id, stdout_tail, stderr_tail, exit_code) in [
            (
                "run-a",
                Some(long_output.as_str()),
                Some("warning: x"),
                Some(0),
            ),
            ("run-b", None, None, None),
        ] {
            sqlx::query(
                "INSERT INTO post_processing_script_runs_legacy_0210 (
                    id, script_id, script_name, title_id, facet, status, exit_code,
                    stdout_tail, stderr_tail, duration_ms, env_payload_json, started_at,
                    completed_at
                 ) VALUES (?1, 'script-1', 'Notify', 'title-1', 'movie', 'success', ?2,
                           ?3, ?4, 12, '{\"k\":1}', '2026-09-05T00:00:00Z',
                           '2026-09-05T00:00:01Z')",
            )
            .bind(id)
            .bind(exit_code)
            .bind(stdout_tail)
            .bind(stderr_tail)
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut tx = pool.begin().await.unwrap();
        compress_post_processing_output_sqlite(&mut tx)
            .await
            .unwrap();
        let rows = sqlx::query(
            "SELECT id, script_id, title_id, facet, status, exit_code, stdout_tail,
                    stderr_tail, duration_ms, env_payload_json, started_at, completed_at
               FROM post_processing_script_runs
              ORDER BY id",
        )
        .fetch_all(&mut *tx)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);

        let stdout: Option<Vec<u8>> = rows[0].try_get("stdout_tail").unwrap();
        let stdout = stdout.expect("run-a stdout survives");
        assert!(stdout.len() < long_output.len() / 4);
        assert_eq!(decode_script_output_tail(&stdout).unwrap(), long_output);
        let stderr: Option<Vec<u8>> = rows[0].try_get("stderr_tail").unwrap();
        assert_eq!(
            decode_script_output_tail(&stderr.unwrap()).unwrap(),
            "warning: x"
        );
        assert_eq!(
            rows[0].try_get::<String, _>("script_id").unwrap(),
            "script-1"
        );
        assert_eq!(
            rows[0].try_get::<Option<String>, _>("title_id").unwrap(),
            Some("title-1".to_string())
        );
        assert_eq!(
            rows[0].try_get::<Option<i64>, _>("exit_code").unwrap(),
            Some(0)
        );
        assert_eq!(
            rows[0].try_get::<Option<i64>, _>("duration_ms").unwrap(),
            Some(12)
        );
        assert_eq!(
            rows[0]
                .try_get::<Option<String>, _>("env_payload_json")
                .unwrap(),
            Some("{\"k\":1}".to_string())
        );
        assert_eq!(
            rows[0]
                .try_get::<Option<String>, _>("completed_at")
                .unwrap(),
            Some("2026-09-05T00:00:01Z".to_string())
        );

        assert_eq!(rows[1].try_get::<String, _>("id").unwrap(), "run-b");
        assert_eq!(
            rows[1]
                .try_get::<Option<Vec<u8>>, _>("stdout_tail")
                .unwrap(),
            None
        );
        assert_eq!(
            rows[1]
                .try_get::<Option<Vec<u8>>, _>("stderr_tail")
                .unwrap(),
            None
        );
        assert_eq!(
            rows[1].try_get::<Option<i64>, _>("exit_code").unwrap(),
            None
        );
    }
}
