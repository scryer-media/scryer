use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, ClientJobLocator, DownloadSubmissionIdentity, ImportArtifact,
    ImportArtifactRepository, ImportRepository, ManualImportSelection,
    ManualImportSelectionCandidate,
};
use scryer_domain::{ImportRecord, ImportStatus, ImportTransferPhase, ImportType};

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

async fn active_binding_download_id(
    datastore: &StoreDatastore,
    locator: &ClientJobLocator,
) -> AppResult<Option<scryer_domain::download_identity::DownloadId>> {
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT download_id FROM download_client_bindings
         WHERE ended_at IS NULL
           AND native_item_id IS NOT NULL
           AND COALESCE(client_config_id, '') = {}
           AND LOWER(COALESCE(client_type_snapshot, '')) = {}
           AND native_item_id = {}
         ORDER BY created_at, download_id
         LIMIT 1",
        &[
            SqlArg::Text(locator.client_id_or_empty().to_string()),
            SqlArg::Text(locator.client_type.clone()),
            SqlArg::Text(locator.item_id.clone()),
        ],
    )
    .await?;
    row.map(|row| {
        let value = row.text("download_id")?;
        scryer_domain::download_identity::DownloadId::parse(&value).ok_or_else(|| {
            AppError::Repository(format!(
                "invalid canonical download id {value:?} in binding"
            ))
        })
    })
    .transpose()
}

#[async_trait]
impl ImportRepository for ImportStore {
    async fn queue_import_request(
        &self,
        source_identity: ClientJobLocator,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        queue_import_request(&self.datastore, source_identity, import_type, payload_json).await
    }

    async fn queue_import_request_with_identity(
        &self,
        source_identity: ClientJobLocator,
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

    async fn queue_import_request_with_identity_for_download(
        &self,
        source_identity: ClientJobLocator,
        import_type: String,
        payload_json: String,
        submission_identity: Option<DownloadSubmissionIdentity>,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    ) -> AppResult<String> {
        queue_import_request_with_identity_for_download(
            &self.datastore,
            source_identity,
            import_type,
            payload_json,
            submission_identity,
            canonical_download_id,
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

    async fn canonical_download_id_for_import(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_domain::download_identity::DownloadId>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT canonical_download_id FROM imports WHERE id = {} LIMIT 1",
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(row
            .map(|row| row.opt_text("canonical_download_id"))
            .transpose()?
            .flatten()
            .as_deref()
            .and_then(scryer_domain::download_identity::DownloadId::parse))
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
        identities: &[ClientJobLocator],
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

    async fn list_completed_manual_imports(
        &self,
        updated_after: chrono::DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<ImportRecord>> {
        fetch_imports(
            self.datastore.read_exec(),
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM imports
                 WHERE import_type = {{}} AND status = 'completed'
                   AND updated_at >= {{}}
                 ORDER BY updated_at DESC LIMIT {{}}"
            ),
            &[
                SqlArg::Text(ImportType::ManualImport.as_str().to_string()),
                SqlArg::Timestamp(updated_after),
                SqlArg::I64((limit as i64).clamp(1, 500)),
            ],
        )
        .await
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
                    let existing_canonical_download_id = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT canonical_download_id FROM manual_import_selections
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
                    .await?
                    .map(|row| row.opt_text("canonical_download_id"))
                    .transpose()?
                    .flatten();
                    let canonical_download_id = selection
                        .canonical_download_id
                        .map(|download_id| download_id.to_string())
                        .or(existing_canonical_download_id);
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
                          canonical_download_id,
                          release_evidence_json, trusted_source_root, archive_workspace_root,
                          consumed_at, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, NULL, {}, {})",
                        &[
                            SqlArg::Text(selection.id.clone()),
                            SqlArg::Text(selection.actor_user_id.clone()),
                            SqlArg::Text(selection.title_id.clone()),
                            identity_args[0].clone(),
                            identity_args[1].clone(),
                            identity_args[2].clone(),
                            SqlArg::OptText(canonical_download_id),
                            SqlArg::OptText(selection.release_evidence_json.clone()),
                            SqlArg::Text(selection.trusted_source_root.clone()),
                            SqlArg::OptText(selection.archive_workspace_root.clone()),
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
                                // Retain the legacy column for already-created
                                // databases, but never persist filename-derived
                                // quality as manual-import score evidence.
                                SqlArg::OptText(None),
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
        source_identity: &ClientJobLocator,
    ) -> AppResult<Option<ManualImportSelection>> {
        let selection = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, actor_user_id, title_id, source_client_id, source_system, source_ref,
                    canonical_download_id, release_evidence_json, trusted_source_root, archive_workspace_root
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

    async fn find_manual_import_selection_for_download(
        &self,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
        actor_user_id: &str,
        title_id: &str,
        source_identity: &ClientJobLocator,
    ) -> AppResult<Option<ManualImportSelection>> {
        let canonical_download_id = match canonical_download_id {
            Some(id) => Some(id.to_string()),
            None => active_binding_download_id(&self.datastore, source_identity)
                .await?
                .map(|id| id.to_string()),
        };
        let Some(canonical_download_id) = canonical_download_id else {
            return Ok(None);
        };
        let canonical = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, actor_user_id, title_id, source_client_id, source_system, source_ref,
                    canonical_download_id, release_evidence_json, trusted_source_root, archive_workspace_root
             FROM manual_import_selections
             WHERE canonical_download_id = {} AND actor_user_id = {} AND title_id = {}
               AND consumed_at IS NULL
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            &[
                SqlArg::Text(canonical_download_id),
                SqlArg::Text(actor_user_id.to_string()),
                SqlArg::Text(title_id.to_string()),
            ],
        )
        .await?;
        let canonical = if let Some(selection) = canonical {
            let selection_id = selection.text("id")?;
            let candidates = SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                "SELECT id, canonical_path, quality
                 FROM manual_import_selection_candidates
                 WHERE selection_id = {}",
                &[SqlArg::Text(selection_id)],
            )
            .await?;
            Some(manual_import_selection_from_rows(selection, candidates)?)
        } else {
            None
        };
        Ok(canonical)
    }

    async fn get_manual_import_selection(
        &self,
        selection_id: &str,
        actor_user_id: &str,
    ) -> AppResult<Option<ManualImportSelection>> {
        let selection = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, actor_user_id, title_id, source_client_id, source_system, source_ref,
                    canonical_download_id, release_evidence_json, trusted_source_root, archive_workspace_root
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
                        "SELECT id, actor_user_id, title_id, source_client_id, source_system, source_ref,
                                canonical_download_id, release_evidence_json, trusted_source_root, archive_workspace_root
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
        source_identity: &ClientJobLocator,
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
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(ManualImportSelection {
        id: row.text("id")?,
        actor_user_id: row.text("actor_user_id")?,
        title_id: row.text("title_id")?,
        canonical_download_id: row
            .opt_text("canonical_download_id")?
            .as_deref()
            .and_then(scryer_domain::download_identity::DownloadId::parse),
        release_evidence_json: row.opt_text("release_evidence_json")?,
        trusted_source_root: row.text("trusted_source_root")?,
        archive_workspace_root: row.opt_text("archive_workspace_root")?,
        source_identity: ClientJobLocator::new(
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
        self.insert_artifact_for_download(artifact, None).await
    }

    async fn insert_artifact_for_download(
        &self,
        artifact: ImportArtifact,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    ) -> AppResult<()> {
        let canonical_download_id = canonical_download_id.map(ToString::to_string);
        SqlRuntime::run_in_transaction(&self.datastore, "insert_import_artifact", move |tx| {
            let artifact = artifact.clone();
            let canonical_download_id = canonical_download_id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO download_import_artifacts
                     (id, source_client_id, source_system, source_ref, canonical_download_id, import_id, relative_path, normalized_file_name,
                      media_kind, title_id, episode_id, season_number, episode_number,
                      result, reason_code, imported_media_file_id, created_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(artifact.id),
                        SqlArg::OptText(artifact.source_client_id),
                        SqlArg::Text(artifact.source_system),
                        SqlArg::Text(artifact.source_ref),
                        SqlArg::OptText(canonical_download_id),
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

    async fn insert_artifacts_for_download(
        &self,
        artifacts: Vec<ImportArtifact>,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    ) -> AppResult<()> {
        let canonical_download_id = canonical_download_id.map(ToString::to_string);
        SqlRuntime::run_in_transaction(&self.datastore, "insert_import_artifacts", move |tx| {
            let artifacts = artifacts.clone();
            let canonical_download_id = canonical_download_id.clone();
            Box::pin(async move {
                for artifact in artifacts {
                    SqlRuntime::execute(
                        SqlExec::Tx(&mut *tx),
                        "INSERT INTO download_import_artifacts
                         (id, source_client_id, source_system, source_ref, canonical_download_id, import_id, relative_path, normalized_file_name,
                          media_kind, title_id, episode_id, season_number, episode_number,
                          result, reason_code, imported_media_file_id, created_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                        &[
                            SqlArg::Text(artifact.id),
                            SqlArg::OptText(artifact.source_client_id),
                            SqlArg::Text(artifact.source_system),
                            SqlArg::Text(artifact.source_ref),
                            SqlArg::OptText(canonical_download_id.clone()),
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
                }
                Ok(())
            })
        })
        .await
    }

    async fn list_by_source_identity(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Vec<ImportArtifact>> {
        let args = [
            SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            SqlArg::Text(identity.client_type.clone()),
            SqlArg::Text(identity.item_id.clone()),
        ];
        let exact = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, source_client_id, source_system, source_ref, import_id, relative_path,
                    normalized_file_name, media_kind, title_id, episode_id,
                    season_number, episode_number, result, reason_code,
                    imported_media_file_id, created_at
             FROM download_import_artifacts
             WHERE COALESCE(source_client_id, '') = {}
               AND source_system = {}
               AND source_ref = {}
             ORDER BY created_at",
            &args,
        )
        .await?;
        let rows = if exact.is_empty() {
            SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                "SELECT id, source_client_id, source_system, source_ref, import_id, relative_path,
                        normalized_file_name, media_kind, title_id, episode_id,
                        season_number, episode_number, result, reason_code,
                        imported_media_file_id, created_at
                 FROM download_import_artifacts
                 WHERE LOWER(TRIM(COALESCE(source_client_id, ''))) = {}
                   AND LOWER(TRIM(source_system)) = {}
                   AND TRIM(source_ref) = {}
                 ORDER BY created_at",
                &args,
            )
            .await?
        } else {
            exact
        };
        rows.into_iter()
            .map(|row| import_artifact_from_row(&row))
            .collect()
    }

    async fn list_by_source_identity_for_download(
        &self,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Vec<ImportArtifact>> {
        let canonical_download_id = match canonical_download_id {
            Some(id) => Some(id.to_string()),
            None => active_binding_download_id(&self.datastore, identity)
                .await?
                .map(|id| id.to_string()),
        };
        let Some(canonical_download_id) = canonical_download_id else {
            return Ok(Vec::new());
        };
        let canonical = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, source_client_id, source_system, source_ref, import_id, relative_path,
                    normalized_file_name, media_kind, title_id, episode_id,
                    season_number, episode_number, result, reason_code,
                    imported_media_file_id, created_at
             FROM download_import_artifacts
             WHERE canonical_download_id = {}
             ORDER BY created_at",
            &[SqlArg::Text(canonical_download_id)],
        )
        .await?
        .into_iter()
        .map(|row| import_artifact_from_row(&row))
        .collect::<AppResult<Vec<_>>>()?;
        Ok(canonical)
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &ClientJobLocator,
        result: &str,
    ) -> AppResult<u64> {
        let args = [
            SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            SqlArg::Text(identity.client_type.clone()),
            SqlArg::Text(identity.item_id.clone()),
            SqlArg::Text(result.to_string()),
        ];
        let exact = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM download_import_artifacts
             WHERE COALESCE(source_client_id, '') = {}
               AND source_system = {}
               AND source_ref = {}
               AND result = {}",
            &args,
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing import artifact count".into()))?;
        let exact_count = exact.i64("count")? as u64;
        if exact_count > 0 {
            return Ok(exact_count);
        }

        let legacy = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM download_import_artifacts
             WHERE LOWER(TRIM(COALESCE(source_client_id, ''))) = {}
               AND LOWER(TRIM(source_system)) = {}
               AND TRIM(source_ref) = {}
               AND result = {}",
            &args,
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing import artifact count".into()))?;
        Ok(legacy.i64("count")? as u64)
    }

    async fn count_by_result_for_source_identity_for_download(
        &self,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
        identity: &ClientJobLocator,
        result: &str,
    ) -> AppResult<u64> {
        let canonical_download_id = match canonical_download_id {
            Some(id) => Some(id.to_string()),
            None => active_binding_download_id(&self.datastore, identity)
                .await?
                .map(|id| id.to_string()),
        };
        let Some(canonical_download_id) = canonical_download_id else {
            return Ok(0);
        };
        let canonical = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM download_import_artifacts
             WHERE canonical_download_id = {} AND result = {}",
            &[
                SqlArg::Text(canonical_download_id),
                SqlArg::Text(result.to_string()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing canonical import artifact count".into()))?
        .i64("count")? as u64;
        Ok(canonical)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use scryer_application::{ImportArtifactRepository, ImportRepository};
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn store() -> ImportStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE imports (
                 id TEXT PRIMARY KEY,
                 source_client_id TEXT,
                 source_system TEXT NOT NULL,
                 source_ref TEXT NOT NULL,
                 import_type TEXT NOT NULL,
                 status TEXT NOT NULL,
                 payload_json TEXT,
                 rename_plan_json TEXT,
                 result_json TEXT,
                 download_id TEXT,
                 canonical_download_id TEXT,
                 import_transfer_phase TEXT,
                 import_transfer_bytes INTEGER,
                 import_transfer_total_bytes INTEGER,
                 import_transfer_started_at TEXT,
                 import_transfer_updated_at TEXT,
                 started_at TEXT,
                 finished_at TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 UNIQUE(source_client_id, source_system, source_ref, import_type)
             )",
        )
        .execute(&pool)
        .await
        .expect("imports table should be created");
        sqlx::query(
            "CREATE TABLE download_import_artifacts (
                 id TEXT PRIMARY KEY,
                 source_client_id TEXT,
                 source_system TEXT NOT NULL,
                 source_ref TEXT NOT NULL,
                 canonical_download_id TEXT,
                 import_id TEXT,
                 relative_path TEXT,
                 normalized_file_name TEXT NOT NULL,
                 media_kind TEXT NOT NULL,
                 title_id TEXT,
                 episode_id TEXT,
                 season_number INTEGER,
                 episode_number INTEGER,
                 result TEXT NOT NULL,
                 reason_code TEXT,
                 imported_media_file_id TEXT,
                 created_at TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("import artifacts table should be created");
        sqlx::query(
            "CREATE TABLE manual_import_selections (
                 id TEXT PRIMARY KEY,
                 actor_user_id TEXT NOT NULL,
                 title_id TEXT NOT NULL,
                 source_client_id TEXT NOT NULL,
                 source_system TEXT NOT NULL,
                 source_ref TEXT NOT NULL,
                 canonical_download_id TEXT,
                 release_evidence_json TEXT,
                 trusted_source_root TEXT NOT NULL,
                 archive_workspace_root TEXT,
                 consumed_at TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("manual selections table should be created");
        sqlx::query(
            "CREATE TABLE manual_import_selection_candidates (
                 id TEXT PRIMARY KEY,
                 selection_id TEXT NOT NULL,
                 canonical_path TEXT NOT NULL,
                 quality TEXT,
                 created_at TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("manual selection candidates table should be created");
        ImportStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn source_identity(item_id: &str) -> ClientJobLocator {
        ClientJobLocator::new(Some("client-1"), "nzbget", item_id)
    }

    fn artifact(id: &str, source_identity: &ClientJobLocator, result: &str) -> ImportArtifact {
        ImportArtifact {
            id: id.to_string(),
            source_client_id: source_identity.client_id.clone(),
            source_system: source_identity.client_type.clone(),
            source_ref: source_identity.item_id.clone(),
            import_id: None,
            relative_path: None,
            normalized_file_name: format!("{id}.mkv"),
            media_kind: "episode".to_string(),
            title_id: Some("title-1".to_string()),
            episode_id: Some("episode-1".to_string()),
            season_number: Some(1),
            episode_number: Some(1),
            result: result.to_string(),
            reason_code: None,
            imported_media_file_id: None,
            created_at: Utc::now(),
        }
    }

    fn selection(id: &str, source_identity: ClientJobLocator) -> ManualImportSelection {
        ManualImportSelection {
            id: id.to_string(),
            actor_user_id: "user-1".to_string(),
            title_id: "title-1".to_string(),
            source_identity,
            canonical_download_id: None,
            release_evidence_json: None,
            trusted_source_root: "/downloads/release".to_string(),
            archive_workspace_root: None,
            candidates: Vec::new(),
        }
    }

    #[tokio::test]
    async fn artifact_reads_find_canonical_rows_before_legacy_tuples() {
        let store = store().await;
        let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
        let written_source_identity = source_identity("canonical-artifact");
        store
            .insert_artifact_for_download(
                artifact("canonical-artifact", &written_source_identity, "imported"),
                Some(&canonical_download_id),
            )
            .await
            .expect("canonical artifact should be written");

        let unrelated_source_identity = source_identity("other-artifact");
        let artifacts = store
            .list_by_source_identity_for_download(
                Some(&canonical_download_id),
                &unrelated_source_identity,
            )
            .await
            .expect("canonical artifact lookup should succeed");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, "canonical-artifact");
        assert_eq!(
            store
                .count_by_result_for_source_identity_for_download(
                    Some(&canonical_download_id),
                    &unrelated_source_identity,
                    "imported",
                )
                .await
                .expect("canonical artifact count should succeed"),
            1
        );
    }

    #[tokio::test]
    async fn manual_selection_reads_find_canonical_rows_before_legacy_tuples() {
        let store = store().await;
        let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
        let written_source_identity = source_identity("canonical-selection");
        store
            .replace_manual_import_selection_for_download(
                selection("canonical-selection", written_source_identity),
                Some(&canonical_download_id),
            )
            .await
            .expect("canonical selection should be written");

        let selection = store
            .find_manual_import_selection_for_download(
                Some(&canonical_download_id),
                "user-1",
                "title-1",
                &source_identity("other-selection"),
            )
            .await
            .expect("canonical selection lookup should succeed")
            .expect("canonical selection should be found");
        assert_eq!(selection.id, "canonical-selection");
        assert_eq!(selection.canonical_download_id, Some(canonical_download_id));
    }
}
