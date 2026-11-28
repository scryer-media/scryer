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

/// How a resolution row's stored file versions are held to the files now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionCheck {
    /// Every stored version compared whole, change time included: a file
    /// touched since the proof, even with its length and modification time
    /// put back, no longer holds it.
    Exact,
    /// The change time set aside: this pass renamed the source back from its
    /// set-aside name, which moves the change time of the file, and of the
    /// destination too where both names are one file.
    SourcePutBack,
}

impl VersionCheck {
    /// The comparison of one file's version read now with the stored one.
    pub fn holds(self) -> fn(Option<&str>, &str) -> bool {
        match self {
            Self::Exact => |actual, expected| actual == Some(expected),
            Self::SourcePutBack => version_holds,
        }
    }
}

/// What a holding proof found at the source's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceName {
    Present,
    Gone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionDisposition {
    Placed,
    Identical,
    Preserved,
    Hardlink,
    Symlink,
    Disposal(super::conflicts::DisposalProof),
}

impl FileResolution {
    pub fn is_disposal(&self) -> bool {
        matches!(self.disposition, ResolutionDisposition::Disposal(_))
    }

    /// Whether both files still hold the content this row recorded, as the
    /// disk service judges a stored version: the same file, length and
    /// modification time. A rename, a permission change or another link's
    /// removal moves a file's change time without changing its content, so a
    /// change time alone does not stale the proof. For a gate that removes
    /// nothing; one that removes a file asks [`Self::proof_holds_by`].
    pub async fn proof_is_current(&self) -> bool {
        self.proof_holds_by(VersionCheck::SourcePutBack).await
    }

    /// [`Self::proof_is_current`] with every stored version compared whole,
    /// change time included, for a caller that reads an unexplained change
    /// time as a change to the file's links.
    pub async fn proof_is_unchanged(&self) -> bool {
        self.proof_holds_by(VersionCheck::Exact).await
    }

    /// Whether both files still hold what this row recorded, each stored
    /// version compared as `check` says.
    pub async fn proof_holds_by(&self, check: VersionCheck) -> bool {
        self.proof_holds(check.holds()).await.is_some()
    }

    /// `None` when the proof does not hold; otherwise whether it held with
    /// the source name gone, so a caller needs no second read of it.
    async fn proof_holds(&self, holds: fn(Option<&str>, &str) -> bool) -> Option<SourceName> {
        if !self.completed || self.is_disposal() {
            return None;
        }
        let source = stored_path_to_path_buf(&self.source_path);
        let destination = stored_path_to_path_buf(&self.destination_path);
        // The disk service's read of the source itself, a link not followed:
        // the same read on every platform, and the one a hardlink proof
        // compares with the destination's.
        let source_read = Disk::process().fingerprint(&source).await;
        let destination_version = file_version(&destination).await.ok();
        let expected = self.destination_version.as_deref()?;
        let held = |held: bool, source: SourceName| held.then_some(source);
        if source_read
            .as_ref()
            .is_err_and(|reading| reading.exists() == Some(false))
        {
            // Cleanup may have committed before its title checkpoint. Unlinking
            // a hardlink changes ctime, but not its identity, length or mtime.
            return held(
                holds(destination_version.as_deref(), expected)
                    || (self.disposition == ResolutionDisposition::Hardlink
                        && destination_version.as_deref().is_some_and(|actual| {
                            crate::disk::StoredVersion::new(actual)
                                .same_content(crate::disk::StoredVersion::new(expected))
                                == crate::disk::Sameness::Same
                        })),
                SourceName::Gone,
            );
        }
        if self.disposition == ResolutionDisposition::Hardlink {
            let (Ok(source_read), Ok(destination_read)) =
                (source_read, Disk::process().fingerprint(&destination).await)
            else {
                return None;
            };
            return held(
                super::execution::same_regular_file(&source_read, &destination_read),
                SourceName::Present,
            );
        }
        if !holds(destination_version.as_deref(), expected) {
            return None;
        }
        let source_version = file_version(&source).await.ok();
        held(
            match self.source_version.as_deref() {
                Some(source_expected) => holds(source_version.as_deref(), source_expected),
                None => source_version.is_none(),
            },
            SourceName::Present,
        )
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
    /// The destination's full hash, the disk service's answer, is compared
    /// against the record's `identity_hash`, the full-file BLAKE3 the
    /// transfer wrote. A record carrying no `identity_hash` proves nothing
    /// about content on its own, so the two copies' full hashes are compared
    /// to each other instead — never unlinked on the strength of a version
    /// alone. A file that changed while it was read proves nothing.
    ///
    /// All of that is at full `depth`, the user's setting. At quick depth the
    /// two files are compared by their lengths and live head and tail.
    ///
    /// A hardlink this operation placed whose source name is already gone,
    /// removed by this operation's own cleanup, leaves one name to judge: the
    /// retained name holding the recorded version is the proof at quick
    /// depth, and a recorded hash is still held to at either depth.
    ///
    /// The stored versions are compared as `versions` says: whole, unless
    /// this cleanup has just put the source back from its set-aside name.
    pub async fn proof_supports_disposal(
        &self,
        depth: VerificationDepth,
        versions: VersionCheck,
    ) -> AppResult<bool> {
        let Some(source_name) = self.proof_holds(versions.holds()).await else {
            return Ok(false);
        };
        let destination = stored_path_to_path_buf(&self.destination_path);
        let linked_and_gone =
            self.disposition == ResolutionDisposition::Hardlink && source_name == SourceName::Gone;
        let check = match self.identity_hash.as_deref() {
            None if linked_and_gone && depth != VerificationDepth::Full => return Ok(true),
            Some(identity_hash) if linked_and_gone => {
                Disk::process()
                    .check_against(
                        crate::disk::CheckSide::new(destination.as_path()),
                        Some(identity_hash),
                    )
                    .await
            }
            _ if depth != VerificationDepth::Full => {
                let source = stored_path_to_path_buf(&self.source_path);
                Disk::process()
                    .check(
                        crate::disk::CheckSide::new(source.as_path()),
                        crate::disk::CheckSide::new(destination.as_path()),
                        crate::disk::Depth::Quick,
                    )
                    .await
            }
            Some(identity_hash) => {
                Disk::process()
                    .check_against(
                        crate::disk::CheckSide::new(destination.as_path()),
                        Some(identity_hash),
                    )
                    .await
            }
            None => {
                let source = stored_path_to_path_buf(&self.source_path);
                Disk::process()
                    .check(
                        crate::disk::CheckSide::new(source.as_path()),
                        crate::disk::CheckSide::new(destination.as_path()),
                        crate::disk::Depth::Full,
                    )
                    .await
            }
        };
        match check {
            crate::disk::Check::Same => Ok(true),
            crate::disk::Check::Differs | crate::disk::Check::Changed => Ok(false),
            crate::disk::Check::Unread(why) => Err(file_error(&destination, why)),
        }
    }
}

use super::executor::{
    FileAbandonment, FileMoveRequest, PlannedFile, PlannedTitle, TitleFileMover,
    move_error_may_be_storage_loss,
};
use super::model::{
    AppliedVerificationDepth, FileVerificationOutcome, KnownSourceContent, VerificationDepth,
};
use super::verify::{CopyProgress, VerifiedFile, hash_existing_file_with_progress};
use crate::disk::{Disk, EntryKind, FullHashAnswer, HashAsk, KnownHash, PathReading, Remove};
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
            Self::NotTransferred => AppError::validation_refused(crate::error_reasons::location::RELATED_MEDIA_FILE_COULD_NOT_TRANSFERRED, "The related media file could not be transferred. Its companion files were preserved."
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
                Err(error) if move_error_may_be_storage_loss(error) => None,
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

    /// Test seam: companions currently subscribed to a media file's result,
    /// so a test can tell a parked companion from one not yet polled far enough.
    #[cfg(test)]
    fn companions_waiting_on(&self, operation_id: &str, title_id: &str, source: &Path) -> usize {
        self.media_results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(operation_id.into(), title_id.into(), source.to_path_buf()))
            .map_or(0, MediaResultSender::receiver_count)
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
                        AppError::validation_refused(
                            crate::error_reasons::location::RELATED_MEDIA_TRANSFER_INTERRUPTED,
                            "The related media transfer was interrupted. The source was preserved.",
                        )
                    })?;
                }
            };
            let mut name = resolved.file_stem().unwrap_or_default().to_os_string();
            name.push(relative);
            return Ok(Some(resolved.with_file_name(name)));
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
            return Err(AppError::validation_refused(
                crate::error_reasons::location::SOURCE_DESTINATION_SAME_PATH,
                "The source and destination are the same path.",
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
        let base = super::collisions::stored_collision_rename_base(
            &path_to_stored_string(Path::new(original.file_name().unwrap_or_default())),
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
            let reading = Disk::process().uncached(&destination).await;
            let existing = match reading.exists() {
                Some(true) => {
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
                        request.depth,
                    )
                    .await?;
                    // Distinct occupied names are additional work, not retries.
                    compared_before = compared_before
                        .saturating_add(read.load(std::sync::atomic::Ordering::Relaxed));
                    Some(proof)
                }
                Some(false) => None,
                None => return Err(unread(&destination, reading)),
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
                    destination = original.with_file_name(stored_path_to_path_buf(&name));
                    number = number.saturating_add(1);
                    continue;
                }
                record.disposition = if proof.is_link() {
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
            let placed_by_mover = existing.is_none();
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
                return Err(AppError::validation_refused(
                    crate::error_reasons::location::DESTINATION_CHANGED_DURING_COMPARISON,
                    "The destination changed during comparison. The source was preserved; try again when the files are no longer changing.",
                ));
            }
            if verified.is_link() {
                let reading = Disk::process().uncached(source).await;
                if reading.exists() != Some(true) {
                    return Err(unread(source, reading));
                }
                record.disposition = if reading.is(EntryKind::Symlink) {
                    ResolutionDisposition::Symlink
                } else {
                    ResolutionDisposition::Hardlink
                };
            } else {
                let probed = file_version(source).await;
                // At full depth the bytes the copy streamed also have to agree
                // with the full hash the catalog already holds for this source,
                // where the disk service finds that record still fits the file.
                // Nothing is read for this: both hashes are already in hand.
                let disagrees_with_stored_hash = match probed.as_ref() {
                    Ok(version) => {
                        record.source_version.as_deref() == Some(version.as_str())
                            && copy_disagrees_with_stored_hash(
                                request.depth,
                                request.file.source_content.as_ref(),
                                verified.hashes.as_ref(),
                                source,
                            )
                            .await
                    }
                    Err(_) => false,
                };
                // The mover held the copy's hashes back for this decision: a
                // copy refused here must leave the catalog's stored hash as it
                // was, so a retry over the same files is refused again. Every
                // other outcome persists them, as the mover always has.
                if placed_by_mover && !disagrees_with_stored_hash {
                    mover
                        .persist_deferred_hashes(
                            FileMoveRequest {
                                file: &file,
                                ..request
                            },
                            &verified,
                        )
                        .await;
                }
                let source_version = probed?;
                if record.source_version != Some(source_version) {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::location::SOURCE_CHANGED_DURING_TRANSFER,
                        "The source changed during transfer. It was preserved; try again when the file is no longer changing.",
                    ));
                }
                if disagrees_with_stored_hash {
                    verified.outcome = FileVerificationOutcome::Mismatch;
                    verified.detail = Some(format!(
                        "{} does not match the content recorded for {}. The source was preserved.",
                        destination.display(),
                        source.display()
                    ));
                    return Ok(verified);
                }
            }
            record.source_version = Some(file_version(source).await?);
            record.destination_version = Some(file_version(&destination).await?);
            record.identity_hash = verified
                .hashes
                .as_ref()
                .map(|hashes| hashes.full_blake3.clone());
            // The retained link's full hash is recorded only at full depth; at
            // quick depth its version and link count are the proof.
            if record.identity_hash.is_none()
                && record.disposition == ResolutionDisposition::Hardlink
                && request.depth == VerificationDepth::Full
                && self
                    .store
                    .get_location_operation(request.operation_id)
                    .await?
                    .is_some_and(|operation| {
                        matches!(
                            operation.operation_type,
                            super::model::LocationOperationType::FolderConsolidation
                                | super::model::LocationOperationType::FolderKeepOne
                        )
                    })
            {
                let hashes = hash_existing_file_with_progress(&destination, CopyProgress::none())
                    .await
                    .map_err(|error| file_error(&destination, error))?;
                if record.source_version != Some(file_version(source).await?)
                    || record.destination_version != Some(file_version(&destination).await?)
                {
                    return Err(AppError::validation_refused(
                        super::conflicts::refusal::FILE_CHANGED,
                        "A retained hardlink changed while its content proof was recorded; source files were preserved",
                    ));
                }
                record.identity_hash = Some(hashes.full_blake3);
            }
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
    let reading = Disk::process().uncached(&partial).await;
    match reading.exists() {
        Some(false) => return Ok(()),
        None => return Err(unread(&partial, reading)),
        Some(true) => {}
    }
    let source = Disk::process()
        .uncached(stored_path_to_path_buf(&row.source_path))
        .await;
    if !reading.is(EntryKind::File) || reading.size() > Some(source.size().unwrap_or_default()) {
        return Err(AppError::validation_refused(
            crate::error_reasons::location::DESTINATION_OCCUPIED,
            format!(
                "The destination staging path {} is occupied by a file this move did not write. Both files were preserved; move or remove that file and try again.",
                partial.display()
            ),
        ));
    }
    Disk::process()
        .remove(&partial, Remove::File)
        .await
        .map_err(|error| {
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

pub(super) fn companion_suffix(companion: &Path, media: &Path) -> Option<std::ffi::OsString> {
    let relative = companion.strip_prefix(media.parent()?).ok()?;
    let suffix =
        crate::stored_paths::native_name_suffix(relative.as_os_str(), media.file_stem()?, false)?;
    let first = suffix.as_encoded_bytes().first().copied();
    matches!(first, Some(b'.' | b'-')).then_some(suffix)
}

fn numbered_name(base: &str, number: u32) -> String {
    path_to_stored_string(crate::stored_paths::suffix_path_stem(
        &stored_path_to_path_buf(base),
        &format!(" ({number})"),
    ))
}

fn unread(path: &Path, reading: PathReading) -> AppError {
    file_error(path, reading.absence())
}

fn file_error(path: &Path, error: impl std::fmt::Display) -> AppError {
    tracing::warn!(path = %path.display(), error = %error, "file comparison failed");
    AppError::validation_refused(
        crate::error_reasons::location::COULD_NOT_COMPARE,
        format!(
            "Could not compare {}. Check that it is readable and try again; the source was preserved.",
            path.display()
        ),
    )
}

pub async fn file_version(path: &Path) -> AppResult<String> {
    Ok(file_version_and_metadata(path).await?.0)
}

/// Read a regular file and its persisted version without following links.
pub(crate) async fn regular_file_snapshot(path: &Path) -> AppResult<(String, crate::disk::Mark)> {
    let mark = Disk::process()
        .fingerprint(path)
        .await
        .map_err(|reading| file_error(path, reading.absence()))?;
    if mark.kind() != Some(crate::disk::EntryKind::File) {
        return Err(file_error(path, "not a regular file"));
    }
    #[cfg(unix)]
    let version = mark.legacy_unix_version();
    #[cfg(not(unix))]
    let version = mark
        .len()
        .zip(
            mark.modified()
                .and_then(crate::file_source_signature::file_source_signature_from_modified),
        )
        .map(|(len, signature)| format!("{len}:{}:{}", signature.scheme, signature.value));
    Ok((
        version.ok_or_else(|| file_error(path, "file version is unavailable"))?,
        mark,
    ))
}

/// Whether a file read now as `actual` still holds the content a row stored
/// as `expected`: the same text, or the disk service's answer that both name
/// one file with the same length and modification time. No probe.
pub(super) fn version_holds(actual: Option<&str>, expected: &str) -> bool {
    actual.is_some_and(|actual| {
        actual == expected
            || crate::disk::StoredVersion::new(actual)
                .same_content(crate::disk::StoredVersion::new(expected))
                == crate::disk::Sameness::Same
    })
}

/// Whether a full-depth copy's streamed hash contradicts the full hash the
/// catalog holds for its source, where the disk service finds that record
/// still fits the file. Nothing is read: a record that no longer fits has
/// nothing to say.
///
/// Probes: one stat of `source`, followed, when there is a record.
async fn copy_disagrees_with_stored_hash(
    depth: VerificationDepth,
    known: Option<&KnownSourceContent>,
    hashes: Option<&super::model::StreamedContentHashes>,
    source: &Path,
) -> bool {
    let (Some(known), Some(hashes)) = (known, hashes) else {
        return false;
    };
    if depth != VerificationDepth::Full {
        return false;
    }
    match Disk::process()
        .full_hash(known.ask(source).recorded_only())
        .await
        .into_known()
    {
        Some(recorded) => {
            hashes.full_blake3 != recorded.blake3 || hashes.size_bytes != recorded.size_bytes
        }
        None => false,
    }
}

/// [`file_version`] together with the metadata it was read from, for a caller
/// that needs both from the one probe.
#[allow(clippy::disallowed_methods)] // disk-guard: raw dev/ino/ctime versions
async fn file_version_and_metadata(path: &Path) -> AppResult<(String, std::fs::Metadata)> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| file_error(path, error))?;
    let version = file_version_from_metadata(path, &metadata).await?;
    Ok((version, metadata))
}

#[allow(clippy::disallowed_methods)] // disk-guard: symlink target versions
async fn file_version_from_metadata(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> AppResult<String> {
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
            crate::file_source_signature::file_source_signature_from_metadata(metadata)?;
        Ok(format!(
            "{}:{}:{}",
            metadata.len(),
            signature.scheme,
            signature.value
        ))
    }
}

#[allow(clippy::disallowed_methods)] // disk-guard: raw dev/ino and link count guard
pub async fn compare_existing(
    source: &Path,
    destination: &Path,
    progress: &CopyProgress,
    known: Option<&KnownSourceContent>,
    depth: VerificationDepth,
) -> AppResult<VerifiedFile> {
    let (_, source_metadata) = file_version_and_metadata(source).await?;
    let (_, destination_metadata) = file_version_and_metadata(destination).await?;
    // A source that is a symbolic link was copied as the file it names, so
    // it is compared as that file: the disk service reads it through the
    // link, on every platform alike, so that every later read of the source
    // is held to the same file. The destination is held to its own name.
    // std's metadata names no file on Windows, so there the disk service
    // reads each unlinked name once more; on unix the metadata's own mark is
    // used.
    let source_read = match source_metadata.file_type().is_symlink() {
        true => Disk::process().fingerprint_followed(source).await,
        false => super::execution::mark_beside(source, &source_metadata).await,
    }
    .map_err(|reading| unread(source, reading))?;
    let destination_read = super::execution::mark_beside(destination, &destination_metadata)
        .await
        .map_err(|reading| unread(destination, reading))?;
    if source_read.kind() != Some(EntryKind::File) || !destination_metadata.is_file() {
        return Err(AppError::validation_refused(
            crate::error_reasons::location::EXISTING_PATH_NOT_REGULAR_FILE,
            "An existing path is not a regular file. Resolve the conflicting path and try again; the source was preserved.",
        ));
    }
    let source_len = source_read.len().unwrap_or_default();
    // A pair that cannot be told apart is checked as one file would be: the
    // same path or one entry is refused, and two entries are compared below.
    if super::execution::regular_file_sameness(&source_read, &destination_read)
        != crate::disk::Sameness::Different
    {
        let resolved = super::execution::Resolved::of(source, destination).await;
        if resolved.one_path() {
            return Err(AppError::validation_refused(
                crate::error_reasons::location::SOURCE_DESTINATION_SAME_PATH,
                "The source and destination are the same path.",
            ));
        }
        match super::execution::shared_inode(source, destination, &source_read, &destination_read)
            .await
        {
            super::execution::SharedInode::SecondLink => {
                return Ok(VerifiedFile::same_filesystem_rename(
                    source.to_path_buf(),
                    destination.to_path_buf(),
                    VerificationDepth::Full,
                ));
            }
            super::execution::SharedInode::OneEntry => {
                return Err(super::execution::one_entry_refusal());
            }
            // Synthesized inodes: prove the pair by content below.
            super::execution::SharedInode::Distinct => {}
        }
    }
    // A file read here must still be the one first stat'ed above: a change
    // between that stat and the read is as much a change while being
    // compared as one during the read.
    let changed = || {
        AppError::validation_refused(
            crate::error_reasons::location::FILE_CHANGED_WHILE_BEING_COMPARED,
            "A file changed while it was being compared. The source was preserved; try again when the files are no longer changing.",
        )
    };
    // The quick check first, at either depth (FR-073): a length difference or
    // a differing head/tail sample proves different content without reading
    // either file end to end, and the transfer copies straight away. At quick
    // depth a pair it cannot tell apart is the source's copy; at full depth
    // that pair earns the full comparison.
    match Disk::process()
        .check(
            crate::disk::CheckSide::new(source),
            crate::disk::CheckSide::new(destination),
            crate::disk::Depth::Quick,
        )
        .await
    {
        crate::disk::Check::Same => {}
        crate::disk::Check::Differs => return Ok(different_content(source, destination, depth)),
        crate::disk::Check::Changed => return Err(changed()),
        crate::disk::Check::Unread(why) => return Err(file_error(destination, why)),
    }
    if depth != VerificationDepth::Full {
        return Ok(VerifiedFile {
            source_path: source.to_path_buf(),
            destination_path: destination.to_path_buf(),
            hashes: None,
            depth: AppliedVerificationDepth::exact(depth),
            outcome: FileVerificationOutcome::Verified,
            detail: None,
            destination_mark: None,
            content_matched: true,
        });
    }
    // Each side's full hash is the disk service's answer. The source's comes
    // from what its catalog row recorded while the file still fits that
    // record, and then only the destination is read; otherwise both are.
    // Every read is held to the file it began on, and one that saw its file
    // change refuses the comparison.
    let recorded = match known {
        Some(known) => Disk::process()
            .full_hash(known.ask(source).recorded_only())
            .await
            .into_known(),
        None => None,
    };
    let source_size = match recorded {
        Some(_) => 0,
        None => source_len,
    };
    let total = source_size.saturating_add(destination_metadata.len());
    progress.comparison(0, total);
    let source_hash = match (recorded, known) {
        (Some(recorded), _) => recorded,
        (None, known) => {
            let ask = known.map_or_else(|| HashAsk::new(source), |known| known.ask(source));
            let read = compared_full_hash(source, ask, progress, 0, total).await?;
            if !source_read.compare(&read.mark).is_untouched() {
                return Err(changed());
            }
            read
        }
    };
    let destination_hash = compared_full_hash(
        destination,
        HashAsk::new(destination),
        progress,
        source_size,
        total,
    )
    .await?;
    if !destination_read
        .compare(&destination_hash.mark)
        .is_untouched()
    {
        return Err(changed());
    }
    let identical = source_hash.blake3 == destination_hash.blake3
        && source_hash.size_bytes == destination_hash.size_bytes;
    if !identical {
        return Ok(different_content(source, destination, depth));
    }
    let hashes = streamed_hashes(&destination_hash)
        .or_else(|| streamed_hashes(&source_hash))
        .ok_or_else(|| file_error(destination, "no whole-file CRC was taken of it"))?;
    Ok(VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: Some(hashes),
        depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
        outcome: FileVerificationOutcome::Verified,
        detail: None,
        destination_mark: None,
        content_matched: false,
    })
}

/// One side's full hash for [`compare_existing`], its read reported from
/// `offset` of the comparison's `total`.
async fn compared_full_hash(
    path: &Path,
    ask: HashAsk,
    progress: &CopyProgress,
    offset: u64,
    total: u64,
) -> AppResult<KnownHash> {
    let sink = progress.clone();
    let reported = CopyProgress::none()
        .with_transfer_sink(move |_, bytes| sink.comparison(offset.saturating_add(bytes), total));
    match Disk::process()
        .full_hash(ask.progress(reported.disk_progress()))
        .await
    {
        FullHashAnswer::Read(known)
        | FullHashAnswer::Stored(known)
        | FullHashAnswer::Cached(known) => Ok(known),
        FullHashAnswer::Changed => Err(AppError::validation_refused(
            crate::error_reasons::location::FILE_CHANGED_WHILE_BEING_COMPARED,
            "A file changed while it was being compared. The source was preserved; try again when the files are no longer changing.",
        )),
        FullHashAnswer::Unreadable(refusal) => Err(file_error(path, refusal)),
        FullHashAnswer::Cancelled | FullHashAnswer::Unrecorded => {
            Err(file_error(path, "its full hash was not taken"))
        }
    }
}

/// A full hash as the catalog stores one, where it carries its CRC.
fn streamed_hashes(known: &KnownHash) -> Option<super::model::StreamedContentHashes> {
    Some(super::model::StreamedContentHashes {
        size_bytes: known.size_bytes,
        crc_algorithm: super::model::MoveCrcAlgorithm::Crc64Nvme,
        move_crc: known.crc?,
        full_blake3: known.blake3.clone(),
    })
}

/// The comparison's answer for a pair proven different: no source hashes are
/// claimed, because nothing was verified against anything — the resolver moves
/// on to the next candidate name and the transfer copies the file.
fn different_content(source: &Path, destination: &Path, depth: VerificationDepth) -> VerifiedFile {
    VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: None,
        depth: AppliedVerificationDepth::exact(depth),
        outcome: FileVerificationOutcome::Mismatch,
        detail: Some(format!(
            "{} contains different content. Both files were preserved.",
            destination.display()
        )),
        destination_mark: None,
        content_matched: false,
    }
}

#[cfg(test)]
mod native_name_tests {
    use super::*;

    #[test]
    fn literal_codec_prefix_survives_resolver_numbering() {
        let name = "scryer-path-v1:u:movie.mkv";
        let encoded = path_to_stored_string(name);
        assert_eq!(
            stored_path_to_path_buf(&numbered_name(&encoded, 2)),
            Path::new("scryer-path-v1:u:movie (2).mkv")
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn native_companion_suffix_and_numbering_preserve_opaque_units() {
        let (platform, marker) = if cfg!(windows) {
            ("w", "%uD800")
        } else {
            ("u", "%FF")
        };
        let media = stored_path_to_path_buf(&format!(
            "scryer-path-v1:{platform}:/library/movie{marker}.mkv"
        ));
        let companion = stored_path_to_path_buf(&format!(
            "scryer-path-v1:{platform}:/library/movie{marker}.en{marker}.srt"
        ));
        assert_eq!(
            companion_suffix(&companion, &media),
            Some(
                stored_path_to_path_buf(&format!("scryer-path-v1:{platform}:.en{marker}.srt"))
                    .into_os_string()
            )
        );
        let numbered = numbered_name(&path_to_stored_string(&media), 3);
        assert_eq!(
            stored_path_to_path_buf(&numbered),
            stored_path_to_path_buf(&format!(
                "scryer-path-v1:{platform}:/library/movie{marker} (3).mkv"
            ))
        );
    }
}
