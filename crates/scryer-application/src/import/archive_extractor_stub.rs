use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};

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
    _destination: Option<ArchiveExtractionDestination>,
    _password: Option<&str>,
    _archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<Option<PathBuf>> {
    Ok(None)
}

pub fn archive_extraction_would_be_needed(_dir: &Path) -> AppResult<bool> {
    Ok(false)
}

pub fn is_password_required_error(_error: &AppError) -> bool {
    false
}

pub fn is_timeout_error(error: &AppError) -> bool {
    matches!(error, AppError::ArchiveExtractionTimedOut { .. })
}

pub async fn cleanup_extracted_dir(_dir: &Path) {}
