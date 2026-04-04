use scryer_application::{AppError, AppResult};
use sqlx::{Row, SqlitePool};

pub(crate) async fn delete_release_decisions_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query("DELETE FROM release_decisions WHERE created_at < datetime('now', ?)")
        .bind(&modifier)
        .execute(pool)
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "housekeeping: release_decisions cleanup failed: {e}"
            ))
        })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_release_attempts_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query(
        "DELETE FROM release_download_attempts WHERE created_at < datetime('now', ?) AND outcome != 'pending'",
    )
    .bind(&modifier)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(format!("housekeeping: release_attempts cleanup failed: {e}")))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_dispatched_event_outboxes_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query(
        "DELETE FROM event_outboxes WHERE status = 'dispatched' AND created_at < datetime('now', ?)",
    )
    .bind(&modifier)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(format!("housekeeping: event_outboxes cleanup failed: {e}")))?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_history_events_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query("DELETE FROM history_events WHERE created_at < datetime('now', ?)")
        .bind(&modifier)
        .execute(pool)
        .await
        .map_err(|e| {
            AppError::Repository(format!("housekeeping: history_events cleanup failed: {e}"))
        })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn delete_domain_events_older_than_query(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<u32> {
    let modifier = format!("-{days} days");
    let result = sqlx::query("DELETE FROM domain_events WHERE occurred_at < datetime('now', ?)")
        .bind(&modifier)
        .execute(pool)
        .await
        .map_err(|e| {
            AppError::Repository(format!("housekeeping: domain_events cleanup failed: {e}"))
        })?;

    Ok(result.rows_affected() as u32)
}

pub(crate) async fn list_all_media_file_paths_query(
    pool: &SqlitePool,
) -> AppResult<Vec<(String, String)>> {
    let rows = sqlx::query("SELECT id, file_path FROM media_files")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Repository(format!("housekeeping: list media files failed: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: String = row.get("id");
        let file_path: String = row.get("file_path");
        out.push((id, file_path));
    }
    Ok(out)
}

pub(crate) async fn delete_media_files_by_ids_query(
    pool: &SqlitePool,
    ids: &[String],
) -> AppResult<u32> {
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
    let sql = format!(
        "DELETE FROM media_files WHERE id IN ({})",
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&sql);
    for id in ids {
        query = query.bind(id);
    }

    let result = query.execute(pool).await.map_err(|e| {
        AppError::Repository(format!("housekeeping: delete media files failed: {e}"))
    })?;

    Ok(result.rows_affected() as u32)
}
