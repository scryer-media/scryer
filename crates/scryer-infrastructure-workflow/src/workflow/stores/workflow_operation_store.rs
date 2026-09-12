use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, JobKey, JobRunRecord, JobRunRepository, MaintenanceSearchJobRunCreation,
    WorkflowOperationInfo, WorkflowOperationRepository,
};
use scryer_domain::{MaintenanceActionJobReceipt, MaintenanceActionJobReceiptState};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

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

    async fn create_maintenance_search_job_run(
        &self,
        run: &JobRunRecord,
        receipt: &MaintenanceActionJobReceipt,
    ) -> AppResult<MaintenanceSearchJobRunCreation> {
        receipt
            .validate_schema()
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if receipt.state != MaintenanceActionJobReceiptState::Accepted
            || receipt.job_run_id.as_deref() != Some(run.id.as_str())
            || run.job_key != JobKey::AcquisitionSearch
        {
            return Err(AppError::Validation(
                "maintenance Search requires an accepted receipt for its acquisition job run"
                    .into(),
            ));
        }

        let run = run.clone();
        let receipt = receipt.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_maintenance_search_job_run",
            move |tx| {
                let run = run.clone();
                let receipt = receipt.clone();
                Box::pin(async move {
                    if let Some(existing) = find_accepted_maintenance_search_receipt(tx, &receipt)
                        .await?
                    {
                        return Ok(MaintenanceSearchJobRunCreation::Existing { receipt: existing });
                    }
                    if let Some(existing) = find_maintenance_search_receipt_for_attempt(tx, &receipt)
                        .await?
                    {
                        return Ok(MaintenanceSearchJobRunCreation::Existing { receipt: existing });
                    }

                    let inserted = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO maintenance_action_job_receipts
                         (candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                          schema_version, logical_request_key, request_hash, job_run_id, state,
                          reconciliation_evidence_json, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT DO NOTHING",
                        &maintenance_search_receipt_args(&receipt),
                    )
                    .await?;
                    if inserted != 1 {
                        let existing = find_accepted_maintenance_search_receipt(tx, &receipt)
                            .await?
                            .or(find_maintenance_search_receipt_for_attempt(tx, &receipt).await?)
                            .ok_or_else(|| {
                                AppError::Repository(
                                    "maintenance Search receipt insert raced without a durable winner".into(),
                                )
                            })?;
                        return Ok(MaintenanceSearchJobRunCreation::Existing { receipt: existing });
                    }

                    let progress_json = json_arg_for_tx(tx, run.progress_json.as_deref())?;
                    let summary_json = json_arg_for_tx(tx, run.summary_json.as_deref())?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO workflow_operations
                         (id, operation_type, status, job_key, trigger_source, actor_user_id,
                          progress_json, summary_json, summary_text, error_text, started_at,
                          completed_at, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                        &[
                            SqlArg::Text(run.id.clone()),
                            SqlArg::Text(run.operation_type.clone()),
                            SqlArg::Text(run.status.as_str().to_string()),
                            SqlArg::Text(run.job_key.as_str().to_string()),
                            SqlArg::Text(run.trigger_source.as_str().to_string()),
                            SqlArg::OptText(run.actor_user_id.clone()),
                            progress_json,
                            summary_json,
                            SqlArg::OptText(run.summary_text.clone()),
                            SqlArg::OptText(run.error_text.clone()),
                            SqlArg::Timestamp(run.started_at),
                            SqlArg::OptTimestamp(run.completed_at),
                            SqlArg::Timestamp(run.created_at),
                            SqlArg::Timestamp(run.updated_at),
                        ],
                    )
                    .await?;
                    let run = fetch_optional_workflow_operation(SqlExec::Tx(tx), &run.id)
                        .await?
                        .ok_or_else(|| {
                            AppError::Repository(
                                "failed to reload atomic maintenance Search job run".into(),
                            )
                        })?;
                    Ok(MaintenanceSearchJobRunCreation::Created {
                        run: Box::new(job_run_record_from_workflow(run)?),
                        receipt,
                    })
                })
            },
        )
        .await
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

fn maintenance_search_receipt_args(receipt: &MaintenanceActionJobReceipt) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(receipt.key.candidate_id.clone()),
        SqlArg::I64(receipt.key.match_generation),
        SqlArg::I64(receipt.key.revision_number),
        SqlArg::Text(receipt.key.step_id.clone()),
        SqlArg::I64(receipt.dispatch_attempt),
        SqlArg::I32(receipt.schema_version as i32),
        SqlArg::Text(receipt.logical_request_key.clone()),
        SqlArg::Text(receipt.request_hash.clone()),
        SqlArg::OptText(receipt.job_run_id.clone()),
        SqlArg::Text(receipt.state.as_storage_str().to_string()),
        SqlArg::Text(receipt.reconciliation_evidence_json.clone()),
        SqlArg::Timestamp(receipt.created_at),
        SqlArg::Timestamp(receipt.updated_at),
    ]
}

async fn find_accepted_maintenance_search_receipt(
    tx: &mut SqlTx<'_>,
    receipt: &MaintenanceActionJobReceipt,
) -> AppResult<Option<MaintenanceActionJobReceipt>> {
    SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                schema_version, logical_request_key, request_hash, job_run_id, state,
                reconciliation_evidence_json, created_at, updated_at
           FROM maintenance_action_job_receipts
          WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
            AND step_id = {} AND request_hash = {} AND state IN ('accepted', 'completed')
          ORDER BY created_at ASC, dispatch_attempt ASC
          LIMIT 1",
        &[
            SqlArg::Text(receipt.key.candidate_id.clone()),
            SqlArg::I64(receipt.key.match_generation),
            SqlArg::I64(receipt.key.revision_number),
            SqlArg::Text(receipt.key.step_id.clone()),
            SqlArg::Text(receipt.request_hash.clone()),
        ],
    )
    .await?
    .as_ref()
    .map(maintenance_search_receipt_from_row)
    .transpose()
}

async fn find_maintenance_search_receipt_for_attempt(
    tx: &mut SqlTx<'_>,
    receipt: &MaintenanceActionJobReceipt,
) -> AppResult<Option<MaintenanceActionJobReceipt>> {
    SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                schema_version, logical_request_key, request_hash, job_run_id, state,
                reconciliation_evidence_json, created_at, updated_at
           FROM maintenance_action_job_receipts
          WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
            AND step_id = {} AND dispatch_attempt = {}
          LIMIT 1",
        &[
            SqlArg::Text(receipt.key.candidate_id.clone()),
            SqlArg::I64(receipt.key.match_generation),
            SqlArg::I64(receipt.key.revision_number),
            SqlArg::Text(receipt.key.step_id.clone()),
            SqlArg::I64(receipt.dispatch_attempt),
        ],
    )
    .await?
    .as_ref()
    .map(maintenance_search_receipt_from_row)
    .transpose()
}

fn maintenance_search_receipt_from_row(row: &SqlRow) -> AppResult<MaintenanceActionJobReceipt> {
    let state =
        MaintenanceActionJobReceiptState::parse_storage(&row.text("state")?).ok_or_else(|| {
            AppError::Repository("maintenance Search receipt has an invalid state".into())
        })?;
    let receipt = MaintenanceActionJobReceipt {
        schema_version: row.i32("schema_version")? as u32,
        key: scryer_domain::MaintenanceActionStepKey {
            candidate_id: row.text("candidate_id")?,
            match_generation: row.i64("match_generation")?,
            revision_number: row.i64("revision_number")?,
            step_id: row.text("step_id")?,
        },
        dispatch_attempt: row.i64("dispatch_attempt")?,
        logical_request_key: row.text("logical_request_key")?,
        request_hash: row.text("request_hash")?,
        job_run_id: row.opt_text("job_run_id")?,
        state,
        reconciliation_evidence_json: row.text("reconciliation_evidence_json")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    };
    receipt
        .validate_schema()
        .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(receipt)
}
