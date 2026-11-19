use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};

#[derive(Default)]
pub(crate) struct ArchiveWorkspaceReference;

impl ArchiveWorkspaceReference {
    pub(crate) fn track(&mut self, _root: Option<&Path>) {}
    pub(crate) fn retain(&mut self) {}
    pub(crate) fn mark_video_imported(&mut self) {}
}

pub(crate) fn owned_archive_workspaces_for(
    _parent: &Path,
    _owner_id: &str,
) -> crate::import_workflow::OwnedWorkspaceLookup {
    crate::import_workflow::OwnedWorkspaceLookup {
        workspaces: Vec::new(),
        complete: true,
    }
}

pub(crate) async fn remove_released_held_workspace(
    _root: &Path,
    _imported_workspace_paths: &std::collections::HashSet<String>,
    _imports_of_unknown_origin: bool,
) -> Result<(), crate::import_workflow::HeldWorkspacePreserved> {
    Err(crate::import_workflow::HeldWorkspacePreserved::NotOwned)
}

pub(crate) fn owned_archive_workspace_relative_path(_source: &Path) -> Option<String> {
    None
}

#[derive(Debug, Clone)]
pub struct ArchiveExtractionDestination {
    _staging_parent: PathBuf,
    _import_id: String,
}

impl ArchiveExtractionDestination {
    pub fn new(staging_parent: impl Into<PathBuf>, import_id: impl Into<String>) -> Self {
        Self {
            _staging_parent: staging_parent.into(),
            _import_id: import_id.into(),
        }
    }

    pub fn with_stale_cleanup_parent(self, _parent: impl Into<PathBuf>) -> Self {
        self
    }

    pub fn staging_parent(&self) -> &Path {
        &self._staging_parent
    }

    pub(crate) fn for_download(
        library_root: impl Into<PathBuf>,
        workspace_name: impl Into<String>,
        owner_id: impl Into<String>,
    ) -> Self {
        let _ = workspace_name.into();
        Self {
            _staging_parent: library_root
                .into()
                .join(crate::import::archive_extraction::ARCHIVE_EXTRACTION_DIR_NAME),
            _import_id: owner_id.into(),
        }
    }

    pub(crate) fn workspace_root(&self) -> Option<PathBuf> {
        None
    }
}

/// Never constructed: without archive support no download has archives to
/// plan.
pub(crate) enum PlannedArchiveExtraction {}

impl PlannedArchiveExtraction {
    pub(crate) fn digest(&self) -> &str {
        match *self {}
    }
}

pub(crate) struct DownloadArchiveOutput {
    pub(crate) files: u64,
    pub(crate) bytes: u64,
}

pub(crate) async fn plan_download_extraction(
    _dir: &Path,
    _is_sample: fn(&Path) -> bool,
    _archive_provider: Option<&Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<Option<PlannedArchiveExtraction>> {
    Ok(None)
}

pub(crate) async fn download_archive_digest(_dir: &Path) -> AppResult<Option<String>> {
    Ok(None)
}

pub(crate) async fn extract_planned_download(
    plan: PlannedArchiveExtraction,
    _destination: &ArchiveExtractionDestination,
    _passwords: &crate::import::archive_passwords::ArchivePasswordCandidates,
    _archive_provider: &Arc<dyn ArchiveExtractorPluginProvider>,
) -> AppResult<DownloadArchiveOutput> {
    match plan {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DownloadWorkspacePresence {
    // Shared callers match on it; this build never reports it.
    #[allow(dead_code)]
    Missing,
    Owned,
    Foreign,
}

/// Without archive support nothing proves ownership, so nothing is removed.
pub(crate) async fn download_workspace_presence(
    _root: &Path,
    _owner_id: &str,
) -> DownloadWorkspacePresence {
    DownloadWorkspacePresence::Foreign
}

/// Without archive support ownership is never proven, so a workspace is never
/// removed and its record is kept.
pub(crate) const PROVES_WORKSPACE_OWNERSHIP: bool = false;

pub(crate) async fn download_workspace_marked_at(
    _root: &Path,
    _owner_id: &str,
) -> Option<std::time::SystemTime> {
    None
}

pub(crate) async fn sweep_stale_archive_workspaces_in(_parent: &Path) -> usize {
    0
}

pub(crate) fn is_download_workspace_name(name: &str) -> bool {
    name.len() == 16
        && name
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) async fn remove_download_workspace(_root: &Path, _owner_id: &str) -> bool {
    false
}

pub(crate) fn download_workspace_name(hex: &str) -> String {
    hex.to_string()
}

pub async fn extract_archives_if_needed(
    _dir: &Path,
    _is_sample: fn(&Path) -> bool,
    _destination: Option<ArchiveExtractionDestination>,
    _passwords: &crate::import::archive_passwords::ArchivePasswordCandidates,
    _archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<Option<PathBuf>> {
    Ok(None)
}

pub fn archive_extraction_would_be_needed(
    _dir: &Path,
    _is_sample: fn(&Path) -> bool,
) -> AppResult<bool> {
    Ok(false)
}

pub fn is_password_required_error(_error: &AppError) -> bool {
    false
}

pub fn is_timeout_error(error: &AppError) -> bool {
    matches!(error, AppError::ArchiveExtractionTimedOut { .. })
}

pub async fn cleanup_extracted_dir(_dir: &Path) {}
pub(crate) async fn abandon_extracted_dir(_dir: &Path) {}

pub fn replaced_archive_sources(
    _workspace: &Path,
) -> AppResult<std::collections::HashSet<PathBuf>> {
    Ok(Default::default())
}

pub fn is_archive_workspace_output(_source: &Path, _dest: &Path) -> bool {
    false
}

pub fn initialize_archive_workspace_ownership(_state_dir: &Path) -> AppResult<()> {
    Ok(())
}

pub fn is_owned_archive_workspace(_root: &Path) -> bool {
    false
}

pub(crate) fn is_owned_archive_workspace_handle(_root: &Path, _directory: &std::fs::File) -> bool {
    false
}
