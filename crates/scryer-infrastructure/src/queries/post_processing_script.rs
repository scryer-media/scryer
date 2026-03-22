use scryer_application::{AppError, AppResult};
use scryer_domain::{PostProcessingScript, PostProcessingScriptRun};
use sqlx::{Row, SqlitePool};

pub(crate) async fn list_scripts_query(pool: &SqlitePool) -> AppResult<Vec<PostProcessingScript>> {
    let rows = sqlx::query(
        "SELECT id, name, description, script_type, script_content, applied_facets,
                execution_mode, timeout_secs, priority, enabled, debug,
                created_at, updated_at
           FROM post_processing_scripts
          ORDER BY priority ASC, name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.into_iter().map(|row| row_to_script(&row)).collect()
}

pub(crate) async fn get_script_by_id_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<PostProcessingScript>> {
    let row = sqlx::query(
        "SELECT id, name, description, script_type, script_content, applied_facets,
                execution_mode, timeout_secs, priority, enabled, debug,
                created_at, updated_at
           FROM post_processing_scripts
          WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_script(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn insert_script_query(
    pool: &SqlitePool,
    script: &PostProcessingScript,
) -> AppResult<PostProcessingScript> {
    let facets_json = serde_json::to_string(&script.applied_facets)
        .map_err(|e| AppError::Repository(e.to_string()))?;

    sqlx::query(
        "INSERT INTO post_processing_scripts
            (id, name, description, script_type, script_content, applied_facets,
             execution_mode, timeout_secs, priority, enabled, debug,
             created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&script.id)
    .bind(&script.name)
    .bind(&script.description)
    .bind(script.script_type.as_str())
    .bind(&script.script_content)
    .bind(&facets_json)
    .bind(script.execution_mode.as_str())
    .bind(script.timeout_secs)
    .bind(script.priority)
    .bind(script.enabled)
    .bind(script.debug)
    .bind(script.created_at.to_rfc3339())
    .bind(script.updated_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(script.clone())
}

pub(crate) async fn update_script_query(
    pool: &SqlitePool,
    script: &PostProcessingScript,
) -> AppResult<PostProcessingScript> {
    let facets_json = serde_json::to_string(&script.applied_facets)
        .map_err(|e| AppError::Repository(e.to_string()))?;

    sqlx::query(
        "UPDATE post_processing_scripts
            SET name = ?, description = ?, script_type = ?, script_content = ?,
                applied_facets = ?, execution_mode = ?, timeout_secs = ?,
                priority = ?, enabled = ?, debug = ?, updated_at = ?
          WHERE id = ?",
    )
    .bind(&script.name)
    .bind(&script.description)
    .bind(script.script_type.as_str())
    .bind(&script.script_content)
    .bind(&facets_json)
    .bind(script.execution_mode.as_str())
    .bind(script.timeout_secs)
    .bind(script.priority)
    .bind(script.enabled)
    .bind(script.debug)
    .bind(script.updated_at.to_rfc3339())
    .bind(&script.id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(script.clone())
}

pub(crate) async fn delete_script_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM post_processing_scripts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;
    Ok(())
}

pub(crate) async fn list_enabled_for_facet_query(
    pool: &SqlitePool,
    facet: &str,
) -> AppResult<Vec<PostProcessingScript>> {
    // applied_facets is a JSON array stored as text; use JSON instr for matching.
    let rows = sqlx::query(
        "SELECT id, name, description, script_type, script_content, applied_facets,
                execution_mode, timeout_secs, priority, enabled, debug,
                created_at, updated_at
           FROM post_processing_scripts
          WHERE enabled = 1
            AND (applied_facets = '[]' OR instr(applied_facets, ?) > 0)
          ORDER BY priority ASC, name",
    )
    .bind(format!("\"{}\"", facet))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.into_iter().map(|row| row_to_script(&row)).collect()
}

pub(crate) async fn record_run_query(
    pool: &SqlitePool,
    run: &PostProcessingScriptRun,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO post_processing_script_runs
            (id, script_id, script_name, title_id, title_name, facet, file_path,
             status, exit_code, stdout_tail, stderr_tail, duration_ms,
             env_payload_json, started_at, completed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&run.id)
    .bind(&run.script_id)
    .bind(&run.script_name)
    .bind(&run.title_id)
    .bind(&run.title_name)
    .bind(&run.facet)
    .bind(&run.file_path)
    .bind(run.status.as_str())
    .bind(run.exit_code)
    .bind(&run.stdout_tail)
    .bind(&run.stderr_tail)
    .bind(run.duration_ms)
    .bind(&run.env_payload_json)
    .bind(&run.started_at)
    .bind(&run.completed_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(())
}

pub(crate) async fn list_runs_for_script_query(
    pool: &SqlitePool,
    script_id: &str,
    limit: usize,
) -> AppResult<Vec<PostProcessingScriptRun>> {
    let rows = sqlx::query(
        "SELECT id, script_id, script_name, title_id, title_name, facet, file_path,
                status, exit_code, stdout_tail, stderr_tail, duration_ms,
                env_payload_json, started_at, completed_at
           FROM post_processing_script_runs
          WHERE script_id = ?
          ORDER BY started_at DESC
          LIMIT ?",
    )
    .bind(script_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.into_iter().map(|row| row_to_run(&row)).collect()
}

pub(crate) async fn list_runs_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    limit: usize,
) -> AppResult<Vec<PostProcessingScriptRun>> {
    let rows = sqlx::query(
        "SELECT id, script_id, script_name, title_id, title_name, facet, file_path,
                status, exit_code, stdout_tail, stderr_tail, duration_ms,
                env_payload_json, started_at, completed_at
           FROM post_processing_script_runs
          WHERE title_id = ?
          ORDER BY started_at DESC
          LIMIT ?",
    )
    .bind(title_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Repository(e.to_string()))?;

    rows.into_iter().map(|row| row_to_run(&row)).collect()
}

fn row_to_script(row: &sqlx::sqlite::SqliteRow) -> AppResult<PostProcessingScript> {
    use chrono::{DateTime, Utc};

    let facets_json: String = row
        .try_get("applied_facets")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let applied_facets: Vec<String> = serde_json::from_str(&facets_json).unwrap_or_default();

    let created_str: String = row
        .try_get("created_at")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let updated_str: String = row
        .try_get("updated_at")
        .map_err(|e| AppError::Repository(e.to_string()))?;

    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = DateTime::parse_from_rfc3339(&updated_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    let enabled_int: i32 = row
        .try_get("enabled")
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let debug_int: i32 = row
        .try_get("debug")
        .map_err(|e| AppError::Repository(e.to_string()))?;

    Ok(PostProcessingScript {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        name: row
            .try_get("name")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        description: row
            .try_get("description")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        script_type: row
            .try_get::<String, _>("script_type")
            .map_err(|e| AppError::Repository(e.to_string()))
            .and_then(|s| {
                scryer_domain::ScriptType::parse(&s)
                    .ok_or_else(|| AppError::Repository(format!("invalid script_type: {s}")))
            })?,
        script_content: row
            .try_get("script_content")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        applied_facets,
        execution_mode: row
            .try_get::<String, _>("execution_mode")
            .map_err(|e| AppError::Repository(e.to_string()))
            .and_then(|s| {
                scryer_domain::ExecutionMode::parse(&s)
                    .ok_or_else(|| AppError::Repository(format!("invalid execution_mode: {s}")))
            })?,
        timeout_secs: row
            .try_get("timeout_secs")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        priority: row
            .try_get("priority")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        enabled: enabled_int != 0,
        debug: debug_int != 0,
        created_at,
        updated_at,
    })
}

fn row_to_run(row: &sqlx::sqlite::SqliteRow) -> AppResult<PostProcessingScriptRun> {
    Ok(PostProcessingScriptRun {
        id: row
            .try_get("id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        script_id: row
            .try_get("script_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        script_name: row
            .try_get("script_name")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_id: row
            .try_get("title_id")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        title_name: row
            .try_get("title_name")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        facet: row
            .try_get("facet")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        file_path: row
            .try_get("file_path")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        status: {
            let s: String = row
                .try_get("status")
                .map_err(|e| AppError::Repository(e.to_string()))?;
            scryer_domain::ScriptRunStatus::parse(&s)
                .unwrap_or(scryer_domain::ScriptRunStatus::Failed)
        },
        exit_code: row
            .try_get("exit_code")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        stdout_tail: row
            .try_get("stdout_tail")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        stderr_tail: row
            .try_get("stderr_tail")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        duration_ms: row
            .try_get("duration_ms")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        env_payload_json: row
            .try_get("env_payload_json")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        started_at: row
            .try_get("started_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
        completed_at: row
            .try_get("completed_at")
            .map_err(|e| AppError::Repository(e.to_string()))?,
    })
}
