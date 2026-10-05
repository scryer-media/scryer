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
