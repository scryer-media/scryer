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
    encryption_key: std::sync::Arc<std::sync::RwLock<Option<crate::encryption::EncryptionKey>>>,
}

impl WorkflowOperationStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self {
            datastore,
            encryption_key: Default::default(),
        }
    }

    pub fn with_encryption_key(
        mut self,
        key: std::sync::Arc<std::sync::RwLock<Option<crate::encryption::EncryptionKey>>>,
    ) -> Self {
        self.encryption_key = key;
        self
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
    async fn accept_private_job(
        &self,
        run: &JobRunRecord,
        request_key: &str,
        input: &str,
    ) -> AppResult<(JobRunRecord, bool)> {
        let key = crate::config_store::current_encryption_key(&self.encryption_key)?;
        let encrypted =
            crate::config_store::encrypt_value(key.as_ref(), input, "job private input", true)?;
        let run = run.clone();
        let request_key = request_key.to_owned();
        SqlRuntime::run_in_transaction(&self.datastore, "accept_private_job", move |tx| {
            let run = run.clone(); let request_key = request_key.clone(); let encrypted = encrypted.clone();
            Box::pin(async move {
                let inserted = SqlRuntime::execute(SqlExec::Tx(tx),
                    "INSERT INTO workflow_private_inputs (run_id, actor_user_id, request_key, input_encrypted) VALUES ({}, {}, {}, {}) ON CONFLICT (actor_user_id, request_key) DO NOTHING",
                    &[SqlArg::Text(run.id.clone()), SqlArg::OptText(run.actor_user_id.clone()), SqlArg::Text(request_key.clone()), SqlArg::Text(encrypted)]).await?;
                if inserted == 0 {
                    let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx), "SELECT run_id FROM workflow_private_inputs WHERE actor_user_id = {} AND request_key = {}",
                        &[SqlArg::OptText(run.actor_user_id.clone()), SqlArg::Text(request_key)]).await?.ok_or_else(|| AppError::Repository("private job receipt missing".into()))?;
                    let record = fetch_optional_workflow_operation(SqlExec::Tx(tx), &row.text("run_id")?).await?.ok_or_else(|| AppError::Repository("private job missing".into()))?;
                    return Ok((job_run_record_from_workflow(record)?, false));
                }
                let progress = json_arg_for_tx(tx, run.progress_json.as_deref())?;
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "INSERT INTO workflow_operations (id, operation_type, status, job_key, trigger_source, actor_user_id, progress_json, started_at, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &[SqlArg::Text(run.id.clone()), SqlArg::Text(run.operation_type.clone()), SqlArg::Text(run.status.as_str().into()), SqlArg::Text(run.job_key.as_str().into()), SqlArg::Text(run.trigger_source.as_str().into()), SqlArg::OptText(run.actor_user_id.clone()), progress, SqlArg::Timestamp(run.started_at), SqlArg::Timestamp(run.created_at), SqlArg::Timestamp(run.updated_at)]).await?;
                Ok((run, true))
            })
        }).await
    }

    async fn private_job_input(&self, run_id: &str, actor_id: &str) -> AppResult<Option<String>> {
        let key = crate::config_store::current_encryption_key(&self.encryption_key)?;
        SqlRuntime::fetch_optional(self.datastore.read_exec(), "SELECT input_encrypted FROM workflow_private_inputs WHERE run_id = {} AND actor_user_id = {} AND input_encrypted IS NOT NULL", &[SqlArg::Text(run_id.into()), SqlArg::Text(actor_id.into())]).await?
            .map(|row| crate::config_store::decrypt_value(key.as_ref(), row.text("input_encrypted")?, "job private input", true)).transpose()
    }

    async fn update_private_job_input(
        &self,
        run_id: &str,
        actor_id: &str,
        input: &str,
    ) -> AppResult<()> {
        let key = crate::config_store::current_encryption_key(&self.encryption_key)?;
        let encrypted =
            crate::config_store::encrypt_value(key.as_ref(), input, "job private input", true)?;
        let count = SqlRuntime::execute_write(&self.datastore, "update_private_job_input", "UPDATE workflow_private_inputs SET input_encrypted = {} WHERE run_id = {} AND actor_user_id = {} AND input_encrypted IS NOT NULL", vec![SqlArg::Text(encrypted), SqlArg::Text(run_id.into()), SqlArg::Text(actor_id.into())]).await?;
        if count != 1 {
            return Err(AppError::Repository("private job input unavailable".into()));
        }
        Ok(())
    }

    async fn purge_private_job_input(&self, run_id: &str) -> AppResult<()> {
        // The receipt row stays so a replayed request key still finds its run.
        SqlRuntime::execute_write(
            &self.datastore,
            "purge_private_job_input",
            "UPDATE workflow_private_inputs SET input_encrypted = NULL WHERE run_id = {} AND input_encrypted IS NOT NULL",
            vec![SqlArg::Text(run_id.into())],
        )
        .await?;
        Ok(())
    }

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

    async fn list_unresolved_private_jobs(&self, job_key: JobKey) -> AppResult<Vec<JobRunRecord>> {
        SqlRuntime::fetch_all(self.datastore.read_exec(), "SELECT workflow_operations.* FROM workflow_operations INNER JOIN workflow_private_inputs ON workflow_operations.id = workflow_private_inputs.run_id WHERE job_key = {} AND status != 'completed' AND workflow_private_inputs.input_encrypted IS NOT NULL ORDER BY started_at ASC LIMIT 1000", &[SqlArg::Text(job_key.as_str().into())]).await?.into_iter().map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow)).collect()
    }

    async fn find_private_job(
        &self,
        actor_id: &str,
        request_key: &str,
    ) -> AppResult<Option<JobRunRecord>> {
        SqlRuntime::fetch_optional(self.datastore.read_exec(), "SELECT workflow_operations.* FROM workflow_operations INNER JOIN workflow_private_inputs ON workflow_operations.id = workflow_private_inputs.run_id WHERE workflow_private_inputs.actor_user_id = {} AND request_key = {}", &[SqlArg::Text(actor_id.into()), SqlArg::Text(request_key.into())]).await?.map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow)).transpose()
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
            return Err(AppError::validation_refused(
                crate::error_reasons::WORKFLOW_ACQUISITION_RECEIPT_REQUIRED,
                "maintenance Search requires an accepted receipt for its acquisition job run",
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
                        .ok_or_else(|| {
                            AppError::not_found_reason(
                                crate::error_reasons::WORKFLOW_OPERATION_NOT_FOUND,
                                format!("workflow operation {id}"),
                            )
                        })
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

    async fn list_job_runs_by_operation_type(
        &self,
        job_key: JobKey,
        operation_type: &str,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT * FROM workflow_operations
             WHERE job_key = {} AND operation_type = {}
             ORDER BY started_at DESC, id DESC LIMIT {}",
            &[
                SqlArg::Text(job_key.as_str().to_string()),
                SqlArg::Text(operation_type.to_string()),
                SqlArg::I64(limit as i64),
            ],
        )
        .await?
        .into_iter()
        .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
        .collect()
    }

    async fn list_job_runs_by_operation_type_before(
        &self,
        job_key: JobKey,
        operation_type: &str,
        before: Option<(chrono::DateTime<chrono::Utc>, String)>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        let mut sql =
            "SELECT * FROM workflow_operations WHERE job_key = {} AND operation_type = {}"
                .to_owned();
        let mut args = vec![
            SqlArg::Text(job_key.as_str().to_owned()),
            SqlArg::Text(operation_type.to_owned()),
        ];
        if let Some((started_at, id)) = before {
            sql.push_str(" AND (started_at < {} OR (started_at = {} AND id < {}))");
            args.extend([
                SqlArg::Timestamp(started_at),
                SqlArg::Timestamp(started_at),
                SqlArg::Text(id),
            ]);
        }
        sql.push_str(" ORDER BY started_at DESC, id DESC LIMIT {}");
        args.push(SqlArg::I64(limit.min(i64::MAX as usize) as i64));
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .into_iter()
            .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
            .collect()
    }

    async fn list_latest_job_run_per_operation_type(
        &self,
        job_key: JobKey,
    ) -> AppResult<Vec<JobRunRecord>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT * FROM (
                 SELECT workflow_operations.*,
                        ROW_NUMBER() OVER (
                            PARTITION BY operation_type
                            ORDER BY started_at DESC, id DESC
                        ) AS operation_rank
                 FROM workflow_operations
                 WHERE job_key = {}
             ) ranked
             WHERE operation_rank = 1
             ORDER BY started_at DESC, id DESC",
            &[SqlArg::Text(job_key.as_str().to_string())],
        )
        .await?
        .into_iter()
        .map(|row| workflow_operation_from_row(&row).and_then(job_run_record_from_workflow))
        .collect()
    }

    async fn list_latest_job_run_per_job_key(
        &self,
        job_keys: &[JobKey],
    ) -> AppResult<Vec<JobRunRecord>> {
        if job_keys.is_empty() {
            return Ok(Vec::new());
        }
        // Each arm is the single-key `list_job_runs(Some(key), 1)` read with
        // its own ORDER BY and LIMIT; the arm number keeps the keys' order.
        let arms = (0..job_keys.len())
            .map(|arm| {
                format!(
                    "SELECT {arm} AS latest_arm, newest.* FROM (
                         SELECT * FROM workflow_operations WHERE job_key = {{}}
                         ORDER BY started_at DESC LIMIT 1
                     ) newest"
                )
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        let sql = format!("SELECT * FROM ({arms}) latest ORDER BY latest_arm");
        let args = job_keys
            .iter()
            .map(|job_key| SqlArg::Text(job_key.as_str().to_string()))
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
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
