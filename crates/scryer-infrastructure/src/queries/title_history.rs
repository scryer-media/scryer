use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::Id;
use sqlx::SqlitePool;

#[derive(sqlx::FromRow)]
pub(crate) struct TitleHistoryRow {
    pub id: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub event_type: String,
    pub source_title: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub data_json: Option<String>,
    pub occurred_at: String,
    pub created_at: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_title_history_event_query(
    pool: &SqlitePool,
    title_id: &str,
    episode_id: Option<&str>,
    collection_id: Option<&str>,
    event_type: &str,
    source_title: Option<&str>,
    quality: Option<&str>,
    download_id: Option<&str>,
    data_json: Option<&str>,
) -> AppResult<String> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO title_history
         (id, title_id, episode_id, collection_id, event_type, source_title, quality, download_id, data_json, occurred_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(title_id)
    .bind(episode_id)
    .bind(collection_id)
    .bind(event_type)
    .bind(source_title)
    .bind(quality)
    .bind(download_id)
    .bind(data_json)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(id)
}

pub(crate) async fn list_title_history_query(
    pool: &SqlitePool,
    event_types: Option<&[&str]>,
    title_ids: Option<&[String]>,
    download_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> AppResult<(Vec<TitleHistoryRow>, i64)> {
    let mut where_clauses = Vec::new();

    if let Some(types) = event_types {
        if !types.is_empty() {
            let quoted: Vec<String> = types.iter().map(|t| format!("'{}'", t)).collect();
            where_clauses.push(format!("event_type IN ({})", quoted.join(", ")));
        }
    }

    if let Some(ids) = title_ids {
        if !ids.is_empty() {
            let quoted: Vec<String> = ids.iter().map(|id| format!("'{}'", id)).collect();
            where_clauses.push(format!("title_id IN ({})", quoted.join(", ")));
        }
    }

    if download_id.is_some() {
        where_clauses.push("download_id = ?".to_string());
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clauses.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) as cnt FROM title_history{}", where_sql);
    let select_sql = format!(
        "SELECT id, title_id, episode_id, collection_id, event_type, source_title, quality, download_id, data_json, occurred_at, created_at
         FROM title_history{}
         ORDER BY occurred_at DESC
         LIMIT ? OFFSET ?",
        where_sql
    );

    // Count query
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(did) = download_id {
        count_query = count_query.bind(did);
    }
    let total = count_query
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    // Data query
    let mut data_query = sqlx::query_as::<_, TitleHistoryRow>(&select_sql);
    if let Some(did) = download_id {
        data_query = data_query.bind(did);
    }
    data_query = data_query.bind(limit as i64).bind(offset as i64);

    let rows = data_query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok((rows, total))
}

pub(crate) async fn list_title_history_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    event_types: Option<&[&str]>,
    limit: usize,
    offset: usize,
) -> AppResult<(Vec<TitleHistoryRow>, i64)> {
    let type_filter = if let Some(types) = event_types {
        if !types.is_empty() {
            let quoted: Vec<String> = types.iter().map(|t| format!("'{}'", t)).collect();
            format!(" AND event_type IN ({})", quoted.join(", "))
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let count_sql = format!(
        "SELECT COUNT(*) as cnt FROM title_history WHERE title_id = ?{}",
        type_filter
    );
    let select_sql = format!(
        "SELECT id, title_id, episode_id, collection_id, event_type, source_title, quality, download_id, data_json, occurred_at, created_at
         FROM title_history
         WHERE title_id = ?{}
         ORDER BY occurred_at DESC
         LIMIT ? OFFSET ?",
        type_filter
    );

    let total = sqlx::query_scalar::<_, i64>(&count_sql)
        .bind(title_id)
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let rows = sqlx::query_as::<_, TitleHistoryRow>(&select_sql)
        .bind(title_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok((rows, total))
}

pub(crate) async fn list_title_history_for_episode_query(
    pool: &SqlitePool,
    episode_id: &str,
    limit: usize,
) -> AppResult<Vec<TitleHistoryRow>> {
    let rows = sqlx::query_as::<_, TitleHistoryRow>(
        "SELECT id, title_id, episode_id, collection_id, event_type, source_title, quality, download_id, data_json, occurred_at, created_at
         FROM title_history
         WHERE episode_id = ?
         ORDER BY occurred_at DESC
         LIMIT ?",
    )
    .bind(episode_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(rows)
}

pub(crate) async fn find_title_history_by_download_id_query(
    pool: &SqlitePool,
    download_id: &str,
) -> AppResult<Vec<TitleHistoryRow>> {
    let rows = sqlx::query_as::<_, TitleHistoryRow>(
        "SELECT id, title_id, episode_id, collection_id, event_type, source_title, quality, download_id, data_json, occurred_at, created_at
         FROM title_history
         WHERE download_id = ?
         ORDER BY occurred_at DESC",
    )
    .bind(download_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(rows)
}

pub(crate) async fn delete_title_history_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM title_history WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}
