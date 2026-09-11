//! Durable existing-file decisions, separate from fresh-copy integrity checks.

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileResolution {
    pub operation_id: String,
    pub title_id: String,
    pub source_path: String,
    pub original_destination: String,
    pub destination_path: String,
    pub disposition: ResolutionDisposition,
    pub completed: bool,
    pub source_version: Option<String>,
    pub destination_version: Option<String>,
    pub identity_hash: Option<String>,
    pub warning: Option<String>,
    #[serde(default)]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionDisposition {
    Placed,
    Identical,
    Preserved,
    Hardlink,
    Symlink,
}

impl FileResolution {
    pub async fn proof_is_current(&self) -> bool {
        if !self.completed {
            return false;
        }
        let source = stored_path_to_path_buf(&self.source_path);
        let destination = stored_path_to_path_buf(&self.destination_path);
        let source_metadata = tokio::fs::symlink_metadata(&source).await;
        let destination_version = file_version(&destination).await.ok();
        let Some(expected) = self.destination_version.as_deref() else {
            return false;
        };
        if source_metadata
            .as_ref()
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            // Cleanup may have committed before its title checkpoint. Unlinking
            // a hardlink changes ctime, but not its identity, length or mtime.
            return destination_version.as_deref() == Some(expected)
                || (self.disposition == ResolutionDisposition::Hardlink
                    && destination_version.as_deref().is_some_and(|actual| {
                        actual.split(':').take(5).eq(expected.split(':').take(5))
                    }));
        }
        if self.disposition == ResolutionDisposition::Hardlink {
            return match (
                source_metadata,
                tokio::fs::symlink_metadata(&destination).await,
            ) {
                (Ok(source), Ok(destination)) => {
                    super::execution::same_regular_file(&source, &destination)
                }
                _ => false,
            };
        }
        destination_version.as_deref() == Some(expected)
            && file_version(&source).await.ok() == self.source_version
    }
}

use super::executor::{FileMoveRequest, TitleFileMover};
use super::model::{AppliedVerificationDepth, FileVerificationOutcome, VerificationDepth};
use super::verify::{CopyProgress, VerifiedFile, hash_existing_file_with_progress};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult, LocationOperationRepository};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// The first resolver to reach a destination publishes where the media file
/// landed; later companions of the same file wait on it.
type MediaResultSender = tokio::sync::watch::Sender<Option<Result<PathBuf, String>>>;

pub struct ConflictResolver {
    pub store: Arc<dyn LocationOperationRepository>,
    pub contexts: BTreeMap<String, (String, bool)>,
    claims: Mutex<BTreeMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    media_results: Mutex<BTreeMap<(String, String, PathBuf), MediaResultSender>>,
}

impl ConflictResolver {
    pub fn new(
        store: Arc<dyn LocationOperationRepository>,
        contexts: BTreeMap<String, (String, bool)>,
    ) -> Self {
        Self {
            store,
            contexts,
            claims: Mutex::new(BTreeMap::new()),
            media_results: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn resolve(
        &self,
        mover: &dyn TitleFileMover,
        request: FileMoveRequest<'_>,
    ) -> AppResult<VerifiedFile> {
        let result = self.resolve_file(mover, request).await;
        if request.file.media_file_id.is_some() {
            let destination = match &result {
                Ok(file) if file.permits_source_removal() => Ok(file.destination_path.clone()),
                _ => Err("The related media file could not be transferred. Its companion files were preserved.".into()),
            };
            self.media_result(request, &request.file.source_path)
                .send_replace(Some(destination));
        }
        result
    }

    fn media_result(
        &self,
        request: FileMoveRequest<'_>,
        source: &Path,
    ) -> tokio::sync::watch::Sender<Option<Result<PathBuf, String>>> {
        self.media_results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry((
                request.operation_id.into(),
                request.title.title_id.clone(),
                source.to_path_buf(),
            ))
            .or_insert_with(|| tokio::sync::watch::channel(None).0)
            .clone()
    }

    async fn companion_destination(
        &self,
        request: FileMoveRequest<'_>,
    ) -> AppResult<Option<PathBuf>> {
        if request.file.media_file_id.is_some() {
            return Ok(None);
        }
        if let Some((media, relative)) = request
            .title
            .files
            .iter()
            .filter(|file| file.media_file_id.is_some())
            .filter_map(|media| {
                companion_suffix(&request.file.destination_path, &media.destination_path)
                    .map(|suffix| (media, suffix))
            })
            // Prefer episode.part1 over episode for episode.part1.en.srt.
            .max_by_key(|(media, _)| media.destination_path.as_os_str().len())
        {
            let saved = self
                .store
                .file_resolutions_for_sources(
                    request.operation_id,
                    &request.title.title_id,
                    &[path_to_stored_string(&media.source_path)],
                )
                .await?;
            let resolved = if let Some(row) = saved.first()
                && row.proof_is_current().await
            {
                stored_path_to_path_buf(&row.destination_path)
            } else {
                let mut updates = self.media_result(request, &media.source_path).subscribe();
                loop {
                    if let Some(result) = updates.borrow_and_update().clone() {
                        break result.map_err(AppError::Validation)?;
                    }
                    updates.changed().await.map_err(|_| {
                        AppError::Validation(
                            "The related media transfer was interrupted. The source was preserved."
                                .into(),
                        )
                    })?;
                }
            };
            let stem = resolved.file_stem().unwrap_or_default().to_string_lossy();
            return Ok(Some(resolved.with_file_name(format!("{stem}{relative}"))));
        }
        Ok(None)
    }

    async fn resolve_file(
        &self,
        mover: &dyn TitleFileMover,
        request: FileMoveRequest<'_>,
    ) -> AppResult<VerifiedFile> {
        let source = &request.file.source_path;
        let original = &request.file.destination_path;
        if source == original {
            return Err(AppError::Validation(
                "The source and destination are the same path.".into(),
            ));
        }
        let saved = self
            .store
            .file_resolutions_for_sources(
                request.operation_id,
                &request.title.title_id,
                &[path_to_stored_string(source)],
            )
            .await?
            .into_iter()
            .find(|row| row.source_path == path_to_stored_string(source));
        let (label, recycle) = self
            .contexts
            .get(&request.title.title_id)
            .cloned()
            .unwrap_or(("source".into(), false));
        let mut destination = match saved.as_ref() {
            Some(row) => stored_path_to_path_buf(&row.destination_path),
            None => self
                .companion_destination(request)
                .await?
                .unwrap_or_else(|| original.clone()),
        };
        let base = super::collisions::collision_rename_base(
            &original.file_name().unwrap_or_default().to_string_lossy(),
            &super::collisions::sanitize_suffix_label(&label),
        );
        let mut number = 1u32;
        let mut compared_before = 0u64;
        loop {
            let claim = self
                .claims
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(destination.clone())
                .or_default()
                .clone();
            let _claim = claim.lock().await;
            let mut record = FileResolution {
                operation_id: request.operation_id.into(),
                title_id: request.title.title_id.clone(),
                source_path: path_to_stored_string(source),
                original_destination: path_to_stored_string(original),
                destination_path: path_to_stored_string(&destination),
                disposition: ResolutionDisposition::Placed,
                completed: false,
                source_version: Some(file_version(source).await?),
                destination_version: None,
                identity_hash: None,
                warning: None,
                reason_code: None,
            };
            let existing = match tokio::fs::symlink_metadata(&destination).await {
                Ok(_) => {
                    record.destination_version = Some(file_version(&destination).await?);
                    let read = Arc::new(std::sync::atomic::AtomicU64::new(0));
                    let count = read.clone();
                    let outer = request.progress.clone();
                    let comparison =
                        CopyProgress::none().with_comparison_sink(move |bytes, total| {
                            count.store(bytes, std::sync::atomic::Ordering::Relaxed);
                            outer.comparison(
                                compared_before.saturating_add(bytes),
                                compared_before.saturating_add(total),
                            );
                        });
                    let proof = compare_existing(source, &destination, &comparison).await?;
                    // Distinct occupied names are additional work, not retries.
                    compared_before = compared_before
                        .saturating_add(read.load(std::sync::atomic::Ordering::Relaxed));
                    Some(proof)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(file_error(&destination, error)),
            };
            let preserved = destination != *original;
            if let Some(proof) = existing.as_ref() {
                let identical = proof.permits_source_removal();
                let hardlink = identical && proof.hashes.is_none();
                if !identical || (!hardlink && !recycle && !preserved) {
                    let name = if number == 1 {
                        base.clone()
                    } else {
                        numbered_name(&base, number)
                    };
                    destination = original.with_file_name(name);
                    number = number.saturating_add(1);
                    continue;
                }
                record.disposition = if hardlink {
                    ResolutionDisposition::Hardlink
                } else if preserved {
                    ResolutionDisposition::Preserved
                } else {
                    ResolutionDisposition::Identical
                };
            } else if preserved {
                record.disposition = ResolutionDisposition::Preserved;
            }
            if preserved {
                record.reason_code = Some("incoming_preserved".into());
                record.warning = Some(format!(
                    "Incoming {} was preserved as {} to keep both versions.",
                    original.display(),
                    destination.display()
                ));
            }
            // A resumed incomplete placement uses this same name again.
            self.store.save_file_resolution(&record).await?;
            let mut file = request.file.clone();
            file.destination_path = destination.clone();
            let mut verified = match existing {
                Some(proof) => proof,
                None => {
                    mover
                        .move_file(FileMoveRequest {
                            file: &file,
                            ..request
                        })
                        .await?
                }
            };
            if !verified.permits_source_removal() {
                return Ok(verified);
            }
            if record.destination_version.is_some()
                && record.destination_version != Some(file_version(&destination).await?)
            {
                return Err(AppError::Validation("The destination changed during comparison. The source was preserved; try again when the files are no longer changing.".into()));
            }
            if verified.hashes.is_none() {
                record.disposition = if tokio::fs::symlink_metadata(source)
                    .await
                    .map_err(|error| file_error(source, error))?
                    .file_type()
                    .is_symlink()
                {
                    ResolutionDisposition::Symlink
                } else {
                    ResolutionDisposition::Hardlink
                };
            } else if record.source_version != Some(file_version(source).await?) {
                return Err(AppError::Validation("The source changed during transfer. It was preserved; try again when the file is no longer changing.".into()));
            }
            record.source_version = Some(file_version(source).await?);
            record.destination_version = Some(file_version(&destination).await?);
            record.identity_hash = verified
                .hashes
                .as_ref()
                .map(|hashes| hashes.full_blake3.clone());
            record.completed = true;
            if let Some(warning) = record.warning.clone() {
                verified.detail = Some(warning);
            }
            self.store.save_file_resolution(&record).await?;
            return Ok(verified);
        }
    }
}

/// Files this size or smaller may be hashed end to end for a preview; anything
/// larger is left unproven rather than read, because a preview is an
/// interactive screen (D4).
pub(super) const PREVIEW_FULL_HASH_LIMIT_BYTES: u64 = 10_000_000;

/// Never read more than the preview threshold, even if a file grows mid-read.
pub async fn preview_hash(path: &Path) -> AppResult<Option<String>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&path).map_err(|error| file_error(&path, error))?;
        let before = file.metadata().map_err(|error| file_error(&path, error))?;
        if before.len() > PREVIEW_FULL_HASH_LIMIT_BYTES {
            return Ok(None);
        }
        let mut remaining = before.len();
        let mut hash = blake3::Hasher::new();
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let wanted = remaining.min(buffer.len() as u64) as usize;
            let read = file
                .read(&mut buffer[..wanted])
                .map_err(|error| file_error(&path, error))?;
            if read == 0 {
                return Ok(None);
            }
            hash.update(&buffer[..read]);
            remaining -= read as u64;
        }
        let after = file.metadata().map_err(|error| file_error(&path, error))?;
        Ok(super::backfill::same_file_version(&before, &after)
            .then(|| hash.finalize().to_hex().to_string()))
    })
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?
}

fn companion_suffix(companion: &Path, media: &Path) -> Option<String> {
    let relative = companion.strip_prefix(media.parent()?).ok()?.to_str()?;
    let suffix = relative.strip_prefix(media.file_stem()?.to_str()?)?;
    (suffix.starts_with('.') || suffix.starts_with('-')).then(|| suffix.to_string())
}

fn numbered_name(base: &str, number: u32) -> String {
    let path = Path::new(base);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    match path.extension() {
        Some(ext) => format!("{stem} ({number}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({number})"),
    }
}

fn file_error(path: &Path, error: impl std::fmt::Display) -> AppError {
    tracing::warn!(path = %path.display(), error = %error, "file comparison failed");
    AppError::Validation(format!(
        "Could not compare {}. Check that it is readable and try again; the source was preserved.",
        path.display()
    ))
}

pub async fn file_version(path: &Path) -> AppResult<String> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| file_error(path, error))?;
    if metadata.file_type().is_symlink() {
        let target = tokio::fs::read_link(path)
            .await
            .map_err(|error| file_error(path, error))?;
        return Ok(format!(
            "symlink:{}:{:?}",
            path_to_stored_string(&target),
            metadata.modified()
        ));
    }
    if !metadata.is_file() {
        return Err(file_error(path, "not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!(
            "{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let signature =
            crate::file_source_signature::file_source_signature_from_metadata(&metadata)?;
        Ok(format!(
            "{}:{}:{}",
            metadata.len(),
            signature.scheme,
            signature.value
        ))
    }
}

pub async fn compare_existing(
    source: &Path,
    destination: &Path,
    progress: &CopyProgress,
) -> AppResult<VerifiedFile> {
    let source_before = file_version(source).await?;
    let destination_before = file_version(destination).await?;
    let source_metadata = tokio::fs::symlink_metadata(source)
        .await
        .map_err(|error| file_error(source, error))?;
    let destination_metadata = tokio::fs::symlink_metadata(destination)
        .await
        .map_err(|error| file_error(destination, error))?;
    if !source_metadata.is_file() || !destination_metadata.is_file() {
        return Err(AppError::Validation("An existing path is not a regular file. Resolve the conflicting path and try again; the source was preserved.".into()));
    }
    if super::execution::same_regular_file(&source_metadata, &destination_metadata) {
        if tokio::fs::canonicalize(source).await.ok()
            == tokio::fs::canonicalize(destination).await.ok()
        {
            return Err(AppError::Validation(
                "The source and destination are the same path.".into(),
            ));
        }
        return Ok(VerifiedFile::same_filesystem_rename(
            source.to_path_buf(),
            destination.to_path_buf(),
            VerificationDepth::Full,
        ));
    }
    let source_size = source_metadata.len();
    let total = source_size.saturating_add(destination_metadata.len());
    progress.comparison(0, total);
    let first = progress.clone();
    let source_progress =
        CopyProgress::none().with_transfer_sink(move |_, bytes| first.comparison(bytes, total));
    let second = progress.clone();
    let destination_progress = CopyProgress::none().with_transfer_sink(move |_, bytes| {
        second.comparison(source_size.saturating_add(bytes), total)
    });
    let source_hashes = hash_existing_file_with_progress(source, source_progress)
        .await
        .map_err(|error| file_error(source, error))?;
    let destination_hashes = hash_existing_file_with_progress(destination, destination_progress)
        .await
        .map_err(|error| file_error(destination, error))?;
    if source_before != file_version(source).await?
        || destination_before != file_version(destination).await?
    {
        return Err(AppError::Validation("A file changed while it was being compared. The source was preserved; try again when the files are no longer changing.".into()));
    }
    let identical = source_hashes.full_blake3 == destination_hashes.full_blake3
        && source_hashes.size_bytes == destination_hashes.size_bytes;
    Ok(VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: Some(source_hashes),
        depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
        outcome: if identical {
            FileVerificationOutcome::Verified
        } else {
            FileVerificationOutcome::Mismatch
        },
        detail: (!identical).then(|| {
            "The destination contains different content. Both files were preserved.".into()
        }),
    })
}
