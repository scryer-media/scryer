use chrono::Utc;
use scryer_application::{AppError, AppResult, TitleMediaFile};
use scryer_domain::Id;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

pub(crate) async fn insert_media_file_query(
    pool: &SqlitePool,
    title_id: &str,
    file_path: &str,
    size_bytes: i64,
    quality_label: Option<String>,
) -> AppResult<String> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO media_files
         (id, title_id, file_path, size_bytes, quality_id, scan_status, created_at)
         VALUES (?, ?, ?, ?, ?, 'imported', ?)
         ON CONFLICT(file_path) DO UPDATE SET
            title_id = excluded.title_id,
            size_bytes = excluded.size_bytes,
            quality_id = excluded.quality_id,
            scan_status = excluded.scan_status",
    )
    .bind(&id)
    .bind(title_id)
    .bind(file_path)
    .bind(size_bytes)
    .bind(&quality_label)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(id)
}

pub(crate) async fn link_file_to_episode_query(
    pool: &SqlitePool,
    file_id: &str,
    episode_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO file_episode_map (file_id, episode_id)
         VALUES (?, ?)
         ON CONFLICT(file_id, episode_id) DO NOTHING",
    )
    .bind(file_id)
    .bind(episode_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn list_media_files_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<TitleMediaFile>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT mf.id, mf.title_id, fem.episode_id, mf.file_path,
                mf.size_bytes, mf.quality_id, mf.scan_status, mf.created_at
         FROM media_files mf
         LEFT JOIN file_episode_map fem ON fem.file_id = mf.id
         WHERE mf.title_id = ?
         ORDER BY mf.created_at DESC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_title_media_file(row)?);
    }
    Ok(out)
}

fn row_to_title_media_file(row: &SqliteRow) -> AppResult<TitleMediaFile> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_id: Option<String> = row
        .try_get("episode_id")
        .unwrap_or(None);
    let file_path: String = row
        .try_get("file_path")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let size_bytes: i64 = row
        .try_get("size_bytes")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let quality_label: Option<String> = row
        .try_get("quality_id")
        .unwrap_or(None);
    let scan_status: String = row
        .try_get("scan_status")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(TitleMediaFile {
        id,
        title_id,
        episode_id,
        file_path,
        size_bytes,
        quality_label,
        scan_status,
        created_at,
    })
}
