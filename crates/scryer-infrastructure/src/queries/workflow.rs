use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadSubmission, PendingReleaseStatus, ReleaseDownloadAttemptOutcome,
    SuccessfulGrabCommit, WantedStatus,
};
use scryer_domain::{Id, ImportRecord, ImportStatus, ImportType};
use sqlx::Row;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::types::{
    ReleaseDownloadFailureSignatureRecord, TitleReleaseBlocklistRecord, WorkflowOperationRecord,
};

pub(crate) async fn create_workflow_operation_query(
    pool: &SqlitePool,
    operation_type: String,
    status: String,
    actor_user_id: Option<String>,
    progress_json: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
) -> AppResult<WorkflowOperationRecord> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    let started_at = started_at.unwrap_or_else(|| now.clone());

    sqlx::query(
        "INSERT INTO workflow_operations
         (id, operation_type, status, actor_user_id, progress_json, started_at, completed_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&operation_type)
    .bind(&status)
    .bind(&actor_user_id)
    .bind(&progress_json)
    .bind(&started_at)
    .bind(&completed_at)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(WorkflowOperationRecord {
        id,
        operation_type,
        status,
        actor_user_id,
        title_id: None,
        collection_id: None,
        episode_id: None,
        release_id: None,
        media_file_id: None,
        external_reference: None,
        progress_json,
        started_at: Some(started_at),
        completed_at,
        created_at: now.clone(),
        updated_at: now,
    })
}

pub(crate) async fn create_release_download_attempt_query(
    pool: &SqlitePool,
    title_id: Option<String>,
    source_hint: Option<String>,
    source_title: Option<String>,
    outcome: ReleaseDownloadAttemptOutcome,
    error_message: Option<String>,
    source_password: Option<String>,
) -> AppResult<()> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, error_message, source_password, attempted_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&title_id)
    .bind(&source_hint)
    .bind(&source_title)
    .bind(outcome.as_str())
    .bind(&error_message)
    .bind(&source_password)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn get_latest_source_password_query(
    pool: &SqlitePool,
    title_id: Option<&str>,
    source_hint: Option<&str>,
    source_title: Option<&str>,
) -> AppResult<Option<String>> {
    let mut sql = String::from(
        "SELECT source_password
         FROM release_download_attempts
         WHERE source_password IS NOT NULL",
    );

    let mut filters = Vec::new();
    if title_id.is_some() {
        filters.push("title_id = ?");
    }
    if source_hint.is_some() {
        filters.push("source_hint = ?");
    }
    if source_title.is_some() {
        filters.push("source_title = ?");
    }

    if !filters.is_empty() {
        sql.push(' ');
        sql.push_str("AND ");
        sql.push_str(&filters.join(" AND "));
    }

    sql.push_str(" ORDER BY attempted_at DESC LIMIT 1");

    let mut query = sqlx::query(&sql);
    if let Some(title_id) = title_id {
        query = query.bind(title_id);
    }
    if let Some(source_hint) = source_hint {
        query = query.bind(source_hint);
    }
    if let Some(source_title) = source_title {
        query = query.bind(source_title);
    }

    let row = query
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(row
            .try_get::<Option<String>, _>("source_password")
            .map_err(|err| AppError::Repository(err.to_string()))?),
        None => Ok(None),
    }
}

pub(crate) async fn create_import_request_query(
    pool: &SqlitePool,
    source_system: String,
    source_ref: String,
    import_type: String,
    payload_json: String,
) -> AppResult<String> {
    let id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    let is_rename = ImportType::parse(&import_type).is_some_and(|t| t.is_rename());
    let rename_plan_json = if is_rename {
        Some(payload_json.clone())
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO imports
         (id, source_system, source_ref, import_type, status, payload_json, rename_plan_json, result_json, started_at, finished_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(source_system, source_ref, import_type) DO UPDATE SET
            status = excluded.status,
            payload_json = excluded.payload_json,
            rename_plan_json = excluded.rename_plan_json,
            result_json = NULL,
            started_at = NULL,
            finished_at = NULL,
            updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&source_system)
    .bind(&source_ref)
    .bind(&import_type)
    .bind(ImportStatus::Pending.as_str())
    .bind(&payload_json)
    .bind(&rename_plan_json)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let row = sqlx::query(
        "SELECT id
         FROM imports
         WHERE source_system = ?
           AND source_ref = ?
           AND import_type = ?",
    )
    .bind(&source_system)
    .bind(&source_ref)
    .bind(&import_type)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let persisted_id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(persisted_id)
}

pub(crate) async fn get_import_by_id_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<ImportRecord>> {
    let row = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE id = ?
         LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(ImportRecord {
            id: row
                .try_get("id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_system: row
                .try_get("source_system")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_ref: row
                .try_get("source_ref")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            import_type: {
                let s: String = row
                    .try_get("import_type")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportType::parse(&s)
                    .ok_or_else(|| AppError::Repository(format!("unknown import_type: {s}")))?
            },
            status: {
                let s: String = row
                    .try_get("status")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportStatus::parse(&s).unwrap_or_default()
            },
            payload_json: row
                .try_get("payload_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            result_json: row
                .try_get("result_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            started_at: row
                .try_get("started_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            finished_at: row
                .try_get("finished_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
        })),
        None => Ok(None),
    }
}

pub(crate) async fn get_import_by_source_ref_query(
    pool: &SqlitePool,
    source_system: &str,
    source_ref: &str,
) -> AppResult<Option<ImportRecord>> {
    let row = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE source_system = ? AND source_ref = ?
         LIMIT 1",
    )
    .bind(source_system)
    .bind(source_ref)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(ImportRecord {
            id: row
                .try_get("id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_system: row
                .try_get("source_system")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_ref: row
                .try_get("source_ref")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            import_type: {
                let s: String = row
                    .try_get("import_type")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportType::parse(&s)
                    .ok_or_else(|| AppError::Repository(format!("unknown import_type: {s}")))?
            },
            status: {
                let s: String = row
                    .try_get("status")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportStatus::parse(&s).unwrap_or_default()
            },
            payload_json: row
                .try_get("payload_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            result_json: row
                .try_get("result_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            started_at: row
                .try_get("started_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            finished_at: row
                .try_get("finished_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
        })),
        None => Ok(None),
    }
}

pub(crate) async fn update_import_status_query(
    pool: &SqlitePool,
    import_id: &str,
    status: &str,
    result_json: Option<String>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let is_terminal = ImportStatus::parse(status).is_some_and(|s| s.is_terminal());

    sqlx::query(
        "UPDATE imports
         SET status = ?,
             result_json = ?,
             started_at = CASE WHEN started_at IS NULL THEN ? ELSE started_at END,
             finished_at = CASE WHEN ? THEN ? ELSE finished_at END,
             updated_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(&result_json)
    .bind(&now)
    .bind(is_terminal)
    .bind(&now)
    .bind(&now)
    .bind(import_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn recover_stale_processing_imports_query(
    pool: &SqlitePool,
    stale_seconds: i64,
) -> AppResult<u64> {
    let now = Utc::now();
    let cutoff = (now - chrono::Duration::seconds(stale_seconds)).to_rfc3339();
    let now_str = now.to_rfc3339();

    let result = sqlx::query(
        "UPDATE imports
         SET status = 'failed',
             result_json = '{\"error\":\"stale processing recovery\"}',
             finished_at = ?,
             updated_at = ?
         WHERE status = 'processing'
           AND updated_at < ?",
    )
    .bind(&now_str)
    .bind(&now_str)
    .bind(&cutoff)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(result.rows_affected())
}

pub(crate) async fn list_pending_imports_query(pool: &SqlitePool) -> AppResult<Vec<ImportRecord>> {
    let rows = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         WHERE status IN ('queued', 'pending', 'running', 'processing')
         ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(ImportRecord {
            id: row
                .try_get("id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_system: row
                .try_get("source_system")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_ref: row
                .try_get("source_ref")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            import_type: {
                let s: String = row
                    .try_get("import_type")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportType::parse(&s)
                    .ok_or_else(|| AppError::Repository(format!("unknown import_type: {s}")))?
            },
            status: {
                let s: String = row
                    .try_get("status")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportStatus::parse(&s).unwrap_or_default()
            },
            payload_json: row
                .try_get("payload_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            result_json: row
                .try_get("result_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            started_at: row
                .try_get("started_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            finished_at: row
                .try_get("finished_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
        });
    }

    Ok(out)
}

pub(crate) async fn list_imports_query(
    pool: &SqlitePool,
    limit: i64,
) -> AppResult<Vec<ImportRecord>> {
    let limit = limit.clamp(1, 500);
    let rows = sqlx::query(
        "SELECT id, source_system, source_ref, import_type, status,
                payload_json, result_json, started_at, finished_at,
                created_at, updated_at
         FROM imports
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(ImportRecord {
            id: row
                .try_get("id")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_system: row
                .try_get("source_system")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            source_ref: row
                .try_get("source_ref")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            import_type: {
                let s: String = row
                    .try_get("import_type")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportType::parse(&s)
                    .ok_or_else(|| AppError::Repository(format!("unknown import_type: {s}")))?
            },
            status: {
                let s: String = row
                    .try_get("status")
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                ImportStatus::parse(&s).unwrap_or_default()
            },
            payload_json: row
                .try_get("payload_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            result_json: row
                .try_get("result_json")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            started_at: row
                .try_get("started_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            finished_at: row
                .try_get("finished_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|e| AppError::Repository(e.to_string()))?,
        });
    }

    Ok(out)
}

pub(crate) async fn list_failed_release_download_attempts_query(
    pool: &SqlitePool,
    limit: i64,
) -> AppResult<Vec<ReleaseDownloadFailureSignatureRecord>> {
    let limit = limit.clamp(1, 20_000);
    let rows = sqlx::query(
        "SELECT source_hint, source_title
         FROM (
           SELECT source_hint, source_title, MAX(attempted_at) AS last_attempted_at
           FROM release_download_attempts
           WHERE outcome = 'failed'
             AND (source_hint IS NOT NULL OR source_title IS NOT NULL)
           GROUP BY source_hint, source_title
         )
         ORDER BY last_attempted_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let source_hint: Option<String> = row
            .try_get("source_hint")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let source_title: Option<String> = row
            .try_get("source_title")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        out.push(ReleaseDownloadFailureSignatureRecord {
            source_hint,
            source_title,
        });
    }

    Ok(out)
}

pub(crate) async fn list_failed_release_download_attempts_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    limit: i64,
) -> AppResult<Vec<TitleReleaseBlocklistRecord>> {
    let limit = limit.clamp(1, 1_000);
    let rows = sqlx::query(
        "SELECT source_hint, source_title, error_message, attempted_at
         FROM (
           SELECT source_hint,
                  source_title,
                  error_message,
                  attempted_at,
                  ROW_NUMBER() OVER (
                    PARTITION BY source_hint, source_title
                    ORDER BY attempted_at DESC
                  ) AS row_number
           FROM release_download_attempts
           WHERE outcome = 'failed'
             AND title_id = ?
             AND (source_hint IS NOT NULL OR source_title IS NOT NULL)
         )
         WHERE row_number = 1
         ORDER BY attempted_at DESC
         LIMIT ?",
    )
    .bind(title_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let source_hint: Option<String> = row
            .try_get("source_hint")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let source_title: Option<String> = row
            .try_get("source_title")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let error_message: Option<String> = row
            .try_get("error_message")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let attempted_at: String = row
            .try_get("attempted_at")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        out.push(TitleReleaseBlocklistRecord {
            source_hint,
            source_title,
            error_message,
            attempted_at,
        });
    }

    Ok(out)
}

pub(crate) async fn record_download_submission_query(
    pool: &SqlitePool,
    title_id: &str,
    facet: &str,
    download_client_type: &str,
    download_client_item_id: &str,
    source_title: Option<&str>,
    collection_id: Option<&str>,
) -> AppResult<()> {
    let id = Id::new().0;

    sqlx::query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_type, download_client_item_id, source_title, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(download_client_type, download_client_item_id) DO UPDATE
         SET title_id = excluded.title_id,
             facet = excluded.facet,
             source_title = excluded.source_title,
             collection_id = excluded.collection_id",
    )
    .bind(&id)
    .bind(title_id)
    .bind(facet)
    .bind(download_client_type)
    .bind(download_client_item_id)
    .bind(source_title)
    .bind(collection_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn commit_successful_grab_query(
    pool: &SqlitePool,
    commit: &SuccessfulGrabCommit,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    record_download_submission_tx(&mut tx, &commit.download_submission).await?;

    sqlx::query(
        "UPDATE wanted_items
         SET status = ?, next_search_at = ?, last_search_at = ?,
             search_count = ?, current_score = ?, grabbed_release = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(WantedStatus::Grabbed.as_str())
    .bind(Option::<String>::None)
    .bind(commit.last_search_at.as_deref())
    .bind(commit.search_count)
    .bind(commit.current_score)
    .bind(&commit.grabbed_release)
    .bind(&now)
    .bind(&commit.wanted_item_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    if let Some(pending_release_id) = commit.grabbed_pending_release_id.as_deref() {
        sqlx::query(
            "UPDATE pending_releases
             SET status = ?, grabbed_at = ?
             WHERE id = ?",
        )
        .bind(PendingReleaseStatus::Grabbed.as_str())
        .bind(commit.grabbed_at.as_deref())
        .bind(pending_release_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    supersede_pending_release_siblings_tx(
        &mut tx,
        &commit.wanted_item_id,
        commit.grabbed_pending_release_id.as_deref(),
    )
    .await?;

    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

async fn record_download_submission_tx(
    tx: &mut Transaction<'_, Sqlite>,
    submission: &DownloadSubmission,
) -> AppResult<()> {
    let id = Id::new().0;

    sqlx::query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_type, download_client_item_id, source_title, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(download_client_type, download_client_item_id) DO UPDATE
         SET title_id = excluded.title_id,
             facet = excluded.facet,
             source_title = excluded.source_title,
             collection_id = excluded.collection_id",
    )
    .bind(&id)
    .bind(&submission.title_id)
    .bind(&submission.facet)
    .bind(&submission.download_client_type)
    .bind(&submission.download_client_item_id)
    .bind(&submission.source_title)
    .bind(&submission.collection_id)
    .execute(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

async fn supersede_pending_release_siblings_tx(
    tx: &mut Transaction<'_, Sqlite>,
    wanted_item_id: &str,
    except_id: Option<&str>,
) -> AppResult<()> {
    match except_id {
        Some(except_id) => {
            sqlx::query(
                "UPDATE pending_releases
                 SET status = 'superseded'
                 WHERE wanted_item_id = ?
                   AND id != ?
                   AND status IN ('waiting', 'standby')",
            )
            .bind(wanted_item_id)
            .bind(except_id)
            .execute(&mut **tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        }
        None => {
            sqlx::query(
                "UPDATE pending_releases
                 SET status = 'superseded'
                 WHERE wanted_item_id = ?
                   AND status IN ('waiting', 'standby')",
            )
            .bind(wanted_item_id)
            .execute(&mut **tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        }
    }

    Ok(())
}

pub(crate) async fn find_download_submission_query(
    pool: &SqlitePool,
    download_client_type: &str,
    download_client_item_id: &str,
) -> AppResult<Option<DownloadSubmission>> {
    let row = sqlx::query(
        "SELECT title_id, facet, download_client_type, download_client_item_id, source_title, collection_id
         FROM download_submissions
         WHERE download_client_type = ? AND download_client_item_id = ?",
    )
    .bind(download_client_type)
    .bind(download_client_item_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(DownloadSubmission {
            title_id: row
                .try_get("title_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            facet: row
                .try_get("facet")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            download_client_type: row
                .try_get("download_client_type")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            download_client_item_id: row
                .try_get("download_client_item_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            source_title: row
                .try_get("source_title")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            collection_id: row.try_get("collection_id").unwrap_or(None),
        })),
        None => Ok(None),
    }
}

pub(crate) async fn list_download_submissions_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<DownloadSubmission>> {
    let rows = sqlx::query(
        "SELECT title_id, facet, download_client_type, download_client_item_id, source_title, collection_id
         FROM download_submissions
         WHERE title_id = ?",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(DownloadSubmission {
            title_id: row
                .try_get("title_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            facet: row
                .try_get("facet")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            download_client_type: row
                .try_get("download_client_type")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            download_client_item_id: row
                .try_get("download_client_item_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            source_title: row
                .try_get("source_title")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            collection_id: row.try_get("collection_id").unwrap_or(None),
        });
    }

    Ok(out)
}

pub(crate) async fn delete_download_submission_by_client_item_id_query(
    pool: &SqlitePool,
    download_client_item_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM download_submissions WHERE download_client_item_id = ?")
        .bind(download_client_item_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn delete_download_submissions_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM download_submissions WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

// ── TrackedDownloads (plan 055) ──────────────────────────────────────────────

pub(crate) async fn update_tracked_state_query(
    pool: &SqlitePool,
    download_client_type: &str,
    download_client_item_id: &str,
    tracked_state: &str,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE download_submissions
         SET tracked_state = ?, tracked_state_at = ?
         WHERE download_client_type = ? AND download_client_item_id = ?",
    )
    .bind(tracked_state)
    .bind(&now)
    .bind(download_client_type)
    .bind(download_client_item_id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn get_tracked_state_query(
    pool: &SqlitePool,
    download_client_type: &str,
    download_client_item_id: &str,
) -> AppResult<Option<String>> {
    let row = sqlx::query(
        "SELECT tracked_state FROM download_submissions
         WHERE download_client_type = ? AND download_client_item_id = ?",
    )
    .bind(download_client_type)
    .bind(download_client_item_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => {
            let state: Option<String> = row
                .try_get("tracked_state")
                .map_err(|err| AppError::Repository(err.to_string()))?;
            Ok(state)
        }
        None => Ok(None),
    }
}

pub(crate) async fn insert_import_artifact_query(
    pool: &SqlitePool,
    artifact: &scryer_application::ImportArtifact,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO download_import_artifacts
         (id, source_system, source_ref, import_id, relative_path, normalized_file_name,
          media_kind, title_id, episode_id, season_number, episode_number,
          result, reason_code, imported_media_file_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&artifact.id)
    .bind(&artifact.source_system)
    .bind(&artifact.source_ref)
    .bind(&artifact.import_id)
    .bind(&artifact.relative_path)
    .bind(&artifact.normalized_file_name)
    .bind(&artifact.media_kind)
    .bind(&artifact.title_id)
    .bind(&artifact.episode_id)
    .bind(artifact.season_number)
    .bind(artifact.episode_number)
    .bind(&artifact.result)
    .bind(&artifact.reason_code)
    .bind(&artifact.imported_media_file_id)
    .bind(artifact.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn list_import_artifacts_by_source_ref_query(
    pool: &SqlitePool,
    source_system: &str,
    source_ref: &str,
) -> AppResult<Vec<scryer_application::ImportArtifact>> {
    let rows = sqlx::query(
        "SELECT id, source_system, source_ref, import_id, relative_path,
                normalized_file_name, media_kind, title_id, episode_id,
                season_number, episode_number, result, reason_code,
                imported_media_file_id, created_at
         FROM download_import_artifacts
         WHERE source_system = ? AND source_ref = ?
         ORDER BY created_at",
    )
    .bind(source_system)
    .bind(source_ref)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(scryer_application::ImportArtifact {
            id: row.try_get("id").map_err(|e| AppError::Repository(e.to_string()))?,
            source_system: row.try_get("source_system").map_err(|e| AppError::Repository(e.to_string()))?,
            source_ref: row.try_get("source_ref").map_err(|e| AppError::Repository(e.to_string()))?,
            import_id: row.try_get("import_id").map_err(|e| AppError::Repository(e.to_string()))?,
            relative_path: row.try_get("relative_path").map_err(|e| AppError::Repository(e.to_string()))?,
            normalized_file_name: row.try_get("normalized_file_name").map_err(|e| AppError::Repository(e.to_string()))?,
            media_kind: row.try_get("media_kind").map_err(|e| AppError::Repository(e.to_string()))?,
            title_id: row.try_get("title_id").map_err(|e| AppError::Repository(e.to_string()))?,
            episode_id: row.try_get("episode_id").map_err(|e| AppError::Repository(e.to_string()))?,
            season_number: row.try_get("season_number").map_err(|e| AppError::Repository(e.to_string()))?,
            episode_number: row.try_get("episode_number").map_err(|e| AppError::Repository(e.to_string()))?,
            result: row.try_get("result").map_err(|e| AppError::Repository(e.to_string()))?,
            reason_code: row.try_get("reason_code").map_err(|e| AppError::Repository(e.to_string()))?,
            imported_media_file_id: row.try_get("imported_media_file_id").map_err(|e| AppError::Repository(e.to_string()))?,
            created_at: {
                let s: String = row.try_get("created_at").map_err(|e| AppError::Repository(e.to_string()))?;
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now())
            },
        });
    }
    Ok(out)
}

pub(crate) async fn count_import_artifacts_by_result_query(
    pool: &SqlitePool,
    source_system: &str,
    source_ref: &str,
    result: &str,
) -> AppResult<u64> {
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM download_import_artifacts
         WHERE source_system = ? AND source_ref = ? AND result = ?",
    )
    .bind(source_system)
    .bind(source_ref)
    .bind(result)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let count: i64 = row
        .try_get("cnt")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(count as u64)
}
