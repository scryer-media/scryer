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

    /// Whether this record proves enough to dispose of the source.
    ///
    /// [`Self::proof_is_current`] is the fast negative and stays the first
    /// question: a `file_version` mismatch short-circuits without reading a
    /// byte. It is not, however, an answer about *content*. A version is length
    /// and timestamps, and a destination can be rewritten between the copy and
    /// the resume in a way that preserves them — a same-length rewrite, a
    /// restore from backup on a filesystem whose signature cannot tell the two
    /// apart. Disposal is irreversible, so past the fast negative the bytes
    /// themselves have to agree with what was recorded.
    ///
    /// The destination is re-read and compared against the record's
    /// `identity_hash`, the full-file BLAKE3 the transfer wrote. A record
    /// carrying no `identity_hash` proves nothing about content on its own, so
    /// the two copies are re-hashed and compared to each other instead — never
    /// unlinked on the strength of a version alone.
    pub async fn proof_supports_disposal(&self) -> AppResult<bool> {
        if !self.proof_is_current().await {
            return Ok(false);
        }
        let destination = stored_path_to_path_buf(&self.destination_path);
        let destination_hashes =
            hash_existing_file_with_progress(&destination, CopyProgress::none())
                .await
                .map_err(|error| file_error(&destination, error))?;
        if let Some(identity_hash) = self.identity_hash.as_deref() {
            return Ok(destination_hashes.full_blake3 == identity_hash);
        }
        let source = stored_path_to_path_buf(&self.source_path);
        let source_hashes = hash_existing_file_with_progress(&source, CopyProgress::none())
            .await
            .map_err(|error| file_error(&source, error))?;
        Ok(source_hashes.size_bytes == destination_hashes.size_bytes
            && source_hashes.full_blake3 == destination_hashes.full_blake3)
    }
}

use super::executor::{
    FileAbandonment, FileMoveRequest, PlannedFile, PlannedTitle, TitleFileMover,
    move_error_is_transient,
};
use super::model::{
    AppliedVerificationDepth, FileVerificationOutcome, KnownSourceContent, VerificationDepth,
};
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
type MediaResultSender = tokio::sync::watch::Sender<Option<Result<PathBuf, MediaUnplaced>>>;

/// Why a media file its companions were waiting on was never placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaUnplaced {
    /// It failed, or the runner gave up on it: the companions fail with it.
    NotTransferred,
    /// The user canceled while it waited: the companions are canceled with it.
    Canceled,
}

impl MediaUnplaced {
    fn into_error(self) -> AppError {
        match self {
            Self::NotTransferred => AppError::Validation(
                "The related media file could not be transferred. Its companion files were preserved."
                    .into(),
            ),
            Self::Canceled => AppError::Canceled(
                "The related media file's transfer was canceled. Its companion files were preserved."
                    .into(),
            ),
        }
    }
}

pub struct ConflictResolver {
    pub store: Arc<dyn LocationOperationRepository>,
    /// Each title's source-library label, for the FR-074 suffix.
    pub contexts: BTreeMap<String, String>,
    claims: Mutex<BTreeMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    media_results: Mutex<BTreeMap<(String, String, PathBuf), MediaResultSender>>,
}

impl ConflictResolver {
    pub fn new(
        store: Arc<dyn LocationOperationRepository>,
        contexts: BTreeMap<String, String>,
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
                Ok(file) if file.permits_source_removal() => {
                    Some(Ok(file.destination_path.clone()))
                }
                // Not a verdict yet: the runner retries a transient failure, and
                // waits out a storage outage before it does, so the companions
                // keep waiting for the attempt that places the media file. The
                // runner reports a file it gives up on through
                // `media_abandoned`.
                Err(error) if move_error_is_transient(error) => None,
                Ok(file) if file.outcome == FileVerificationOutcome::Unavailable => None,
                _ => Some(Err(MediaUnplaced::NotTransferred)),
            };
            if let Some(destination) = destination {
                self.media_result(
                    request.operation_id,
                    &request.title.title_id,
                    &request.file.source_path,
                )
                .send_replace(Some(destination));
            }
        }
        result
    }

    /// The runner stopped trying to place `file`. When it is a media file, the
    /// companions still waiting on it settle now instead of waiting on an
    /// attempt that will never come: failed with it when it was given up on,
    /// canceled with it when a cancel handed it back (FR-092).
    pub fn media_abandoned(
        &self,
        operation_id: &str,
        title: &PlannedTitle,
        file: &PlannedFile,
        abandonment: FileAbandonment,
    ) {
        if file.media_file_id.is_none() {
            return;
        }
        let unplaced = match abandonment {
            FileAbandonment::GivenUp => MediaUnplaced::NotTransferred,
            FileAbandonment::Canceled => MediaUnplaced::Canceled,
        };
        self.media_result(operation_id, &title.title_id, &file.source_path)
            .send_if_modified(|value| {
                if value.is_some() {
                    return false;
                }
                *value = Some(Err(unplaced));
                true
            });
    }

    fn media_result(&self, operation_id: &str, title_id: &str, source: &Path) -> MediaResultSender {
        self.media_results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry((operation_id.into(), title_id.into(), source.to_path_buf()))
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
                let mut updates = self
                    .media_result(
                        request.operation_id,
                        &request.title.title_id,
                        &media.source_path,
                    )
                    .subscribe();
                loop {
                    if let Some(result) = updates.borrow_and_update().clone() {
                        break result.map_err(MediaUnplaced::into_error)?;
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
        let label = self
            .contexts
            .get(&request.title.title_id)
            .cloned()
            .unwrap_or_else(|| "source".into());
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
                    let proof = compare_existing(
                        source,
                        &destination,
                        &comparison,
                        request.file.source_content.as_ref(),
                    )
                    .await?;
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
                // Different content moves on to the next candidate name; a
                // proven-identical file merges into the copy that is already
                // there, whichever name it sits under (FR-073). The source is
                // then redundant and is dropped at cleanup, recycle bin or
                // not: a copy proven byte-identical while both exist is not
                // user data that can be lost (C4).
                if !proof.permits_source_removal() {
                    let name = if number == 1 {
                        base.clone()
                    } else {
                        numbered_name(&base, number)
                    };
                    destination = original.with_file_name(name);
                    number = number.saturating_add(1);
                    continue;
                }
                record.disposition = if proof.hashes.is_none() {
                    ResolutionDisposition::Hardlink
                } else {
                    ResolutionDisposition::Identical
                };
            } else if preserved {
                record.disposition = ResolutionDisposition::Preserved;
            }
            if record.disposition == ResolutionDisposition::Identical {
                // The designed outcome of a may-merge item, not a warning: the
                // counters and the asset listing name the dropped copy.
                record.reason_code = Some("identical_merged".into());
            } else if preserved {
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
            if existing.is_none()
                && let Some(row) = saved.as_ref()
            {
                clear_own_abandoned_partial(row, &destination).await?;
            }
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

/// Clear the staging partial an interrupted attempt of *this* operation left
/// behind — and nothing else.
///
/// The persisted resolution row is the evidence. A saved, still-incomplete row
/// naming this source and this destination means this operation had already
/// claimed the destination and begun writing its partial beside it, so an
/// occupant of the staging path is that attempt's own work and clearing it is
/// what lets the resumed copy start from a clean file. Without such a row the
/// copy refuses the occupied staging path instead
/// ([`super::verify`]'s `ensure_partial_path_free`): a file that merely carries
/// the suffix is a user file, and deleting one is unrecoverable.
///
/// The row is not the only test. An abandoned partial is a prefix of the source
/// it was copying, so it is a plain file no longer than that source; anything
/// else at the name is somebody else's, whatever the record says, and it is
/// preserved exactly as an occupied destination is.
async fn clear_own_abandoned_partial(row: &FileResolution, destination: &Path) -> AppResult<()> {
    if row.completed || row.destination_path != path_to_stored_string(destination) {
        return Ok(());
    }
    let partial = super::verify::partial_destination_path(destination);
    let metadata = match tokio::fs::symlink_metadata(&partial).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(file_error(&partial, error)),
    };
    let source = stored_path_to_path_buf(&row.source_path);
    let source_len = tokio::fs::symlink_metadata(&source)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    if !metadata.is_file() || metadata.len() > source_len {
        return Err(AppError::Validation(format!(
            "The destination staging path {} is occupied by a file this move did not write. Both files were preserved; move or remove that file and try again.",
            partial.display()
        )));
    }
    tokio::fs::remove_file(&partial).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to clear this move's abandoned partial copy at {}: {error}",
            partial.display()
        ))
    })?;
    tracing::info!(
        operation_id = %row.operation_id,
        title_id = %row.title_id,
        partial = %partial.display(),
        "cleared the abandoned partial copy this operation's interrupted attempt left behind"
    );
    Ok(())
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
    known: Option<&KnownSourceContent>,
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
    // The quick check first (FR-073): a size difference or a differing
    // head/tail sample proves different content without reading either file
    // end to end, and the transfer copies straight away. Only a pair the quick
    // check cannot tell apart earns the full comparison.
    if source_metadata.len() != destination_metadata.len()
        || !sampled_proofs_agree(source, destination).await?
    {
        return Ok(different_content(source, destination));
    }
    // The source's bytes were already hashed end to end when the catalog
    // recorded them; while its signature still matches, that hash stands in
    // for a second read and only the destination is read.
    if let Some(known) = known.filter(|known| known.matches(&source_metadata)) {
        let total = destination_metadata.len();
        progress.comparison(0, total);
        let sink = progress.clone();
        let destination_progress =
            CopyProgress::none().with_transfer_sink(move |_, bytes| sink.comparison(bytes, total));
        let destination_hashes =
            hash_existing_file_with_progress(destination, destination_progress)
                .await
                .map_err(|error| file_error(destination, error))?;
        if source_before != file_version(source).await?
            || destination_before != file_version(destination).await?
        {
            return Err(AppError::Validation("A file changed while it was being compared. The source was preserved; try again when the files are no longer changing.".into()));
        }
        let identical = destination_hashes.full_blake3 == known.full_blake3
            && destination_hashes.size_bytes == known.size_bytes;
        if !identical {
            return Ok(different_content(source, destination));
        }
        return Ok(VerifiedFile {
            source_path: source.to_path_buf(),
            destination_path: destination.to_path_buf(),
            hashes: Some(destination_hashes),
            depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
            outcome: FileVerificationOutcome::Verified,
            detail: None,
        });
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
    if !identical {
        return Ok(different_content(source, destination));
    }
    Ok(VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: Some(source_hashes),
        depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
        outcome: FileVerificationOutcome::Verified,
        detail: None,
    })
}

/// Whether the sampled head/tail proofs of two same-sized files agree. A
/// disagreement is proof of different bytes; agreement only means the full
/// comparison has to decide.
async fn sampled_proofs_agree(source: &Path, destination: &Path) -> AppResult<bool> {
    let (source, destination) = (source.to_path_buf(), destination.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let left = crate::fs_integrity::import_content_proof(&source)
            .map_err(|error| file_error(&source, error))?;
        let right = crate::fs_integrity::import_content_proof(&destination)
            .map_err(|error| file_error(&destination, error))?;
        Ok(left.size_bytes == right.size_bytes && left.sample_blake3 == right.sample_blake3)
    })
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?
}

/// The comparison's answer for a pair proven different: no source hashes are
/// claimed, because nothing was verified against anything — the resolver moves
/// on to the next candidate name and the transfer copies the file.
fn different_content(source: &Path, destination: &Path) -> VerifiedFile {
    VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: None,
        depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
        outcome: FileVerificationOutcome::Mismatch,
        detail: Some(format!(
            "{} contains different content. Both files were preserved.",
            destination.display()
        )),
    }
}
