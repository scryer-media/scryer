//! Verified copy machinery: one streaming pass, two hashers (D2).
//!
//! Every byte a location operation copies is read once at the source and fed to
//! both a streaming CRC (the move-corruption check, FR-040) and a full-file
//! BLAKE3 (the dedup identity, FR-041/D4). The CRC is what a full-depth
//! destination read-back is compared against (FR-042); the BLAKE3 is persisted
//! with the media file so dedup never has to re-read anything (D4).
//!
//! # Chosen CRC algorithm: CRC-64/NVME
//!
//! [`MOVE_CRC_ALGORITHM`] is `crc_fast::CrcAlgorithm::Crc64Nvme`, persisted under
//! the tag [`MoveCrcAlgorithm::Crc64Nvme`] so a future default change can never
//! be mistaken for corruption.
//!
//! Measured with a throwaway harness (256 MiB in-memory buffer, 1 MiB update
//! chunks, best of 7 passes, `crc-fast` and `blake3` at `opt-level = 3`) on an
//! Apple M5 Max, native `aarch64-apple-darwin`. `crc-fast` reported the
//! `aarch64-neon-pmull-sha3` tier for every candidate, i.e. the hardware-
//! accelerated path was live throughout. The host was under heavy concurrent
//! build load, so these are contended lower bounds — the ranges are the spread
//! across four repeats of the whole sweep, and only the *relative* ordering is
//! load-independent:
//!
//! | Candidate | Throughput |
//! |---|---|
//! | CRC-64/NVME | 0.36 – 0.58 GiB/s |
//! | CRC-64/GO-ISO | 0.40 – 0.59 GiB/s |
//! | CRC-64/XZ (ECMA-182) | 0.33 – 0.54 GiB/s |
//! | CRC-32/ISCSI (Castagnoli) | 0.88 – 1.09 GiB/s |
//! | CRC-32/ISO-HDLC | 0.52 – 1.08 GiB/s |
//! | BLAKE3 (single-threaded, reference) | 1.27 – 2.31 GiB/s |
//! | Combined pass (CRC-64/NVME + BLAKE3) | 0.26 – 0.46 GiB/s |
//!
//! Why CRC-64/NVME:
//!
//! - It is the fastest CRC-64 variant the crate offers on this hardware.
//!   CRC-64/GO-ISO tied with it inside the measurement noise; CRC-64/XZ was
//!   consistently last. NVME wins the tie as the crate's own headline reflected
//!   CRC-64 (it has a dedicated `crc64_nvme()` entry point) and as a current
//!   standard rather than a legacy one.
//! - The CRC-32 variants are roughly twice as fast but 32 bits is too narrow a
//!   check for multi-gigabyte media files, which is exactly the size class this
//!   subsystem exists to move safely (C4: evidence scales with irreversibility).
//! - The extra CRC-64 cost is not on the critical path. A copy is bounded by
//!   device and network throughput, and the combined pass is bounded by BLAKE3,
//!   which D2 requires regardless. Even contended, the combined pass exceeds any
//!   spinning disk or network share this runs over.
//!
//! Re-measure if the default ever moves: `MoveCrcAlgorithm` exists so stored
//! CRCs stay interpretable across such a change.
//!
//! # Depth-governed verification (FR-042/043, D3)
//!
//! [`VerifiedCopier`] performs the copy and then proves the destination at the
//! requested depth:
//!
//! - **Full** (the default) re-reads the whole destination with the platform's
//!   cache bypass engaged (`F_NOCACHE` on macOS, `POSIX_FADV_DONTNEED` on Linux)
//!   and compares the read-back CRC and byte count against the values streamed
//!   during the copy.
//! - **Quick check** compares the destination's sampled head+tail proof against
//!   the source's — [`crate::fs_integrity`]'s existing windows, not a second
//!   sampling scheme — plus an exact size compare against the streamed size.
//!
//! Quick is the universal floor: when the full read-back cannot run, the result
//! is the quick check stamped [`AppliedVerificationDepth::quick_fallback`] with
//! a `detail` naming the reason, never a pass and never something weaker.
//!
//! # The partial-copy name (FR-033, "crash mid-copy is resumable")
//!
//! A copy that claims its own destination never writes under the destination's
//! real name. It writes to a sibling [`partial_destination_path`] and promotes
//! that name onto the destination with
//! [`crate::fs_safety::promote_staged_file`], which never replaces anything.
//! The consequence is the one resume depends on: after a crash, ENOSPC, or a
//! dropped network mount, the destination name either does not exist (so the
//! resumed run copies it cleanly) or holds a complete file (so the resumed run
//! proves it). It is never a truncated file under the real name, which the old
//! `create_new` claim would have turned into a permanent "File exists" failure.
//!
//! An abandoned partial is this operation's own work by construction — the
//! ownership guard means no other operation may write these paths — so the next
//! attempt removes it before copying rather than leaving litter behind.
//!
//! Nothing here deletes, recycles, or rolls anything back. A mismatched
//! destination is kept exactly as written so it can be inspected, and the source
//! is never touched: source removal is the executor's step, gated on
//! [`FileVerificationOutcome::permits_source_removal`] (FR-044).

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use crc_fast::CrcAlgorithm;

use crate::location::model::{
    AppliedVerificationDepth, FileVerificationOutcome, FileVerificationRecord, MoveCrcAlgorithm,
    StreamedContentHashes, VerificationDepth,
};
use crate::{AppError, AppResult};

/// The `crc-fast` algorithm used for the streaming move-corruption check.
pub const MOVE_CRC_ALGORITHM: CrcAlgorithm = CrcAlgorithm::Crc64Nvme;

/// The persisted tag written alongside every CRC produced by
/// [`MOVE_CRC_ALGORITHM`] (FR-041 "algorithm-tagged").
pub const MOVE_CRC_ALGORITHM_TAG: MoveCrcAlgorithm = MoveCrcAlgorithm::Crc64Nvme;

/// Feeds every copied buffer to both hashers so the source is read exactly once
/// (D2). The CRC is the move-corruption check compared against the destination
/// read-back (FR-040); the BLAKE3 is the dedup identity persisted with the media
/// file (FR-041, D4).
pub struct StreamedContentHasher {
    crc: crc_fast::Digest,
    blake3: blake3::Hasher,
    size_bytes: u64,
}

impl StreamedContentHasher {
    pub fn new() -> Self {
        Self {
            crc: crc_fast::Digest::new(MOVE_CRC_ALGORITHM),
            blake3: blake3::Hasher::new(),
            size_bytes: 0,
        }
    }

    /// Absorb one buffer of copied bytes.
    pub fn update(&mut self, buffer: &[u8]) {
        self.crc.update(buffer);
        self.blake3.update(buffer);
        self.size_bytes = self.size_bytes.saturating_add(buffer.len() as u64);
    }

    /// Bytes absorbed so far.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Finish both hashers and produce the pair persisted with the media file.
    pub fn finalize(self) -> StreamedContentHashes {
        StreamedContentHashes {
            size_bytes: self.size_bytes,
            crc_algorithm: MOVE_CRC_ALGORITHM_TAG,
            move_crc: self.crc.finalize(),
            full_blake3: self.blake3.finalize().to_hex().to_string(),
        }
    }
}

impl Default for StreamedContentHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// The `crc-fast` algorithm a persisted tag names, so a stored CRC stays
/// comparable after the default moves.
pub fn crc_algorithm_for(tag: MoveCrcAlgorithm) -> CrcAlgorithm {
    match tag {
        MoveCrcAlgorithm::Crc64Nvme => CrcAlgorithm::Crc64Nvme,
    }
}

// ── The verified copy ────────────────────────────────────────────────────────

/// Bytes moved per copy iteration. One buffer is written and fed to both hashers
/// before the next read, so the source is read exactly once (D2).
const COPY_CHUNK_BYTES: usize = 1024 * 1024;

/// Bytes per read-back iteration. Kept equal to the copy chunk: the read-back is
/// sequential and this is large enough that the per-call overhead disappears
/// against device throughput.
const READ_BACK_CHUNK_BYTES: usize = 1024 * 1024;

/// Who reserved the destination name.
///
/// A copy must never widen a destination it does not hold: `rename(2)` and a
/// plain `create` both replace silently, which is the failure
/// [`crate::fs_safety`] exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationClaim {
    /// This copy claims the destination name itself (`create_new`), failing if
    /// anything already holds it.
    ClaimHere,
    /// The caller already claimed or staged the destination (as
    /// [`crate::fs_safety`]'s movers do); open it for writing without creating.
    AlreadyHeld,
}

/// Suffix the in-progress copy of a destination file carries until it is
/// complete and promoted onto the real name.
///
/// Deterministic on purpose: the next attempt has to be able to *find* the
/// partial a crashed attempt abandoned, and a random name would leave one
/// behind on every interruption.
pub const PARTIAL_COPY_SUFFIX: &str = ".scryer-partial";

/// Where a copy writes while it is still in progress: a sibling of
/// `destination` in the same directory, so the promotion is a rename inside one
/// filesystem and the fsynced directory entry covers both names.
pub fn partial_destination_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{name}{PARTIAL_COPY_SUFFIX}"))
}

/// Observes bytes as they are written, so a caller can show intra-file progress
/// for a copy that runs for hours (FR-091).
///
/// The sink is called on the blocking copy thread once per chunk and must do
/// nothing but bookkeeping — the runner accumulates into an atomic and does the
/// persisting on its own, throttled, schedule.
#[derive(Clone, Default)]
pub struct CopyProgress(Option<Arc<dyn Fn(u64) + Send + Sync>>);

impl CopyProgress {
    /// A copy nobody is watching.
    pub fn none() -> Self {
        Self(None)
    }

    pub fn from_fn(sink: impl Fn(u64) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(sink)))
    }

    /// Report `bytes` newly written to the destination.
    pub fn advance(&self, bytes: u64) {
        if let Some(sink) = &self.0 {
            sink(bytes);
        }
    }

    pub fn is_observed(&self) -> bool {
        self.0.is_some()
    }
}

impl std::fmt::Debug for CopyProgress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CopyProgress")
            .field("observed", &self.is_observed())
            .finish()
    }
}

/// One file's worth of verified-copy work.
#[derive(Debug, Clone)]
pub struct VerifiedCopyRequest {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// The operation's configured depth (FR-042); the applied depth may be the
    /// quick floor instead.
    pub depth: VerificationDepth,
    pub claim: DestinationClaim,
    /// Where intra-file byte progress is reported, when anyone is watching.
    pub progress: CopyProgress,
}

/// What verifying one destination file concluded, without the identifiers that
/// turn it into a persisted row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationAssessment {
    /// Requested vs applied depth. Only meaningful when `outcome` is
    /// [`FileVerificationOutcome::Verified`] or
    /// [`FileVerificationOutcome::Mismatch`]: an `Unavailable` file was never
    /// proven at any depth.
    pub depth: AppliedVerificationDepth,
    pub outcome: FileVerificationOutcome,
    /// Why the outcome is not a plain pass, or why the depth fell back.
    pub detail: Option<String>,
}

/// Everything the executor needs to persist a [`FileVerificationRecord`] for one
/// file, plus the source-removal gate (FR-044).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFile {
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    /// Hashes streamed during the copy; `None` for a same-filesystem rename,
    /// which copies no bytes (FR-032).
    pub hashes: Option<StreamedContentHashes>,
    pub depth: AppliedVerificationDepth,
    pub outcome: FileVerificationOutcome,
    pub detail: Option<String>,
}

impl VerifiedFile {
    /// The FR-044 gate: only a verified destination unblocks touching the
    /// source. Every source recycle/removal decision must read this, never the
    /// absence of an error.
    pub fn permits_source_removal(&self) -> bool {
        self.outcome.permits_source_removal()
    }

    /// The FR-032 fast path: a same-filesystem rename moved the file, so no
    /// bytes were copied and there is nothing to verify. Recorded so the
    /// operation's per-file history has no gaps.
    pub fn same_filesystem_rename(
        source: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
        requested: VerificationDepth,
    ) -> Self {
        Self {
            source_path: source.into(),
            destination_path: destination.into(),
            hashes: None,
            depth: AppliedVerificationDepth::exact(requested),
            outcome: FileVerificationOutcome::Verified,
            detail: Some(
                "same-filesystem rename: no bytes were copied, so no verification pass was needed (FR-032)"
                    .to_string(),
            ),
        }
    }

    /// Attach the operation's identifiers and become the persisted row (D5).
    pub fn into_record(
        self,
        identity: FileVerificationIdentity<'_>,
        verified_at: DateTime<Utc>,
    ) -> FileVerificationRecord {
        FileVerificationRecord {
            operation_id: identity.operation_id.to_string(),
            title_id: identity.title_id.to_string(),
            media_file_id: identity.media_file_id.map(str::to_string),
            source_path: crate::stored_paths::path_to_stored_string(&self.source_path),
            destination_path: crate::stored_paths::path_to_stored_string(&self.destination_path),
            hashes: self.hashes,
            depth: self.depth,
            outcome: self.outcome,
            detail: self.detail,
            verified_at,
        }
    }
}

/// The identifiers a [`VerifiedFile`] does not know about itself.
#[derive(Debug, Clone, Copy)]
pub struct FileVerificationIdentity<'a> {
    pub operation_id: &'a str,
    pub title_id: &'a str,
    /// `None` for companion assets, which are moved and verified but are not
    /// media-file records.
    pub media_file_id: Option<&'a str>,
}

/// Where a persisted verification record is written.
///
/// Declared here so the verified copy has a seam to record against without this
/// module reaching for a repository. The executor owns the implementation and
/// the wiring; verification itself stays pure filesystem work.
#[async_trait::async_trait]
pub trait FileVerificationRecorder: Send + Sync {
    async fn record_file_verification(&self, record: FileVerificationRecord) -> AppResult<()>;
}

/// How the destination was opened for a full-depth read-back.
pub enum ReadBackHandle {
    /// Open and ready to read.
    Ready {
        file: File,
        bypass: CacheBypass,
    },
    /// The read-back cannot run here; verification falls back to the quick floor
    /// with this reason (FR-042).
    Unsupported(String),
    /// The destination itself could not be opened, so nothing can be proven.
    Unavailable(String),
}

/// Whether the read-back is actually bypassing the page cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheBypass {
    /// The platform's bypass is engaged for this descriptor.
    Engaged,
    /// This platform offers no bypass mechanism. FR-042 asks for cache bypass
    /// "where the platform allows", so the full read-back still runs; the
    /// weaker guarantee is noted in the record's detail.
    NotAvailableOnPlatform,
    /// The platform has a bypass but the filesystem rejected it for this file.
    /// A cached full read-back is still stronger evidence than the quick floor,
    /// so the read-back proceeds and the weaker guarantee is noted in the
    /// record's detail rather than downgrading the applied depth.
    Rejected(String),
}

/// Opens a destination for the full-depth read-back.
///
/// Production uses [`open_cache_bypassed`]; tests inject failures to exercise
/// the quick-floor fallback.
pub type ReadBackOpener = Arc<dyn Fn(&Path) -> ReadBackHandle + Send + Sync>;

/// Copies files with the streamed hash pair and proves the result at the
/// configured depth.
///
/// Cheap to clone; the executor can hold one per operation.
#[derive(Clone)]
pub struct VerifiedCopier {
    read_back_opener: ReadBackOpener,
}

impl std::fmt::Debug for VerifiedCopier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("VerifiedCopier").finish_non_exhaustive()
    }
}

impl Default for VerifiedCopier {
    fn default() -> Self {
        Self::new()
    }
}

impl VerifiedCopier {
    pub fn new() -> Self {
        Self {
            read_back_opener: Arc::new(open_cache_bypassed as fn(&Path) -> ReadBackHandle),
        }
    }

    /// Substitute the read-back opener. The seam exists so the fallback path is
    /// testable without an exotic filesystem.
    pub fn with_read_back_opener(opener: ReadBackOpener) -> Self {
        Self {
            read_back_opener: opener,
        }
    }

    /// Copy `source` onto `destination`, then prove the destination at
    /// `depth`.
    ///
    /// Never returns a partial success: a copy failure is an error and leaves
    /// whatever it wrote in place for the caller to roll back (the same
    /// contract [`crate::fs_safety`] rolls back against). Verification failures
    /// are outcomes, not errors — a mismatched destination is a fact to record
    /// and surface, not an exception.
    pub async fn copy_and_verify(&self, request: VerifiedCopyRequest) -> AppResult<VerifiedFile> {
        let hashes = self
            .copy_with_progress(
                &request.source,
                &request.destination,
                request.claim,
                &request.progress,
            )
            .await?;
        let assessment = self
            .verify(&request.source, &request.destination, &hashes, request.depth)
            .await?;

        Ok(VerifiedFile {
            source_path: request.source,
            destination_path: request.destination,
            hashes: Some(hashes),
            depth: assessment.depth,
            outcome: assessment.outcome,
            detail: assessment.detail,
        })
    }

    /// One streaming copy pass with nobody watching the byte counter.
    pub async fn copy(
        &self,
        source: &Path,
        destination: &Path,
        claim: DestinationClaim,
    ) -> AppResult<StreamedContentHashes> {
        self.copy_with_progress(source, destination, claim, &CopyProgress::none())
            .await
    }

    /// One streaming copy pass: every buffer is written and fed to both hashers
    /// (D2). The destination is fsynced, and its directory entry with it.
    ///
    /// A [`DestinationClaim::ClaimHere`] copy goes through
    /// [`partial_destination_path`] and is promoted onto the destination only
    /// once it is complete, so an interruption never leaves a truncated file
    /// under the real name. A [`DestinationClaim::AlreadyHeld`] copy writes in
    /// place: the caller already staged the name and owns the rollback.
    pub async fn copy_with_progress(
        &self,
        source: &Path,
        destination: &Path,
        claim: DestinationClaim,
        progress: &CopyProgress,
    ) -> AppResult<StreamedContentHashes> {
        match claim {
            DestinationClaim::AlreadyHeld => {
                let source = source.to_path_buf();
                let destination = destination.to_path_buf();
                let progress = progress.clone();
                spawn_verify_blocking(move || {
                    copy_with_streamed_hashes(
                        &source,
                        &destination,
                        DestinationClaim::AlreadyHeld,
                        &progress,
                    )
                })
                .await?
            }
            DestinationClaim::ClaimHere => {
                self.copy_through_partial(source, destination, progress)
                    .await
            }
        }
    }

    /// Write to the partial name, then promote it onto the destination.
    async fn copy_through_partial(
        &self,
        source: &Path,
        destination: &Path,
        progress: &CopyProgress,
    ) -> AppResult<StreamedContentHashes> {
        let partial = partial_destination_path(destination);
        // A partial at this path can only be an earlier attempt of this
        // operation's own copy of this file: the ownership guard means nothing
        // else may write here. Clearing it is what makes the retry — and the
        // resumed run — start from a clean file rather than appending to a
        // truncated one.
        clear_abandoned_partial(&partial).await?;

        let hashes = {
            let source = source.to_path_buf();
            let partial = partial.clone();
            let progress = progress.clone();
            spawn_verify_blocking(move || {
                copy_with_streamed_hashes(
                    &source,
                    &partial,
                    DestinationClaim::ClaimHere,
                    &progress,
                )
            })
            .await?
        };
        let hashes = match hashes {
            Ok(hashes) => hashes,
            Err(error) => {
                // The source is untouched, so the incomplete partial is worth
                // nothing to anyone; leaving it would only confuse the next
                // attempt's stat.
                remove_partial_best_effort(&partial).await;
                return Err(error);
            }
        };

        if let Err(error) = crate::fs_safety::promote_staged_file(
            &partial,
            destination,
            crate::fs_safety::MoveOptions::default(),
        )
        .await
        {
            remove_partial_best_effort(&partial).await;
            return Err(AppError::Repository(format!(
                "failed to place the completed copy at {}: {error}",
                destination.display()
            )));
        }

        // The promotion is a second directory-entry change, so the entry the
        // reader will look for needs its own fsync.
        sync_parent_directory_after_promotion(destination).await;
        Ok(hashes)
    }

    /// Prove a destination file that is already there against its source, for
    /// the crash window between a promotion and the verification record that
    /// should have followed it.
    ///
    /// The source is read once to recover the hashes the interrupted copy
    /// streamed, then the destination is proven at `requested` exactly as a
    /// fresh copy would be. A destination that does not prove comes back as a
    /// [`FileVerificationOutcome::Mismatch`] — never a silent removal, never a
    /// pass.
    pub async fn verify_existing_destination(
        &self,
        source: &Path,
        destination: &Path,
        requested: VerificationDepth,
    ) -> AppResult<VerifiedFile> {
        let hashes = {
            let source = source.to_path_buf();
            spawn_verify_blocking(move || hash_source(&source)).await?
        }?;
        let assessment = self.verify(source, destination, &hashes, requested).await?;

        Ok(VerifiedFile {
            source_path: source.to_path_buf(),
            destination_path: destination.to_path_buf(),
            hashes: Some(hashes),
            depth: assessment.depth,
            outcome: assessment.outcome,
            detail: join_details(
                Some(format!(
                    "{} was already in place with no verification record and was proven against the source",
                    destination.display()
                )),
                assessment.detail,
            ),
        })
    }

    /// Prove `destination` against the hashes streamed while it was written.
    pub async fn verify(
        &self,
        source: &Path,
        destination: &Path,
        hashes: &StreamedContentHashes,
        requested: VerificationDepth,
    ) -> AppResult<VerificationAssessment> {
        let copier = self.clone();
        let source = source.to_path_buf();
        let destination = destination.to_path_buf();
        let hashes = hashes.clone();
        spawn_verify_blocking(move || {
            Ok(copier.verify_blocking(&source, &destination, &hashes, requested))
        })
        .await?
    }

    /// Blocking twin of [`VerifiedCopier::verify`], for callers already on a
    /// blocking thread (the download-client copy path, FR-045).
    pub fn verify_blocking(
        &self,
        source: &Path,
        destination: &Path,
        hashes: &StreamedContentHashes,
        requested: VerificationDepth,
    ) -> VerificationAssessment {
        match requested {
            VerificationDepth::Quick => {
                let (outcome, detail) = quick_check(source, destination, hashes);
                VerificationAssessment {
                    depth: AppliedVerificationDepth::exact(VerificationDepth::Quick),
                    outcome,
                    detail,
                }
            }
            VerificationDepth::Full => self.full_verification(source, destination, hashes),
        }
    }

    fn full_verification(
        &self,
        source: &Path,
        destination: &Path,
        hashes: &StreamedContentHashes,
    ) -> VerificationAssessment {
        match (self.read_back_opener)(destination) {
            ReadBackHandle::Unavailable(reason) => VerificationAssessment {
                depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
                outcome: FileVerificationOutcome::Unavailable,
                detail: Some(reason),
            },
            ReadBackHandle::Unsupported(reason) => {
                quick_floor_fallback(source, destination, hashes, &reason)
            }
            ReadBackHandle::Ready { file, bypass } => match full_read_back(file, hashes) {
                Ok((outcome, detail)) => VerificationAssessment {
                    depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
                    outcome,
                    detail: join_details(bypass_note(bypass), detail),
                },
                Err(reason) => quick_floor_fallback(source, destination, hashes, &reason),
            },
        }
    }
}

/// Runs one blocking filesystem step off the async runtime, the way
/// [`crate::fs_integrity`] does.
async fn spawn_verify_blocking<T, F>(work: F) -> AppResult<AppResult<T>>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        AppError::Repository(format!("verified copy task failed to join: {error}"))
    })
}

/// Removes a partial an earlier attempt abandoned, so this attempt can claim
/// the name.
///
/// A partial that cannot be removed is an error rather than something to work
/// around: the copy would otherwise fail on the claim anyway, and the reason
/// the caller needs to see is the removal failure, not "File exists".
async fn clear_abandoned_partial(partial: &Path) -> AppResult<()> {
    match tokio::fs::remove_file(partial).await {
        Ok(()) => {
            tracing::info!(
                partial = %partial.display(),
                "cleared an abandoned partial copy left by an interrupted attempt"
            );
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to clear the abandoned partial copy at {}: {error}",
            partial.display()
        ))),
    }
}

/// Drops an incomplete partial after a failed attempt. Best effort: a stray
/// partial costs a stat on the next attempt, never correctness, and the source
/// it was copied from is untouched.
async fn remove_partial_best_effort(partial: &Path) {
    if let Err(error) = tokio::fs::remove_file(partial).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            partial = %partial.display(),
            %error,
            "could not remove an incomplete partial copy; the next attempt will clear it"
        );
    }
}

async fn sync_parent_directory_after_promotion(destination: &Path) {
    let destination = destination.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        sync_parent_directory_best_effort(&destination);
    })
    .await;
}

/// Streams the source through both hashers without writing anything, so an
/// already-placed destination can be compared against what a copy of that
/// source would have produced.
fn hash_source(source: &Path) -> AppResult<StreamedContentHashes> {
    let mut input = File::open(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to open copy source: {}: {error}",
            source.display()
        ))
    })?;

    let mut hasher = StreamedContentHasher::new();
    let mut buffer = vec![0u8; COPY_CHUNK_BYTES];
    loop {
        let read = input.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!(
                "failed to read copy source: {}: {error}",
                source.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

/// Copies `source` into a destination this call either claims or was handed,
/// hashing every buffer on the way through.
fn copy_with_streamed_hashes(
    source: &Path,
    destination: &Path,
    claim: DestinationClaim,
    progress: &CopyProgress,
) -> AppResult<StreamedContentHashes> {
    let mut input = File::open(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to open copy source: {}: {error}",
            source.display()
        ))
    })?;

    let mut output = open_destination(destination, claim)?;

    let mut hasher = StreamedContentHasher::new();
    let mut buffer = vec![0u8; COPY_CHUNK_BYTES];
    loop {
        let read = input.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!(
                "failed to read copy source: {}: {error}",
                source.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read]).map_err(|error| {
            AppError::Repository(format!(
                "failed to write copy destination: {}: {error}",
                destination.display()
            ))
        })?;
        progress.advance(read as u64);
    }

    output.flush().map_err(|error| {
        AppError::Repository(format!(
            "failed to flush copy destination: {}: {error}",
            destination.display()
        ))
    })?;
    output.sync_all().map_err(|error| {
        AppError::Repository(format!(
            "failed to fsync copy destination: {}: {error}",
            destination.display()
        ))
    })?;
    drop(output);

    // The file's own fsync does not promise its directory entry survives a
    // crash. Best-effort because filesystems that reject a directory fsync are
    // not a reason to fail a copy that otherwise succeeded.
    sync_parent_directory_best_effort(destination);

    // Carry the source's mode so a copy does not silently change access. The
    // operation's configured permissions are applied afterwards by the executor
    // (FR-031) and take precedence; this is only the floor, matching what
    // `fs_safety` already does for its own cross-device copies.
    carry_source_mode_best_effort(source, destination);

    Ok(hasher.finalize())
}

fn open_destination(destination: &Path, claim: DestinationClaim) -> AppResult<File> {
    let mut options = std::fs::OpenOptions::new();
    match claim {
        // `create_new` is the claim: it fails rather than replacing anything.
        DestinationClaim::ClaimHere => options.write(true).create_new(true),
        // The caller holds the name; opening without `create` means the claim,
        // not this call, is what decided the file may be written.
        DestinationClaim::AlreadyHeld => options.write(true).truncate(true),
    };
    options.open(destination).map_err(|error| {
        AppError::Repository(format!(
            "failed to open copy destination: {}: {error}",
            destination.display()
        ))
    })
}

#[cfg(unix)]
fn sync_parent_directory_best_effort(destination: &Path) {
    let Some(parent) = destination.parent() else {
        return;
    };
    if parent.as_os_str().is_empty() {
        return;
    }
    if let Ok(directory) = File::open(parent)
        && let Err(error) = directory.sync_all()
    {
        tracing::debug!(
            parent = %parent.display(),
            %error,
            "could not fsync the destination directory after a verified copy"
        );
    }
}

#[cfg(not(unix))]
fn sync_parent_directory_best_effort(_destination: &Path) {}

#[cfg(unix)]
fn carry_source_mode_best_effort(source: &Path, destination: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = std::fs::metadata(source) else {
        return;
    };
    let mode = metadata.permissions().mode();
    let _ = std::fs::set_permissions(destination, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn carry_source_mode_best_effort(_source: &Path, _destination: &Path) {}

// ── Depth: full ──────────────────────────────────────────────────────────────

/// Reads the whole destination back and compares it against what the copy
/// streamed.
///
/// `Err` means the read-back could not be completed, which is the FR-042
/// fallback trigger, not a verdict on the content.
fn full_read_back(
    mut file: File,
    hashes: &StreamedContentHashes,
) -> Result<(FileVerificationOutcome, Option<String>), String> {
    let mut digest = crc_fast::Digest::new(crc_algorithm_for(hashes.crc_algorithm));
    let mut read_bytes: u64 = 0;
    let mut buffer = vec![0u8; READ_BACK_CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("destination read-back failed: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        read_bytes = read_bytes.saturating_add(read as u64);
    }

    if read_bytes != hashes.size_bytes {
        return Ok((
            FileVerificationOutcome::Mismatch,
            Some(format!(
                "destination is {read_bytes} bytes; {} bytes were copied",
                hashes.size_bytes
            )),
        ));
    }

    let read_back_crc = digest.finalize();
    if read_back_crc != hashes.move_crc {
        return Ok((
            FileVerificationOutcome::Mismatch,
            Some(format!(
                "destination {} read-back is {read_back_crc:#018x}; the copy streamed {:#018x}",
                hashes.crc_algorithm.as_str(),
                hashes.move_crc
            )),
        ));
    }

    Ok((FileVerificationOutcome::Verified, None))
}

fn bypass_note(bypass: CacheBypass) -> Option<String> {
    match bypass {
        CacheBypass::Engaged => None,
        CacheBypass::NotAvailableOnPlatform => Some(
            "the destination read-back was not cache-bypassed: this platform offers no bypass mechanism"
                .to_string(),
        ),
        CacheBypass::Rejected(reason) => Some(format!(
            "the destination read-back was not cache-bypassed: {reason}"
        )),
    }
}

/// Opens the destination with the platform's page-cache bypass engaged.
///
/// macOS: `F_NOCACHE` on the descriptor, which keeps this read out of the
/// unified buffer cache. Pages already resident from the write may still be
/// served, so the bypass is a strong hint rather than a guarantee of a device
/// read.
///
/// Linux: `POSIX_FADV_DONTNEED` over the whole file, issued after the copy's
/// fsync so the clean pages can actually be dropped and the read-back has to go
/// back to the device. `O_DIRECT` is deliberately not used: it demands
/// block-aligned buffers and offsets and is unsupported on several filesystems
/// media libraries live on (tmpfs, some network mounts), which would turn most
/// full verifications into quick fallbacks.
///
/// Everywhere else: no mechanism is wired, so the read-back runs uncached and
/// says so — FR-042 asks for bypass "where the platform allows".
pub fn open_cache_bypassed(path: &Path) -> ReadBackHandle {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return ReadBackHandle::Unavailable(format!(
                "could not open the destination for verification: {}: {error}",
                path.display()
            ));
        }
    };

    engage_cache_bypass(file)
}

#[cfg(target_os = "macos")]
fn engage_cache_bypass(file: File) -> ReadBackHandle {
    use std::os::unix::io::AsRawFd;

    // SAFETY: `file` owns the descriptor for the duration of the call.
    let code = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) };
    if code == -1 {
        return ReadBackHandle::Ready {
            file,
            bypass: CacheBypass::Rejected(format!(
                "F_NOCACHE was rejected for this file: {}",
                std::io::Error::last_os_error()
            )),
        };
    }
    ReadBackHandle::Ready {
        file,
        bypass: CacheBypass::Engaged,
    }
}

#[cfg(target_os = "linux")]
fn engage_cache_bypass(file: File) -> ReadBackHandle {
    use std::os::unix::io::AsRawFd;

    // SAFETY: `file` owns the descriptor for the duration of the call. A length
    // of 0 means "to end of file".
    let code = unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
    // posix_fadvise reports errors by return value, not errno.
    if code != 0 {
        return ReadBackHandle::Ready {
            file,
            bypass: CacheBypass::Rejected(format!(
                "POSIX_FADV_DONTNEED was rejected for this file: {}",
                std::io::Error::from_raw_os_error(code)
            )),
        };
    }
    ReadBackHandle::Ready {
        file,
        bypass: CacheBypass::Engaged,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn engage_cache_bypass(file: File) -> ReadBackHandle {
    ReadBackHandle::Ready {
        file,
        bypass: CacheBypass::NotAvailableOnPlatform,
    }
}

// ── Depth: quick, and the floor ──────────────────────────────────────────────

/// The FR-042 floor: whatever stopped the full read-back, the file still gets
/// the quick check, stamped as a fallback with the reason.
fn quick_floor_fallback(
    source: &Path,
    destination: &Path,
    hashes: &StreamedContentHashes,
    reason: &str,
) -> VerificationAssessment {
    let (outcome, detail) = quick_check(source, destination, hashes);
    VerificationAssessment {
        depth: AppliedVerificationDepth::quick_fallback(),
        outcome,
        detail: join_details(
            Some(format!(
                "full verification fell back to the quick check: {reason}"
            )),
            detail,
        ),
    }
}

/// Size compare against the streamed size, then the sampled head+tail proof of
/// the destination against the source's.
///
/// The sampling is [`crate::fs_integrity`]'s, unchanged: same windows, same
/// digest, so scan signatures and move verification can never disagree about
/// what "the same file" means. Its blind spot travels with it — content between
/// the two windows is not covered, which is exactly why full is the default.
fn quick_check(
    source: &Path,
    destination: &Path,
    hashes: &StreamedContentHashes,
) -> (FileVerificationOutcome, Option<String>) {
    let destination_size = match std::fs::metadata(destination) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return (
                FileVerificationOutcome::Unavailable,
                Some(format!(
                    "could not stat the destination: {}: {error}",
                    destination.display()
                )),
            );
        }
    };
    if destination_size != hashes.size_bytes {
        return (
            FileVerificationOutcome::Mismatch,
            Some(format!(
                "destination is {destination_size} bytes; {} bytes were copied",
                hashes.size_bytes
            )),
        );
    }

    let destination_proof = match crate::fs_integrity::import_content_proof(destination) {
        Ok(proof) => proof,
        Err(error) => {
            return (
                FileVerificationOutcome::Unavailable,
                Some(format!("could not read the destination: {error}")),
            );
        }
    };
    // The source is still present: FR-044 forbids touching it until this passes.
    // If it cannot be read, the comparison did not run, so nothing is proven.
    let source_proof = match crate::fs_integrity::import_content_proof(source) {
        Ok(proof) => proof,
        Err(error) => {
            return (
                FileVerificationOutcome::Unavailable,
                Some(format!(
                    "could not re-read the source for the quick check: {error}"
                )),
            );
        }
    };

    match crate::fs_integrity::verify_content_proofs(
        &source.display().to_string(),
        &source_proof,
        &destination.display().to_string(),
        &destination_proof,
    ) {
        Ok(()) => (FileVerificationOutcome::Verified, None),
        Err(error) => (FileVerificationOutcome::Mismatch, Some(error.to_string())),
    }
}

fn join_details(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

// ── The rename fast path (FR-032) ────────────────────────────────────────────

/// Whether `source` and the volume `destination` will land on are the same
/// filesystem, i.e. whether the move can be a rename that needs no verification
/// pass at all (FR-032).
///
/// This is a pre-flight probe, complementary to
/// [`crate::fs_safety::is_cross_device_error`], which classifies an `EXDEV` a
/// rename already returned. Both directions fail safe: an unknown answer is
/// `false`, and a `false` only ever costs a copy that is verified.
///
/// The destination usually does not exist yet, so the nearest existing ancestor
/// directory is what gets compared.
pub async fn same_filesystem(source: &Path, destination: &Path) -> bool {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    tokio::task::spawn_blocking(move || same_filesystem_blocking(&source, &destination))
        .await
        .unwrap_or(false)
}

#[cfg(unix)]
fn same_filesystem_blocking(source: &Path, destination: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    // `symlink_metadata`: a symlinked source lives on the filesystem holding the
    // link, not the one holding its target.
    let Ok(source_metadata) = std::fs::symlink_metadata(source) else {
        return false;
    };
    let Some(anchor) = nearest_existing_ancestor(destination) else {
        return false;
    };
    let Ok(destination_metadata) = std::fs::metadata(anchor) else {
        return false;
    };
    source_metadata.dev() == destination_metadata.dev()
}

#[cfg(not(unix))]
fn same_filesystem_blocking(_source: &Path, _destination: &Path) -> bool {
    // No cheap device probe here; the copy path verifies, so answering "no" only
    // costs work, never safety.
    false
}

#[cfg(unix)]
fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        if current.as_os_str().is_empty() {
            return None;
        }
        if std::fs::symlink_metadata(current).is_ok() {
            return Some(current);
        }
        candidate = current.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The streamed pair must match the one-shot values for the same bytes, and
    /// the CRC must be the algorithm the persisted tag claims.
    #[test]
    fn streamed_hashes_match_one_shot_values() {
        let data: Vec<u8> = (0..(3 * 1024 * 1024u32 + 7))
            .map(|index| (index % 251) as u8)
            .collect();

        let mut hasher = StreamedContentHasher::new();
        for chunk in data.chunks(64 * 1024) {
            hasher.update(chunk);
        }
        let hashes = hasher.finalize();

        assert_eq!(hashes.size_bytes, data.len() as u64);
        assert_eq!(hashes.crc_algorithm, MoveCrcAlgorithm::Crc64Nvme);
        assert_eq!(
            hashes.move_crc,
            crc_fast::checksum(MOVE_CRC_ALGORITHM, &data)
        );
        assert_eq!(hashes.full_blake3, blake3::hash(&data).to_hex().to_string());
    }

    /// A single flipped bit must change both the CRC and the BLAKE3 — the whole
    /// point of the streamed pair (FR-040, SC-006).
    #[test]
    fn streamed_hashes_detect_a_flipped_bit() {
        let clean: Vec<u8> = (0..1024 * 1024u32)
            .map(|index| (index % 251) as u8)
            .collect();
        let mut corrupted = clean.clone();
        corrupted[512 * 1024] ^= 0b0000_0001;

        let mut clean_hasher = StreamedContentHasher::new();
        clean_hasher.update(&clean);
        let clean_hashes = clean_hasher.finalize();

        let mut corrupted_hasher = StreamedContentHasher::new();
        corrupted_hasher.update(&corrupted);
        let corrupted_hashes = corrupted_hasher.finalize();

        assert_eq!(clean_hashes.size_bytes, corrupted_hashes.size_bytes);
        assert_ne!(clean_hashes.move_crc, corrupted_hashes.move_crc);
        assert_ne!(clean_hashes.full_blake3, corrupted_hashes.full_blake3);
    }

    /// The persisted algorithm tag round-trips through its setting form.
    #[test]
    fn move_crc_algorithm_tag_round_trips() {
        assert_eq!(MOVE_CRC_ALGORITHM_TAG.as_str(), "crc64_nvme");
        assert_eq!(
            MoveCrcAlgorithm::from_setting("crc64_nvme"),
            Ok(MOVE_CRC_ALGORITHM_TAG)
        );
        assert!(MoveCrcAlgorithm::from_setting("crc32_iscsi").is_err());
        assert_eq!(
            crc_algorithm_for(MOVE_CRC_ALGORITHM_TAG),
            MOVE_CRC_ALGORITHM
        );
    }

    // ── Verified copy ────────────────────────────────────────────────────────

    /// Bigger than two sample windows, so the middle of the file is outside what
    /// the quick check ever looks at.
    const THREE_WINDOWS: usize = crate::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES * 3;

    fn write_pattern(path: &Path, len: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..len).map(|index| (index % 251) as u8).collect();
        std::fs::write(path, &data).expect("write test file");
        data
    }

    /// Flips one byte in place, leaving the size untouched: the corruption a
    /// full read-back exists to catch.
    fn flip_byte(path: &Path, offset: usize) {
        let mut bytes = std::fs::read(path).expect("read for corruption");
        bytes[offset] ^= 0b0000_0001;
        std::fs::write(path, bytes).expect("write corruption");
    }

    fn request(source: &Path, destination: &Path, depth: VerificationDepth) -> VerifiedCopyRequest {
        VerifiedCopyRequest {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            depth,
            claim: DestinationClaim::ClaimHere,
            progress: CopyProgress::none(),
        }
    }

    /// A clean copy verifies at full depth, carries both hashes, and produces
    /// the same CRC every time it is run.
    #[tokio::test]
    async fn full_verification_round_trips_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let data = write_pattern(&source, THREE_WINDOWS + 321);

        let copier = VerifiedCopier::new();
        let first = copier
            .copy_and_verify(request(
                &source,
                &dir.path().join("first.bin"),
                VerificationDepth::Full,
            ))
            .await
            .expect("copy and verify");

        assert_eq!(first.outcome, FileVerificationOutcome::Verified);
        assert!(first.permits_source_removal());
        assert_eq!(
            first.depth,
            AppliedVerificationDepth::exact(VerificationDepth::Full)
        );
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert_eq!(
            first.detail, None,
            "a clean cache-bypassed read-back has nothing to explain"
        );

        let hashes = first.hashes.clone().expect("copies produce hashes");
        assert_eq!(hashes.size_bytes, data.len() as u64);
        assert_eq!(hashes.crc_algorithm, MOVE_CRC_ALGORITHM_TAG);
        assert_eq!(
            hashes.move_crc,
            crc_fast::checksum(MOVE_CRC_ALGORITHM, &data)
        );
        assert_eq!(hashes.full_blake3, blake3::hash(&data).to_hex().to_string());
        assert_eq!(
            std::fs::read(dir.path().join("first.bin")).unwrap(),
            data,
            "the destination must be byte-identical to the source"
        );

        let second = copier
            .copy_and_verify(request(
                &source,
                &dir.path().join("second.bin"),
                VerificationDepth::Full,
            ))
            .await
            .expect("copy and verify");
        assert_eq!(second.hashes.expect("hashes"), hashes);
    }

    /// SC-006: a byte flipped after the write is caught before the source may be
    /// touched.
    #[tokio::test]
    async fn full_verification_detects_a_flipped_byte() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");

        let copier = VerifiedCopier::new();
        let hashes = copier
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await
            .expect("copy");

        flip_byte(&destination, THREE_WINDOWS / 2);

        let assessment = copier
            .verify(&source, &destination, &hashes, VerificationDepth::Full)
            .await
            .expect("verify");

        assert_eq!(assessment.outcome, FileVerificationOutcome::Mismatch);
        assert!(
            !assessment.outcome.permits_source_removal(),
            "a mismatch must never unblock source removal (FR-044)"
        );
        assert!(
            assessment
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("read-back")),
            "the detail should name the failing comparison: {:?}",
            assessment.detail
        );
        assert!(
            destination.exists(),
            "a mismatched destination is kept for diagnosis"
        );
        assert!(source.exists(), "the source is never touched by verification");
    }

    /// The quick check covers the sampled head and tail windows.
    #[tokio::test]
    async fn quick_verification_detects_corruption_inside_the_sampled_windows() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let copier = VerifiedCopier::new();

        for (label, offset) in [("head", 7usize), ("tail", THREE_WINDOWS - 7)] {
            let destination = dir.path().join(format!("{label}.bin"));
            let hashes = copier
                .copy(&source, &destination, DestinationClaim::ClaimHere)
                .await
                .expect("copy");
            flip_byte(&destination, offset);

            let assessment = copier
                .verify(&source, &destination, &hashes, VerificationDepth::Quick)
                .await
                .expect("verify");

            assert_eq!(
                assessment.outcome,
                FileVerificationOutcome::Mismatch,
                "{label} corruption must fail the quick check"
            );
            assert_eq!(
                assessment.depth,
                AppliedVerificationDepth::exact(VerificationDepth::Quick)
            );
        }
    }

    /// The quick check's blind spot, asserted on purpose: it samples the first
    /// and last window only, so a same-size change between them is outside its
    /// guarantee (`fs_integrity` documents the same limit for scans). This is
    /// exactly why full is the default depth — the same file fails at full
    /// depth. Quick's promise is "verified (quick)", and FR-043 makes that
    /// visible so the reduced guarantee is auditable, never silent.
    #[tokio::test]
    async fn quick_verification_does_not_cover_the_unsampled_middle() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");

        let copier = VerifiedCopier::new();
        let hashes = copier
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await
            .expect("copy");
        flip_byte(
            &destination,
            crate::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 17,
        );

        let quick = copier
            .verify(&source, &destination, &hashes, VerificationDepth::Quick)
            .await
            .expect("verify");
        assert_eq!(
            quick.outcome,
            FileVerificationOutcome::Verified,
            "the quick check intentionally does not read the middle of the file"
        );

        let full = copier
            .verify(&source, &destination, &hashes, VerificationDepth::Full)
            .await
            .expect("verify");
        assert_eq!(
            full.outcome,
            FileVerificationOutcome::Mismatch,
            "full depth is what covers the whole file"
        );
    }

    /// FR-042 floor: when the full read-back cannot run, the file still gets the
    /// quick check, stamped as a fallback with the reason.
    #[tokio::test]
    async fn a_read_back_that_cannot_run_falls_back_to_the_quick_floor() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");

        let copier = VerifiedCopier::with_read_back_opener(Arc::new(|_path: &Path| {
            ReadBackHandle::Unsupported("cache bypass is unavailable on this filesystem".to_string())
        }));
        let hashes = copier
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await
            .expect("copy");

        let assessment = copier
            .verify(&source, &destination, &hashes, VerificationDepth::Full)
            .await
            .expect("verify");

        assert_eq!(assessment.outcome, FileVerificationOutcome::Verified);
        assert_eq!(assessment.depth, AppliedVerificationDepth::quick_fallback());
        assert_eq!(assessment.depth.requested, VerificationDepth::Full);
        assert_eq!(assessment.depth.applied, VerificationDepth::Quick);
        assert!(assessment.depth.fell_back);
        let detail = assessment.detail.expect("a fallback must explain itself");
        assert!(
            detail.contains("cache bypass is unavailable on this filesystem"),
            "the detail must carry the reason: {detail}"
        );
    }

    /// A fallback does not become a pass: the quick floor still judges content.
    #[tokio::test]
    async fn a_fallback_still_fails_a_corrupted_destination() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");

        let copier = VerifiedCopier::with_read_back_opener(Arc::new(|_path: &Path| {
            ReadBackHandle::Unsupported("injected read-back failure".to_string())
        }));
        let hashes = copier
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await
            .expect("copy");
        flip_byte(&destination, 3);

        let assessment = copier
            .verify(&source, &destination, &hashes, VerificationDepth::Full)
            .await
            .expect("verify");

        assert_eq!(assessment.outcome, FileVerificationOutcome::Mismatch);
        assert_eq!(assessment.depth, AppliedVerificationDepth::quick_fallback());
    }

    /// A destination that vanished is unavailable, not verified — at either
    /// depth.
    #[tokio::test]
    async fn a_missing_destination_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, 4096);
        let destination = dir.path().join("destination.bin");

        let copier = VerifiedCopier::new();
        let hashes = copier
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await
            .expect("copy");
        std::fs::remove_file(&destination).unwrap();

        for depth in [VerificationDepth::Full, VerificationDepth::Quick] {
            let assessment = copier
                .verify(&source, &destination, &hashes, depth)
                .await
                .expect("verify");
            assert_eq!(
                assessment.outcome,
                FileVerificationOutcome::Unavailable,
                "{depth:?} verification of a missing destination must not pass"
            );
            assert!(!assessment.outcome.permits_source_removal());
            assert!(assessment.detail.is_some());
        }
    }

    /// A truncated destination is a mismatch at both depths, even though the
    /// bytes it does hold are correct.
    #[tokio::test]
    async fn a_size_mismatch_fails_at_both_depths() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let data = write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");

        let copier = VerifiedCopier::new();
        let hashes = copier
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await
            .expect("copy");
        std::fs::write(&destination, &data[..data.len() - 4096]).unwrap();

        for depth in [VerificationDepth::Full, VerificationDepth::Quick] {
            let assessment = copier
                .verify(&source, &destination, &hashes, depth)
                .await
                .expect("verify");
            assert_eq!(
                assessment.outcome,
                FileVerificationOutcome::Mismatch,
                "{depth:?} verification must reject a truncated destination"
            );
            assert!(
                assessment
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("bytes")),
                "{depth:?} detail should report the sizes: {:?}",
                assessment.detail
            );
        }
    }

    /// FR-033's crash rule, from the writing side: while the copy is running,
    /// the destination's real name does not exist at all — the bytes are going
    /// to the partial. A crash at any point during the copy therefore leaves
    /// either nothing or a complete file under the destination name, never a
    /// truncated one.
    #[tokio::test]
    async fn a_copy_writes_through_a_partial_and_never_a_truncated_destination() {
        use std::sync::Mutex;

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let data = write_pattern(&source, COPY_CHUNK_BYTES * 2 + 4096);
        let destination = dir.path().join("destination.bin");
        let partial = partial_destination_path(&destination);

        let observed: Arc<Mutex<Vec<(bool, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let progress = CopyProgress::from_fn({
            let observed = observed.clone();
            let destination = destination.clone();
            let partial = partial.clone();
            move |_bytes| {
                observed
                    .lock()
                    .expect("lock")
                    .push((destination.exists(), partial.exists()));
            }
        });

        let hashes = VerifiedCopier::new()
            .copy_with_progress(
                &source,
                &destination,
                DestinationClaim::ClaimHere,
                &progress,
            )
            .await
            .expect("copy");

        let observed = observed.lock().expect("lock").clone();
        assert!(
            observed.len() >= 3,
            "a three-chunk copy should report three times: {observed:?}"
        );
        for (destination_exists, partial_exists) in &observed {
            assert!(
                !destination_exists,
                "the destination name must not exist while the copy is in flight"
            );
            assert!(partial_exists, "the bytes go to the partial");
        }

        assert_eq!(std::fs::read(&destination).unwrap(), data);
        assert!(
            !partial.exists(),
            "the partial is consumed by the promotion"
        );
        assert_eq!(hashes.size_bytes, data.len() as u64);
    }

    /// A partial an interrupted attempt abandoned is this operation's own work,
    /// so the next attempt clears it and copies cleanly rather than failing on
    /// a name it cannot claim.
    #[tokio::test]
    async fn an_abandoned_partial_is_cleared_before_the_next_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let data = write_pattern(&source, 8192);
        let destination = dir.path().join("destination.bin");
        std::fs::write(
            partial_destination_path(&destination),
            b"the first few bytes a crashed attempt managed to write",
        )
        .unwrap();

        let verified = VerifiedCopier::new()
            .copy_and_verify(request(&source, &destination, VerificationDepth::Full))
            .await
            .expect("copy and verify");

        assert_eq!(verified.outcome, FileVerificationOutcome::Verified);
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            data,
            "the destination holds the whole source, not the abandoned prefix"
        );
        assert!(!partial_destination_path(&destination).exists());
    }

    /// The window between a completed copy's promotion and the verification
    /// record that should have followed it: the destination is already there
    /// and proves against the source, so it is recorded rather than copied a
    /// second time.
    #[tokio::test]
    async fn an_unrecorded_destination_that_matches_the_source_is_proven_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let data = write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");
        std::fs::copy(&source, &destination).expect("stage the interrupted run's destination");

        let verified = VerifiedCopier::new()
            .verify_existing_destination(&source, &destination, VerificationDepth::Full)
            .await
            .expect("verify in place");

        assert_eq!(verified.outcome, FileVerificationOutcome::Verified);
        assert!(verified.permits_source_removal());
        let hashes = verified.hashes.expect("the source is hashed to prove it");
        assert_eq!(hashes.full_blake3, blake3::hash(&data).to_hex().to_string());
        assert!(
            verified
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("destination.bin")),
            "the record says which file was proven in place: {:?}",
            verified.detail
        );
    }

    /// The same window, with a file that is *not* this operation's finished
    /// work: it is reported as a mismatch naming the path, never removed and
    /// never assumed good (C4).
    #[tokio::test]
    async fn an_unrecorded_destination_that_does_not_match_is_a_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");
        std::fs::write(&destination, b"somebody else's file").unwrap();

        let verified = VerifiedCopier::new()
            .verify_existing_destination(&source, &destination, VerificationDepth::Full)
            .await
            .expect("verify in place");

        assert_eq!(verified.outcome, FileVerificationOutcome::Mismatch);
        assert!(!verified.permits_source_removal());
        assert!(
            verified
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("destination.bin")),
            "{:?}",
            verified.detail
        );
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"somebody else's file",
            "a destination that did not prove is kept exactly as it is"
        );
    }

    /// The copy claims the destination name rather than replacing whatever holds
    /// it.
    #[tokio::test]
    async fn claiming_a_taken_destination_fails_instead_of_replacing_it() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, 2048);
        let destination = dir.path().join("destination.bin");
        std::fs::write(&destination, b"someone else's file").unwrap();

        let result = VerifiedCopier::new()
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await;

        assert!(result.is_err(), "a taken destination must not be written");
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"someone else's file",
            "the existing file must be untouched"
        );
        assert!(
            !partial_destination_path(&destination).exists(),
            "the copy that could not be placed does not leave its partial behind"
        );
    }

    /// A destination the caller already claimed is written without being created
    /// again.
    #[tokio::test]
    async fn an_already_held_destination_is_written_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let data = write_pattern(&source, 8192);
        let destination = dir.path().join("claimed.bin");
        // What `fs_safety`'s claim leaves behind: an empty file holding the name.
        std::fs::write(&destination, b"").unwrap();

        let verified = VerifiedCopier::new()
            .copy_and_verify(VerifiedCopyRequest {
                source: source.clone(),
                destination: destination.clone(),
                depth: VerificationDepth::Full,
                claim: DestinationClaim::AlreadyHeld,
                progress: CopyProgress::none(),
            })
            .await
            .expect("copy and verify");

        assert_eq!(verified.outcome, FileVerificationOutcome::Verified);
        assert_eq!(std::fs::read(&destination).unwrap(), data);
    }

    /// FR-032: a same-filesystem rename copies nothing, so its record carries no
    /// hashes — and still unblocks the executor's cleanup.
    #[test]
    fn a_same_filesystem_rename_records_no_hashes() {
        let verified = VerifiedFile::same_filesystem_rename(
            Path::new("/library/old/show.mkv"),
            Path::new("/library/new/show.mkv"),
            VerificationDepth::Full,
        );

        assert!(verified.hashes.is_none());
        assert!(verified.permits_source_removal());

        let record = verified.into_record(
            FileVerificationIdentity {
                operation_id: "op-1",
                title_id: "title-1",
                media_file_id: Some("file-1"),
            },
            Utc::now(),
        );
        assert_eq!(record.operation_id, "op-1");
        assert_eq!(record.media_file_id.as_deref(), Some("file-1"));
        assert!(record.hashes.is_none());
        assert_eq!(record.outcome, FileVerificationOutcome::Verified);
    }

    /// The verified copy hands the executor a complete record.
    #[tokio::test]
    async fn a_verified_copy_becomes_a_persisted_record() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, 4096);
        let destination = dir.path().join("destination.bin");

        let verified = VerifiedCopier::new()
            .copy_and_verify(request(&source, &destination, VerificationDepth::Full))
            .await
            .expect("copy and verify");

        let record = verified.into_record(
            FileVerificationIdentity {
                operation_id: "op-2",
                title_id: "title-2",
                media_file_id: None,
            },
            Utc::now(),
        );

        assert_eq!(record.outcome, FileVerificationOutcome::Verified);
        assert_eq!(record.media_file_id, None);
        assert_eq!(
            record.destination_path,
            crate::stored_paths::path_to_stored_string(&destination)
        );
        let hashes = record.hashes.expect("a copy records its hashes");
        assert_eq!(hashes.size_bytes, 4096);
        assert!(!hashes.full_blake3.is_empty());
    }

    /// The FR-032 pre-flight: two paths under one temp directory are on one
    /// filesystem, and a source that is not there answers "no" rather than
    /// guessing.
    #[tokio::test]
    async fn same_filesystem_probe_answers_for_a_destination_that_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, 16);
        let destination = dir.path().join("nested/not-created-yet/destination.bin");

        #[cfg(unix)]
        assert!(same_filesystem(&source, &destination).await);

        assert!(
            !same_filesystem(&dir.path().join("missing.bin"), &destination).await,
            "an unknown answer must fail towards copy + verification"
        );
    }

    /// The real opener engages the platform's bypass on a plain local file.
    /// Isolated from the round-trip test so a platform surprise points straight
    /// at the mechanism.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn the_platform_opener_engages_its_cache_bypass() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.bin");
        std::fs::write(&path, vec![1u8; 4096]).unwrap();

        match open_cache_bypassed(&path) {
            ReadBackHandle::Ready { bypass, .. } => assert_eq!(bypass, CacheBypass::Engaged),
            ReadBackHandle::Unsupported(reason) => {
                panic!("expected a cache-bypassed read-back, got: {reason}")
            }
            ReadBackHandle::Unavailable(reason) => panic!("expected an open file, got: {reason}"),
        }
    }

    /// A destination that cannot be opened is reported as unavailable by the
    /// real opener, not as a fallback.
    #[test]
    fn the_platform_opener_reports_a_missing_file_as_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        match open_cache_bypassed(&dir.path().join("nope.bin")) {
            ReadBackHandle::Unavailable(_) => {}
            other => panic!(
                "expected unavailable, got {}",
                match other {
                    ReadBackHandle::Ready { .. } => "a ready handle",
                    ReadBackHandle::Unsupported(_) => "unsupported",
                    ReadBackHandle::Unavailable(_) => unreachable!(),
                }
            ),
        }
    }
}
