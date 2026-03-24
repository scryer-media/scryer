use chrono::Utc;
use scryer_application::{AppError, AppResult, ReleaseDecision, WantedItem};
use sqlx::sqlite::SqliteRow;
use sqlx::{Executor, Row, Sqlite, SqlitePool, Transaction};

fn upsert_wanted_item_sql(item: &WantedItem) -> &'static str {
    if item.collection_id.is_some() {
        // Interstitial movie: unique by collection_id
        "INSERT INTO wanted_items
         (id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
          last_search_at, search_count, baseline_date, status, grabbed_release, current_score,
          created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(collection_id) WHERE collection_id IS NOT NULL DO UPDATE SET
            search_phase = excluded.search_phase,
            next_search_at = CASE
                WHEN wanted_items.search_count > 0 THEN wanted_items.next_search_at
                ELSE excluded.next_search_at
            END,
            baseline_date = excluded.baseline_date,
            status = CASE
                WHEN wanted_items.status IN ('completed', 'paused') AND excluded.status = 'wanted'
                THEN wanted_items.status
                ELSE excluded.status
            END,
            updated_at = excluded.updated_at"
    } else if item.episode_id.is_some() {
        "INSERT INTO wanted_items
         (id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
          last_search_at, search_count, baseline_date, status, grabbed_release, current_score,
          created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(title_id, episode_id) DO UPDATE SET
            search_phase = excluded.search_phase,
            next_search_at = CASE
                WHEN wanted_items.search_count > 0 THEN wanted_items.next_search_at
                ELSE excluded.next_search_at
            END,
            baseline_date = excluded.baseline_date,
            status = CASE
                WHEN wanted_items.status IN ('completed', 'paused') AND excluded.status = 'wanted'
                THEN wanted_items.status
                ELSE excluded.status
            END,
            updated_at = excluded.updated_at"
    } else {
        "INSERT INTO wanted_items
         (id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
          last_search_at, search_count, baseline_date, status, grabbed_release, current_score,
          created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(title_id) WHERE episode_id IS NULL AND collection_id IS NULL DO UPDATE SET
            search_phase = excluded.search_phase,
            next_search_at = CASE
                WHEN wanted_items.search_count > 0 THEN wanted_items.next_search_at
                ELSE excluded.next_search_at
            END,
            baseline_date = excluded.baseline_date,
            status = CASE
                WHEN wanted_items.status IN ('completed', 'paused') AND excluded.status = 'wanted'
                THEN wanted_items.status
                ELSE excluded.status
            END,
            updated_at = excluded.updated_at"
    }
}

async fn execute_upsert_wanted_item<'e, E>(executor: E, item: &WantedItem) -> AppResult<String>
where
    E: Executor<'e, Database = Sqlite>,
{
    let now = Utc::now().to_rfc3339();

    sqlx::query(upsert_wanted_item_sql(item))
        .bind(&item.id)
        .bind(&item.title_id)
        .bind(&item.episode_id)
        .bind(&item.collection_id)
        .bind(&item.media_type)
        .bind(&item.search_phase)
        .bind(&item.next_search_at)
        .bind(&item.last_search_at)
        .bind(item.search_count)
        .bind(&item.baseline_date)
        .bind(item.status.as_str())
        .bind(&item.grabbed_release)
        .bind(item.current_score)
        .bind(&now)
        .bind(&now)
        .execute(executor)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(item.id.clone())
}

async fn fetch_seed_target_query<'e, E>(executor: E, item: &WantedItem) -> AppResult<Option<WantedItem>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row: Option<SqliteRow> = if let Some(collection_id) = item.collection_id.as_deref() {
        sqlx::query(
            "SELECT id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
                    last_search_at, search_count, baseline_date, status, grabbed_release,
                    current_score, created_at, updated_at
             FROM wanted_items
             WHERE title_id = ? AND collection_id = ?",
        )
        .bind(&item.title_id)
        .bind(collection_id)
        .fetch_optional(executor)
        .await
    } else if let Some(episode_id) = item.episode_id.as_deref() {
        sqlx::query(
            "SELECT id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
                    last_search_at, search_count, baseline_date, status, grabbed_release,
                    current_score, created_at, updated_at
             FROM wanted_items
             WHERE title_id = ? AND episode_id = ?",
        )
        .bind(&item.title_id)
        .bind(episode_id)
        .fetch_optional(executor)
        .await
    } else {
        sqlx::query(
            "SELECT id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
                    last_search_at, search_count, baseline_date, status, grabbed_release,
                    current_score, created_at, updated_at
             FROM wanted_items
             WHERE title_id = ? AND episode_id IS NULL AND collection_id IS NULL",
        )
        .bind(&item.title_id)
        .fetch_optional(executor)
        .await
    }
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(ref row) => Ok(Some(row_to_wanted_item(row)?)),
        None => Ok(None),
    }
}

fn merge_seeded_wanted_item(item: &WantedItem, existing: Option<&WantedItem>) -> WantedItem {
    let mut seeded = item.clone();

    if let Some(existing) = existing {
        seeded.id = existing.id.clone();
        if existing.search_count > 0 {
            seeded.next_search_at = existing.next_search_at.clone();
        }
        if item.status == scryer_application::WantedStatus::Wanted
            && existing.status != scryer_application::WantedStatus::Wanted
        {
            seeded.status = existing.status;
        }
    }

    seeded
}

pub(crate) async fn upsert_wanted_item_query(
    pool: &SqlitePool,
    item: &WantedItem,
) -> AppResult<String> {
    execute_upsert_wanted_item(pool, item).await
}

pub(crate) async fn ensure_wanted_item_seeded_query(
    pool: &SqlitePool,
    item: &WantedItem,
) -> AppResult<String> {
    let mut tx: Transaction<'_, Sqlite> = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let existing = fetch_seed_target_query(&mut *tx, item).await?;
    let seeded = merge_seeded_wanted_item(item, existing.as_ref());
    execute_upsert_wanted_item(&mut *tx, &seeded).await?;

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(existing.map_or(item.id.clone(), |existing| existing.id))
}

pub(crate) async fn list_due_wanted_items_query(
    pool: &SqlitePool,
    now: &str,
    batch_limit: i64,
) -> AppResult<Vec<WantedItem>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT w.id, w.title_id, w.episode_id, w.collection_id, e.season_number,
                w.media_type, w.search_phase, w.next_search_at,
                w.last_search_at, w.search_count, w.baseline_date, w.status, w.grabbed_release,
                w.current_score, w.created_at, w.updated_at
         FROM wanted_items w
         LEFT JOIN episodes e ON e.id = w.episode_id
         WHERE w.status = 'wanted' AND (w.next_search_at IS NULL OR w.next_search_at <= ?)
         ORDER BY w.next_search_at ASC
         LIMIT ?",
    )
    .bind(now)
    .bind(batch_limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_wanted_item(row)?);
    }
    Ok(out)
}

/// Reset `next_search_at` to now for wanted items that have been searched
/// but never found a viable candidate. This recovers from scenarios where a
/// bug caused searches to return 0 results and items got rescheduled far out.
pub(crate) async fn reset_fruitless_wanted_items_query(
    pool: &SqlitePool,
    now: &str,
) -> AppResult<u64> {
    let result = sqlx::query(
        "UPDATE wanted_items
         SET next_search_at = ?, updated_at = ?
         WHERE status = 'wanted'
           AND search_count > 0
           AND current_score IS NULL",
    )
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected())
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn update_wanted_item_status_query(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    next_search_at: Option<&str>,
    last_search_at: Option<&str>,
    search_count: i64,
    current_score: Option<i32>,
    grabbed_release: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE wanted_items
         SET status = ?, next_search_at = ?, last_search_at = ?,
             search_count = ?, current_score = ?, grabbed_release = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(next_search_at)
    .bind(last_search_at)
    .bind(search_count)
    .bind(current_score)
    .bind(grabbed_release)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn get_wanted_item_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    episode_id: Option<&str>,
) -> AppResult<Option<WantedItem>> {
    let row: Option<SqliteRow> = match episode_id {
        Some(ep_id) => {
            sqlx::query(
                "SELECT id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
                        last_search_at, search_count, baseline_date, status, grabbed_release,
                        current_score, created_at, updated_at
                 FROM wanted_items
                 WHERE title_id = ? AND episode_id = ?",
            )
            .bind(title_id)
            .bind(ep_id)
            .fetch_optional(pool)
            .await
        }
        None => {
            sqlx::query(
                "SELECT id, title_id, episode_id, collection_id, media_type, search_phase, next_search_at,
                        last_search_at, search_count, baseline_date, status, grabbed_release,
                        current_score, created_at, updated_at
                 FROM wanted_items
                 WHERE title_id = ? AND episode_id IS NULL",
            )
            .bind(title_id)
            .fetch_optional(pool)
            .await
        }
    }
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(ref r) => Ok(Some(row_to_wanted_item(r)?)),
        None => Ok(None),
    }
}

pub(crate) async fn delete_wanted_items_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM wanted_items WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn insert_release_decision_query(
    pool: &SqlitePool,
    decision: &ReleaseDecision,
) -> AppResult<String> {
    sqlx::query(
        "INSERT INTO release_decisions
         (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
          decision_code, candidate_score, current_score, score_delta, explanation_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&decision.id)
    .bind(&decision.wanted_item_id)
    .bind(&decision.title_id)
    .bind(&decision.release_title)
    .bind(&decision.release_url)
    .bind(decision.release_size_bytes)
    .bind(&decision.decision_code)
    .bind(decision.candidate_score)
    .bind(decision.current_score)
    .bind(decision.score_delta)
    .bind(&decision.explanation_json)
    .bind(&decision.created_at)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(decision.id.clone())
}

pub(crate) async fn get_wanted_item_by_id_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<WantedItem>> {
    let row: Option<SqliteRow> = sqlx::query(
        "SELECT id, title_id, episode_id, media_type, search_phase, next_search_at,
                last_search_at, search_count, baseline_date, status, grabbed_release,
                current_score, created_at, updated_at
         FROM wanted_items WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(ref r) => Ok(Some(row_to_wanted_item(r)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_wanted_items_query(
    pool: &SqlitePool,
    status: Option<&str>,
    media_type: Option<&str>,
    title_id: Option<&str>,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<WantedItem>> {
    let mut sql = String::from(
        "SELECT w.id, w.title_id, t.name AS title_name, w.episode_id, w.media_type,
                w.search_phase, w.next_search_at, w.last_search_at, w.search_count,
                w.baseline_date, w.status, w.grabbed_release, w.current_score,
                w.created_at, w.updated_at
         FROM wanted_items w
         LEFT JOIN titles t ON t.id = w.title_id
         WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(s) = status {
        sql.push_str(" AND w.status = ?");
        binds.push(s.to_string());
    }
    if let Some(mt) = media_type {
        sql.push_str(" AND w.media_type = ?");
        binds.push(mt.to_string());
    }
    if let Some(tid) = title_id {
        sql.push_str(" AND w.title_id = ?");
        binds.push(tid.to_string());
    }

    sql.push_str(" ORDER BY w.updated_at DESC LIMIT ? OFFSET ?");

    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    query = query.bind(limit).bind(offset);

    let rows: Vec<SqliteRow> = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_wanted_item(row)?);
    }
    Ok(out)
}

pub(crate) async fn count_wanted_items_query(
    pool: &SqlitePool,
    status: Option<&str>,
    media_type: Option<&str>,
    title_id: Option<&str>,
) -> AppResult<i64> {
    let mut sql = String::from("SELECT COUNT(*) as cnt FROM wanted_items WHERE 1=1");
    let mut binds: Vec<String> = Vec::new();

    if let Some(s) = status {
        sql.push_str(" AND status = ?");
        binds.push(s.to_string());
    }
    if let Some(mt) = media_type {
        sql.push_str(" AND media_type = ?");
        binds.push(mt.to_string());
    }
    if let Some(tid) = title_id {
        sql.push_str(" AND title_id = ?");
        binds.push(tid.to_string());
    }

    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }

    let row: SqliteRow = query
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let count: i64 = row
        .try_get("cnt")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(count)
}

pub(crate) async fn list_release_decisions_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    limit: i64,
) -> AppResult<Vec<ReleaseDecision>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
         FROM release_decisions
         WHERE title_id = ?
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(title_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_release_decision(row)?);
    }
    Ok(out)
}

pub(crate) async fn list_release_decisions_for_wanted_item_query(
    pool: &SqlitePool,
    wanted_item_id: &str,
    limit: i64,
) -> AppResult<Vec<ReleaseDecision>> {
    let rows: Vec<SqliteRow> = sqlx::query(
        "SELECT id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
                decision_code, candidate_score, current_score, score_delta, explanation_json, created_at
         FROM release_decisions
         WHERE wanted_item_id = ?
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(wanted_item_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_release_decision(row)?);
    }
    Ok(out)
}

fn row_to_release_decision(row: &SqliteRow) -> AppResult<ReleaseDecision> {
    Ok(ReleaseDecision {
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
        decision_code: row
            .try_get("decision_code")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        candidate_score: row
            .try_get("candidate_score")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        current_score: row.try_get("current_score").unwrap_or(None),
        score_delta: row.try_get("score_delta").unwrap_or(None),
        explanation_json: row.try_get("explanation_json").unwrap_or(None),
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    })
}

fn row_to_wanted_item(row: &SqliteRow) -> AppResult<WantedItem> {
    Ok(WantedItem {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_id: row
            .try_get("title_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_name: row.try_get("title_name").unwrap_or(None),
        episode_id: row.try_get("episode_id").unwrap_or(None),
        collection_id: row.try_get("collection_id").unwrap_or(None),
        season_number: row.try_get("season_number").unwrap_or(None),
        media_type: row
            .try_get("media_type")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        search_phase: row
            .try_get("search_phase")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        next_search_at: row.try_get("next_search_at").unwrap_or(None),
        last_search_at: row.try_get("last_search_at").unwrap_or(None),
        search_count: row
            .try_get("search_count")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        baseline_date: row.try_get("baseline_date").unwrap_or(None),
        status: {
            let s: String = row
                .try_get("status")
                .map_err(|e| AppError::Repository(e.to_string()))?;
            scryer_application::WantedStatus::parse(&s).unwrap_or_default()
        },
        grabbed_release: row.try_get("grabbed_release").unwrap_or(None),
        current_score: row.try_get("current_score").unwrap_or(None),
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    })
}
