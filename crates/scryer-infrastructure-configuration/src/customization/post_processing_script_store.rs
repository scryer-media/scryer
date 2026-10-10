use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, PostProcessingScriptRepository};
use scryer_domain::{
    PostProcessingScript, PostProcessingScriptRun, ScriptLanguage, ScriptSchedule, ScriptTrigger,
};
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
                 created_at, updated_at, language, trigger, schedule_json, run_on_startup)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
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
                    priority = {}, enabled = {}, debug = {}, updated_at = {},
                    language = {}, trigger = {}, schedule_json = {}, run_on_startup = {}
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
                args[13].clone(),
                args[14].clone(),
                args[15].clone(),
                args[16].clone(),
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
                AND trigger = {{}}
              ORDER BY priority ASC, name"
        );
        let scripts = fetch_scripts(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Bool(true),
                SqlArg::Text(ScriptTrigger::PostImport.as_str().to_string()),
            ],
        )
        .await?;
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

    async fn list_enabled_scheduled(&self) -> AppResult<Vec<PostProcessingScript>> {
        let sql = format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS}
               FROM post_processing_scripts
              WHERE enabled = {{}}
                AND trigger = {{}}
              ORDER BY name"
        );
        fetch_scripts(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Bool(true),
                SqlArg::Text(ScriptTrigger::Schedule.as_str().to_string()),
            ],
        )
        .await
    }

    async fn list_scripts_by_trigger(
        &self,
        trigger: ScriptTrigger,
    ) -> AppResult<Vec<PostProcessingScript>> {
        let sql = format!(
            "SELECT {POST_PROCESSING_SCRIPT_COLUMNS}
               FROM post_processing_scripts
              WHERE trigger = {{}}
              ORDER BY priority ASC, name"
        );
        fetch_scripts(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(trigger.as_str().to_string())],
        )
        .await
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

    async fn update_run(&self, run: PostProcessingScriptRun) -> AppResult<()> {
        let mut args = match &self.datastore {
            StoreDatastore::Sqlite { .. } => sqlite_run_args(&run)?,
            StoreDatastore::Postgres { .. } => postgres_run_args(&run)?,
        };
        // The insert arguments lead with the id; the update binds it last.
        let id = args.remove(0);
        args.push(id);
        execute_write(
            &self.datastore,
            "update_post_processing_script_run",
            "UPDATE post_processing_script_runs
                SET script_id = {}, script_name = {}, title_id = {}, title_name = {},
                    facet = {}, file_path = {}, status = {}, exit_code = {},
                    stdout_tail = {}, stderr_tail = {}, duration_ms = {},
                    env_payload_json = {}, started_at = {}, completed_at = {}
              WHERE id = {}",
            args,
        )
        .await
    }

    async fn reconcile_interrupted_runs(&self) -> AppResult<u64> {
        let sql = format!(
            "SELECT {POST_PROCESSING_RUN_COLUMNS}
               FROM post_processing_script_runs
              WHERE status = {{}}"
        );
        let running = fetch_runs(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(
                scryer_domain::ScriptRunStatus::Running.as_str().to_string(),
            )],
        )
        .await?;
        let count = running.len() as u64;
        let completed_at = Utc::now().to_rfc3339();
        for run in running {
            let stderr_tail = Some(match run.stderr_tail.as_deref() {
                Some(tail) if !tail.is_empty() => {
                    format!("{tail}\n{INTERRUPTED_SCRIPT_RUN_SUMMARY}")
                }
                _ => INTERRUPTED_SCRIPT_RUN_SUMMARY.to_string(),
            });
            self.update_run(PostProcessingScriptRun {
                status: scryer_domain::ScriptRunStatus::Failed,
                stderr_tail,
                completed_at: Some(completed_at.clone()),
                ..run
            })
            .await?;
        }
        Ok(count)
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
    applied_facets, execution_mode, timeout_secs, priority, enabled, debug, created_at, updated_at,
    language, trigger, schedule_json, run_on_startup";

const POST_PROCESSING_RUN_COLUMNS: &str = "id, script_id, script_name, title_id, title_name, facet,
    file_path, status, exit_code, stdout_tail, stderr_tail, duration_ms, env_payload_json,
    started_at, completed_at";

async fn fetch_scripts(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<PostProcessingScript>> {
    // One undecodable row (a language or trigger written by a newer build,
    // say) must not hide every other script, so list paths skip it.
    Ok(SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .filter_map(|row| match row_to_script(row) {
            Ok(script) => Some(script),
            Err(error) => {
                tracing::warn!(
                    script_id = row.text("id").ok().as_deref().unwrap_or("<unknown>"),
                    error = %error,
                    "skipping undecodable post-processing script row"
                );
                None
            }
        })
        .collect())
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
        SqlArg::Text(script.language.as_str().to_string()),
        SqlArg::Text(script.trigger.as_str().to_string()),
        SqlArg::OptText(
            script
                .schedule
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(repo_err)?,
        ),
        SqlArg::Bool(script.run_on_startup),
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

/// What a script run left `running` by a previous process records as its
/// outcome when the host starts again.
const INTERRUPTED_SCRIPT_RUN_SUMMARY: &str = "interrupted by restart";

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
    let language_raw = row.text("language")?;
    let trigger_raw = row.text("trigger")?;
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
        language: ScriptLanguage::parse(&language_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid script language: {language_raw}"))
        })?,
        trigger: ScriptTrigger::parse(&trigger_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid script trigger: {trigger_raw}"))
        })?,
        schedule: schedule(row)?,
        run_on_startup: row.bool("run_on_startup")?,
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

/// An undecodable schedule reads as no schedule, so the script stays visible
/// for editing but is never runnable.
fn schedule(row: &SqlRow) -> AppResult<Option<ScriptSchedule>> {
    let Some(raw) = row
        .opt_text("schedule_json")?
        .filter(|raw| !raw.trim().is_empty())
    else {
        return Ok(None);
    };
    match serde_json::from_str(&raw) {
        Ok(schedule) => Ok(Some(schedule)),
        Err(error) => {
            tracing::warn!(
                script_id = row.text("id").ok().as_deref().unwrap_or("<unknown>"),
                error = %error,
                "ignoring undecodable script schedule"
            );
            Ok(None)
        }
    }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use scryer_domain::{ExecutionMode, ScheduleWeekday, ScriptType};
    use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

    use super::*;

    const SCRIPT_MIGRATIONS: &[&str] = &[
        include_str!("../../../scryer/src/db/migrations/0051_post_processing_scripts.sql"),
        include_str!("../../../scryer/src/db/migrations/0282_script_triggers.sql"),
    ];

    async fn store() -> (PostProcessingScriptStore, SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        for migration in SCRIPT_MIGRATIONS {
            for statement in migration
                .split(';')
                .map(str::trim)
                .filter(|sql| !sql.is_empty())
            {
                sqlx::query(statement)
                    .execute(&pool)
                    .await
                    .expect("script migration should apply");
            }
        }
        (
            PostProcessingScriptStore::new(StoreDatastore::sqlite(
                pool.clone(),
                Arc::new(tokio::sync::Mutex::new(())),
            )),
            pool,
        )
    }

    fn script(id: &str, trigger: ScriptTrigger) -> PostProcessingScript {
        let now = Utc::now();
        PostProcessingScript {
            id: id.to_string(),
            name: format!("fixture {id}"),
            description: String::new(),
            script_type: ScriptType::Inline,
            script_content: "echo fixture".to_string(),
            applied_facets: vec![],
            execution_mode: ExecutionMode::Blocking,
            timeout_secs: 60,
            priority: 0,
            enabled: true,
            debug: false,
            language: ScriptLanguage::Shell,
            trigger,
            schedule: None,
            run_on_startup: false,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn new_trigger_fields_round_trip_through_create_and_update() {
        let (store, _pool) = store().await;
        let mut scheduled = script("scheduled", ScriptTrigger::Schedule);
        scheduled.language = ScriptLanguage::Python;
        scheduled.schedule = Some(ScriptSchedule::Weekly {
            days: vec![ScheduleWeekday::Monday, ScheduleWeekday::Friday],
            time_local: "03:30".to_string(),
        });
        scheduled.run_on_startup = true;
        store.create_script(scheduled).await.expect("create");

        let loaded = store
            .get_script("scheduled")
            .await
            .expect("get")
            .expect("script exists");
        assert_eq!(loaded.language, ScriptLanguage::Python);
        assert_eq!(loaded.trigger, ScriptTrigger::Schedule);
        assert_eq!(
            loaded.schedule,
            Some(ScriptSchedule::Weekly {
                days: vec![ScheduleWeekday::Monday, ScheduleWeekday::Friday],
                time_local: "03:30".to_string(),
            })
        );
        assert!(loaded.run_on_startup);

        let mut updated = loaded;
        updated.language = ScriptLanguage::Go;
        updated.schedule = Some(ScriptSchedule::Cron {
            expression: "*/15 * * * *".to_string(),
        });
        updated.run_on_startup = false;
        store.update_script(updated).await.expect("update");

        let reloaded = store
            .get_script("scheduled")
            .await
            .expect("get")
            .expect("script exists");
        assert_eq!(reloaded.language, ScriptLanguage::Go);
        assert_eq!(
            reloaded.schedule,
            Some(ScriptSchedule::Cron {
                expression: "*/15 * * * *".to_string(),
            })
        );
        assert!(!reloaded.run_on_startup);
    }

    #[tokio::test]
    async fn rows_written_before_trigger_columns_read_as_import_shell_scripts() {
        let (store, pool) = store().await;
        sqlx::query(
            "INSERT INTO post_processing_scripts
                (id, name, script_type, script_content, created_at, updated_at)
             VALUES ('legacy', 'fixture legacy', 'inline', 'echo fixture',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert legacy row");

        let loaded = store
            .get_script("legacy")
            .await
            .expect("get")
            .expect("script exists");
        assert_eq!(loaded.language, ScriptLanguage::Shell);
        assert_eq!(loaded.trigger, ScriptTrigger::PostImport);
        assert_eq!(loaded.schedule, None);
        assert!(!loaded.run_on_startup);
    }

    #[tokio::test]
    async fn facet_listing_excludes_scheduled_scripts() {
        let (store, _pool) = store().await;
        store
            .create_script(script("import", ScriptTrigger::PostImport))
            .await
            .expect("create import script");
        let mut scheduled = script("scheduled", ScriptTrigger::Schedule);
        scheduled.schedule = Some(ScriptSchedule::Manual);
        store.create_script(scheduled).await.expect("create");
        let mut disabled = script("disabled-scheduled", ScriptTrigger::Schedule);
        disabled.enabled = false;
        store.create_script(disabled).await.expect("create");

        let for_facet = store.list_enabled_for_facet("movie").await.expect("facet");
        assert_eq!(
            for_facet.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["import"]
        );

        let scheduled = store.list_enabled_scheduled().await.expect("scheduled");
        assert_eq!(
            scheduled.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["scheduled"]
        );

        let mut by_trigger = store
            .list_scripts_by_trigger(ScriptTrigger::Schedule)
            .await
            .expect("by trigger")
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        by_trigger.sort();
        assert_eq!(by_trigger, vec!["disabled-scheduled", "scheduled"]);
    }

    #[tokio::test]
    async fn undecodable_rows_never_become_runnable_or_hide_other_scripts() {
        let (store, pool) = store().await;
        let mut valid = script("valid", ScriptTrigger::Schedule);
        valid.schedule = Some(ScriptSchedule::Manual);
        store.create_script(valid).await.expect("create");
        sqlx::query(
            "INSERT INTO post_processing_scripts
                (id, name, script_type, script_content, created_at, updated_at,
                 trigger, schedule_json)
             VALUES ('bogus-schedule', 'fixture bogus', 'inline', 'echo fixture',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     'schedule', '{\"kind\":\"bogus\"}')",
        )
        .execute(&pool)
        .await
        .expect("insert bogus schedule row");
        sqlx::query(
            "INSERT INTO post_processing_scripts
                (id, name, script_type, script_content, created_at, updated_at,
                 trigger)
             VALUES ('unknown-trigger', 'fixture unknown', 'inline', 'echo fixture',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'on_full_moon')",
        )
        .execute(&pool)
        .await
        .expect("insert unknown trigger row");

        let bogus = store
            .get_script("bogus-schedule")
            .await
            .expect("get")
            .expect("script exists");
        assert_eq!(bogus.schedule, None);

        let scheduled = store.list_enabled_scheduled().await.expect("scheduled");
        let mut runnable = scheduled
            .iter()
            .filter(|script| script.schedule.is_some())
            .map(|script| script.id.as_str())
            .collect::<Vec<_>>();
        runnable.sort();
        assert_eq!(runnable, vec!["valid"]);

        let mut all = store
            .list_scripts()
            .await
            .expect("list")
            .into_iter()
            .map(|script| script.id)
            .collect::<Vec<_>>();
        all.sort();
        assert_eq!(all, vec!["bogus-schedule", "valid"]);
    }

    #[tokio::test]
    async fn a_running_run_is_finished_in_place_by_update_run() {
        let (store, _pool) = store().await;
        let mut scheduled = script("scheduled", ScriptTrigger::Schedule);
        scheduled.schedule = Some(ScriptSchedule::Manual);
        store.create_script(scheduled).await.expect("create");
        let started_at = Utc::now().to_rfc3339();
        let running = PostProcessingScriptRun {
            id: "run-1".to_string(),
            script_id: "scheduled".to_string(),
            script_name: "fixture scheduled".to_string(),
            title_id: None,
            title_name: None,
            facet: None,
            file_path: None,
            status: scryer_domain::ScriptRunStatus::Running,
            exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            duration_ms: None,
            env_payload_json: Some("{}".to_string()),
            started_at: started_at.clone(),
            completed_at: None,
        };
        store.record_run(running.clone()).await.expect("record");
        let recorded = store
            .list_runs_for_script("scheduled", 10)
            .await
            .expect("list");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].status, scryer_domain::ScriptRunStatus::Running);

        let finished = PostProcessingScriptRun {
            status: scryer_domain::ScriptRunStatus::Success,
            exit_code: Some(0),
            stdout_tail: Some("fixture output".to_string()),
            duration_ms: Some(12),
            completed_at: Some(Utc::now().to_rfc3339()),
            ..running
        };
        store.update_run(finished).await.expect("update");

        let runs = store
            .list_runs_for_script("scheduled", 10)
            .await
            .expect("list");
        assert_eq!(runs.len(), 1, "the update replaces the row, never adds one");
        assert_eq!(runs[0].id, "run-1");
        assert_eq!(runs[0].status, scryer_domain::ScriptRunStatus::Success);
        assert_eq!(runs[0].exit_code, Some(0));
        assert_eq!(runs[0].stdout_tail.as_deref(), Some("fixture output"));
        assert_eq!(runs[0].duration_ms, Some(12));
        assert!(runs[0].completed_at.is_some());
    }

    #[tokio::test]
    async fn runs_left_running_are_failed_as_interrupted() {
        let (store, _pool) = store().await;
        let mut scheduled = script("scheduled", ScriptTrigger::Schedule);
        scheduled.schedule = Some(ScriptSchedule::Manual);
        store.create_script(scheduled).await.expect("create");
        let run = |id: &str, status: scryer_domain::ScriptRunStatus| PostProcessingScriptRun {
            id: id.to_string(),
            script_id: "scheduled".to_string(),
            script_name: "fixture scheduled".to_string(),
            title_id: None,
            title_name: None,
            facet: None,
            file_path: None,
            status,
            exit_code: None,
            stdout_tail: Some("partial output".to_string()),
            stderr_tail: None,
            duration_ms: None,
            env_payload_json: Some("{}".to_string()),
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
        };
        store
            .record_run(run("left-running", scryer_domain::ScriptRunStatus::Running))
            .await
            .expect("record running");
        let finished = PostProcessingScriptRun {
            exit_code: Some(0),
            completed_at: Some(Utc::now().to_rfc3339()),
            ..run("finished", scryer_domain::ScriptRunStatus::Success)
        };
        store.record_run(finished).await.expect("record finished");

        assert_eq!(
            store.reconcile_interrupted_runs().await.expect("reconcile"),
            1
        );
        assert_eq!(
            store
                .reconcile_interrupted_runs()
                .await
                .expect("reconcile again"),
            0,
            "nothing is left running"
        );

        let runs = store
            .list_runs_for_script("scheduled", 10)
            .await
            .expect("list");
        let by_id = |id: &str| runs.iter().find(|run| run.id == id).expect("run");
        let interrupted = by_id("left-running");
        assert_eq!(interrupted.status, scryer_domain::ScriptRunStatus::Failed);
        assert_eq!(
            interrupted.stderr_tail.as_deref(),
            Some(INTERRUPTED_SCRIPT_RUN_SUMMARY)
        );
        assert_eq!(interrupted.stdout_tail.as_deref(), Some("partial output"));
        assert!(interrupted.completed_at.is_some());
        let untouched = by_id("finished");
        assert_eq!(untouched.status, scryer_domain::ScriptRunStatus::Success);
        assert_eq!(untouched.stderr_tail, None);
    }
}
