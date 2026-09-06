use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, PostProcessingScriptRepository};
use scryer_domain::{PostProcessingScript, PostProcessingScriptRun};
use scryer_infrastructure_sql::script_output::{
    decode_script_output_tail, encode_script_output_tail,
};
use sqlx::Row;

use crate::postgres::timestamp::{parse_optional_rfc3339_timestamp, parse_rfc3339_timestamp};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};
use crate::storage::sql::json::{canonical_json_arg, json_text_or};

#[derive(Clone)]
pub struct PostProcessingScriptStore {
    datastore: StoreDatastore,
}

impl PostProcessingScriptStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl PostProcessingScriptRepository for PostProcessingScriptStore {
    async fn list_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
        let sql = format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS}
               FROM post_processing_scripts
              ORDER BY priority ASC, name"
        );
        fetch_scripts(self.datastore.read_exec(), &sql, &[]).await
    }

    async fn get_script(&self, id: &str) -> AppResult<Option<PostProcessingScript>> {
        let sql = format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS}
               FROM post_processing_scripts
              WHERE id = {{}}"
        );
        fetch_optional_script(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn create_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        let args = script_args(&script)?;
        execute_write(
            &self.datastore,
            "create_post_processing_script",
            "INSERT INTO post_processing_scripts
                (id, name, description, script_type, script_content, applied_facets,
                 execution_mode, timeout_secs, priority, enabled, debug,
                 created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            args,
        )
        .await?;
        Ok(script)
    }

    async fn update_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        let args = script_args(&script)?;
        execute_write(
            &self.datastore,
            "update_post_processing_script",
            "UPDATE post_processing_scripts
                SET name = {}, description = {}, script_type = {}, script_content = {},
                    applied_facets = {}, execution_mode = {}, timeout_secs = {},
                    priority = {}, enabled = {}, debug = {}, updated_at = {}
              WHERE id = {}",
            vec![
                args[1].clone(),
                args[2].clone(),
                args[3].clone(),
                args[4].clone(),
                args[5].clone(),
                args[6].clone(),
                args[7].clone(),
                args[8].clone(),
                args[9].clone(),
                args[10].clone(),
                args[12].clone(),
                args[0].clone(),
            ],
        )
        .await?;
        Ok(script)
    }

    async fn delete_script(&self, id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_post_processing_script",
            "DELETE FROM post_processing_scripts WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn list_enabled_for_facet(&self, facet: &str) -> AppResult<Vec<PostProcessingScript>> {
        let sql = format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS}
               FROM post_processing_scripts
              WHERE enabled = {{}}
              ORDER BY priority ASC, name"
        );
        let scripts =
            fetch_scripts(self.datastore.read_exec(), &sql, &[SqlArg::Bool(true)]).await?;
        Ok(scripts
            .into_iter()
            .filter(|script| {
                script.applied_facets.is_empty()
                    || script
                        .applied_facets
                        .iter()
                        .any(|candidate| candidate == facet)
            })
            .collect())
    }

    async fn record_run(&self, run: PostProcessingScriptRun) -> AppResult<()> {
        let args = match &self.datastore {
            StoreDatastore::Sqlite { .. } => sqlite_run_args(&run)?,
            StoreDatastore::Postgres { .. } => postgres_run_args(&run)?,
        };
        execute_write(
            &self.datastore,
            "record_post_processing_script_run",
            "INSERT INTO post_processing_script_runs
                (id, script_id, script_name, title_id, title_name, facet, file_path,
                 status, exit_code, stdout_tail, stderr_tail, duration_ms,
                 env_payload_json, started_at, completed_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            args,
        )
        .await
    }

    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        let sql = format!(
            "SELECT {POST_PROCESSING_RUN_COLUMNS}
               FROM post_processing_script_runs
              WHERE script_id = {{}}
              ORDER BY started_at DESC
              LIMIT {{}}"
        );
        fetch_runs(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(script_id.to_string()),
                SqlArg::I64(limit as i64),
            ],
        )
        .await
    }

    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        let sql = format!(
            "SELECT {POST_PROCESSING_RUN_COLUMNS}
               FROM post_processing_script_runs
              WHERE title_id = {{}}
              ORDER BY started_at DESC
              LIMIT {{}}"
        );
        fetch_runs(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I64(limit as i64),
            ],
        )
        .await
    }
}

const POST_PROCESSING_SCRIPT_COLUMNS: &str = "id, name, description, script_type, script_content,
    applied_facets, execution_mode, timeout_secs, priority, enabled, debug, created_at, updated_at";

const POST_PROCESSING_RUN_COLUMNS: &str = "id, script_id, script_name, title_id, title_name, facet,
    file_path, status, exit_code, stdout_tail, stderr_tail, duration_ms, env_payload_json,
    started_at, completed_at";

async fn fetch_scripts(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<PostProcessingScript>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(row_to_script)
        .collect()
}

async fn fetch_optional_script(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<PostProcessingScript>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .as_ref()
        .map(row_to_script)
        .transpose()
}

async fn fetch_runs(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<PostProcessingScriptRun>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(row_to_run)
        .collect()
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: &'static str,
    args: Vec<SqlArg>,
) -> AppResult<()> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let args = args.clone();
        Box::pin(async move {
            SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await?;
            Ok(())
        })
    })
    .await
}

fn script_args(script: &PostProcessingScript) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(script.id.clone()),
        SqlArg::Text(script.name.clone()),
        SqlArg::Text(script.description.clone()),
        SqlArg::Text(script.script_type.as_str().to_string()),
        SqlArg::Text(script.script_content.clone()),
        canonical_json_arg(&script.applied_facets)?,
        SqlArg::Text(script.execution_mode.as_str().to_string()),
        SqlArg::I64(script.timeout_secs),
        SqlArg::I32(script.priority),
        SqlArg::Bool(script.enabled),
        SqlArg::Bool(script.debug),
        SqlArg::Timestamp(script.created_at),
        SqlArg::Timestamp(script.updated_at),
    ])
}

fn sqlite_run_args(run: &PostProcessingScriptRun) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(run.id.clone()),
        SqlArg::Text(run.script_id.clone()),
        SqlArg::Text(run.script_name.clone()),
        SqlArg::OptText(run.title_id.clone()),
        SqlArg::OptText(run.title_name.clone()),
        SqlArg::OptText(run.facet.clone()),
        SqlArg::OptText(run.file_path.clone()),
        SqlArg::Text(run.status.as_str().to_string()),
        SqlArg::OptI32(run.exit_code),
        SqlArg::OptBytes(encode_output_tail(run.stdout_tail.as_deref())?),
        SqlArg::OptBytes(encode_output_tail(run.stderr_tail.as_deref())?),
        SqlArg::OptI64(run.duration_ms),
        SqlArg::OptText(run.env_payload_json.clone()),
        SqlArg::Text(run.started_at.clone()),
        SqlArg::OptText(run.completed_at.clone()),
    ])
}

/// Output tails are stored as zstd frames; see
/// [`scryer_infrastructure_sql::script_output`].
fn encode_output_tail(tail: Option<&str>) -> AppResult<Option<Vec<u8>>> {
    tail.map(|text| encode_script_output_tail(text).map_err(repo_err))
        .transpose()
}

fn decode_output_tail(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    row.opt_bytes(column)?
        .as_deref()
        .map(|bytes| decode_script_output_tail(bytes).map_err(repo_err))
        .transpose()
}

fn postgres_run_args(run: &PostProcessingScriptRun) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(run.id.clone()),
        SqlArg::Text(run.script_id.clone()),
        SqlArg::Text(run.script_name.clone()),
        SqlArg::OptText(run.title_id.clone()),
        SqlArg::OptText(run.title_name.clone()),
        SqlArg::OptText(run.facet.clone()),
        SqlArg::OptText(run.file_path.clone()),
        SqlArg::Text(run.status.as_str().to_string()),
        SqlArg::OptI32(run.exit_code),
        SqlArg::OptBytes(encode_output_tail(run.stdout_tail.as_deref())?),
        SqlArg::OptBytes(encode_output_tail(run.stderr_tail.as_deref())?),
        SqlArg::OptI64(run.duration_ms),
        SqlArg::OptText(run.env_payload_json.clone()),
        SqlArg::Timestamp(parse_rfc3339_timestamp(
            &run.started_at,
            "post_processing_script_runs.started_at",
        )?),
        SqlArg::OptTimestamp(parse_optional_rfc3339_timestamp(
            run.completed_at.as_deref(),
            "post_processing_script_runs.completed_at",
        )?),
    ])
}

fn row_to_script(row: &SqlRow) -> AppResult<PostProcessingScript> {
    let script_type_raw = row.text("script_type")?;
    let execution_mode_raw = row.text("execution_mode")?;
    Ok(PostProcessingScript {
        id: row.text("id")?,
        name: row.text("name")?,
        description: row.text("description")?,
        script_type: scryer_domain::ScriptType::parse(&script_type_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid script_type: {script_type_raw}"))
        })?,
        script_content: row.text("script_content")?,
        applied_facets: applied_facets(row)?,
        execution_mode: scryer_domain::ExecutionMode::parse(&execution_mode_raw).ok_or_else(
            || AppError::Repository(format!("invalid execution_mode: {execution_mode_raw}")),
        )?,
        timeout_secs: row.i64("timeout_secs")?,
        priority: row.i32("priority")?,
        enabled: row.bool("enabled")?,
        debug: row.bool("debug")?,
        created_at: timestamp_or_now(row, "created_at")?,
        updated_at: timestamp_or_now(row, "updated_at")?,
    })
}

fn row_to_run(row: &SqlRow) -> AppResult<PostProcessingScriptRun> {
    let status_raw = row.text("status")?;
    Ok(PostProcessingScriptRun {
        id: row.text("id")?,
        script_id: row.text("script_id")?,
        script_name: row.text("script_name")?,
        title_id: row.opt_text("title_id")?,
        title_name: row.opt_text("title_name")?,
        facet: row.opt_text("facet")?,
        file_path: row.opt_text("file_path")?,
        status: scryer_domain::ScriptRunStatus::parse(&status_raw)
            .unwrap_or(scryer_domain::ScriptRunStatus::Failed),
        exit_code: row.opt_i32("exit_code")?,
        stdout_tail: decode_output_tail(row, "stdout_tail")?,
        stderr_tail: decode_output_tail(row, "stderr_tail")?,
        duration_ms: row.opt_i64("duration_ms")?,
        env_payload_json: row.opt_text("env_payload_json")?,
        started_at: timestamp_text(row, "started_at")?,
        completed_at: optional_timestamp_text(row, "completed_at")?,
    })
}

fn applied_facets(row: &SqlRow) -> AppResult<Vec<String>> {
    let raw = json_text_or(row, "applied_facets", "[]")?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn timestamp_or_now(row: &SqlRow, column: &str) -> AppResult<DateTime<Utc>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            Ok(DateTime::parse_from_rfc3339(&raw)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()))
        }
        SqlRow::Postgres(_) => row.timestamp(column),
    }
}

fn timestamp_text(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(_) => row.text(column),
        SqlRow::Postgres(_) => row.timestamp(column).map(|value| value.to_rfc3339()),
    }
}

fn optional_timestamp_text(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(_) => row.opt_text(column),
        SqlRow::Postgres(_) => row
            .opt_timestamp(column)
            .map(|value| value.map(|value| value.to_rfc3339())),
    }
}
