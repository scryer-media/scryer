use async_graphql::{Context, Error, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::RenameApplyResult;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use crate::context::{actor_from_ctx, app_from_ctx, settings_db_from_ctx, to_gql_error};
use crate::mappers::{from_library_scan_summary, from_media_rename_apply};
use crate::types::*;
use crate::utils::parse_facet;

static RENAME_IDEMPOTENCY_KEYS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn claim_rename_idempotency_key(scope: &str, key: Option<String>) -> GqlResult<Option<String>> {
    let Some(raw_key) = key else {
        return Ok(None);
    };

    let normalized = raw_key.trim();
    if normalized.is_empty() {
        return Err(Error::new("idempotencyKey cannot be empty"));
    }

    let composite = format!("{scope}:{normalized}");
    let store = &*RENAME_IDEMPOTENCY_KEYS;
    let mut guard = store
        .lock()
        .map_err(|_| Error::new("failed to lock rename idempotency key store"))?;
    if !guard.insert(composite.clone()) {
        return Err(Error::new("duplicate idempotencyKey"));
    }

    Ok(Some(composite))
}

async fn record_rename_apply_audit(
    db: &scryer_infrastructure::SqliteServices,
    actor_user_id: &str,
    operation: &str,
    facet: &str,
    title_id: Option<&str>,
    idempotency_key: Option<&str>,
    result: &RenameApplyResult,
) -> Result<(), scryer_application::AppError> {
    let now = Utc::now().to_rfc3339();
    let plan_fingerprint = result.plan_fingerprint.clone();
    let progress_json = json!({
        "operation": operation,
        "facet": facet,
        "title_id": title_id,
        "idempotency_key": idempotency_key,
        "plan_fingerprint": plan_fingerprint.clone(),
        "total": result.total,
        "applied": result.applied,
        "skipped": result.skipped,
        "failed": result.failed,
    })
    .to_string();

    let _ = db
        .create_workflow_operation(
            operation,
            "completed",
            Some(actor_user_id.to_string()),
            Some(progress_json),
            Some(now.clone()),
            Some(now),
        )
        .await?;

    let source_ref = if let Some(key) = idempotency_key {
        format!("{operation}:{key}")
    } else if let Some(title_id) = title_id {
        format!("{operation}:title:{title_id}:{plan_fingerprint}")
    } else {
        format!("{operation}:facet:{facet}:{plan_fingerprint}")
    };
    let payload_json = serde_json::to_string(result)
        .unwrap_or_else(|_| "{\"error\":\"failed_to_serialize_rename_apply_result\"}".to_string());

    let _ = db
        .create_import_request(
            "scryer_rename".to_string(),
            source_ref,
            "rename_apply_result".to_string(),
            payload_json,
        )
        .await?;

    Ok(())
}

#[derive(Default)]
pub(crate) struct LibraryMutations;

#[Object]
impl LibraryMutations {
    async fn scan_library(
        &self,
        ctx: &Context<'_>,
        facet: String,
    ) -> GqlResult<LibraryScanSummaryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let facet =
            parse_facet(Some(facet)).ok_or_else(|| Error::new("invalid facet for scanLibrary"))?;
        let summary = app
            .scan_library(&actor, facet)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_scan_summary(summary))
    }

    async fn scan_title_library(
        &self,
        ctx: &Context<'_>,
        input: TitleIdInput,
    ) -> GqlResult<LibraryScanSummaryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let summary = app
            .scan_title_library(&actor, &input.title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_scan_summary(summary))
    }

    async fn apply_media_rename(
        &self,
        ctx: &Context<'_>,
        input: MediaRenameApplyInput,
    ) -> GqlResult<MediaRenameApplyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;
        let MediaRenameApplyInput {
            facet,
            title_id,
            fingerprint,
            idempotency_key,
        } = input;
        let facet = parse_facet(Some(facet))
            .ok_or_else(|| Error::new("invalid facet for applyMediaRename"))?;
        let facet_name = match facet {
            scryer_domain::MediaFacet::Movie => "movie",
            scryer_domain::MediaFacet::Tv => "tv",
            scryer_domain::MediaFacet::Anime => "anime",
            scryer_domain::MediaFacet::Other => "other",
        };
        let idempotency_key = claim_rename_idempotency_key("apply_media_rename", idempotency_key)?;

        let result = app
            .apply_rename_for_title(&actor, &title_id, facet, &fingerprint)
            .await
            .map_err(to_gql_error)?;
        let _ = record_rename_apply_audit(
            &db,
            &actor.id,
            "rename_apply_title",
            facet_name,
            Some(&title_id),
            idempotency_key.as_deref(),
            &result,
        )
        .await;

        Ok(from_media_rename_apply(result))
    }

    async fn delete_media_file(
        &self,
        ctx: &Context<'_>,
        input: DeleteMediaFileInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_media_file(
            &actor,
            &input.file_id,
            input.delete_from_disk.unwrap_or(true),
        )
        .await
        .map(|_| true)
        .map_err(to_gql_error)
    }

    async fn apply_media_rename_bulk(
        &self,
        ctx: &Context<'_>,
        input: MediaRenameBulkApplyInput,
    ) -> GqlResult<MediaRenameApplyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let db = settings_db_from_ctx(ctx)?;
        let MediaRenameBulkApplyInput {
            facet,
            fingerprint,
            idempotency_key,
        } = input;
        let facet = parse_facet(Some(facet))
            .ok_or_else(|| Error::new("invalid facet for applyMediaRenameBulk"))?;
        let facet_name = match facet {
            scryer_domain::MediaFacet::Movie => "movie",
            scryer_domain::MediaFacet::Tv => "tv",
            scryer_domain::MediaFacet::Anime => "anime",
            scryer_domain::MediaFacet::Other => "other",
        };
        let idempotency_key =
            claim_rename_idempotency_key("apply_media_rename_bulk", idempotency_key)?;

        let result = app
            .apply_rename_for_facet(&actor, facet, &fingerprint)
            .await
            .map_err(to_gql_error)?;
        let _ = record_rename_apply_audit(
            &db,
            &actor.id,
            "rename_apply_facet",
            facet_name,
            None,
            idempotency_key.as_deref(),
            &result,
        )
        .await;

        Ok(from_media_rename_apply(result))
    }

    async fn rehydrate_all_metadata(
        &self,
        ctx: &Context<'_>,
        language: String,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let db = settings_db_from_ctx(ctx)?;

        let language = language.trim().to_ascii_lowercase();
        if language.is_empty() {
            return Err(Error::new("language is required"));
        }

        // Save language preference
        db.upsert_setting_value(
            "system",
            "metadata_language",
            None,
            &language,
            "rehydrate_metadata",
            Some(actor.id),
        )
        .await
        .map_err(to_gql_error)?;

        // Clear metadata_language on all titles to mark them stale
        let cleared = app
            .services
            .titles
            .clear_metadata_language_for_all()
            .await
            .map_err(to_gql_error)?;

        tracing::info!(language = %language, titles_cleared = cleared, "metadata rehydration queued");

        // Wake the metadata hydration loop
        app.services.hydration_wake.notify_one();

        Ok(true)
    }
}
