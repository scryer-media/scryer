use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadSourceIdentity, DownloadSubmissionIdentity, ImportArtifact,
    ImportArtifactRepository, ImportRepository, ManualImportSelection,
    ManualImportSelectionCandidate,
};
use scryer_domain::{Id, ImportRecord, ImportStatus, ImportTransferPhase, ImportType};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct ImportStore {
    datastore: StoreDatastore,
}

impl ImportStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

fn already_imported_status_predicate(datastore: &StoreDatastore) -> String {
    let skip_reason = match datastore {
        StoreDatastore::Sqlite { .. } => "json_extract(result_json, '$.skip_reason')".to_string(),
        StoreDatastore::Postgres { .. } => "result_json #>> '{skip_reason}'".to_string(),
    };
    format!("(status = 'completed' OR (status = 'skipped' AND {skip_reason} = 'already_imported'))")
}

#[async_trait]
impl ImportRepository for ImportStore {
    async fn queue_import_request(
        &self,
        source_identity: DownloadSourceIdentity,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        queue_import_request(&self.datastore, source_identity, import_type, payload_json).await
    }

    async fn queue_import_request_with_identity(
        &self,
        source_identity: DownloadSourceIdentity,
        import_type: String,
        payload_json: String,
        submission_identity: Option<DownloadSubmissionIdentity>,
    ) -> AppResult<String> {
        queue_import_request_with_identity(
            &self.datastore,
            source_identity,
            import_type,
            payload_json,
            submission_identity,
        )
        .await
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT {IMPORT_COLUMNS} FROM imports WHERE id = {{}} LIMIT 1"),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.map(|row| import_record_from_row(&row)).transpose()
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        let import_id = import_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_import_status", move |tx| {
            let import_id = import_id.clone();
            let result_json = result_json.clone();
            Box::pin(async move {
                let now = Utc::now();
                let is_terminal = status.is_terminal();
                let result_arg = json_arg_for_tx(tx, result_json.as_deref())?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE imports
                     SET status = {},
                         result_json = {},
                         import_transfer_phase = CASE WHEN {} THEN NULL ELSE import_transfer_phase END,
                         started_at = CASE WHEN started_at IS NULL THEN {} ELSE started_at END,
                         finished_at = CASE WHEN {} THEN {} ELSE finished_at END,
                         updated_at = {}
                     WHERE id = {}",
                    &[
                        SqlArg::Text(status.as_str().to_string()),
                        result_arg,
                        SqlArg::Bool(is_terminal),
                        SqlArg::Timestamp(now),
                        SqlArg::Bool(is_terminal),
                        SqlArg::Timestamp(now),
                        SqlArg::Timestamp(now),
                        SqlArg::Text(import_id),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn update_import_transfer_progress(
        &self,
        import_id: &str,
        phase: ImportTransferPhase,
        bytes: i64,
        total_bytes: i64,
    ) -> AppResult<()> {
        let import_id = import_id.to_string();
        let bytes = bytes.max(0);
        let total_bytes = total_bytes.max(bytes);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_import_transfer_progress",
            move |tx| {
                let import_id = import_id.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE imports
                     SET import_transfer_phase = {},
                         import_transfer_bytes = {},
                         import_transfer_total_bytes = {},
                         import_transfer_started_at = CASE
                             WHEN import_transfer_started_at IS NULL THEN {}
                             ELSE import_transfer_started_at
                         END,
                         import_transfer_updated_at = {},
                         updated_at = {}
                     WHERE id = {}
                       AND status IN ('pending', 'running', 'processing')",
                        &[
                            SqlArg::Text(phase.as_str().to_string()),
                            SqlArg::I64(bytes),
                            SqlArg::I64(total_bytes),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                            SqlArg::Text(import_id),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        recover_stale_processing_imports(&self.datastore, None, stale_seconds).await
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64> {
        recover_stale_processing_imports(&self.datastore, Some(import_type), stale_seconds).await
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        fetch_imports(
            self.datastore.read_exec(),
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM imports
                 WHERE status IN ('queued', 'pending', 'running', 'processing')
                 ORDER BY created_at ASC"
            ),
            &[],
        )
        .await
    }

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>> {
        fetch_imports(
            self.datastore.read_exec(),
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM imports
                 WHERE import_type = {{}}
                   AND status IN ('queued', 'pending', 'running', 'processing')
                 ORDER BY created_at ASC"
            ),
            &[SqlArg::Text(import_type.as_str().to_string())],
        )
        .await
    }

    async fn list_imports_for_identities(
        &self,
        identities: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<ImportRecord>> {
        let identities = dedupe_identities(identities);
        if identities.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = Vec::with_capacity(identities.len() * 3);
        let clauses = identities
            .iter()
            .map(|identity| {
                args.push(SqlArg::Text(normalize_download_client_id(
                    identity.client_id.as_deref(),
                )));
                args.push(SqlArg::Text(identity.client_type.clone()));
                args.push(SqlArg::Text(identity.item_id.clone()));
                "(COALESCE(source_client_id, '') = {} AND source_system = {} AND source_ref = {})"
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        fetch_imports(
            self.datastore.read_exec(),
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM imports WHERE {clauses} ORDER BY updated_at DESC"
            ),
            &args,
        )
        .await
    }

    async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool> {
        let already_imported_predicate = already_imported_status_predicate(&self.datastore);
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT COUNT(1) AS count
             FROM imports
             WHERE COALESCE(source_client_id, '') = {{}}
               AND source_system = {{}}
               AND source_ref = {{}}
               AND {already_imported_predicate}"
            ),
            &[
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing import count".into()))?;
        Ok(row.i64("count")? > 0)
    }

    async fn is_already_imported_by_download_id(
        &self,
        source_identity: &DownloadSourceIdentity,
        identity: &DownloadSubmissionIdentity,
    ) -> AppResult<bool> {
        let Some(download_id) = identity
            .download_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(false);
        };

        let already_imported_predicate = already_imported_status_predicate(&self.datastore);
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT COUNT(1) AS count
             FROM imports
             WHERE {already_imported_predicate}
               AND COALESCE(source_client_id, '') = {{}}
               AND source_system = {{}}
               AND download_id = {{}}"
            ),
            &[
                SqlArg::Text(normalize_download_client_id(
                    source_identity.client_id.as_deref(),
                )),
                SqlArg::Text(source_identity.client_type.clone()),
                SqlArg::Text(download_id.to_string()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing import identity count".into()))?;
        Ok(row.i64("count")? > 0)
    }

    async fn replace_manual_import_selection(
        &self,
        selection: ManualImportSelection,
    ) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_manual_import_selection",
            move |tx| {
                let selection = selection.clone();
                Box::pin(async move {
                    let identity = &selection.source_identity;
                    let identity_args = [
                        SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                        SqlArg::Text(identity.client_type.clone()),
                        SqlArg::Text(identity.item_id.clone()),
                    ];
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM manual_import_selection_candidates
                         WHERE selection_id IN (
                           SELECT id FROM manual_import_selections
                           WHERE actor_user_id = {} AND title_id = {}
                             AND source_client_id = {} AND source_system = {} AND source_ref = {}
                         )",
                        &[
                            SqlArg::Text(selection.actor_user_id.clone()),
                            SqlArg::Text(selection.title_id.clone()),
                            identity_args[0].clone(),
                            identity_args[1].clone(),
                            identity_args[2].clone(),
                        ],
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM manual_import_selections
                         WHERE actor_user_id = {} AND title_id = {}
                           AND source_client_id = {} AND source_system = {} AND source_ref = {}",
                        &[
                            SqlArg::Text(selection.actor_user_id.clone()),
                            SqlArg::Text(selection.title_id.clone()),
                            identity_args[0].clone(),
                            identity_args[1].clone(),
                            identity_args[2].clone(),
                        ],
                    )
                    .await?;

                    let now = Utc::now();
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO manual_import_selections
                         (id, actor_user_id, title_id, source_client_id, source_system, source_ref,
                          consumed_at, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, NULL, {}, {})",
                        &[
                            SqlArg::Text(selection.id.clone()),
                            SqlArg::Text(selection.actor_user_id.clone()),
                            SqlArg::Text(selection.title_id.clone()),
                            identity_args[0].clone(),
                            identity_args[1].clone(),
                            identity_args[2].clone(),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    for candidate in &selection.candidates {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "INSERT INTO manual_import_selection_candidates
                             (id, selection_id, canonical_path, quality, created_at)
                             VALUES ({}, {}, {}, {}, {})",
                            &[
                                SqlArg::Text(candidate.id.clone()),
                                SqlArg::Text(selection.id.clone()),
                                SqlArg::Text(candidate.canonical_path.clone()),
                                SqlArg::OptText(candidate.quality.clone()),
                                SqlArg::Timestamp(now),
                            ],
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn find_manual_import_selection(
        &self,
        actor_user_id: &str,
        title_id: &str,
        source_identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<ManualImportSelection>> {
        let selection = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, actor_user_id, title_id, source_client_id, source_system, source_ref
             FROM manual_import_selections
             WHERE actor_user_id = {} AND title_id = {}
               AND source_client_id = {} AND source_system = {} AND source_ref = {}
               AND consumed_at IS NULL",
            &[
                SqlArg::Text(actor_user_id.to_string()),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(normalize_download_client_id(
                    source_identity.client_id.as_deref(),
                )),
                SqlArg::Text(source_identity.client_type.clone()),
                SqlArg::Text(source_identity.item_id.clone()),
            ],
        )
        .await?;
        let Some(selection) = selection else {
            return Ok(None);
        };
        let selection_id = selection.text("id")?;
        let candidates = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, canonical_path, quality
             FROM manual_import_selection_candidates
             WHERE selection_id = {}",
            &[SqlArg::Text(selection_id)],
        )
        .await?;
        manual_import_selection_from_rows(selection, candidates).map(Some)
    }

    async fn get_manual_import_selection(
        &self,
        selection_id: &str,
        actor_user_id: &str,
    ) -> AppResult<Option<ManualImportSelection>> {
        let selection = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, actor_user_id, title_id, source_client_id, source_system, source_ref
             FROM manual_import_selections
             WHERE id = {} AND actor_user_id = {} AND consumed_at IS NULL",
            &[
                SqlArg::Text(selection_id.to_string()),
                SqlArg::Text(actor_user_id.to_string()),
            ],
        )
        .await?;
        let Some(selection) = selection else {
            return Ok(None);
        };
        let candidates = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, canonical_path, quality
             FROM manual_import_selection_candidates
             WHERE selection_id = {}",
            &[SqlArg::Text(selection_id.to_string())],
        )
        .await?;
        manual_import_selection_from_rows(selection, candidates).map(Some)
    }

    async fn consume_manual_import_selection(
        &self,
        selection_id: &str,
        actor_user_id: &str,
        candidate_ids: &[String],
    ) -> AppResult<Option<ManualImportSelection>> {
        if candidate_ids.is_empty() {
            return Ok(None);
        }
        let selection_id = selection_id.to_string();
        let actor_user_id = actor_user_id.to_string();
        let candidate_ids = candidate_ids.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "consume_manual_import_selection",
            move |tx| {
                let selection_id = selection_id.clone();
                let actor_user_id = actor_user_id.clone();
                let candidate_ids = candidate_ids.clone();
                Box::pin(async move {
                    let selection = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT id, actor_user_id, title_id, source_client_id, source_system, source_ref
                         FROM manual_import_selections
                         WHERE id = {} AND actor_user_id = {} AND consumed_at IS NULL",
                        &[
                            SqlArg::Text(selection_id.clone()),
                            SqlArg::Text(actor_user_id),
                        ],
                    )
                    .await?;
                    let Some(selection) = selection else {
                        return Ok(None);
                    };
                    let placeholders = (0..candidate_ids.len())
                        .map(|_| "{}")
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut candidate_args = Vec::with_capacity(candidate_ids.len() + 1);
                    candidate_args.push(SqlArg::Text(selection_id.clone()));
                    candidate_args.extend(candidate_ids.iter().cloned().map(SqlArg::Text));
                    let candidates = SqlRuntime::fetch_all(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT id, canonical_path, quality
                             FROM manual_import_selection_candidates
                             WHERE selection_id = {{}} AND id IN ({placeholders})"
                        ),
                        &candidate_args,
                    )
                    .await?;
                    if candidates.len() != candidate_ids.len() {
                        return Ok(None);
                    }
                    let consumed = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE manual_import_selections
                         SET consumed_at = {}, updated_at = {}
                         WHERE id = {} AND consumed_at IS NULL",
                        &[
                            SqlArg::Timestamp(Utc::now()),
                            SqlArg::Timestamp(Utc::now()),
                            SqlArg::Text(selection_id),
                        ],
                    )
                    .await?;
                    if consumed != 1 {
                        return Ok(None);
                    }
                    manual_import_selection_from_rows(selection, candidates).map(Some)
                })
            },
        )
        .await
    }

    async fn delete_manual_import_selections_for_source(
        &self,
        source_identity: &DownloadSourceIdentity,
    ) -> AppResult<()> {
        let identity = source_identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_manual_import_selections_for_source",
            move |tx| {
                let identity = identity.clone();
                Box::pin(async move {
                    let args = [
                        SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                        SqlArg::Text(identity.client_type),
                        SqlArg::Text(identity.item_id),
                    ];
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM manual_import_selection_candidates
                         WHERE selection_id IN (
                           SELECT id FROM manual_import_selections
                           WHERE source_client_id = {} AND source_system = {} AND source_ref = {}
                         )",
                        &args,
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM manual_import_selections
                         WHERE source_client_id = {} AND source_system = {} AND source_ref = {}",
                        &args,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        fetch_imports(
            self.datastore.read_exec(),
            &format!("SELECT {IMPORT_COLUMNS} FROM imports ORDER BY created_at DESC LIMIT {{}}"),
            &[SqlArg::I64((limit as i64).clamp(1, 500))],
        )
        .await
    }
}

fn manual_import_selection_from_rows(
    row: crate::queries::sql_runtime::SqlRow,
    candidate_rows: Vec<crate::queries::sql_runtime::SqlRow>,
) -> AppResult<ManualImportSelection> {
    let candidates = candidate_rows
        .into_iter()
        .map(|candidate| {
            Ok(ManualImportSelectionCandidate {
                id: candidate.text("id")?,
                canonical_path: candidate.text("canonical_path")?,
                quality: candidate.opt_text("quality")?,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(ManualImportSelection {
        id: row.text("id")?,
        actor_user_id: row.text("actor_user_id")?,
        title_id: row.text("title_id")?,
        source_identity: DownloadSourceIdentity::new(
            row.opt_text("source_client_id")?.as_deref(),
            row.text("source_system")?,
            row.text("source_ref")?,
        ),
        candidates,
    })
}

#[async_trait]
impl ImportArtifactRepository for ImportStore {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        SqlRuntime::run_in_transaction(&self.datastore, "insert_import_artifact", move |tx| {
            let artifact = artifact.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO download_import_artifacts
                     (id, source_client_id, source_system, source_ref, import_id, relative_path, normalized_file_name,
                      media_kind, title_id, episode_id, season_number, episode_number,
                      result, reason_code, imported_media_file_id, created_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(artifact.id),
                        SqlArg::OptText(artifact.source_client_id),
                        SqlArg::Text(artifact.source_system),
                        SqlArg::Text(artifact.source_ref),
                        SqlArg::OptText(artifact.import_id),
                        SqlArg::OptText(artifact.relative_path),
                        SqlArg::Text(artifact.normalized_file_name),
                        SqlArg::Text(artifact.media_kind),
                        SqlArg::OptText(artifact.title_id),
                        SqlArg::OptText(artifact.episode_id),
                        SqlArg::OptI32(artifact.season_number),
                        SqlArg::OptI32(artifact.episode_number),
                        SqlArg::Text(artifact.result),
                        SqlArg::OptText(artifact.reason_code),
                        SqlArg::OptText(artifact.imported_media_file_id),
                        SqlArg::Timestamp(artifact.created_at),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn list_by_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Vec<ImportArtifact>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, source_client_id, source_system, source_ref, import_id, relative_path,
                    normalized_file_name, media_kind, title_id, episode_id,
                    season_number, episode_number, result, reason_code,
                    imported_media_file_id, created_at
             FROM download_import_artifacts
             WHERE COALESCE(source_client_id, '') = {} AND source_system = {} AND source_ref = {}
             ORDER BY created_at",
            &[
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
            ],
        )
        .await?
        .into_iter()
        .map(|row| import_artifact_from_row(&row))
        .collect()
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
        result: &str,
    ) -> AppResult<u64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM download_import_artifacts
             WHERE COALESCE(source_client_id, '') = {} AND source_system = {} AND source_ref = {} AND result = {}",
            &[
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(result.to_string()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing import artifact count".into()))?;
        Ok(row.i64("count")? as u64)
    }
}
