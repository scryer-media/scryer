use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, JobKey, JobRunRecord, JobRunRepository, WorkflowOperationInfo,
    WorkflowOperationRepository,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct WorkflowOperationStore {
    datastore: StoreDatastore,
}

impl WorkflowOperationStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl WorkflowOperationRepository for WorkflowOperationStore {
    async fn create_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo> {
        let record = create_workflow_operation(
            &self.datastore,
            NewWorkflowOperation {
                operation_type,
                status,
                job_key: None,
                trigger_source: None,
                actor_user_id,
                progress_json,
                summary_json: None,
                summary_text: None,
                error_text: None,
                started_at,
                completed_at,
            },
        )
        .await?;
        Ok(workflow_operation_info(record))
    }
}

#[async_trait]
impl JobRunRepository for WorkflowOperationStore {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let record = create_workflow_operation(
            &self.datastore,
            NewWorkflowOperation {
                operation_type: run.operation_type.clone(),
                status: run.status.as_str().to_string(),
                job_key: Some(run.job_key.as_str().to_string()),
                trigger_source: Some(run.trigger_source.as_str().to_string()),
                actor_user_id: run.actor_user_id.clone(),
                progress_json: run.progress_json.clone(),
                summary_json: run.summary_json.clone(),
                summary_text: run.summary_text.clone(),
                error_text: run.error_text.clone(),
                started_at: Some(run.started_at.to_rfc3339()),
                completed_at: run.completed_at.map(|value| value.to_rfc3339()),
            },
        )
        .await?;
        job_run_record_from_workflow(record)
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let id = run.id.clone();
        let status = run.status.as_str().to_string();
        let progress_json = run.progress_json.clone();
        let summary_json = run.summary_json.clone();
        let summary_text = run.summary_text.clone();
        let error_text = run.error_text.clone();
        let completed_at = run.completed_at.map(|value| value.to_rfc3339());
        let record = SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_job_workflow_operation",
            move |tx| {
                let id = id.clone();
                let status = status.clone();
                let progress_json = progress_json.clone();
                let summary_json = summary_json.clone();
                let summary_text = summary_text.clone();
                let error_text = error_text.clone();
                let completed_at = completed_at.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    let progress_arg = json_arg_for_tx(tx, progress_json.as_deref())?;
                    let summary_arg = json_arg_for_tx(tx, summary_json.as_deref())?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE workflow_operations
                         SET status = {},
                             progress_json = {},
                             summary_json = {},
                             summary_text = {},
                             error_text = {},
                             completed_at = {},
                             updated_at = {}
                         WHERE id = {}",
                        &[
                            SqlArg::Text(status),
                            progress_arg,
                            summary_arg,
                            SqlArg::OptText(summary_text),
                            SqlArg::OptText(error_text),
                            opt_timestamp_arg(completed_at.as_deref()),
                            SqlArg::Timestamp(now),
                            SqlArg::Text(id.clone()),
                        ],
                    )
                    .await?;
                    fetch_optional_workflow_operation(SqlExec::Tx(tx), &id)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("workflow operation {id}")))
                })
            },
        )
        .await?;
        job_run_record_from_workflow(record)
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        fetch_optional_workflow_operation(self.datastore.read_exec(), run_id)
            .await?
            .map(job_run_record_from_workflow)
            .transpose()
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        let limit = limit as i64;
        let (sql, args) = if let Some(job_key) = job_key {
            (
                "SELECT * FROM workflow_operations WHERE job_key = {} ORDER BY started_at DESC LIMIT {}",
                vec![
                    SqlArg::Text(job_key.as_str().to_string()),
                    SqlArg::I64(limit),
                ],
            )
        } else {
            (
                "SELECT * FROM workflow_operations WHERE job_key IS NOT NULL ORDER BY started_at DESC LIMIT {}",
                vec![SqlArg::I64(limit)],
            )
        };
        SqlRuntime::fetch_all(self.datastore.read_exec(), sql, &args)
            .await?
            .into_iter()
            .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
            .collect()
    }

    async fn list_job_runs_for_actor(
        &self,
        job_key: Option<JobKey>,
        actor_user_id: &str,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        let limit = limit as i64;
        let (sql, args) = if let Some(job_key) = job_key {
            (
                "SELECT * FROM workflow_operations WHERE job_key = {} AND actor_user_id = {} ORDER BY started_at DESC LIMIT {}",
                vec![
                    SqlArg::Text(job_key.as_str().to_string()),
                    SqlArg::Text(actor_user_id.to_string()),
                    SqlArg::I64(limit),
                ],
            )
        } else {
            (
                "SELECT * FROM workflow_operations WHERE job_key IS NOT NULL AND actor_user_id = {} ORDER BY started_at DESC LIMIT {}",
                vec![SqlArg::Text(actor_user_id.to_string()), SqlArg::I64(limit)],
            )
        };
        SqlRuntime::fetch_all(self.datastore.read_exec(), sql, &args)
            .await?
            .into_iter()
            .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
            .collect()
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT * FROM workflow_operations
             WHERE job_key IS NOT NULL
               AND status IN ('queued', 'running', 'discovering')
             ORDER BY started_at ASC",
            &[],
        )
        .await?
        .into_iter()
        .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
        .collect()
    }

    async fn reconcile_interrupted_job_runs(&self, excluded_run_ids: &[String]) -> AppResult<u64> {
        reconcile_interrupted_job_runs(&self.datastore, excluded_run_ids).await
    }
}
