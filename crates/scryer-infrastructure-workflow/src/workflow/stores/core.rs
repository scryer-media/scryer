use std::collections::HashSet;

use chrono::{DateTime, Utc};
use scryer_application::{
    AcquisitionScopeStatus, AppError, AppResult, ClientJobLocator, DownloadQueueCommandRecord,
    DownloadSubmission, DownloadSubmissionActorSnapshot, DownloadSubmissionIdentity,
    ExternalImportMonitorSnapshotChunk, ExternalImportMonitorSnapshotEntryKind, ImportArtifact,
    JobKey, JobRunRecord, JobRunStatus, JobTriggerSource, PendingReleaseStatus, SubmissionScope,
    SuccessfulGrabCommit, WorkflowOperationInfo,
};
use scryer_domain::download_identity::DownloadId;
use scryer_domain::{
    DomainEvent, DomainEventActorKind, DomainEventFilter, DomainEventStream, DomainEventType,
    DownloadQueueCommandAction, DownloadQueueDeleteStatus, Id, ImportRecord, ImportStatus,
    ImportTransferPhase, ImportType, MediaFacet, MediaFileDeletedReason, NewDomainEvent,
    TitleHistoryEventType,
};
use scryer_infrastructure_sql::domain_event_payload::{
    decode_domain_event_payload, derive_domain_event_projections, encode_domain_event_payload,
};
use serde_json::Value as JsonValue;
use sqlx::{Row, types::Json};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::types::WorkflowOperationRecord;

use super::download_submission_store::claim_or_create_binding_download_id_tx;

pub const DOMAIN_EVENT_COLUMNS: &str = "sequence, event_id, occurred_at, actor_kind, actor_user_id, actor_display_name, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, event_type, payload_json, import_status, media_file_delete_reason, download_id";
pub const DOWNLOAD_SUBMISSION_COLUMNS: &str = "id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_provider_id, source_provider_name, source_kind, source_title, info_hash, release_size_bytes, request_signature, purpose, episode_id, collection_id, series_movie_link_id";
pub const IMPORT_COLUMNS: &str = "id, source_client_id, source_system, source_ref, import_type, status, payload_json, result_json, download_id, import_transfer_phase, import_transfer_bytes, import_transfer_total_bytes, import_transfer_started_at, import_transfer_updated_at, started_at, finished_at, created_at, updated_at";
pub const DOWNLOAD_QUEUE_COMMAND_COLUMNS: &str = "id, action, canonical_download_id, client_id, client_type, download_client_item_id, is_history, status, error_text, requested_by_user_id, started_at, finished_at, created_at, updated_at";

#[derive(Clone)]
pub struct NewWorkflowOperation {
    pub operation_type: String,
    pub status: String,
    pub job_key: Option<String>,
    pub trigger_source: Option<String>,
    pub actor_user_id: Option<String>,
    pub progress_json: Option<String>,
    pub summary_json: Option<String>,
    pub summary_text: Option<String>,
    pub error_text: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

pub async fn append_domain_events(
    datastore: &StoreDatastore,
    events: Vec<NewDomainEvent>,
) -> AppResult<Vec<DomainEvent>> {
    SqlRuntime::run_in_transaction(datastore, "append_domain_events", move |tx| {
        let events = events.clone();
        Box::pin(async move {
            let mut out = Vec::with_capacity(events.len());
            for event in events {
                let payload = serde_json::to_value(&event.payload).map_err(repo_err)?;
                let event_type = event.payload.event_type().as_str();
                let encoded_payload = encode_domain_event_payload(&payload).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to encode domain event {} ({event_type}): {error}",
                        event.event_id
                    ))
                })?;
                let projections = derive_domain_event_projections(event_type, &payload);
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO domain_events (
                        event_id, occurred_at, actor_kind, actor_user_id, actor_display_name,
                        title_id, facet, correlation_id, causation_id, schema_version,
                        stream_kind, stream_id, event_type, payload_json, import_status,
                        media_file_delete_reason, download_id
                     ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(event.event_id.clone()),
                        SqlArg::Timestamp(event.occurred_at),
                        SqlArg::Text(event.actor_kind.as_str().to_string()),
                        SqlArg::OptText(event.actor_user_id.clone()),
                        SqlArg::Text(event.actor_display_name.clone()),
                        SqlArg::OptText(event.title_id.clone()),
                        SqlArg::OptText(
                            event.facet.as_ref().map(|facet| facet.as_str().to_string()),
                        ),
                        SqlArg::OptText(event.correlation_id.clone()),
                        SqlArg::OptText(event.causation_id.clone()),
                        SqlArg::I32(event.schema_version),
                        SqlArg::Text(event.stream.kind().to_string()),
                        SqlArg::OptText(event.stream.identifier().map(str::to_string)),
                        SqlArg::Text(event.payload.event_type().as_str().to_string()),
                        SqlArg::OptBytes(Some(encoded_payload)),
                        SqlArg::OptText(projections.import_status),
                        SqlArg::OptText(projections.media_file_delete_reason),
                        SqlArg::OptText(projections.download_id),
                    ],
                )
                .await?;
                let stored = fetch_domain_event_by_event_id(SqlExec::Tx(tx), &event.event_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::Repository("failed to reload inserted domain event".into())
                    })?;
                out.push(stored);
            }
            Ok(out)
        })
    })
    .await
}

pub async fn append_domain_event_tx(
    tx: &mut SqlTx<'_>,
    event: NewDomainEvent,
) -> AppResult<DomainEvent> {
    let payload = serde_json::to_value(&event.payload).map_err(repo_err)?;
    let event_type = event.payload.event_type().as_str();
    let encoded_payload = encode_domain_event_payload(&payload).map_err(|error| {
        AppError::Repository(format!(
            "failed to encode domain event {} ({event_type}): {error}",
            event.event_id
        ))
    })?;
    let projections = derive_domain_event_projections(event_type, &payload);
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO domain_events (
            event_id, occurred_at, actor_kind, actor_user_id, actor_display_name,
            title_id, facet, correlation_id, causation_id, schema_version,
            stream_kind, stream_id, event_type, payload_json, import_status,
            media_file_delete_reason, download_id
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(event.event_id.clone()),
            SqlArg::Timestamp(event.occurred_at),
            SqlArg::Text(event.actor_kind.as_str().to_string()),
            SqlArg::OptText(event.actor_user_id.clone()),
            SqlArg::Text(event.actor_display_name.clone()),
            SqlArg::OptText(event.title_id.clone()),
            SqlArg::OptText(event.facet.as_ref().map(|facet| facet.as_str().to_string())),
            SqlArg::OptText(event.correlation_id.clone()),
            SqlArg::OptText(event.causation_id.clone()),
            SqlArg::I32(event.schema_version),
            SqlArg::Text(event.stream.kind().to_string()),
            SqlArg::OptText(event.stream.identifier().map(str::to_string)),
            SqlArg::Text(event.payload.event_type().as_str().to_string()),
            SqlArg::OptBytes(Some(encoded_payload)),
            SqlArg::OptText(projections.import_status),
            SqlArg::OptText(projections.media_file_delete_reason),
            SqlArg::OptText(projections.download_id),
        ],
    )
    .await?;

    fetch_domain_event_by_event_id(SqlExec::Tx(tx), &event.event_id)
        .await?
        .ok_or_else(|| AppError::Repository("failed to reload inserted domain event".into()))
}

pub async fn commit_successful_grab_tx(
    tx: &mut SqlTx<'_>,
    commit: &SuccessfulGrabCommit,
) -> AppResult<()> {
    let mut wanted_item_ids = commit.covered_wanted_item_ids.clone();
    if !wanted_item_ids
        .iter()
        .any(|id| id == &commit.wanted_item_id)
    {
        wanted_item_ids.push(commit.wanted_item_id.clone());
    }
    wanted_item_ids.sort();
    wanted_item_ids.dedup();

    for wanted_item_id in &wanted_item_ids {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE wanted_items
             SET status = {}, last_search_at = {},
                 grabbed_release = {}, updated_at = {}
             WHERE id = {}",
            &[
                SqlArg::Text(AcquisitionScopeStatus::Grabbed.as_str().to_string()),
                opt_timestamp_arg(commit.last_search_at.as_deref()),
                SqlArg::Text(commit.grabbed_release.clone()),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(wanted_item_id.clone()),
            ],
        )
        .await?;
    }

    if let Some(pending_release_id) = commit.grabbed_pending_release_id.as_deref() {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE pending_releases SET status = {}, grabbed_at = {} WHERE id = {}",
            &[
                SqlArg::Text(PendingReleaseStatus::Grabbed.as_str().to_string()),
                opt_timestamp_arg(commit.grabbed_at.as_deref()),
                SqlArg::Text(pending_release_id.to_string()),
            ],
        )
        .await?;
    }

    for wanted_item_id in &wanted_item_ids {
        match commit.grabbed_pending_release_id.as_deref() {
            Some(except_id) => {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE pending_releases
                     SET status = 'superseded'
                     WHERE wanted_item_id = {}
                       AND id != {}
                       AND status = 'waiting'",
                    &[
                        SqlArg::Text(wanted_item_id.clone()),
                        SqlArg::Text(except_id.to_string()),
                    ],
                )
                .await?;
            }
            None => {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE pending_releases
                     SET status = 'superseded'
                     WHERE wanted_item_id = {}
                       AND status = 'waiting'",
                    &[SqlArg::Text(wanted_item_id.clone())],
                )
                .await?;
            }
        }
    }
    Ok(())
}

pub async fn record_download_submission_tx(
    tx: &mut SqlTx<'_>,
    submission: &DownloadSubmission,
) -> AppResult<Option<DownloadId>> {
    record_download_submission_tx_inner(tx, submission).await
}

async fn ensure_submission_download_tx(
    tx: &mut SqlTx<'_>,
    submission: &DownloadSubmission,
    origin: &str,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO downloads (id, origin, created_at)
         VALUES ({}, {}, {})
         ON CONFLICT(id) DO NOTHING",
        &[
            SqlArg::Text(submission.download_id.to_string()),
            SqlArg::Text(origin.to_string()),
            SqlArg::Timestamp(Utc::now()),
        ],
    )
    .await?;
    Ok(())
}

/// Persist a submit whose HTTP request may have reached the client but whose
/// response never supplied a native client item identifier. The row remains
/// deliberately unbound so legacy tuple readers cannot mistake it for a
/// tracked client job.
pub async fn record_ambiguous_download_submission_tx(
    tx: &mut SqlTx<'_>,
    submission: &DownloadSubmission,
) -> AppResult<()> {
    let download_client_id = normalize_download_client_id(submission.download_client_id.as_deref());
    let canonical_id = submission.download_id.to_string();
    let client_config_id = submission
        .download_client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let client_type_snapshot = (!submission.download_client_type.trim().is_empty())
        .then(|| submission.download_client_type.clone());
    let client_name_snapshot = if let Some(client_config_id) = client_config_id.as_deref() {
        SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT name FROM download_clients WHERE id = {} LIMIT 1",
            &[SqlArg::Text(client_config_id.to_string())],
        )
        .await?
        .map(|row| row.text("name"))
        .transpose()?
        .or_else(|| client_type_snapshot.clone())
    } else {
        client_type_snapshot.clone()
    };
    let now = Utc::now();
    let origin = if submission.title_id.trim().is_empty() {
        "foreign_observation"
    } else {
        "scryer_submission"
    };

    ensure_submission_download_tx(tx, submission, origin).await?;

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_id, download_client_type,
          download_client_item_id, source_hint, source_provider_id,
          source_provider_name, source_kind, source_title, info_hash,
          release_size_bytes, request_signature, purpose, episode_id,
          collection_id, series_movie_link_id, download_id)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(id) DO NOTHING",
        &[
            SqlArg::Text(canonical_id.clone()),
            SqlArg::Text(submission.title_id.clone()),
            SqlArg::Text(submission.facet.clone()),
            SqlArg::Text(download_client_id),
            SqlArg::Text(submission.download_client_type.clone()),
            SqlArg::OptText(None),
            SqlArg::OptText(submission.source_hint.clone()),
            SqlArg::OptText(submission.source_provider_id.clone()),
            SqlArg::OptText(submission.source_provider_name.clone()),
            SqlArg::OptText(
                submission
                    .source_kind
                    .map(|value| value.as_str().to_string()),
            ),
            SqlArg::OptText(submission.source_title.clone()),
            SqlArg::OptText(submission.info_hash.clone()),
            SqlArg::OptI64(submission.release_size_bytes),
            SqlArg::OptText(submission.request_signature.clone()),
            SqlArg::Text(submission.purpose.as_str().to_string()),
            SqlArg::OptText(None),
            SqlArg::OptText(None),
            SqlArg::OptText(None),
            SqlArg::OptText(Some(submission.download_id.to_wire())),
        ],
    )
    .await?;

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_client_bindings
         (download_id, client_config_id, client_type_snapshot, client_name_snapshot,
          native_item_id, created_at, ended_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(download_id) DO NOTHING",
        &[
            SqlArg::Text(canonical_id),
            SqlArg::OptText(client_config_id),
            SqlArg::OptText(client_type_snapshot),
            SqlArg::OptText(client_name_snapshot),
            SqlArg::OptText(None),
            SqlArg::Timestamp(now),
            SqlArg::OptTimestamp(None),
        ],
    )
    .await?;
    Ok(())
}

async fn record_download_submission_tx_inner(
    tx: &mut SqlTx<'_>,
    submission: &DownloadSubmission,
) -> AppResult<Option<DownloadId>> {
    let mut submission = submission.clone();
    let download_client_id = normalize_download_client_id(submission.download_client_id.as_deref());
    if !download_client_id.is_empty()
        && !submission.download_client_type.trim().is_empty()
        && !submission.download_client_item_id.trim().is_empty()
    {
        let locator = ClientJobLocator::from_submission(&submission);
        submission.download_id =
            claim_or_create_binding_download_id_tx(tx, &locator, Some(submission.download_id))
                .await?;
    }
    let (episode_id, collection_id, series_movie_link_id) =
        persisted_submission_scope(&submission.scope);
    let is_orphan = matches!(&submission.scope, SubmissionScope::Orphan)
        && submission.title_id.trim().is_empty();
    ensure_submission_download_tx(
        tx,
        &submission,
        if is_orphan {
            "foreign_observation"
        } else {
            "scryer_submission"
        },
    )
    .await?;
    let conflict_clause = if is_orphan {
        "ON CONFLICT(id) DO NOTHING"
    } else {
        "ON CONFLICT(id) DO UPDATE
         SET title_id = excluded.title_id,
             facet = excluded.facet,
             source_hint = excluded.source_hint,
             source_provider_id = excluded.source_provider_id,
             source_provider_name = excluded.source_provider_name,
             source_kind = excluded.source_kind,
             source_title = excluded.source_title,
             info_hash = excluded.info_hash,
             release_size_bytes = excluded.release_size_bytes,
             request_signature = excluded.request_signature,
             purpose = excluded.purpose,
             episode_id = excluded.episode_id,
             collection_id = excluded.collection_id,
             series_movie_link_id = excluded.series_movie_link_id"
    };
    let sql = [
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_provider_id, source_provider_name, source_kind, source_title, info_hash, release_size_bytes, request_signature, purpose, episode_id, collection_id, series_movie_link_id)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        conflict_clause,
    ]
    .join(" ");
    let rows_affected = SqlRuntime::execute(
        SqlExec::Tx(tx),
        &sql,
        &[
            SqlArg::Text(submission.download_id.to_string()),
            SqlArg::Text(submission.title_id.clone()),
            SqlArg::Text(submission.facet.clone()),
            SqlArg::Text(download_client_id.clone()),
            SqlArg::Text(submission.download_client_type.clone()),
            SqlArg::Text(submission.download_client_item_id.clone()),
            SqlArg::OptText(submission.source_hint.clone()),
            SqlArg::OptText(submission.source_provider_id.clone()),
            SqlArg::OptText(submission.source_provider_name.clone()),
            SqlArg::OptText(
                submission
                    .source_kind
                    .map(|value| value.as_str().to_string()),
            ),
            SqlArg::OptText(submission.source_title.clone()),
            SqlArg::OptText(submission.info_hash.clone()),
            SqlArg::OptI64(submission.release_size_bytes),
            SqlArg::OptText(submission.request_signature.clone()),
            SqlArg::Text(submission.purpose.as_str().to_string()),
            SqlArg::OptText(episode_id.map(str::to_string)),
            SqlArg::OptText(collection_id.map(str::to_string)),
            SqlArg::OptText(series_movie_link_id.map(str::to_string)),
        ],
    )
    .await?;
    if rows_affected == 0 {
        return Ok(None);
    }
    replace_download_submission_episode_links_tx(
        tx,
        &submission.download_id,
        persisted_episode_set_ids(&submission.scope),
    )
    .await?;
    Ok(Some(submission.download_id))
}

pub async fn record_download_submission_identity_tx(
    tx: &mut SqlTx<'_>,
    canonical_download_id: &DownloadId,
    submission_identity: &DownloadSubmissionIdentity,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE download_submissions
         SET tracked_state = CASE
                 WHEN COALESCE(download_id, '') != COALESCE({}, '')
                 THEN NULL
                 ELSE tracked_state
             END,
             tracked_state_at = CASE
                 WHEN COALESCE(download_id, '') != COALESCE({}, '')
                 THEN NULL
                 ELSE tracked_state_at
             END,
             download_id = {}
         WHERE id = {}",
        &[
            SqlArg::OptText(submission_identity.download_id.clone()),
            SqlArg::OptText(submission_identity.download_id.clone()),
            SqlArg::OptText(submission_identity.download_id.clone()),
            SqlArg::Text(canonical_download_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

pub async fn record_download_submission_with_identity_tx(
    tx: &mut SqlTx<'_>,
    submission: &DownloadSubmission,
    submission_identity: &DownloadSubmissionIdentity,
) -> AppResult<Option<DownloadId>> {
    let Some(effective_download_id) = record_download_submission_tx_inner(tx, submission).await?
    else {
        return Ok(None);
    };
    record_download_submission_identity_tx(tx, &effective_download_id, submission_identity).await?;
    Ok(Some(effective_download_id))
}

pub async fn replace_download_submission_episode_links_tx(
    tx: &mut SqlTx<'_>,
    download_id: &DownloadId,
    episode_ids: &[String],
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM download_submission_episode_links
         WHERE download_id = {}",
        &[SqlArg::Text(download_id.to_string())],
    )
    .await?;
    for episode_id in episode_ids {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO download_submission_episode_links
             (download_id, episode_id)
             VALUES ({}, {})
             ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(download_id.to_string()),
                SqlArg::Text(episode_id.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn queue_import_request(
    datastore: &StoreDatastore,
    source_identity: ClientJobLocator,
    import_type: String,
    payload_json: String,
) -> AppResult<String> {
    queue_import_request_with_identity(datastore, source_identity, import_type, payload_json, None)
        .await
}

pub async fn queue_import_request_with_identity(
    datastore: &StoreDatastore,
    source_identity: ClientJobLocator,
    import_type: String,
    payload_json: String,
    submission_identity: Option<DownloadSubmissionIdentity>,
) -> AppResult<String> {
    queue_import_request_with_identity_for_download(
        datastore,
        source_identity,
        import_type,
        payload_json,
        submission_identity,
        None,
    )
    .await
}

pub async fn queue_import_request_with_identity_for_download(
    datastore: &StoreDatastore,
    source_identity: ClientJobLocator,
    import_type: String,
    payload_json: String,
    submission_identity: Option<DownloadSubmissionIdentity>,
    canonical_download_id: Option<&DownloadId>,
) -> AppResult<String> {
    let normalized_client_id = source_identity
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let lookup_client_id = normalized_client_id.as_deref().unwrap_or("").to_string();
    let is_rename = ImportType::parse(&import_type).is_some_and(|kind| kind.is_rename());
    let rename_plan_json = is_rename.then_some(payload_json.clone());
    let payload_arg = json_arg_for_datastore(datastore, Some(&payload_json))?;
    let rename_arg = json_arg_for_datastore(datastore, rename_plan_json.as_deref())?;
    let result_arg = json_arg_for_datastore(datastore, None::<&str>)?;
    let download_id = submission_identity
        .as_ref()
        .and_then(|identity| identity.download_id.clone());
    let canonical_download_id = canonical_download_id.map(ToString::to_string);
    let has_download_id = download_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let id = Id::new().0;
    let now = Utc::now();

    SqlRuntime::run_in_transaction(datastore, "create_import_request", move |tx| {
        let source_identity = source_identity.clone();
        let import_type = import_type.clone();
        let normalized_client_id = normalized_client_id.clone();
        let lookup_client_id = lookup_client_id.clone();
        let payload_arg = payload_arg.clone();
        let rename_arg = rename_arg.clone();
        let result_arg = result_arg.clone();
        let download_id = download_id.clone();
        let canonical_download_id = canonical_download_id.clone();
        let has_download_id = has_download_id;
        let id = id.clone();
        Box::pin(async move {
            let source_system = source_identity.client_type.clone();
            let source_ref = source_identity.item_id.clone();
            let import_type_key = import_type.clone();
            if has_download_id
                && let Some(existing_id) = find_active_import_by_download_id_tx(
                    tx,
                    normalized_client_id.as_deref(),
                    &source_system,
                    download_id.as_deref(),
                )
                .await?
            {
                return Ok(existing_id);
            }
            let import_sql = if has_download_id {
                import_request_insert_active_download_id_sql(tx)
            } else {
                import_request_upsert_sql(tx)
            };
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                &import_sql,
                &[
                    SqlArg::Text(id.clone()),
                    SqlArg::OptText(normalized_client_id.clone()),
                    SqlArg::Text(source_system.clone()),
                    SqlArg::Text(source_ref.clone()),
                    SqlArg::Text(import_type),
                    SqlArg::Text(ImportStatus::Pending.as_str().to_string()),
                    payload_arg,
                    rename_arg,
                    result_arg,
                    SqlArg::OptText(download_id.clone()),
                    SqlArg::OptText(canonical_download_id.clone()),
                    SqlArg::OptTimestamp(None),
                    SqlArg::OptTimestamp(None),
                    SqlArg::Timestamp(now),
                    SqlArg::Timestamp(now),
                ],
            )
            .await?;

            if has_download_id {
                return find_active_import_by_download_id_tx(
                    tx,
                    normalized_client_id.as_deref(),
                    &source_system,
                    download_id.as_deref(),
                )
                .await?
                .ok_or_else(|| AppError::Repository("failed to reload durable import".into()));
            }

            let row = SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT id FROM imports
                 WHERE COALESCE(source_client_id, '') = {}
                   AND source_system = {}
                   AND source_ref = {}
                   AND import_type = {}
                 LIMIT 1",
                &[
                    SqlArg::Text(lookup_client_id),
                    SqlArg::Text(source_system),
                    SqlArg::Text(source_ref),
                    SqlArg::Text(import_type_key),
                ],
            )
            .await?
            .ok_or_else(|| AppError::Repository("failed to reload persisted import".into()))?;
            row.text("id")
        })
    })
    .await
}

async fn find_active_import_by_download_id_tx(
    tx: &mut SqlTx<'_>,
    source_client_id: Option<&str>,
    source_system: &str,
    download_id: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(download_id) = download_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id
         FROM imports
         WHERE status IN ('pending', 'running', 'processing')
           AND COALESCE(source_client_id, '') = {}
           AND source_system = {}
           AND download_id = {}
         ORDER BY updated_at DESC, created_at DESC, id DESC
         LIMIT 1",
        &[
            SqlArg::Text(source_client_id.unwrap_or("").to_string()),
            SqlArg::Text(source_system.to_string()),
            SqlArg::Text(download_id.to_string()),
        ],
    )
    .await?;
    row.map(|row| row.text("id")).transpose()
}

pub fn import_request_insert_sql() -> String {
    "INSERT INTO imports
     (id, source_client_id, source_system, source_ref, import_type, status, payload_json, rename_plan_json, result_json, download_id, canonical_download_id, started_at, finished_at, created_at, updated_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})"
        .to_string()
}

pub fn import_request_insert_active_download_id_sql(tx: &SqlTx<'_>) -> String {
    let conflict_clause = match tx {
        SqlTx::Sqlite(_) => "ON CONFLICT DO NOTHING",
        SqlTx::Postgres(_) => {
            "ON CONFLICT ((COALESCE(source_client_id, '')), source_system, download_id)
             WHERE download_id IS NOT NULL AND status IN ('pending', 'running', 'processing')
             DO NOTHING"
        }
    };

    format!("{} {conflict_clause}", import_request_insert_sql())
}

pub fn import_request_upsert_sql(tx: &SqlTx<'_>) -> String {
    import_request_upsert_sql_for_backend(matches!(tx, SqlTx::Postgres(_)))
}

fn import_request_upsert_sql_for_backend(is_postgres: bool) -> String {
    let conflict_clause = import_request_upsert_conflict_clause(is_postgres);

    format!(
        "{} {conflict_clause} SET
            source_client_id = excluded.source_client_id,
            status = excluded.status,
            payload_json = excluded.payload_json,
            rename_plan_json = excluded.rename_plan_json,
            result_json = NULL,
            download_id = excluded.download_id,
            canonical_download_id = COALESCE(excluded.canonical_download_id, imports.canonical_download_id),
            import_transfer_phase = NULL,
            import_transfer_bytes = NULL,
            import_transfer_total_bytes = NULL,
            import_transfer_started_at = NULL,
            import_transfer_updated_at = NULL,
            started_at = NULL,
            finished_at = NULL,
            updated_at = excluded.updated_at",
        import_request_insert_sql()
    )
}

fn import_request_upsert_conflict_clause(is_postgres: bool) -> &'static str {
    if is_postgres {
        "ON CONFLICT ((COALESCE(source_client_id, '')), source_system, source_ref, import_type) WHERE download_id IS NULL DO UPDATE"
    } else {
        "ON CONFLICT DO UPDATE"
    }
}

pub async fn recover_stale_processing_imports(
    datastore: &StoreDatastore,
    import_type: Option<ImportType>,
    stale_seconds: i64,
) -> AppResult<u64> {
    let now = Utc::now();
    let cutoff = now - chrono::Duration::seconds(stale_seconds);
    let mut args = vec![
        json_arg_for_datastore(datastore, Some("{\"error\":\"stale processing recovery\"}"))?,
        SqlArg::Timestamp(now),
        SqlArg::Timestamp(now),
    ];
    let type_filter = if let Some(import_type) = import_type {
        args.push(SqlArg::Text(import_type.as_str().to_string()));
        "AND import_type = {}"
    } else {
        "AND import_type != 'manual_import'"
    };
    args.push(SqlArg::Timestamp(cutoff));
    let rows = execute_write(
        datastore,
        "recover_stale_processing_imports",
        format!(
            "UPDATE imports
             SET status = 'failed',
                 result_json = {{}},
                 finished_at = {{}},
                 updated_at = {{}}
             WHERE status = 'processing'
               {type_filter}
               AND updated_at < {{}}"
        ),
        args,
    )
    .await?;
    Ok(rows)
}

/// Boot-time reconciliation for persisted job runs whose worker died mid-flight.
///
/// Invariant: a `workflow_operations` row in a non-terminal state (`queued`,
/// `running`, `discovering`) is only advanced by the in-process worker that owns
/// it. That worker lives in memory, so once the process restarts it is gone and
/// the run can never reach a terminal state on its own — it is unfinishable.
/// Left alone it would poll as "running" forever (the jobs UI and the
/// acquisition-search view both surface these), so at boot we fail them and
/// clear `progress_json` so any state derived from it (e.g. the
/// acquisition-search view) falls back to the now-terminal `failed` status.
pub async fn reconcile_interrupted_job_runs(
    datastore: &StoreDatastore,
    excluded_run_ids: &[String],
) -> AppResult<u64> {
    let now = Utc::now();
    let excluded_clause = if excluded_run_ids.is_empty() {
        String::new()
    } else {
        format!(" AND id NOT IN ({})", placeholders(excluded_run_ids.len()))
    };
    let mut args = vec![SqlArg::Timestamp(now), SqlArg::Timestamp(now)];
    args.extend(excluded_run_ids.iter().cloned().map(SqlArg::Text));
    let rows = execute_write(
        datastore,
        "reconcile_interrupted_job_runs",
        format!(
            "UPDATE workflow_operations
             SET status = '{}',
                 progress_json = NULL,
                 error_text = 'interrupted by restart',
                 completed_at = {{}},
                 updated_at = {{}}
             WHERE job_key IS NOT NULL
               AND status IN ('{}', '{}', '{}'){excluded_clause}",
            JobRunStatus::Failed.as_str(),
            JobRunStatus::Queued.as_str(),
            JobRunStatus::Running.as_str(),
            JobRunStatus::Discovering.as_str(),
        ),
        args,
    )
    .await?;
    Ok(rows)
}

pub async fn create_workflow_operation(
    datastore: &StoreDatastore,
    operation: NewWorkflowOperation,
) -> AppResult<WorkflowOperationRecord> {
    let progress_arg = json_arg_for_datastore(datastore, operation.progress_json.as_deref())?;
    let summary_arg = json_arg_for_datastore(datastore, operation.summary_json.as_deref())?;
    let id = Id::new().0;
    let now = Utc::now();
    SqlRuntime::run_in_transaction(datastore, "create_workflow_operation", move |tx| {
        let id = id.clone();
        let operation = operation.clone();
        let progress_arg = progress_arg.clone();
        let summary_arg = summary_arg.clone();
        Box::pin(async move {
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "INSERT INTO workflow_operations
                 (id, operation_type, status, job_key, trigger_source, actor_user_id, progress_json, summary_json, summary_text, error_text, started_at, completed_at, created_at, updated_at)
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                &[
                    SqlArg::Text(id.clone()),
                    SqlArg::Text(operation.operation_type.clone()),
                    SqlArg::Text(operation.status.clone()),
                    SqlArg::OptText(operation.job_key.clone()),
                    SqlArg::OptText(operation.trigger_source.clone()),
                    SqlArg::OptText(operation.actor_user_id.clone()),
                    progress_arg,
                    summary_arg,
                    SqlArg::OptText(operation.summary_text.clone()),
                    SqlArg::OptText(operation.error_text.clone()),
                    opt_timestamp_arg(operation.started_at.as_deref()).or_timestamp(now),
                    opt_timestamp_arg(operation.completed_at.as_deref()),
                    SqlArg::Timestamp(now),
                    SqlArg::Timestamp(now),
                ],
            )
            .await?;
            fetch_optional_workflow_operation(SqlExec::Tx(tx), &id)
                .await?
                .ok_or_else(|| AppError::Repository("failed to reload workflow operation".into()))
        })
    })
    .await
}

trait SqlArgExt {
    fn or_timestamp(self, fallback: DateTime<Utc>) -> SqlArg;
}

impl SqlArgExt for SqlArg {
    fn or_timestamp(self, fallback: DateTime<Utc>) -> SqlArg {
        match self {
            SqlArg::OptTimestamp(None) => SqlArg::Timestamp(fallback),
            other => other,
        }
    }
}

pub fn build_domain_event_list_sql(filter: &DomainEventFilter) -> (String, Vec<SqlArg>) {
    let limit = if filter.limit == 0 {
        100
    } else {
        filter.limit.min(500)
    };
    let mut where_clauses = Vec::new();
    let mut args = Vec::new();
    if let Some(event_types) = filter.event_types.as_ref()
        && !event_types.is_empty()
    {
        where_clauses.push(format!(
            "event_type IN ({})",
            placeholders(event_types.len())
        ));
        args.extend(
            event_types
                .iter()
                .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
        );
    }
    if let Some(title_id) = filter.title_id.as_ref() {
        where_clauses.push("title_id = {}".to_string());
        args.push(SqlArg::Text(title_id.clone()));
    }
    if let Some(facet) = filter.facet.as_ref() {
        where_clauses.push("facet = {}".to_string());
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }
    if let Some(after_sequence) = filter.after_sequence {
        where_clauses.push("sequence > {}".to_string());
        args.push(SqlArg::I64(after_sequence));
    }
    if let Some(before_sequence) = filter.before_sequence {
        where_clauses.push("sequence < {}".to_string());
        args.push(SqlArg::I64(before_sequence));
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clauses.join(" AND "))
    };
    let order = if filter.after_sequence.is_some() && filter.before_sequence.is_none() {
        "ASC"
    } else {
        "DESC"
    };
    args.push(SqlArg::I64(limit as i64));
    (
        format!(
            "SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events{where_sql} ORDER BY sequence {order} LIMIT {{}}"
        ),
        args,
    )
}

pub fn build_title_history_filter_sql(
    _datastore: &StoreDatastore,
    event_types: Option<&[TitleHistoryEventType]>,
    title_ids: Option<&[String]>,
    download_id: Option<&str>,
) -> (String, Vec<SqlArg>) {
    let mut clauses = vec!["title_id IS NOT NULL".to_string()];
    let mut args = Vec::new();
    match event_types {
        None => {
            clauses.push(format!(
                "event_type IN ({})",
                placeholders(TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES.len())
            ));
            args.extend(
                TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES
                    .iter()
                    .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
            );
        }
        Some([]) => clauses.push("0".to_string()),
        Some(event_types) => {
            let mut parts = Vec::new();
            for event_type in event_types {
                match event_type {
                    TitleHistoryEventType::Requested => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::MediaRequestSubmitted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Grabbed => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ReleaseGrabbed.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::DownloadFailed => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::DownloadFailed.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Blocklisted => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ReleaseBlocklisted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Scanned => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::MediaFileAnalyzed.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Imported => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ImportCompleted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::ImportFailed => {
                        parts.push("(event_type = {} AND import_status = {})".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ImportRejected.as_str().into(),
                        ));
                        args.push(SqlArg::Text(ImportStatus::Failed.as_str().into()));
                    }
                    TitleHistoryEventType::ImportSkipped => {
                        parts.push("(event_type = {} AND import_status = {})".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::ImportRejected.as_str().into(),
                        ));
                        args.push(SqlArg::Text(ImportStatus::Skipped.as_str().into()));
                    }
                    TitleHistoryEventType::FileUpgraded => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::MediaFileUpgraded.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::FileRecycled => {
                        parts.push(
                            "(event_type = {} AND media_file_delete_reason = {})".to_string(),
                        );
                        args.push(SqlArg::Text(
                            DomainEventType::MediaFileDeleted.as_str().into(),
                        ));
                        args.push(SqlArg::Text(
                            MediaFileDeletedReason::UpgradeCleanup.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::FileDeleted => {
                        parts.push("(event_type = {} AND (media_file_delete_reason IS NULL OR media_file_delete_reason <> {}))".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::MediaFileDeleted.as_str().into(),
                        ));
                        args.push(SqlArg::Text(
                            MediaFileDeletedReason::UpgradeCleanup.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::FileRenamed => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::MediaFileRenamed.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::Rematched => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::TitleRematched.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::SeedingStarted => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::SeedingStarted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::SeedingCompleted => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::SeedingCompleted.as_str().into(),
                        ));
                    }
                    TitleHistoryEventType::DownloadIgnored => {
                        parts.push("event_type = {}".to_string());
                        args.push(SqlArg::Text(
                            DomainEventType::DownloadIgnored.as_str().into(),
                        ));
                    }
                    // No `DomainEventType::DownloadCompleted` exists and no
                    // projection ever produces this history type, so nothing
                    // is stored that could match: a never-match clause is the
                    // correct filter, not a bug. Give it a real event type
                    // here if one is ever recorded.
                    TitleHistoryEventType::DownloadCompleted => parts.push("0".to_string()),
                }
            }
            clauses.push(format!("({})", parts.join(" OR ")));
        }
    }
    if let Some(title_ids) = title_ids {
        if title_ids.is_empty() {
            clauses.push("0".to_string());
        } else if title_ids.len() == 1 {
            clauses.push("title_id = {}".to_string());
            args.push(SqlArg::Text(title_ids[0].clone()));
        } else {
            clauses.push(format!("title_id IN ({})", placeholders(title_ids.len())));
            args.extend(title_ids.iter().cloned().map(SqlArg::Text));
        }
    }
    if let Some(download_id) = download_id {
        clauses.push("download_id = {}".to_string());
        args.push(SqlArg::Text(download_id.to_string()));
    }
    (format!(" WHERE {}", clauses.join(" AND ")), args)
}

/// Domain event types counted by the dashboard activity aggregate, in the
/// order their placeholders are bound.
pub const DASHBOARD_ACTIVITY_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::ReleaseGrabbed,
    DomainEventType::MediaFileUpgraded,
    DomainEventType::ImportCompleted,
    DomainEventType::ImportRejected,
    DomainEventType::DownloadFailed,
];

/// Build the grouped `COUNT(*)` that powers the dashboard activity tiles.
///
/// One statement covers both windows: the `window_key` projection buckets each
/// row into `current` (`>= current_start`) or `previous`, and the grouping keys
/// are the event type plus the import status extracted from the payload, which
/// is what separates a failed import from a skipped one. Library scoping is a
/// join onto `titles` rather than an `IN` list of title ids, so the bind count
/// stays proportional to the number of libraries instead of the catalog size.
///
/// Callers must reject an empty `library_ids` before calling: an empty `IN ()`
/// list is not valid SQL on either dialect.
pub fn build_dashboard_activity_stats_sql(
    _datastore: &StoreDatastore,
    library_ids: &[String],
    previous_start: DateTime<Utc>,
    current_start: DateTime<Utc>,
    current_end: DateTime<Utc>,
) -> (String, Vec<SqlArg>) {
    let mut args = vec![SqlArg::Timestamp(current_start)];
    args.push(SqlArg::Timestamp(previous_start));
    args.push(SqlArg::Timestamp(current_end));
    args.extend(
        DASHBOARD_ACTIVITY_DOMAIN_EVENT_TYPES
            .iter()
            .map(|event_type| SqlArg::Text(event_type.as_str().to_string())),
    );
    args.push(SqlArg::Text(
        DomainEventType::ImportRejected.as_str().into(),
    ));
    args.push(SqlArg::Text(ImportStatus::Failed.as_str().into()));
    args.extend(library_ids.iter().cloned().map(SqlArg::Text));

    let sql = format!(
        "SELECT
             CASE WHEN domain_events.occurred_at >= {{}} THEN 'current' ELSE 'previous' END AS window_key,
             domain_events.event_type AS event_type,
             domain_events.import_status AS import_status,
             COUNT(*) AS event_count
           FROM domain_events
           JOIN titles ON titles.id = domain_events.title_id
          WHERE domain_events.occurred_at >= {{}}
            AND domain_events.occurred_at < {{}}
            AND domain_events.event_type IN ({event_type_placeholders})
            AND (domain_events.event_type <> {{}} OR domain_events.import_status = {{}})
            AND titles.library_id IN ({library_placeholders})
          GROUP BY window_key, event_type, import_status",
        event_type_placeholders = placeholders(DASHBOARD_ACTIVITY_DOMAIN_EVENT_TYPES.len()),
        library_placeholders = placeholders(library_ids.len()),
    );
    (sql, args)
}

pub const TITLE_HISTORY_PAGE_DOMAIN_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::TitleRematched,
    DomainEventType::ReleaseGrabbed,
    DomainEventType::ImportCompleted,
    DomainEventType::ImportRejected,
    DomainEventType::DownloadFailed,
    DomainEventType::DownloadIgnored,
    DomainEventType::ReleaseBlocklisted,
    DomainEventType::MediaFileAnalyzed,
    DomainEventType::MediaFileUpgraded,
    DomainEventType::MediaFileDeleted,
    DomainEventType::MediaFileRenamed,
    DomainEventType::SeedingStarted,
    DomainEventType::SeedingCompleted,
];

pub fn json_extract(datastore: &StoreDatastore, column: &str, first: &str, second: &str) -> String {
    match datastore {
        StoreDatastore::Sqlite { .. } => format!("json_extract({column}, '$.{first}.{second}')"),
        StoreDatastore::Postgres { .. } => format!("{column} #>> '{{{first},{second}}}'"),
    }
}

pub fn download_submission_select_sql(datastore: &StoreDatastore, suffix: &str) -> String {
    let episode_set = match datastore {
        StoreDatastore::Sqlite { .. } => {
            "(SELECT group_concat(link.episode_id, char(31))
                FROM download_submission_episode_links link
               WHERE link.download_id = download_submissions.id)"
        }
        StoreDatastore::Postgres { .. } => {
            "(SELECT string_agg(link.episode_id, chr(31))
                FROM download_submission_episode_links link
               WHERE link.download_id = download_submissions.id)"
        }
    };
    format!(
        "SELECT {DOWNLOAD_SUBMISSION_COLUMNS}, {episode_set} AS episode_set_ids FROM download_submissions {suffix}"
    )
}

pub async fn fetch_domain_events(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DomainEvent>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| domain_event_from_row(&row))
        .collect()
}

pub async fn fetch_domain_event_by_event_id(
    exec: SqlExec<'_, '_>,
    event_id: &str,
) -> AppResult<Option<DomainEvent>> {
    SqlRuntime::fetch_optional(
        exec,
        &format!("SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events WHERE event_id = {{}}"),
        &[SqlArg::Text(event_id.to_string())],
    )
    .await?
    .map(|row| domain_event_from_row(&row))
    .transpose()
}

pub async fn fetch_download_submissions(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DownloadSubmission>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| download_submission_from_row(&row))
        .collect()
}

pub async fn fetch_imports(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<ImportRecord>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| import_record_from_row(&row))
        .collect()
}

pub async fn fetch_snapshot_chunks(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await
        .map_err(map_snapshot_chunk_error)?
        .into_iter()
        .map(|row| snapshot_chunk_from_row(&row))
        .collect()
}

pub async fn fetch_delete_commands(
    exec: SqlExec<'_, '_>,
    suffix: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DownloadQueueCommandRecord>> {
    SqlRuntime::fetch_all(
        exec,
        &format!("SELECT {DOWNLOAD_QUEUE_COMMAND_COLUMNS} FROM download_queue_commands {suffix}"),
        args,
    )
    .await?
    .into_iter()
    .map(|row| download_queue_command_from_row(&row))
    .collect()
}

pub async fn fetch_optional_delete_command(
    exec: SqlExec<'_, '_>,
    suffix: &str,
    args: &[SqlArg],
) -> AppResult<Option<DownloadQueueCommandRecord>> {
    SqlRuntime::fetch_optional(
        exec,
        &format!("SELECT {DOWNLOAD_QUEUE_COMMAND_COLUMNS} FROM download_queue_commands {suffix}"),
        args,
    )
    .await?
    .map(|row| download_queue_command_from_row(&row))
    .transpose()
}

pub async fn fetch_optional_workflow_operation(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<WorkflowOperationRecord>> {
    SqlRuntime::fetch_optional(
        exec,
        "SELECT * FROM workflow_operations WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?
    .map(|row| workflow_operation_from_row(&row))
    .transpose()
}

pub fn domain_event_from_row(row: &SqlRow) -> AppResult<DomainEvent> {
    let event_id = row.text("event_id")?;
    let event_type = row.text("event_type")?;
    let stream_kind = row.text("stream_kind")?;
    let encoded_payload = row.opt_bytes("payload_json")?.ok_or_else(|| {
        AppError::Repository(format!(
            "domain event {event_id} ({event_type}) payload is null"
        ))
    })?;
    let decoded_payload = decode_domain_event_payload(&encoded_payload).map_err(|error| {
        AppError::Repository(format!(
            "failed to decode domain event {event_id} ({event_type}): {error}"
        ))
    })?;
    let payload = serde_json::from_value(decoded_payload).map_err(|error| {
        AppError::Repository(format!(
            "failed to deserialize domain event {event_id} ({event_type}): {error}"
        ))
    })?;
    Ok(DomainEvent {
        sequence: row.i64("sequence")?,
        event_id,
        occurred_at: row.timestamp("occurred_at")?,
        actor_kind: DomainEventActorKind::parse(row.text("actor_kind")?.as_str())
            .unwrap_or(DomainEventActorKind::System),
        actor_user_id: row.opt_text("actor_user_id")?,
        actor_display_name: row.text("actor_display_name")?,
        title_id: row.opt_text("title_id")?,
        facet: row
            .opt_text("facet")?
            .as_deref()
            .and_then(MediaFacet::parse),
        correlation_id: row.opt_text("correlation_id")?,
        causation_id: row.opt_text("causation_id")?,
        schema_version: row.i32("schema_version")?,
        stream: stream_from_parts(&stream_kind, row.opt_text("stream_id")?)?,
        payload,
    })
}

pub fn stream_from_parts(kind: &str, identifier: Option<String>) -> AppResult<DomainEventStream> {
    match kind {
        "global" => Ok(DomainEventStream::Global),
        "title" => identifier
            .map(|title_id| DomainEventStream::Title { title_id })
            .ok_or_else(|| AppError::Repository("domain event missing title stream id".into())),
        "library_scan" => identifier
            .map(|session_id| DomainEventStream::LibraryScan { session_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing library scan stream id".into())
            }),
        "job_run" => identifier
            .map(|run_id| DomainEventStream::JobRun { run_id })
            .ok_or_else(|| AppError::Repository("domain event missing job run stream id".into())),
        "download_queue_item" => identifier
            .map(|item_id| DomainEventStream::DownloadQueueItem { item_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing download queue item stream id".into())
            }),
        other => Err(AppError::Repository(format!(
            "unknown domain event stream kind: {other}"
        ))),
    }
}

pub fn download_submission_from_row(row: &SqlRow) -> AppResult<DownloadSubmission> {
    let download_id = row.text("id")?;
    let download_id = DownloadId::parse(&download_id).ok_or_else(|| {
        AppError::Repository(format!(
            "invalid canonical download id {download_id:?} in download submission"
        ))
    })?;
    let title_id = row.text("title_id")?;
    let episode_id = opt_text_lenient(row, "episode_id")?;
    let collection_id = opt_text_lenient(row, "collection_id")?;
    let series_movie_link_id = opt_text_lenient(row, "series_movie_link_id")?;
    let episode_set_ids = opt_text_lenient(row, "episode_set_ids")?.map(|raw| {
        raw.split('\u{1f}')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let source_kind = row
        .opt_text("source_kind")?
        .as_deref()
        .and_then(scryer_application::DownloadSourceKind::parse);
    Ok(DownloadSubmission {
        download_id,
        scope: SubmissionScope::from_persisted(
            &title_id,
            episode_id,
            collection_id,
            series_movie_link_id,
            episode_set_ids,
        ),
        title_id,
        facet: row.text("facet")?,
        download_client_id: row
            .opt_text("download_client_id")?
            .filter(|value| !value.trim().is_empty()),
        download_client_type: row.text("download_client_type")?,
        download_client_item_id: row.text("download_client_item_id")?,
        source_hint: row.opt_text("source_hint")?,
        source_provider_id: row.opt_text("source_provider_id")?,
        source_provider_name: row.opt_text("source_provider_name")?,
        source_kind,
        source_title: row.opt_text("source_title")?,
        info_hash: row.opt_text("info_hash")?,
        release_size_bytes: row.opt_i64("release_size_bytes")?,
        request_signature: row.opt_text("request_signature")?,
        purpose: row
            .opt_text("purpose")?
            .as_deref()
            .map(scryer_application::DownloadSubmissionPurpose::from_label)
            .unwrap_or_default(),
    })
}

pub fn download_submission_actor_snapshot_from_row(
    row: &SqlRow,
) -> AppResult<Option<DownloadSubmissionActorSnapshot>> {
    let Some(kind_raw) = row.opt_text("actor_kind")? else {
        return Ok(None);
    };
    let kind = DomainEventActorKind::parse(kind_raw.as_str()).unwrap_or(DomainEventActorKind::User);
    let display_name = row
        .opt_text("actor_display_name")?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            row.opt_text("actor_user_id")
                .ok()
                .flatten()
                .unwrap_or_else(|| kind.as_str().to_string())
        });
    Ok(Some(DownloadSubmissionActorSnapshot {
        kind,
        user_id: row.opt_text("actor_user_id")?,
        display_name,
    }))
}

pub fn import_record_from_row(row: &SqlRow) -> AppResult<ImportRecord> {
    let import_type_raw = row.text("import_type")?;
    let status_raw = row.text("status")?;
    Ok(ImportRecord {
        id: row.text("id")?,
        source_client_id: row
            .opt_text("source_client_id")?
            .filter(|value| !value.trim().is_empty()),
        source_system: row.text("source_system")?,
        source_ref: row.text("source_ref")?,
        import_type: ImportType::parse(&import_type_raw).ok_or_else(|| {
            AppError::Repository(format!("unknown import_type: {import_type_raw}"))
        })?,
        status: ImportStatus::parse(&status_raw).unwrap_or_default(),
        payload_json: json_text_from_row(row, "payload_json")?.unwrap_or_default(),
        result_json: json_text_from_row(row, "result_json")?,
        download_id: row.opt_text("download_id")?,
        import_transfer_phase: row
            .opt_text("import_transfer_phase")?
            .as_deref()
            .and_then(ImportTransferPhase::parse),
        import_transfer_bytes: row.opt_i64("import_transfer_bytes")?,
        import_transfer_total_bytes: row.opt_i64("import_transfer_total_bytes")?,
        import_transfer_started_at: opt_timestamp_string(row, "import_transfer_started_at")?,
        import_transfer_updated_at: opt_timestamp_string(row, "import_transfer_updated_at")?,
        started_at: opt_timestamp_string(row, "started_at")?,
        finished_at: opt_timestamp_string(row, "finished_at")?,
        created_at: timestamp_string(row, "created_at")?,
        updated_at: timestamp_string(row, "updated_at")?,
    })
}

pub fn import_artifact_from_row(row: &SqlRow) -> AppResult<ImportArtifact> {
    Ok(ImportArtifact {
        id: row.text("id")?,
        source_client_id: row
            .opt_text("source_client_id")?
            .filter(|value| !value.trim().is_empty()),
        source_system: row.text("source_system")?,
        source_ref: row.text("source_ref")?,
        import_id: row.opt_text("import_id")?,
        relative_path: row.opt_text("relative_path")?,
        normalized_file_name: row.text("normalized_file_name")?,
        media_kind: row.text("media_kind")?,
        title_id: row.opt_text("title_id")?,
        episode_id: row.opt_text("episode_id")?,
        season_number: row.opt_i32("season_number")?,
        episode_number: row.opt_i32("episode_number")?,
        result: row.text("result")?,
        reason_code: row.opt_text("reason_code")?,
        imported_media_file_id: row.opt_text("imported_media_file_id")?,
        created_at: row.timestamp("created_at")?,
    })
}

pub fn snapshot_chunk_from_row(row: &SqlRow) -> AppResult<ExternalImportMonitorSnapshotChunk> {
    let facet_raw = row.text("facet")?;
    let entry_kind_raw = row.text("entry_kind")?;
    Ok(ExternalImportMonitorSnapshotChunk {
        session_id: row.text("session_id")?,
        facet: MediaFacet::parse(&facet_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid monitor snapshot chunk facet: {facet_raw}"))
        })?,
        entry_kind: ExternalImportMonitorSnapshotEntryKind::parse(&entry_kind_raw).ok_or_else(
            || {
                AppError::Repository(format!(
                    "invalid monitor snapshot chunk entry kind: {entry_kind_raw}"
                ))
            },
        )?,
        chunk_index: row.i32("chunk_index")?,
        payload_ndjson: row.text("payload_ndjson")?,
        created_at: timestamp_string(row, "created_at")?,
    })
}

pub fn download_queue_command_from_row(row: &SqlRow) -> AppResult<DownloadQueueCommandRecord> {
    let action = row.text("action")?;
    let status = row.text("status")?;
    Ok(DownloadQueueCommandRecord {
        id: row.text("id")?,
        action: DownloadQueueCommandAction::parse(&action).ok_or_else(|| {
            AppError::Repository(format!("unknown download queue action: {action}"))
        })?,
        canonical_download_id: row
            .opt_text("canonical_download_id")?
            .as_deref()
            .and_then(DownloadId::parse),
        client_id: row
            .opt_text("client_id")?
            .filter(|value| !value.trim().is_empty()),
        client_type: row.text("client_type")?,
        download_client_item_id: row.text("download_client_item_id")?,
        is_history: row.bool("is_history")?,
        status: DownloadQueueDeleteStatus::parse(&status).ok_or_else(|| {
            AppError::Repository(format!("unknown download queue command status: {status}"))
        })?,
        error_text: row.opt_text("error_text")?,
        requested_by_user_id: row.opt_text("requested_by_user_id")?,
        started_at: opt_timestamp_string(row, "started_at")?,
        finished_at: opt_timestamp_string(row, "finished_at")?,
        created_at: timestamp_string(row, "created_at")?,
        updated_at: timestamp_string(row, "updated_at")?,
    })
}

pub fn workflow_operation_from_row(row: &SqlRow) -> AppResult<WorkflowOperationRecord> {
    Ok(WorkflowOperationRecord {
        id: row.text("id")?,
        operation_type: row.text("operation_type")?,
        status: row.text("status")?,
        job_key: row.opt_text("job_key")?,
        trigger_source: row.opt_text("trigger_source")?,
        actor_user_id: row.opt_text("actor_user_id")?,
        title_id: row.opt_text("title_id")?,
        collection_id: row.opt_text("collection_id")?,
        episode_id: row.opt_text("episode_id")?,
        release_id: row.opt_text("release_id")?,
        media_file_id: row.opt_text("media_file_id")?,
        external_reference: row.opt_text("external_reference")?,
        progress_json: json_text_from_row(row, "progress_json")?,
        summary_json: json_text_from_row(row, "summary_json")?,
        summary_text: row.opt_text("summary_text")?,
        error_text: row.opt_text("error_text")?,
        started_at: opt_timestamp_string(row, "started_at")?,
        completed_at: opt_timestamp_string(row, "completed_at")?,
        created_at: timestamp_string(row, "created_at")?,
        updated_at: timestamp_string(row, "updated_at")?,
    })
}

pub fn job_run_record_from_workflow(record: WorkflowOperationRecord) -> AppResult<JobRunRecord> {
    let job_key = record
        .job_key
        .as_deref()
        .and_then(JobKey::parse)
        .ok_or_else(|| AppError::Repository("workflow operation missing valid job_key".into()))?;
    let trigger_source = record
        .trigger_source
        .as_deref()
        .and_then(JobTriggerSource::parse)
        .ok_or_else(|| {
            AppError::Repository("workflow operation missing valid trigger_source".into())
        })?;
    let status = JobRunStatus::parse(&record.status)
        .ok_or_else(|| AppError::Repository("workflow operation missing valid status".into()))?;
    Ok(JobRunRecord {
        id: record.id,
        job_key,
        operation_type: record.operation_type,
        status,
        trigger_source,
        actor_user_id: record.actor_user_id,
        progress_json: record.progress_json,
        summary_json: record.summary_json,
        summary_text: record.summary_text,
        error_text: record.error_text,
        started_at: parse_datetime_or_now(record.started_at.as_deref()),
        completed_at: record
            .completed_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
        created_at: parse_datetime_or_now(Some(&record.created_at)),
        updated_at: parse_datetime_or_now(Some(&record.updated_at)),
    })
}

pub fn workflow_operation_info(record: WorkflowOperationRecord) -> WorkflowOperationInfo {
    WorkflowOperationInfo {
        id: record.id,
        operation_type: record.operation_type,
        status: record.status,
        actor_user_id: record.actor_user_id,
        progress_json: record.progress_json,
        started_at: record.started_at,
        completed_at: record.completed_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

pub async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: String,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await
}

pub async fn update_delete_command_status(
    datastore: &StoreDatastore,
    id: &str,
    status: DownloadQueueDeleteStatus,
    error_text: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now();
    let started_at = match status {
        DownloadQueueDeleteStatus::Running => Some(now),
        _ => None,
    };
    let finished_at = match status {
        DownloadQueueDeleteStatus::Completed | DownloadQueueDeleteStatus::Failed => Some(now),
        _ => None,
    };
    execute_write(
        datastore,
        "update_delete_download_command_status",
        "UPDATE download_queue_commands
         SET status = {},
             error_text = {},
             started_at = COALESCE({}, started_at),
             finished_at = {},
             updated_at = {}
         WHERE id = {}"
            .to_string(),
        vec![
            SqlArg::Text(status.as_str().to_string()),
            SqlArg::OptText(error_text.map(str::to_string)),
            SqlArg::OptTimestamp(started_at),
            SqlArg::OptTimestamp(finished_at),
            SqlArg::Timestamp(now),
            SqlArg::Text(id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

pub fn persisted_submission_scope(
    scope: &SubmissionScope,
) -> (Option<&str>, Option<&str>, Option<&str>) {
    (
        scope.persisted_episode_id(),
        scope.persisted_collection_id(),
        scope.persisted_series_movie_link_id(),
    )
}

pub fn persisted_episode_set_ids(scope: &SubmissionScope) -> &[String] {
    match scope {
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.as_slice(),
        _ => &[],
    }
}

const DOWNLOAD_SUBMISSION_BATCH_LOOKUP_CHUNK_SIZE: usize = 400;

pub fn chunk_download_submission_client_items(
    client_items: &[ClientJobLocator],
) -> Vec<Vec<ClientJobLocator>> {
    let deduped = dedupe_identities(client_items);
    deduped
        .chunks(DOWNLOAD_SUBMISSION_BATCH_LOOKUP_CHUNK_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

pub fn dedupe_identities(identities: &[ClientJobLocator]) -> Vec<ClientJobLocator> {
    let mut seen = HashSet::with_capacity(identities.len());
    let mut deduped = Vec::with_capacity(identities.len());
    for identity in identities {
        if seen.insert((
            normalize_download_client_id(identity.client_id.as_deref()),
            identity.client_type.clone(),
            identity.item_id.clone(),
        )) {
            deduped.push(identity.clone());
        }
    }
    deduped
}

pub fn normalize_download_client_id(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

pub fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn json_arg_for_datastore(
    datastore: &StoreDatastore,
    value: Option<&str>,
) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::OptText(value.map(str::to_string))),
        StoreDatastore::Postgres { .. } => value
            .map(postgres_json_value)
            .transpose()
            .map(SqlArg::OptJson),
    }
}

pub fn json_arg_for_tx(tx: &SqlTx<'_>, value: Option<&str>) -> AppResult<SqlArg> {
    match tx {
        SqlTx::Sqlite(_) => Ok(SqlArg::OptText(value.map(str::to_string))),
        SqlTx::Postgres(_) => value
            .map(postgres_json_value)
            .transpose()
            .map(SqlArg::OptJson),
    }
}

pub fn postgres_json_value(value: &str) -> AppResult<JsonValue> {
    Ok(serde_json::from_str(value).unwrap_or_else(|_| JsonValue::String(value.to_string())))
}

pub fn json_from_row(row: &SqlRow, column: &str) -> AppResult<JsonValue> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            serde_json::from_str(&raw).map_err(repo_err)
        }
        SqlRow::Postgres(row) => {
            let raw: Json<JsonValue> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.0)
        }
    }
}

pub fn json_text_from_row(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
        SqlRow::Postgres(row) => {
            let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.map(|value| json_value_as_string(value.0)))
        }
    }
}

pub fn json_value_as_string(value: JsonValue) -> String {
    match value {
        JsonValue::String(value) => value,
        value => value.to_string(),
    }
}

pub fn opt_text_lenient(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => Ok(row.try_get::<Option<String>, _>(column).ok().flatten()),
        SqlRow::Postgres(row) => Ok(row.try_get::<Option<String>, _>(column).ok().flatten()),
    }
}

pub fn timestamp_string(row: &SqlRow, column: &str) -> AppResult<String> {
    match row {
        SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
        SqlRow::Postgres(row) => {
            let value: DateTime<Utc> = row.try_get(column).map_err(repo_err)?;
            Ok(value.to_rfc3339())
        }
    }
}

pub fn opt_timestamp_string(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
        SqlRow::Postgres(row) => {
            let value: Option<DateTime<Utc>> = row.try_get(column).map_err(repo_err)?;
            Ok(value.map(|value| value.to_rfc3339()))
        }
    }
}

pub fn opt_timestamp_arg(value: Option<&str>) -> SqlArg {
    SqlArg::OptTimestamp(
        value
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
    )
}

pub fn parse_datetime_or_now(value: Option<&str>) -> DateTime<Utc> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

pub fn map_snapshot_chunk_error(error: AppError) -> AppError {
    let message = error.to_string();
    if message.contains("no such table: external_import_monitor_snapshot_chunks") {
        return AppError::Repository(
            "database is missing external_import_monitor_snapshot_chunks; restart with a build that includes migration 0117_external_import_monitor_snapshot_chunks and let migrations complete".into(),
        );
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_event_list_sql_pushes_down_event_type_filter() {
        let (sql, args) = build_domain_event_list_sql(&DomainEventFilter {
            after_sequence: Some(42),
            event_types: Some(vec![
                DomainEventType::TitleAdded,
                DomainEventType::ImportCompleted,
            ]),
            limit: 100,
            ..DomainEventFilter::default()
        });

        assert!(sql.contains("event_type IN ({}, {})"));
        assert!(sql.contains("sequence > {}"));
        assert!(sql.contains("ORDER BY sequence ASC"));
        assert_eq!(args.len(), 4);
    }

    #[test]
    fn postgres_import_request_upsert_qualifies_target_canonical_download_id() {
        let sql = import_request_upsert_sql_for_backend(true);

        assert!(sql.contains(
            "canonical_download_id = COALESCE(excluded.canonical_download_id, imports.canonical_download_id)"
        ));
    }
}
