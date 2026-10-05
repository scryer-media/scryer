/// Why a completed import kept its download's sources. Ordered from the
/// mildest consequence of releasing them to the most severe, so the reasons
/// of several imports combine by taking the greatest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HeldSourcesReason {
    /// Subtitle discovery or delivery is still pending.
    SubtitlesPending,
    /// The import finished, but removing its replaced sources did not.
    SourceCleanupIncomplete,
    /// The hold predates recorded reasons, or its reason is unreadable.
    Unknown,
    /// Archive extraction failed while a loose video was imported. The
    /// unextracted archives were never imported and go with the download.
    ArchiveExtractionFailed,
}

impl HeldSourcesReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SubtitlesPending => "subtitles_pending",
            Self::SourceCleanupIncomplete => "source_cleanup_incomplete",
            Self::Unknown => "unknown",
            Self::ArchiveExtractionFailed => "archive_extraction_failed",
        }
    }

    /// Reads a stored reason; a missing or unrecognised one is `Unknown`.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("subtitles_pending") => Self::SubtitlesPending,
            Some("source_cleanup_incomplete") => Self::SourceCleanupIncomplete,
            Some("archive_extraction_failed") => Self::ArchiveExtractionFailed,
            _ => Self::Unknown,
        }
    }
}

/// What the download client's removal policy does with the download once the
/// existing completed-download cleanup runs, computed from the same title and
/// routing that cleanup uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeldDownloadClientPolicy {
    /// The client removes the completed download.
    Removes,
    /// The client removes the completed download once its seeding
    /// requirements are met.
    RemovesAfterSeeding,
    /// The policy keeps the download, or no configured client can act on it.
    Keeps,
    /// The download's title could not be resolved, so the policy is unknown.
    Unknown,
}

impl HeldDownloadClientPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Removes => "removes",
            Self::RemovesAfterSeeding => "removes_after_seeding",
            Self::Keeps => "keeps",
            Self::Unknown => "unknown",
        }
    }
}

/// What releasing the holds did to the tracked download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldSourcesSettlement {
    /// Import verification proved the download complete; it is imported and
    /// the existing cleanup runs under the client's policy.
    Imported,
    /// Verification did not prove the download complete; it went back to the
    /// ordinary import retry and nothing was cleaned up.
    AwaitingImport,
    /// The download was not blocked on the hold, so its own workflow decides.
    Unchanged,
    /// The download is no longer tracked; it is settled when next observed.
    Untracked,
    /// The holds were released but the tracked download could not be reached.
    /// Releasing again resumes from here.
    NotSettled,
    /// Verification accepted the download only on its fallback while a held
    /// import's reason means it may hold content that was never extracted or
    /// imported. It was not marked imported and nothing was cleaned up.
    Unproven,
}

impl HeldSourcesSettlement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::AwaitingImport => "awaiting_import",
            Self::Unchanged => "unchanged",
            Self::Untracked => "untracked",
            Self::NotSettled => "not_settled",
            Self::Unproven => "unproven",
        }
    }
}

/// Why a released import's extraction workspace was kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HeldWorkspacePreserved {
    /// The workspace's ownership marker could not be authenticated.
    NotOwned,
    /// A symlink, special file, unreadable entry or an entry beyond the bound.
    Unsafe,
    /// A video in it is not recorded, by its path, as imported by the import
    /// that owns the workspace.
    HoldsUnimportedVideo,
    /// An unfinished import or an open manual-import selection may still need it.
    InUse,
    /// What was imported from it could not be established.
    Unverified,
    /// Removal was attempted and the workspace is still there.
    RemovalFailed,
    /// The download was not proven imported, so nothing of it is removed.
    DownloadNotImported,
}

impl HeldWorkspacePreserved {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotOwned => "not_owned",
            Self::Unsafe => "unsafe",
            Self::HoldsUnimportedVideo => "holds_unimported_video",
            Self::InUse => "in_use",
            Self::Unverified => "unverified",
            Self::RemovalFailed => "removal_failed",
            Self::DownloadNotImported => "download_not_imported",
        }
    }
}

/// The owned workspaces found for one import. `complete` is false when the
/// search could not cover every place such a workspace may be.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnedWorkspaceLookup {
    pub workspaces: Vec<PathBuf>,
    pub complete: bool,
}

/// What releasing a download's held sources did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldSourcesRelease {
    pub import_id: String,
    /// Every import of the download whose hold was released.
    pub released_import_ids: Vec<String>,
    pub settlement: HeldSourcesSettlement,
    pub client_policy: HeldDownloadClientPolicy,
    /// Released imports' workspaces that were identified and removed.
    pub workspaces_removed: usize,
    /// Distinct reasons workspaces were kept.
    pub preserved_workspaces: Vec<HeldWorkspacePreserved>,
    /// A released import's workspace could not be searched for in full. Any
    /// workspace not found is preserved, never removed later by inference.
    pub workspace_lookup_incomplete: bool,
}

impl HeldSourcesRelease {
    /// Every identified workspace was removed and none could have been missed.
    pub fn workspace_removed(&self) -> bool {
        self.workspaces_removed > 0
            && self.preserved_workspaces.is_empty()
            && !self.workspace_lookup_incomplete
    }
}

/// The held sources of a download that an operator may release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldSourcesReleaseOffer {
    /// The import the release is requested through.
    pub import_id: String,
    /// Every import of the download the release covers.
    pub import_ids: Vec<String>,
    /// The distinct titles those imports belong to.
    pub title_names: Vec<String>,
    pub reason: HeldSourcesReason,
    pub client_policy: HeldDownloadClientPolicy,
}

fn import_holds_sources(record: &ImportRecord) -> bool {
    record.status == ImportStatus::Completed
        && serde_json::from_str::<serde_json::Value>(&record.payload_json)
            .is_ok_and(|payload| payload["archive_processing_pending"] == true)
}

fn import_hold_reason(record: &ImportRecord) -> HeldSourcesReason {
    let payload = serde_json::from_str::<serde_json::Value>(&record.payload_json).ok();
    HeldSourcesReason::parse(
        payload
            .as_ref()
            .and_then(|payload| payload[crate::ARCHIVE_HOLD_REASON_PAYLOAD_KEY].as_str()),
    )
}

fn result_carries_pending_warning(record: &ImportRecord) -> bool {
    let Some(result) = record
        .result_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
    else {
        return false;
    };
    let carries =
        |object: &serde_json::Value| object["error_message"] == SCENE_SUBTITLE_PENDING_WARNING;
    carries(&result)
        || result["file_results"]
            .as_array()
            .is_some_and(|files| files.iter().any(carries))
}

fn import_hold_was_released(record: &ImportRecord) -> bool {
    serde_json::from_str::<serde_json::Value>(&record.payload_json)
        .is_ok_and(|payload| payload[crate::ARCHIVE_HOLD_RELEASED_PAYLOAD_KEY] == true)
}

/// A completed import whose release is not finished: it still holds its
/// sources, or an operator's release cleared its hold and the pending
/// warning, which is cleared last, still marks that release as unfinished.
fn import_release_pending(record: &ImportRecord) -> bool {
    record.status == ImportStatus::Completed
        && (import_holds_sources(record)
            || (import_hold_was_released(record) && result_carries_pending_warning(record)))
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

/// The comparison the import store applies to source identities.
fn source_match_key(source: &ClientJobLocator) -> (String, String, String) {
    (
        source
            .client_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase(),
        source.client_type.trim().to_ascii_lowercase(),
        source.item_id.trim().to_string(),
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

/// The extraction workspaces that belong to a released import, identified
/// only by their authenticated ownership marker. Nothing is ever inferred
/// from a name, an age, or a path pattern.
fn held_import_workspaces(record: &ImportRecord, title: &Title) -> OwnedWorkspaceLookup {
    if record.import_type == ImportType::ManualImport {
        // A manual import records the workspace it was queued with; the
        // existing completion cleanup removes exactly that root.
        let Ok(payload) = serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
        else {
            return OwnedWorkspaceLookup::default();
        };
        return OwnedWorkspaceLookup {
            workspaces: payload
                .archive_workspace_root
                .map(|root| stored_path_to_path_buf(&root))
                .filter(|root| crate::archive_extractor::is_owned_archive_workspace(root))
                .into_iter()
                .collect(),
            complete: true,
        };
    }
    // An automatic import stages its workspace in the title folder under its
    // own import id. Only the persisted folder is searched: a folder that was
    // never recorded cannot be searched with certainty.
    let Some(folder) = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
    else {
        return OwnedWorkspaceLookup::default();
    };
    crate::archive_extractor::owned_archive_workspaces_for(
        &stored_path_to_path_buf(folder),
        &record.id,
    )
}

fn import_references_workspace(record: &ImportRecord, workspace: &Path) -> bool {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&record.payload_json) else {
        // An unreadable payload might reference anything.
        return true;
    };
    [
        "archive_workspace_root",
        "trusted_source_root",
        "source_path",
    ]
    .iter()
    .filter_map(|key| payload[*key].as_str())
    .map(stored_path_to_path_buf)
    .any(|root| paths_overlap(&root, workspace))
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

/// Every pending import's title, authorized for import resolution. Refuses
/// unless the actor may resolve imports for every one of them.
async fn authorized_release_titles(
    app: &AppUseCase,
    actor: &User,
    pending: &[&ImportRecord],
) -> AppResult<HashMap<String, Title>> {
    let mut titles: HashMap<String, Title> = HashMap::new();
    for record in pending {
        let title_id = held_import_title_id(record).ok_or_else(|| {
            AppError::Validation(
                "an import of this download has no title to authorize its release against".into(),
            )
        })?;
        if titles.contains_key(&title_id) {
            continue;
        }
        let title = app
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        match app
            .require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await
        {
            Ok(()) => {}
            Err(AppError::Unauthorized(_)) => {
                return Err(AppError::Unauthorized(
                    "releasing these sources also releases an import for a title you may not \
                     resolve imports for"
                        .into(),
                ));
            }
            Err(error) => return Err(error),
        }
        titles.insert(title_id, title);
    }
    Ok(titles)
}

/// The client's removal policy for this download, from the same title and
/// routing the completed-download cleanup reads: the download submission's
/// title and the source's client.
async fn held_download_client_policy(
    app: &AppUseCase,
    source: &ClientJobLocator,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
) -> AppResult<HeldDownloadClientPolicy> {
    let client_id = source.client_id.as_deref().unwrap_or("").trim();
    if client_id.is_empty()
        || app
            .services
            .integrations
            .download_client_configs
            .get_by_id(client_id)
            .await?
            .is_none()
    {
        // Cleanup settles without a client and leaves the download as-is.
        return Ok(HeldDownloadClientPolicy::Keeps);
    }
    let Some(submission) = app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id_for_download(canonical_download_id, source)
        .await?
    else {
        return Ok(HeldDownloadClientPolicy::Unknown);
    };
    let Some(title) = app
        .services
        .catalog
        .titles
        .get_by_id(&submission.title_id)
        .await?
    else {
        return Ok(HeldDownloadClientPolicy::Unknown);
    };
    let removes = app
        .read_download_client_routing_entry(Some(&title.library_id), &title.facet, client_id)
        .await?
        .unwrap_or_else(crate::catalog_helpers::default_download_client_routing_entry)
        .remove_completed;
    Ok(if !removes {
        HeldDownloadClientPolicy::Keeps
    } else if crate::seeding_gate::client_type_is_torrent(app, &source.client_type) {
        HeldDownloadClientPolicy::RemovesAfterSeeding
    } else {
        HeldDownloadClientPolicy::Removes
    })
}

/// Verify the released download the way its held imports were verified.
fn held_release_verification(
    pending: &[&ImportRecord],
) -> crate::tracked_downloads::HeldImportVerification {
    let mut automatic = false;
    let mut manual: Option<usize> = None;
    let mut require_positive_proof = false;
    for record in pending {
        // A hold released by an interrupted release no longer carries its
        // reason, which reads as unknown and so also requires proof.
        require_positive_proof |= matches!(
            import_hold_reason(record),
            HeldSourcesReason::ArchiveExtractionFailed | HeldSourcesReason::Unknown
        );
        if record.import_type == ImportType::ManualImport {
            // A mapping that cannot be read cannot be verified; no import
            // covers `usize::MAX` files, so the download never settles on it.
            let mapped = serde_json::from_str::<ManualImportRequestPayload>(&record.payload_json)
                .map_or(usize::MAX, |payload| payload.files.len());
            manual = Some(manual.unwrap_or(0).saturating_add(mapped));
        } else {
            automatic = true;
        }
    }
    crate::tracked_downloads::HeldImportVerification {
        automatic,
        manual_expected_mapping_count: manual,
        require_positive_proof,
    }
}

/// Settle the tracked download.
async fn settle_tracked_download(
    app: &AppUseCase,
    record: &ImportRecord,
    canonical_download_id: Option<scryer_domain::download_identity::DownloadId>,
    verification: crate::tracked_downloads::HeldImportVerification,
) -> HeldSourcesSettlement {
    use crate::tracked_downloads::HeldImportReleaseSettlement;
    let Some(handle) = app.runtime.acquisition.tracked_download_handle.as_ref() else {
        return HeldSourcesSettlement::Untracked;
    };
    let tracked_id = crate::tracked_downloads::tracked_download_id(
        record.source_client_id.as_deref(),
        &record.source_system,
        &record.source_ref,
    );
    match handle
        .release_held_import_for_download(tracked_id, canonical_download_id, verification)
        .await
    {
        Ok(HeldImportReleaseSettlement::Imported) => HeldSourcesSettlement::Imported,
        Ok(HeldImportReleaseSettlement::AwaitingImport) => HeldSourcesSettlement::AwaitingImport,
        Ok(HeldImportReleaseSettlement::Unchanged) => HeldSourcesSettlement::Unchanged,
        Ok(HeldImportReleaseSettlement::Unproven) => HeldSourcesSettlement::Unproven,
        Err(AppError::NotFound(_)) => HeldSourcesSettlement::Untracked,
        Err(error) => {
            tracing::warn!(import_id = %record.id, error = %error,
                "held sources were released but the tracked download could not be settled");
            HeldSourcesSettlement::NotSettled
        }
    }
}

/// Remove the released imports' own workspaces, keeping any that are not
/// proven safe to remove. Nothing is removed unless the download settled as
/// imported. A workspace is checked only against the paths, relative to the
/// workspace, that the import owning it recorded as imported from it, never
/// against another import's files.
async fn remove_released_workspaces(
    app: &AppUseCase,
    source: &ClientJobLocator,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    released: &[&ImportRecord],
    titles: &HashMap<String, Title>,
    settlement: HeldSourcesSettlement,
) -> (usize, Vec<HeldWorkspacePreserved>, bool) {
    let mut lookup_incomplete = false;
    let mut workspaces: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for record in released {
        let Some(title) = held_import_title_id(record).and_then(|id| titles.get(&id)) else {
            lookup_incomplete = true;
            continue;
        };
        let lookup = held_import_workspaces(record, title);
        lookup_incomplete |= !lookup.complete;
        for workspace in lookup.workspaces {
            match workspaces.iter_mut().find(|(path, _)| *path == workspace) {
                Some((_, owners)) => owners.push(record.id.clone()),
                None => workspaces.push((workspace, vec![record.id.clone()])),
            }
        }
    }
    if workspaces.is_empty() {
        return (0, Vec::new(), lookup_incomplete);
    }

    let mut preserved = std::collections::BTreeSet::<HeldWorkspacePreserved>::new();
    if settlement != HeldSourcesSettlement::Imported {
        preserved.insert(HeldWorkspacePreserved::DownloadNotImported);
        return (0, preserved.into_iter().collect(), lookup_incomplete);
    }
    let artifacts = app
        .services
        .workflow
        .import_artifacts
        .list_by_source_identity_for_download(canonical_download_id, source)
        .await
        .ok();
    let selection_roots = app
        .services
        .workflow
        .imports
        .open_manual_selection_roots()
        .await
        .ok();
    let unfinished = app
        .services
        .workflow
        .imports
        .list_pending_imports()
        .await
        .ok();

    let mut removed = 0usize;
    for (workspace, owners) in workspaces {
        let in_use = match (&selection_roots, &unfinished) {
            (Some(roots), Some(unfinished)) => {
                roots
                    .iter()
                    .map(|root| stored_path_to_path_buf(root))
                    .any(|root| paths_overlap(&root, &workspace))
                    || unfinished
                        .iter()
                        .any(|other| import_references_workspace(other, &workspace))
            }
            _ => true,
        };
        if in_use {
            preserved.insert(HeldWorkspacePreserved::InUse);
            continue;
        }
        // One owner, and its artifacts readable, or nothing is proven.
        let (Some(artifacts), [owner]) = (artifacts.as_ref(), owners.as_slice()) else {
            preserved.insert(HeldWorkspacePreserved::Unverified);
            continue;
        };
        let owner_imports: Vec<&crate::ImportArtifact> = artifacts
            .iter()
            .filter(|artifact| {
                artifact.import_id.as_deref() == Some(owner.as_str())
                    && matches!(artifact.result.as_str(), "imported" | "already_present")
            })
            .collect();
        let imported_workspace_paths: HashSet<String> = owner_imports
            .iter()
            .filter_map(|artifact| artifact.workspace_relative_path.clone())
            .filter(|path| !path.is_empty())
            .collect();
        // Recorded with neither path, as every import before workspace paths
        // were kept was: such an import may account for a video here.
        let imports_of_unknown_origin = owner_imports.iter().any(|artifact| {
            artifact.workspace_relative_path.is_none() && artifact.relative_path.is_none()
        });
        match crate::archive_extractor::remove_released_held_workspace(
            &workspace,
            &imported_workspace_paths,
            imports_of_unknown_origin,
        )
        .await
        {
            Ok(()) => removed += 1,
            Err(reason) => {
                preserved.insert(reason);
            }
        }
    }
    (removed, preserved.into_iter().collect(), lookup_incomplete)
}

/// Release the sources a download's completed imports are holding.
///
/// Every condition is checked before anything changes, and every hold of the
/// download is released together in one transaction under the same lock the
/// cleanup claim takes, so the cleanup never sees a partial release. The
/// tracked download is then settled through import verification: it becomes
/// imported, and the existing cleanup runs under the client's policy, only
/// when the download is proven complete. Otherwise it returns to the
/// ordinary import retry and nothing is removed. Finally the released
/// imports' own workspaces are removed when each is proven safe to remove.
///
/// A release that stopped after the holds were released is resumed by
/// releasing again: the pending warning, cleared last, marks it unfinished.
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
    let source = held_import_source(&record);
    let source_imports = imports
        .list_imports_for_identities(std::slice::from_ref(&source))
        .await?;
    if source_imports.iter().any(|other| other.status.is_active()) {
        return Err(AppError::Validation(
            "this download is still being imported; release its sources once that finishes".into(),
        ));
    }
    let pending: Vec<&ImportRecord> = source_imports
        .iter()
        .filter(|other| import_release_pending(other))
        .collect();
    if !pending.iter().any(|other| other.id == record.id) {
        return Err(AppError::Validation(format!(
            "import {import_id} is not holding its sources"
        )));
    }
    let titles = authorized_release_titles(app, actor, &pending).await?;
    // The cleanup claim and the release serialize on the canonical download.
    // Without one there is nothing to serialize on, so the sources stay held.
    let Some(canonical_download_id) = imports.canonical_download_id_for_import(&record.id).await?
    else {
        return Err(AppError::Validation(
            "this download has no stable identity yet; its sources stay held".into(),
        ));
    };
    let canonical_download_id = Some(canonical_download_id);
    let client_policy =
        held_download_client_policy(app, &source, canonical_download_id.as_ref()).await?;

    let released_import_ids: Vec<String> = pending.iter().map(|other| other.id.clone()).collect();
    imports
        .release_archive_holds(
            &source,
            canonical_download_id.as_ref(),
            &released_import_ids,
        )
        .await?;

    // From here the holds are released; every outcome below is reported as
    // what it is rather than as a failure of the release.
    let settlement = settle_tracked_download(
        app,
        &record,
        canonical_download_id.clone(),
        held_release_verification(&pending),
    )
    .await;
    if settlement != HeldSourcesSettlement::NotSettled {
        for other in &pending {
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
    }

    let (workspaces_removed, preserved_workspaces, workspace_lookup_incomplete) =
        remove_released_workspaces(
            app,
            &source,
            canonical_download_id.as_ref(),
            &pending,
            &titles,
            settlement,
        )
        .await;

    let release = HeldSourcesRelease {
        import_id: record.id.clone(),
        released_import_ids,
        settlement,
        client_policy,
        workspaces_removed,
        preserved_workspaces,
        workspace_lookup_incomplete,
    };
    let preserved = release
        .preserved_workspaces
        .iter()
        .map(|reason| reason.as_str())
        .collect::<Vec<_>>()
        .join(",");
    app.emit_download_queue_item_command_issued_event_with_detail(
        actor,
        record.source_ref.clone(),
        scryer_domain::DownloadQueueCommandAction::ReleaseHeldSources,
        Some(format!(
            "imports={}; settlement={}; policy={}; workspaces_removed={}; workspaces_preserved={}; workspace_lookup_incomplete={}",
            release.released_import_ids.join(","),
            release.settlement.as_str(),
            release.client_policy.as_str(),
            release.workspaces_removed,
            if preserved.is_empty() { "none" } else { preserved.as_str() },
            release.workspace_lookup_incomplete,
        )),
    )
    .await;
    Ok(release)
}

impl AppUseCase {
    /// The held sources the actor may release for each queue item, keyed by
    /// source. One import listing covers every source; titles and policy are
    /// read only for sources that hold something. A source is absent when
    /// nothing is held, an import is still running, or the actor may not
    /// resolve imports for every title the release covers. The release
    /// mutation rechecks every condition.
    pub async fn held_sources_release_offers(
        &self,
        actor: &User,
        sources: &[ClientJobLocator],
    ) -> AppResult<HashMap<ClientJobLocator, HeldSourcesReleaseOffer>> {
        let mut offers = HashMap::new();
        if sources.is_empty() {
            return Ok(offers);
        }
        let records = self
            .services
            .workflow
            .imports
            .list_imports_for_identities(sources)
            .await?;
        let mut by_source: HashMap<(String, String, String), Vec<ImportRecord>> = HashMap::new();
        for record in records {
            by_source
                .entry(source_match_key(&held_import_source(&record)))
                .or_default()
                .push(record);
        }
        for source in sources {
            let Some(records) = by_source.get(&source_match_key(source)) else {
                continue;
            };
            if records.iter().any(|record| record.status.is_active()) {
                continue;
            }
            let pending: Vec<&ImportRecord> = records
                .iter()
                .filter(|record| import_release_pending(record))
                .collect();
            let Some(latest) = pending
                .iter()
                .max_by(|left, right| left.updated_at.cmp(&right.updated_at))
            else {
                continue;
            };
            // A failure for one queue item drops only that item's offer.
            let titles = match authorized_release_titles(self, actor, &pending).await {
                Ok(titles) => titles,
                Err(AppError::Unauthorized(_)) => continue,
                Err(error) => {
                    tracing::warn!(import_id = %latest.id, error = %error,
                        "held sources offer skipped: its titles could not be authorized");
                    continue;
                }
            };
            let canonical_download_id = match self
                .services
                .workflow
                .imports
                .canonical_download_id_for_import(&latest.id)
                .await
            {
                // Without a canonical download the release refuses, so
                // nothing is offered.
                Ok(Some(canonical_download_id)) => canonical_download_id,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(import_id = %latest.id, error = %error,
                        "held sources offer skipped: its download identity is unavailable");
                    continue;
                }
            };
            let client_policy =
                match held_download_client_policy(self, source, Some(&canonical_download_id)).await
                {
                    Ok(policy) => policy,
                    Err(error) => {
                        tracing::warn!(import_id = %latest.id, error = %error,
                            "held sources offer skipped: the client policy is unavailable");
                        continue;
                    }
                };
            let reason = pending
                .iter()
                .filter(|record| import_holds_sources(record))
                .map(|record| import_hold_reason(record))
                .max()
                .unwrap_or(HeldSourcesReason::Unknown);
            let mut title_names: Vec<String> =
                titles.values().map(|title| title.name.clone()).collect();
            title_names.sort();
            offers.insert(
                source.clone(),
                HeldSourcesReleaseOffer {
                    import_id: latest.id.clone(),
                    import_ids: pending.iter().map(|record| record.id.clone()).collect(),
                    title_names,
                    reason,
                    client_policy,
                },
            );
        }
        Ok(offers)
    }

    /// The held sources the actor may release for one queue item.
    pub async fn held_sources_release_for_download(
        &self,
        actor: &User,
        source: &ClientJobLocator,
    ) -> AppResult<Option<HeldSourcesReleaseOffer>> {
        Ok(self
            .held_sources_release_offers(actor, std::slice::from_ref(source))
            .await?
            .remove(source))
    }
}
