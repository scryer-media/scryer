use scryer_application::{AppError, AppResult, DownloadSourceKind, PendingRelease};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

pub(crate) async fn insert_pending_release_query(
    pool: &SqlitePool,
    release: &PendingRelease,
) -> AppResult<String> {
    sqlx::query(
        "INSERT INTO pending_releases
         (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
          source_kind, release_score, scoring_log_json, indexer_source, release_guid,
          added_at, delay_until, status, grabbed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&release.id)
    .bind(&release.wanted_item_id)
    .bind(&release.title_id)
    .bind(&release.release_title)
    .bind(&release.release_url)
    .bind(release.release_size_bytes)
    .bind(release.source_kind.map(|value| value.as_str().to_string()))
    .bind(release.release_score)
    .bind(&release.scoring_log_json)
    .bind(&release.indexer_source)
    .bind(&release.release_guid)
    .bind(&release.added_at)
    .bind(&release.delay_until)
    .bind(&release.status)
    .bind(&release.grabbed_at)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(release.id.clone())
}

pub(crate) async fn list_expired_pending_releases_query(
    pool: &SqlitePool,
    now: &str,
) -> AppResult<Vec<PendingRelease>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                source_kind, release_score, scoring_log_json, indexer_source, release_guid,
                added_at, delay_until, status, grabbed_at
         FROM pending_releases
         WHERE status = 'waiting' AND delay_until <= ?
         ORDER BY delay_until ASC",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_pending_release(row)?);
    }
    Ok(out)
}

pub(crate) async fn list_pending_releases_for_wanted_item_query(
    pool: &SqlitePool,
    wanted_item_id: &str,
) -> AppResult<Vec<PendingRelease>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                source_kind, release_score, scoring_log_json, indexer_source, release_guid,
                added_at, delay_until, status, grabbed_at
         FROM pending_releases
         WHERE wanted_item_id = ? AND status = 'waiting'
         ORDER BY release_score DESC",
    )
    .bind(wanted_item_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_pending_release(row)?);
    }
    Ok(out)
}

pub(crate) async fn update_pending_release_status_query(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    grabbed_at: Option<&str>,
) -> AppResult<()> {
    sqlx::query("UPDATE pending_releases SET status = ?, grabbed_at = ? WHERE id = ?")
        .bind(status)
        .bind(grabbed_at)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn supersede_pending_releases_for_wanted_item_query(
    pool: &SqlitePool,
    wanted_item_id: &str,
    except_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE pending_releases SET status = 'superseded'
         WHERE wanted_item_id = ? AND id != ? AND status = 'waiting'",
    )
    .bind(wanted_item_id)
    .bind(except_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn list_waiting_pending_releases_query(
    pool: &SqlitePool,
) -> AppResult<Vec<PendingRelease>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                source_kind, release_score, scoring_log_json, indexer_source, release_guid,
                added_at, delay_until, status, grabbed_at
         FROM pending_releases
         WHERE status = 'waiting'
         ORDER BY delay_until ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_pending_release(row)?);
    }
    Ok(out)
}

pub(crate) async fn get_pending_release_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<PendingRelease>> {
    let row = sqlx::query(
        "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                source_kind, release_score, scoring_log_json, indexer_source, release_guid,
                added_at, delay_until, status, grabbed_at
         FROM pending_releases
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(ref r) => Ok(Some(row_to_pending_release(r)?)),
        None => Ok(None),
    }
}

pub(crate) async fn delete_pending_releases_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM pending_releases WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

fn row_to_pending_release(row: &SqliteRow) -> AppResult<PendingRelease> {
    Ok(PendingRelease {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        wanted_item_id: row
            .try_get("wanted_item_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_id: row
            .try_get("title_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        release_title: row
            .try_get("release_title")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        release_url: row.try_get("release_url").unwrap_or(None),
        release_size_bytes: row.try_get("release_size_bytes").unwrap_or(None),
        source_kind: row
            .try_get::<Option<String>, _>("source_kind")
            .unwrap_or(None)
            .and_then(|value| DownloadSourceKind::parse(&value)),
        release_score: row
            .try_get("release_score")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        scoring_log_json: row.try_get("scoring_log_json").unwrap_or(None),
        indexer_source: row.try_get("indexer_source").unwrap_or(None),
        release_guid: row.try_get("release_guid").unwrap_or(None),
        added_at: row
            .try_get("added_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        delay_until: row
            .try_get("delay_until")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        status: row
            .try_get("status")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        grabbed_at: row.try_get("grabbed_at").unwrap_or(None),
    })
}
