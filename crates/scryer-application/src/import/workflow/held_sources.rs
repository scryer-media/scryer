/// What releasing a completed import's held sources did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldSourcesRelease {
    pub import_id: String,
    /// The import's extraction workspace was identified with certainty and
    /// removed. Otherwise it is left for the stale-workspace sweeper.
    pub workspace_removed: bool,
}

/// A completed import whose sources an operator may release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldSourcesReleaseOffer {
    pub import_id: String,
    /// The download client's removal policy removes the completed download
    /// once its sources are released.
    pub client_removes_download: bool,
}

fn import_holds_sources(record: &ImportRecord) -> bool {
    record.status == ImportStatus::Completed
        && serde_json::from_str::<serde_json::Value>(&record.payload_json)
            .is_ok_and(|payload| payload["archive_processing_pending"] == true)
}

fn held_import_title_id(record: &ImportRecord) -> Option<String> {
    let title_id = |json: &str| {
        let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
        ["title_id", "target_title_id", "manual_title_id"]
            .iter()
            .find_map(|key| {
                value[*key]
                    .as_str()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
            })
    };
    record
        .result_json
        .as_deref()
        .and_then(title_id)
        .or_else(|| title_id(&record.payload_json))
}

fn held_import_source(record: &ImportRecord) -> ClientJobLocator {
    ClientJobLocator::new(
        record.source_client_id.as_deref(),
        &record.source_system,
        &record.source_ref,
    )
}

/// Drop the pending-subtitle warning from a stored import result, keeping
/// every other message. Returns `None` when nothing changed.
fn result_json_without_pending_warning(result_json: &str) -> Option<String> {
    let mut result = serde_json::from_str::<serde_json::Value>(result_json).ok()?;
    let mut changed = false;
    let mut clear = |object: &mut serde_json::Value| {
        if object["error_message"].as_str() == Some(SCENE_SUBTITLE_PENDING_WARNING) {
            object["error_message"] = serde_json::Value::Null;
            changed = true;
        }
    };
    clear(&mut result);
    if let Some(files) = result["file_results"].as_array_mut() {
        files.iter_mut().for_each(&mut clear);
    }
    changed.then(|| result.to_string())
}

/// The extraction workspaces that belong to a held import, identified only
/// by their authenticated ownership marker. Nothing is ever inferred from a
/// name, an age, or a path pattern.
fn held_import_workspaces(record: &ImportRecord, title: &Title) -> Vec<PathBuf> {
    if record.import_type == ImportType::ManualImport {
        // A manual import records the workspace it was queued with; the
        // existing completion cleanup removes exactly that root.
        return serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
            .ok()
            .and_then(|payload| payload.archive_workspace_root)
            .map(|root| stored_path_to_path_buf(&root))
            .filter(|root| crate::archive_extractor::is_owned_archive_workspace(root))
            .into_iter()
            .collect();
    }
    // An automatic import stages its workspace in the title folder under its
    // own import id. Only the persisted folder is searched: a folder that was
    // never recorded cannot be identified with certainty.
    let Some(folder) = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
    else {
        return Vec::new();
    };
    crate::archive_extractor::owned_archive_workspaces_for(
        &stored_path_to_path_buf(folder),
        &record.id,
    )
}

fn import_references_workspace(record: &ImportRecord, workspace: &Path) -> bool {
    serde_json::from_str::<serde_json::Value>(&record.payload_json)
        .ok()
        .and_then(|payload| {
            payload["archive_workspace_root"]
                .as_str()
                .map(stored_path_to_path_buf)
        })
        .is_some_and(|root| root == workspace)
}

async fn held_import_title(app: &AppUseCase, record: &ImportRecord) -> AppResult<Title> {
    let title_id = held_import_title_id(record).ok_or_else(|| {
        AppError::Validation("the import has no title to authorize its release against".into())
    })?;
    app.services
        .catalog
        .titles
        .get_by_id(&title_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))
}

/// Release the sources a completed import is holding for pending subtitles.
///
/// The hold and the pending warning are cleared, the tracked download is
/// settled as imported so the ordinary completed-download cleanup runs under
/// the client's removal policy, and the import's own extraction workspace is
/// removed through the ordinary workspace cleanup when it can be identified
/// with certainty. A workspace that cannot be identified, holds a symlink or
/// another unsafe entry, or is referenced by an unfinished import is left in
/// place.
pub async fn release_held_import_sources(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
) -> AppResult<HeldSourcesRelease> {
    let imports = &app.services.workflow.imports;
    let record = imports
        .get_import_by_id(import_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("import {import_id}")))?;
    let title = held_import_title(app, &record).await?;
    app.require_library_permission(
        actor,
        &title.library_id,
        scryer_domain::LibraryPermission::ResolveImports,
    )
    .await?;
    if record.status != ImportStatus::Completed {
        return Err(AppError::Validation(format!(
            "import {import_id} has status '{}'; only completed imports hold sources",
            record.status.as_str()
        )));
    }
    if !import_holds_sources(&record) {
        return Err(AppError::Validation(format!(
            "import {import_id} is not holding its sources"
        )));
    }

    let source = held_import_source(&record);
    let source_imports = imports
        .list_imports_for_identities(std::slice::from_ref(&source))
        .await?;
    if source_imports.iter().any(|other| other.status.is_active()) {
        return Err(AppError::Validation(
            "this download is still being imported; release its sources once that finishes".into(),
        ));
    }

    // Every completed import of this download that holds sources keeps the
    // download's cleanup deferred, so all of them are released together.
    let held: Vec<&ImportRecord> = source_imports
        .iter()
        .filter(|other| import_holds_sources(other))
        .collect();
    if !held.iter().any(|other| other.id == record.id) {
        return Err(AppError::Validation(
            "the import changed while its release was requested; refresh the queue".into(),
        ));
    }
    let mut released: Vec<String> = Vec::new();
    for other in &held {
        if let Err(error) = imports
            .set_archive_processing_pending(&other.id, false)
            .await
        {
            for id in &released {
                let _ = imports.set_archive_processing_pending(id, true).await;
            }
            return Err(error);
        }
        released.push(other.id.clone());
    }

    // Settling the tracked download runs the existing cleanup now that the
    // hold is gone. A download no longer tracked is cleaned up when it is
    // next observed, so that is not a failure.
    if let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() {
        let canonical_download_id = imports.canonical_download_id_for_import(&record.id).await?;
        let tracked_id = crate::tracked_downloads::tracked_download_id(
            record.source_client_id.as_deref(),
            &record.source_system,
            &record.source_ref,
        );
        match handle
            .mark_imported_for_download(tracked_id, canonical_download_id)
            .await
        {
            Ok(()) | Err(AppError::NotFound(_)) => {}
            Err(error) => {
                // Put the hold back so nothing is cleaned up behind a refusal.
                for id in &released {
                    let _ = imports.set_archive_processing_pending(id, true).await;
                }
                return Err(error);
            }
        }
    }

    for other in &held {
        if let Some(result_json) = other
            .result_json
            .as_deref()
            .and_then(result_json_without_pending_warning)
            && let Err(error) = app
                .update_import_status_and_notify(
                    &other.id,
                    ImportStatus::Completed,
                    Some(result_json),
                )
                .await
        {
            tracing::warn!(import_id = %other.id, error = %error,
                "held sources were released but the pending warning could not be cleared");
        }
    }

    let unfinished = imports.list_pending_imports().await?;
    let mut workspace_removed = false;
    for workspace in held_import_workspaces(&record, &title) {
        if unfinished
            .iter()
            .any(|other| import_references_workspace(other, &workspace))
        {
            continue;
        }
        workspace_removed |=
            crate::archive_extractor::remove_released_held_workspace(&workspace).await;
    }

    app.emit_download_queue_item_command_issued_event(
        actor,
        record.source_ref.clone(),
        scryer_domain::DownloadQueueCommandAction::ReleaseHeldSources,
    )
    .await;

    Ok(HeldSourcesRelease {
        import_id: record.id,
        workspace_removed,
    })
}

impl AppUseCase {
    /// The completed import whose held sources the actor may release for this
    /// queue item, with what the client's removal policy will then do. The
    /// release mutation rechecks every condition.
    pub async fn held_sources_release_for_download(
        &self,
        actor: &User,
        source: &ClientJobLocator,
    ) -> AppResult<Option<HeldSourcesReleaseOffer>> {
        let records = self
            .services
            .workflow
            .imports
            .list_imports_for_identities(std::slice::from_ref(source))
            .await?;
        if records.iter().any(|record| record.status.is_active()) {
            return Ok(None);
        }
        let Some(record) = records
            .into_iter()
            .filter(import_holds_sources)
            .max_by(|left, right| left.updated_at.cmp(&right.updated_at))
        else {
            return Ok(None);
        };
        let title = match held_import_title(self, &record).await {
            Ok(title) => title,
            Err(AppError::NotFound(_) | AppError::Validation(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self
            .require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await
        {
            Ok(()) => {}
            Err(AppError::Unauthorized(_)) => return Ok(None),
            Err(error) => return Err(error),
        }
        let client_id = record.source_client_id.as_deref().unwrap_or("").trim();
        let client_configured = !client_id.is_empty()
            && self
                .services
                .integrations
                .download_client_configs
                .get_by_id(client_id)
                .await?
                .is_some();
        let client_removes_download = client_configured
            && self
                .read_download_client_routing_entry(
                    Some(&title.library_id),
                    &title.facet,
                    client_id,
                )
                .await?
                .unwrap_or_else(crate::catalog_helpers::default_download_client_routing_entry)
                .remove_completed;
        Ok(Some(HeldSourcesReleaseOffer {
            import_id: record.id,
            client_removes_download,
        }))
    }
}
