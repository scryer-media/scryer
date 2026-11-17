//! Verified copies, as the application records them.
//!
//! The disk service copies and proves ([`crate::disk::Disk::place`] with a
//! recorded policy from [`crate::disk::placement`]). This module turns what
//! it reports into the forms the application persists: the streamed hash pair
//! ([`StreamedContentHashes`]), the requested and applied depth, and the
//! per-file verification outcome.
//!
//! Nothing here deletes, recycles, or rolls anything back. A mismatched
//! destination is kept exactly as written so it can be inspected, and the
//! source is never touched: source removal is the executor's step, gated on
//! [`FileVerificationOutcome::permits_source_removal`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};

use crate::disk::{
    ContentHasher, ContentHashes, CrcKind, Depth, Disk, Phase, Placement, Progress, Proof, Refused,
    Verdict, Watch,
};
use crate::location::model::{
    AppliedVerificationDepth, FileVerificationOutcome, FileVerificationRecord, MoveCrcAlgorithm,
    StreamedContentHashes, VerificationDepth,
};
use crate::{AppError, AppResult};

#[doc(hidden)]
pub use crate::disk::ReadBackHandle;
#[cfg(test)]
pub(crate) use crate::disk::{CacheBypass, ReadBackOpener, open_cache_bypassed};
pub use crate::disk::{
    FILE_NAME_MAX_BYTES, RESUMABLE_STAGING_SUFFIX as PARTIAL_COPY_SUFFIX,
    resumable_staging_path as partial_destination_path,
};

/// The persisted tag written alongside every CRC a copy streams.
pub const MOVE_CRC_ALGORITHM_TAG: MoveCrcAlgorithm = MoveCrcAlgorithm::Crc64Nvme;

/// Feeds every buffer to the disk service's hashers and produces the pair
/// persisted with the media file.
#[derive(Default)]
pub struct StreamedContentHasher(ContentHasher);

impl StreamedContentHasher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorb one buffer of copied bytes.
    pub fn update(&mut self, buffer: &[u8]) {
        self.0.update(buffer);
    }

    /// Bytes absorbed so far.
    pub fn size_bytes(&self) -> u64 {
        self.0.size_bytes()
    }

    pub fn finalize(self) -> StreamedContentHashes {
        persisted_hashes(self.0.finalize())
    }
}

/// The pair persisted with a media file, from what a copy streamed.
pub fn persisted_hashes(hashes: ContentHashes) -> StreamedContentHashes {
    StreamedContentHashes {
        size_bytes: hashes.size_bytes,
        crc_algorithm: match hashes.crc_kind {
            CrcKind::Crc64Nvme => MoveCrcAlgorithm::Crc64Nvme,
        },
        move_crc: hashes.crc,
        full_blake3: hashes.blake3,
    }
}

fn disk_hashes(hashes: &StreamedContentHashes) -> ContentHashes {
    ContentHashes {
        size_bytes: hashes.size_bytes,
        crc_kind: match hashes.crc_algorithm {
            MoveCrcAlgorithm::Crc64Nvme => CrcKind::Crc64Nvme,
        },
        crc: hashes.move_crc,
        blake3: hashes.full_blake3.clone(),
    }
}

/// The disk service's depth for a configured one.
pub(crate) fn disk_depth(depth: VerificationDepth) -> Depth {
    match depth {
        VerificationDepth::Quick => Depth::Quick,
        VerificationDepth::Full => Depth::Full,
    }
}

fn assessment(proof: Proof) -> VerificationAssessment {
    VerificationAssessment {
        depth: match (proof.fell_back, proof.asked) {
            (true, _) => AppliedVerificationDepth::quick_fallback(),
            (false, Depth::Full) => AppliedVerificationDepth::exact(VerificationDepth::Full),
            (false, _) => AppliedVerificationDepth::exact(VerificationDepth::Quick),
        },
        outcome: match proof.verdict {
            Verdict::Verified => FileVerificationOutcome::Verified,
            Verdict::Mismatch => FileVerificationOutcome::Mismatch,
            Verdict::Unavailable => FileVerificationOutcome::Unavailable,
        },
        detail: proof.detail,
    }
}

/// Who reserved the destination name of a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationClaim {
    /// The copy claims the destination name itself, failing if anything
    /// already holds it.
    ClaimHere,
    /// The caller already claimed or staged the destination.
    AlreadyHeld,
}

/// Observes bytes as they are written, so a caller can show intra-file progress
/// for a copy that runs for hours.
///
/// The sink is called on the blocking copy thread once per chunk and must do
/// nothing but bookkeeping — the runner accumulates into an atomic and does the
/// persisting on its own, throttled, schedule.
#[derive(Clone, Default)]
pub struct CopyProgress(
    Option<Arc<dyn Fn(u64) + Send + Sync>>,
    Option<Arc<dyn Fn(scryer_domain::ImportTransferPhase, u64) + Send + Sync>>,
    Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
);

impl CopyProgress {
    /// A copy nobody is watching.
    pub fn none() -> Self {
        Self(None, None, None)
    }

    pub fn from_fn(sink: impl Fn(u64) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(sink)), None, None)
    }

    pub fn with_comparison_sink(mut self, sink: impl Fn(u64, u64) + Send + Sync + 'static) -> Self {
        self.2 = Some(Arc::new(sink));
        self
    }

    pub fn comparison(&self, bytes: u64, total: u64) {
        if let Some(sink) = &self.2 {
            sink(bytes, total);
        } else {
            self.phase(scryer_domain::ImportTransferPhase::Verifying, bytes);
        }
    }

    pub fn with_transfer_sink(
        mut self,
        sink: impl Fn(scryer_domain::ImportTransferPhase, u64) + Send + Sync + 'static,
    ) -> Self {
        self.1 = Some(Arc::new(sink));
        self
    }

    /// Absolute bytes for this phase. This is transient telemetry, never a
    /// resumable CRC checkpoint.
    pub fn phase(&self, phase: scryer_domain::ImportTransferPhase, bytes: u64) {
        if let Some(sink) = &self.1 {
            sink(phase, bytes);
        }
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

    /// These sinks as the disk service reports to them: each phase as it
    /// starts, newly written bytes while copying, and the running total while
    /// verifying.
    pub(crate) fn disk_progress(&self) -> Progress {
        if self.0.is_none() && self.1.is_none() {
            return Progress::none();
        }
        let (sinks, written) = (self.clone(), AtomicU64::new(0));
        Progress::to(move |phase, bytes| match phase {
            Phase::Waiting => sinks.phase(scryer_domain::ImportTransferPhase::Waiting, 0),
            Phase::Copying if bytes == 0 => {
                written.store(0, Ordering::Relaxed);
                sinks.phase(scryer_domain::ImportTransferPhase::Copying, 0);
            }
            Phase::Copying => {
                sinks.advance(bytes.saturating_sub(written.swap(bytes, Ordering::Relaxed)));
            }
            Phase::Finishing => sinks.phase(scryer_domain::ImportTransferPhase::Finalizing, bytes),
            Phase::Verifying => sinks.phase(scryer_domain::ImportTransferPhase::Verifying, bytes),
        })
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
/// file, plus the source-removal gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFile {
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    /// Hashes streamed during the copy; `None` for a same-filesystem rename,
    /// which copies no bytes.
    pub hashes: Option<StreamedContentHashes>,
    pub depth: AppliedVerificationDepth,
    pub outcome: FileVerificationOutcome,
    pub detail: Option<String>,
}

impl VerifiedFile {
    /// Only a verified destination unblocks touching the source. Every source
    /// recycle/removal decision must read this, never the absence of an error.
    pub fn permits_source_removal(&self) -> bool {
        self.outcome.permits_source_removal()
    }

    /// A same-filesystem rename moved the file, so no bytes were copied and
    /// there is nothing to verify. Recorded so the operation's per-file
    /// history has no gaps.
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

    /// Attach the operation's identifiers and become the persisted row.
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

fn copy_refusal(refused: Refused, sentence: String) -> AppError {
    match refused {
        Refused::StagingOccupied => AppError::validation_refused(
            crate::error_reasons::location::DESTINATION_OCCUPIED,
            sentence,
        ),
        // Storage that timed out or could not be reached carries its own
        // variant and reason; every other refusal is a repository failure.
        Refused::Unreachable(_) => AppError::temporary_unavailable_reason(
            crate::disk::reason::DISK_PATH_UNREACHABLE,
            sentence,
            None,
        ),
        _ => AppError::Repository(sentence),
    }
}

/// Copy `source` onto `destination` as `policy`, a recorded-copy policy from
/// [`crate::disk::placement`], says, and report what the copy streamed.
///
/// A copy failure is an error. What the destination proved to be is in the
/// answer, not an error: a mismatched destination is a fact to record and
/// surface.
pub async fn recorded_copy(
    disk: &Disk,
    source: &Path,
    destination: &Path,
    policy: Placement,
    progress: &CopyProgress,
) -> AppResult<(StreamedContentHashes, Option<VerificationAssessment>)> {
    let watch = Watch {
        progress: progress.disk_progress(),
        cancel: crate::disk::Cancel::never(),
    };
    let placed = disk
        .place_watched(source, destination, policy, watch)
        .await
        .map_err(|refusal| copy_refusal(refusal.refused, refusal.to_string()))?;
    let copied = placed.copied.ok_or_else(|| {
        AppError::Repository(format!("the copy of {} recorded nothing", source.display()))
    })?;
    Ok((
        persisted_hashes(copied.hashes),
        copied.proof.map(assessment),
    ))
}

/// [`recorded_copy`] under a policy that proves the copy, as the file the
/// executor records.
pub async fn copy_and_verify(
    disk: &Disk,
    source: &Path,
    destination: &Path,
    policy: Placement,
    progress: &CopyProgress,
) -> AppResult<VerifiedFile> {
    let (hashes, proof) = recorded_copy(disk, source, destination, policy, progress).await?;
    let proof = proof.ok_or_else(|| {
        AppError::Repository(format!("the copy of {} was not proven", source.display()))
    })?;
    Ok(VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: Some(hashes),
        depth: proof.depth,
        outcome: proof.outcome,
        detail: proof.detail,
    })
}

/// Prove `destination` against the hashes streamed while it was written.
pub async fn verify(
    disk: &Disk,
    source: &Path,
    destination: &Path,
    hashes: &StreamedContentHashes,
    requested: VerificationDepth,
) -> AppResult<VerificationAssessment> {
    disk.prove_copy(
        source,
        destination,
        disk_hashes(hashes),
        disk_depth(requested),
        Progress::none(),
    )
    .await
    .map(assessment)
    .map_err(join_failure)
}

/// Blocking twin of [`verify`], for callers already on a blocking thread (the
/// download-client copy path).
pub fn verify_blocking(
    disk: &Disk,
    source: &Path,
    destination: &Path,
    hashes: &StreamedContentHashes,
    requested: VerificationDepth,
    progress: &CopyProgress,
) -> VerificationAssessment {
    assessment(disk.prove_copy_blocking(
        source,
        destination,
        &disk_hashes(hashes),
        disk_depth(requested),
        &progress.disk_progress(),
    ))
}

fn join_failure(said: String) -> AppError {
    AppError::Repository(format!("verified copy task failed to join: {said}"))
}

/// Read one existing file end to end and produce its content hashes.
pub async fn hash_existing_file(path: &Path) -> AppResult<StreamedContentHashes> {
    hash_existing_file_with_progress(path, CopyProgress::none()).await
}

pub async fn hash_existing_file_with_progress(
    path: &Path,
    progress: CopyProgress,
) -> AppResult<StreamedContentHashes> {
    Disk::process()
        .hash_file(path, progress.disk_progress())
        .await
        .map_err(join_failure)?
        .map(persisted_hashes)
        .map_err(|refusal| AppError::Repository(refusal.to_string()))
}

/// The quick check alone, against the hashes a copy already recorded: the
/// destination's size against the copied size, then the sampled head and tail
/// of both files against each other. Never reads either file end to end.
pub async fn quick_recheck(
    source: &Path,
    destination: &Path,
    hashes: &StreamedContentHashes,
) -> AppResult<FileVerificationOutcome> {
    let proof = Disk::process()
        .prove_copy(
            source,
            destination,
            disk_hashes(hashes),
            Depth::Quick,
            Progress::none(),
        )
        .await
        .map_err(join_failure)?;
    Ok(assessment(proof).outcome)
}

pub(crate) async fn sync_parent_directory_after_promotion(destination: &Path) {
    let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };
    if let Err(error) = Disk::process().sync_dir(parent).await {
        tracing::debug!(
            parent = %parent.display(),
            %error,
            "could not fsync the destination directory after a verified copy"
        );
    }
}

/// Whether a move from `source` to `destination` can be a rename that needs
/// no verification pass at all. An unknown answer is `false`, and a `false`
/// only ever costs a copy that is verified.
pub async fn same_filesystem(source: &Path, destination: &Path) -> bool {
    Disk::process().renames_between(source, destination).await
}

/// The recorded copy as the tests below drive it: one owner, and the policy
/// each step asks the resolver for.
#[cfg(test)]
pub(crate) struct VerifiedCopier(pub(crate) Disk);

#[cfg(test)]
pub(crate) struct VerifiedCopyRequest {
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) depth: VerificationDepth,
    pub(crate) claim: DestinationClaim,
    pub(crate) progress: CopyProgress,
}

#[cfg(test)]
impl VerifiedCopier {
    pub(crate) fn new() -> Self {
        Self(Disk::process())
    }

    pub(crate) fn with_read_back_opener(opener: ReadBackOpener) -> Self {
        Self(Disk::process().with_read_back_opener(opener))
    }

    pub(crate) async fn copy(
        &self,
        source: &Path,
        destination: &Path,
        claim: DestinationClaim,
    ) -> AppResult<StreamedContentHashes> {
        self.copy_with_progress(source, destination, claim, &CopyProgress::none())
            .await
    }

    pub(crate) async fn copy_with_progress(
        &self,
        source: &Path,
        destination: &Path,
        claim: DestinationClaim,
        progress: &CopyProgress,
    ) -> AppResult<StreamedContentHashes> {
        let policy = crate::disk::placement::unproven_copy(claim);
        recorded_copy(&self.0, source, destination, policy, progress)
            .await
            .map(|(hashes, _)| hashes)
    }

    pub(crate) async fn copy_and_verify(
        &self,
        request: VerifiedCopyRequest,
    ) -> AppResult<VerifiedFile> {
        let policy = crate::disk::placement::verified_copy(request.depth, request.claim);
        copy_and_verify(
            &self.0,
            &request.source,
            &request.destination,
            policy,
            &request.progress,
        )
        .await
    }

    pub(crate) async fn verify(
        &self,
        source: &Path,
        destination: &Path,
        hashes: &StreamedContentHashes,
        requested: VerificationDepth,
    ) -> AppResult<VerificationAssessment> {
        verify(&self.0, source, destination, hashes, requested).await
    }

    pub(crate) async fn verify_existing_destination(
        &self,
        source: &Path,
        destination: &Path,
        _requested: VerificationDepth,
        known: Option<&super::model::KnownSourceContent>,
    ) -> AppResult<VerifiedFile> {
        super::resolution::compare_existing(source, destination, &CopyProgress::none(), known).await
    }
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
            crc_turbo::checksum(crc_turbo::CrcAlgorithm::Crc64Nvme, &data)
        );
        assert_eq!(hashes.full_blake3, blake3::hash(&data).to_hex().to_string());
    }

    /// A single flipped bit must change both the CRC and the BLAKE3 — the whole
    /// point of the streamed pair.
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

        let mut sample = StreamedContentHasher::new();
        sample.update(b"synthetic payload");
        let stored = sample.finalize();
        assert_eq!(stored.crc_algorithm, MOVE_CRC_ALGORITHM_TAG);
        let on_disk = disk_hashes(&stored);
        assert_eq!(on_disk.crc_kind, CrcKind::Crc64Nvme);
        assert_eq!(persisted_hashes(on_disk), stored);
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
            crc_turbo::checksum(crc_turbo::CrcAlgorithm::Crc64Nvme, &data)
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

    /// A byte flipped after the write is caught before the source may be
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
        // The detail is the sentence the file table shows; the comparison
        // itself is logged, not surfaced.
        assert!(
            assessment
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("did not match the source")),
            "the detail should say the copy did not match: {:?}",
            assessment.detail
        );
        assert!(
            destination.exists(),
            "a mismatched destination is kept for diagnosis"
        );
        assert!(
            source.exists(),
            "the source is never touched by verification"
        );
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
    /// depth. Quick's promise is "verified (quick)", and the record makes that
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

    /// The floor: when the full read-back cannot run, the file still gets the
    /// quick check, stamped as a fallback with the reason.
    #[tokio::test]
    async fn a_read_back_that_cannot_run_falls_back_to_the_quick_floor() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");

        let copier = VerifiedCopier::with_read_back_opener(Arc::new(|_path: &Path| {
            ReadBackHandle::Unsupported(
                "cache bypass is unavailable on this filesystem".to_string(),
            )
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

    /// The crash rule, from the writing side: while the copy is running,
    /// the destination's real name does not exist at all — the bytes are going
    /// to the partial. A crash at any point during the copy therefore leaves
    /// either nothing or a complete file under the destination name, never a
    /// truncated one.
    #[tokio::test]
    async fn a_copy_writes_through_a_partial_and_never_a_truncated_destination() {
        use std::sync::Mutex;

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let data = write_pattern(&source, 1024 * 1024 * 2 + 4096);
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

    /// The copier cannot tell an interrupted attempt's own partial from a user
    /// file that happens to carry the suffix, so it removes neither: an
    /// occupied staging path fails the copy and leaves both files alone. The
    /// caller whose durable record *can* prove the partial is its own clears it
    /// first (`ConflictResolver`), which is what makes a resumed run copy
    /// cleanly — see `location::execution::tests`.
    #[tokio::test]
    async fn an_occupied_staging_path_fails_the_copy_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, 8192);
        let destination = dir.path().join("destination.bin");
        let partial = partial_destination_path(&destination);
        let occupant = b"a file that merely carries the staging suffix";
        std::fs::write(&partial, occupant).unwrap();

        let error = VerifiedCopier::new()
            .copy_and_verify(request(&source, &destination, VerificationDepth::Full))
            .await
            .expect_err("an occupied staging path is not the copier's to clear");

        assert!(
            error.to_string().contains("staging path"),
            "the reason names the occupied staging path: {error}"
        );
        assert_eq!(
            std::fs::read(&partial).unwrap(),
            occupant,
            "the occupant is preserved byte for byte"
        );
        assert!(!destination.exists(), "and nothing was placed");
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
            .verify_existing_destination(&source, &destination, VerificationDepth::Full, None)
            .await
            .expect("verify in place");

        assert_eq!(verified.outcome, FileVerificationOutcome::Verified);
        assert!(verified.permits_source_removal());
        let hashes = verified.hashes.expect("the source is hashed to prove it");
        assert_eq!(hashes.full_blake3, blake3::hash(&data).to_hex().to_string());
        // A file proven in place is not an exception, so it carries no detail;
        // the checkpoint names the path on its own.
        assert!(
            verified.detail.is_none(),
            "an in-place proof has nothing to explain: {:?}",
            verified.detail
        );
    }

    /// The same window, with a file that is *not* this operation's finished
    /// work: it is reported as a mismatch, never removed and never assumed
    /// good.
    #[tokio::test]
    async fn an_unrecorded_destination_that_does_not_match_is_a_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, THREE_WINDOWS);
        let destination = dir.path().join("destination.bin");
        std::fs::write(&destination, b"somebody else's file").unwrap();

        let verified = VerifiedCopier::new()
            .verify_existing_destination(&source, &destination, VerificationDepth::Full, None)
            .await
            .expect("verify in place");

        assert_eq!(verified.outcome, FileVerificationOutcome::Mismatch);
        assert!(!verified.permits_source_removal());
        assert!(
            verified.detail.as_deref().is_some_and(|detail| {
                detail.contains("different content") && detail.contains("preserved")
            }),
            "the detail says the destination differs and nothing was removed: {:?}",
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

    /// A same-filesystem rename copies nothing, so its record carries no
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

    /// A copy whose staged name cannot be read for a lasting reason, here a
    /// parent that is a file, is a repository failure with no reason code,
    /// not the temporarily-unavailable variant.
    #[tokio::test]
    async fn a_destination_under_a_file_is_not_reported_as_temporarily_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        write_pattern(&source, 16);
        let blocker = dir.path().join("not-a-folder");
        write_pattern(&blocker, 1);
        let error = recorded_copy(
            &Disk::process(),
            &source,
            &blocker.join("destination.bin"),
            crate::disk::placement::verified_copy(
                VerificationDepth::Quick,
                DestinationClaim::ClaimHere,
            ),
            &CopyProgress::none(),
        )
        .await
        .expect_err("refused");
        assert!(matches!(error, AppError::Repository(_)), "{error:?}");
        assert_eq!(std::fs::read(&blocker).unwrap().len(), 1);
    }

    /// Storage that did not answer is the one refusal reported as temporarily
    /// unavailable, with its reason, and the runner waits it out; a denied or
    /// failed read is not.
    #[test]
    fn only_unreachable_storage_is_reported_as_temporarily_unavailable() {
        let refused = |refused| copy_refusal(refused, "synthetic".to_owned());
        let unreachable = refused(Refused::Unreachable(crate::disk::Side::Destination));
        assert!(matches!(
            unreachable,
            AppError::TemporaryUnavailable {
                reason: Some(crate::disk::reason::DISK_PATH_UNREACHABLE),
                ..
            }
        ));
        assert!(crate::location::executor::move_error_is_transient(
            &unreachable
        ));
        for lasting in [Refused::Denied, Refused::Failed] {
            assert!(matches!(refused(lasting), AppError::Repository(_)));
        }
    }

    /// Two paths under one temp directory are on one filesystem, and a source
    /// that is not there answers "no" rather than guessing.
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

#[cfg(test)]
mod long_name_tests {
    use super::*;

    /// A destination name may be as long as the filesystem allows (255
    /// bytes); the in-progress copy must fit too.
    #[tokio::test]
    async fn copy_to_a_destination_near_the_name_limit_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"episode payload").unwrap();
        let destination = dir.path().join(format!("{}.mkv", "a".repeat(246)));
        assert_eq!(destination.file_name().unwrap().len(), 250);

        VerifiedCopier::new()
            .copy(&source, &destination, DestinationClaim::ClaimHere)
            .await
            .expect("a 250-byte destination name is legal and must copy");
        assert_eq!(std::fs::read(&destination).unwrap(), b"episode payload");
    }

    #[test]
    fn a_name_with_room_for_the_suffix_stages_beside_itself() {
        let destination = Path::new("/library/Show/Episode 07.mkv");

        assert_eq!(
            partial_destination_path(destination),
            Path::new("/library/Show/Episode 07.mkv.scryer-partial")
        );
    }

    #[test]
    fn a_long_name_stages_under_a_bounded_name_that_is_stable_and_distinct() {
        let shared = "é".repeat(120);
        let first = PathBuf::from(format!("/library/{shared} - 07.mkv"));
        let second = PathBuf::from(format!("/library/{shared} - 08.mkv"));

        let partial = partial_destination_path(&first);
        let name = partial.file_name().unwrap().to_str().unwrap();

        assert!(name.len() <= FILE_NAME_MAX_BYTES);
        assert!(name.ends_with(PARTIAL_COPY_SUFFIX));
        assert_eq!(partial.parent(), first.parent());
        assert_eq!(partial, partial_destination_path(&first));
        assert_ne!(partial, partial_destination_path(&second));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn native_partial_names_preserve_identity_for_short_and_long_opaque_names() {
        use crate::stored_paths::stored_path_to_path_buf;
        let (platform, markers) = if cfg!(windows) {
            ("w", ["%uD800", "%uD801"])
        } else {
            ("u", ["%FF", "%FE"])
        };
        for prefix in ["movie".to_string(), "m".repeat(240)] {
            let [first, second] = markers.map(|marker| {
                stored_path_to_path_buf(&format!(
                    "scryer-path-v1:{platform}:/library/{prefix}{marker}.mkv"
                ))
            });
            assert_eq!(first.to_string_lossy(), second.to_string_lossy());
            let partial = partial_destination_path(&first);
            assert_ne!(partial, partial_destination_path(&second));
            assert_eq!(partial, partial_destination_path(&first));
            assert_eq!(partial.parent(), first.parent());
            assert!(partial.file_name().unwrap().as_encoded_bytes().len() <= FILE_NAME_MAX_BYTES);
            if prefix.len() < 240 {
                let mut expected = first.as_os_str().to_os_string();
                expected.push(PARTIAL_COPY_SUFFIX);
                assert_eq!(partial, PathBuf::from(expected));
            }
        }
    }
}
