//! Full-hash backfill: the slow convergence job (FR-047, plan D9).
//!
//! Every other producer of a full BLAKE3 in this feature gets it for free —
//! [`crate::location::verify`] hashes bytes it was already copying. This job
//! exists for the catalog that was never copied: files that were scanned,
//! hardlinked, or imported before 0205 existed, and files a scan invalidated
//! (FR-046). It reads them for no other reason than to hash them, which is why
//! everything below is about *not* getting in the way.
//!
//! # Shape
//!
//! - **Single-threaded.** One file at a time, in id order. No fan-out, no
//!   per-title batching.
//! - **Throttled.** A pause between files and between read chunks
//!   ([`FullHashBackfillOptions`]), plus a bounded number of files per run so a
//!   sweep of a large catalog is many short jobs rather than one long one.
//! - **Resumable.** See the cursor note below.
//! - **Skips** unavailable mounts, files an active location operation owns, and
//!   files that already carry a current full hash.
//!
//! # The cursor (SC-007: interrupt and re-run without rework)
//!
//! The queue is `full_blake3 IS NULL` ordered by `id` — literally migration
//! 0205's `idx_media_files_full_hash_missing` partial index. Every file the job
//! successfully hashes *leaves* the queue, so re-running from the start would
//! already skip all completed work: that half of resumability is implicit in
//! the predicate and needs no state at all.
//!
//! What does need state is the *skipped* files. A file on an unplugged mount,
//! or one an operation owns, stays `NULL` forever; with a bounded run and no
//! cursor the job would re-examine the same stuck prefix every time and never
//! reach the rest of the catalog. So the job persists the id it stopped after
//! ([`FULL_HASH_BACKFILL_CURSOR_KEY`]) and resumes past it.
//!
//! When a page comes back empty the sweep has reached the end: the cursor is
//! cleared so the next run starts over. That re-examines the files that were
//! skipped, which is exactly what should happen — the mount may be back and the
//! operation may have finished. Convergence therefore needs no retry queue, no
//! failure counters, and no dead-letter state; it falls out of "sweep, wrap,
//! sweep again".
//!
//! A cursor pointing at a since-deleted id is harmless: `id > cursor` simply
//! resumes at the next surviving row.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::location::model::PersistedContentHashes;
use crate::location::ownership_guard::OwnedEntity;
use crate::location::verify::StreamedContentHasher;
use crate::settings::keys::FULL_HASH_BACKFILL_CURSOR_KEY;
use crate::stored_paths::stored_path_to_path_buf;
use crate::{AppError, AppResult, AppUseCase, MediaFileHashCandidate};

/// Hashing has its own cadence and lifetime; acquisition, RSS and pending
/// releases never await this worker.
pub async fn start_full_hash_backfill_worker(
    app: AppUseCase,
    shutdown: tokio_util::sync::CancellationToken,
) {
    let delay = Duration::from_secs(
        crate::JobKey::FullHashBackfill
            .initial_delay_seconds()
            .unwrap_or(900)
            .max(0) as u64,
    );
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + delay,
        Duration::from_secs(1800),
    );
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    app.set_job_next_run_at(
        crate::JobKey::FullHashBackfill,
        Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default(),
    )
    .await;
    loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            _ = interval.tick() => {},
        }
        app.set_job_next_run_at(
            crate::JobKey::FullHashBackfill,
            Utc::now() + chrono::Duration::minutes(30),
        )
        .await;
        let run_app = app.clone();
        let mut run = tokio::spawn(async move {
            run_app
                .run_scheduled_job_now(
                    crate::JobKey::FullHashBackfill,
                    crate::JobTriggerSource::ScheduledInterval,
                )
                .await
        });
        let outcome = tokio::select! {
            outcome = &mut run => outcome,
            _ = shutdown.cancelled() => {
                app.runtime.jobs.full_hash_shutdown.cancel();
                // Drain cooperatively; aborting a join handle cannot stop an OS read.
                run.await
            },
        };
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "periodic full-hash backfill failed"),
            Err(error) => tracing::error!(%error, "full-hash backfill task panicked"),
        }
    }
    app.runtime.jobs.full_hash_shutdown.cancel();
}

/// Rows fetched per queue page. Small: a page is a working set the job walks
/// one file at a time, not a batch it processes.
pub const FULL_HASH_BACKFILL_PAGE_SIZE: u32 = 32;

/// Files examined per run. A bound is what keeps this a *job* — a bounded,
/// reportable unit of work — instead of an open-ended background thread that
/// Activity can only ever show as "running".
pub const FULL_HASH_BACKFILL_FILES_PER_RUN: usize = 250;

/// Pause between files. The point is not the delay itself but that the runtime
/// gets a scheduling point between every two files, so a scan or an import
/// never queues behind this job's I/O.
pub const FULL_HASH_BACKFILL_FILE_PAUSE: Duration = Duration::from_millis(250);

/// Bytes read per hashing chunk.
const FULL_HASH_BACKFILL_CHUNK_BYTES: usize = 1024 * 1024;

/// Pause between read chunks, on the blocking thread doing the reading. This is
/// the "low I/O priority" mechanism: neither platform gives us a portable
/// ionice, so the job simply asks for the device less often. At 1 MiB chunks
/// this caps the job near 100 MiB/s of device time even on hardware that could
/// do far more, which is the intent.
const FULL_HASH_BACKFILL_CHUNK_PAUSE: Duration = Duration::from_millis(10);

/// Knobs the job runs with. Production uses [`Default`]; tests drive the pauses
/// to zero so a throttling assertion is about *ordering*, never wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullHashBackfillOptions {
    pub page_size: u32,
    pub files_per_run: usize,
    pub file_pause: Duration,
    pub chunk_pause: Duration,
}

impl Default for FullHashBackfillOptions {
    fn default() -> Self {
        Self {
            page_size: FULL_HASH_BACKFILL_PAGE_SIZE,
            files_per_run: FULL_HASH_BACKFILL_FILES_PER_RUN,
            file_pause: FULL_HASH_BACKFILL_FILE_PAUSE,
            chunk_pause: FULL_HASH_BACKFILL_CHUNK_PAUSE,
        }
    }
}

impl FullHashBackfillOptions {
    /// Same rules, no waiting. For tests only.
    #[cfg(test)]
    pub(crate) fn unthrottled() -> Self {
        Self {
            file_pause: Duration::ZERO,
            chunk_pause: Duration::ZERO,
            ..Self::default()
        }
    }
}

/// Persisted resume position.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FullHashBackfillCursor {
    /// The last media-file id this job examined. `None` means "start of the
    /// queue", which is also what a completed sweep writes back.
    #[serde(default)]
    pub after_id: Option<String>,
}

/// Why one candidate was passed over. Counted rather than logged per file: on a
/// catalog with an unplugged mount this is the common case, not the exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackfillSkip {
    /// The file, or the directory it should live in, is not readable — an
    /// unmounted share, a removed disk, a deleted file.
    UnavailableMount,
    /// A live location operation owns the title or the root (FR-084/SC-007).
    OwnedByOperation,
    /// Something else hashed it between the queue page and this file's turn.
    AlreadyHashed,
}

/// What one run did. Serialized into the job run's summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullHashBackfillSummary {
    /// Candidates taken off the queue.
    pub examined: usize,
    /// Files hashed and persisted.
    pub hashed: usize,
    /// Bytes read to produce those hashes.
    pub bytes_hashed: u64,
    pub skipped_unavailable: usize,
    pub skipped_owned: usize,
    pub skipped_already_hashed: usize,
    /// Files whose read failed partway. They keep their `NULL` hash and come
    /// back on the next sweep.
    pub failed: usize,
    /// True when this run reached the end of the queue, so the next one starts
    /// over from the beginning.
    pub completed_sweep: bool,
    /// Where the next run resumes.
    pub resume_after_id: Option<String>,
    #[serde(default)]
    pub cancelled: bool,
}

impl FullHashBackfillSummary {
    /// One line for the job run row.
    pub fn summary_text(&self) -> String {
        let skipped = self.skipped_unavailable + self.skipped_owned + self.skipped_already_hashed;
        let tail = if self.completed_sweep {
            "; reached the end of the queue"
        } else {
            ""
        };
        format!(
            "Hashed {} of {} file(s) ({} skipped, {} failed){tail}",
            self.hashed, self.examined, skipped, self.failed
        )
    }

    fn record_skip(&mut self, skip: BackfillSkip) {
        match skip {
            BackfillSkip::UnavailableMount => self.skipped_unavailable += 1,
            BackfillSkip::OwnedByOperation => self.skipped_owned += 1,
            BackfillSkip::AlreadyHashed => self.skipped_already_hashed += 1,
        }
    }
}

/// The entities an active operation holds, resolved once per run.
///
/// A `Title` claim is matched by id. A `Root` claim has no id in common with a
/// media-file row, so it is resolved to the root's path and matched by prefix:
/// a file under an owned root is owned even when its title is not individually
/// claimed. Resolution happens once, not per file.
#[derive(Debug, Default)]
struct OwnedScope {
    title_ids: HashSet<String>,
    root_paths: Vec<PathBuf>,
}

impl OwnedScope {
    fn is_empty(&self) -> bool {
        self.title_ids.is_empty() && self.root_paths.is_empty()
    }

    fn owns(&self, candidate: &MediaFileHashCandidate, path: &Path) -> bool {
        if self.title_ids.contains(&candidate.title_id) {
            return true;
        }
        self.root_paths.iter().any(|root| path.starts_with(root))
    }
}

impl AppUseCase {
    /// One bounded pass over the full-hash backfill queue (FR-047).
    ///
    /// Never returns an error for a file it could not read: an unreachable
    /// mount is a normal state for this job, not a job failure. Only a store or
    /// settings failure — something that makes the *queue* unreadable — stops
    /// the run.
    pub(crate) async fn run_full_hash_backfill_job(&self) -> AppResult<FullHashBackfillSummary> {
        self.run_full_hash_backfill_with_options(FullHashBackfillOptions::default())
            .await
    }

    pub(crate) async fn run_full_hash_backfill_with_options(
        &self,
        options: FullHashBackfillOptions,
    ) -> AppResult<FullHashBackfillSummary> {
        // The blocking reader retains a share of this permit if its awaiting
        // future is dropped. A replacement run cannot overlap the old I/O.
        let permit = std::sync::Arc::new(
            self.runtime
                .jobs
                .full_hash_execution_lock
                .clone()
                .try_lock_owned()
                .map_err(|_| {
                    AppError::Validation("Full-hash backfill is already running".into())
                })?,
        );
        let cancellation = self.runtime.jobs.full_hash_shutdown.child_token();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let owned = self.full_hash_backfill_owned_scope().await?;
        let mut cursor = self.read_full_hash_backfill_cursor().await;
        let mut summary = FullHashBackfillSummary::default();

        'sweep: while summary.examined < options.files_per_run {
            if cancellation.is_cancelled() {
                summary.cancelled = true;
                break;
            }
            let remaining = options.files_per_run - summary.examined;
            let page_size = options
                .page_size
                .min(u32::try_from(remaining).unwrap_or(u32::MAX));
            let page = self
                .services
                .library
                .media_files
                .list_media_files_missing_full_hash(cursor.as_deref(), page_size.max(1))
                .await?;

            if page.is_empty() {
                summary.completed_sweep = true;
                cursor = None;
                break;
            }

            for candidate in page {
                if cancellation.is_cancelled() {
                    summary.cancelled = true;
                    break 'sweep;
                }

                match self
                    .backfill_one_media_file(
                        &candidate,
                        &owned,
                        options,
                        &cancellation,
                        permit.clone(),
                    )
                    .await
                {
                    Ok(Some(bytes)) => {
                        summary.hashed += 1;
                        summary.bytes_hashed = summary.bytes_hashed.saturating_add(bytes);
                    }
                    Ok(None) => {}
                    Err(BackfillOutcome::Skipped(skip)) => summary.record_skip(skip),
                    Err(BackfillOutcome::Failed(reason)) => {
                        if cancellation.is_cancelled() {
                            summary.cancelled = true;
                            break 'sweep;
                        }
                        summary.failed += 1;
                        tracing::debug!(
                            media_file_id = %candidate.id,
                            path = %candidate.file_path,
                            reason = %reason,
                            "full-hash backfill could not hash a file; leaving it queued"
                        );
                    }
                }
                // Never advance past an interrupted file. Persist completed
                // progress before yielding or starting another disk read.
                cursor = Some(candidate.id.clone());
                summary.examined += 1;
                self.write_full_hash_backfill_cursor(cursor.clone()).await;

                // Between every two files, unconditionally. Real work gets a
                // scheduling point even when this run is hashing nothing but
                // skips.
                tokio::task::yield_now().await;
                if !options.file_pause.is_zero() {
                    tokio::select! {
                        _ = cancellation.cancelled() => {},
                        _ = tokio::time::sleep(options.file_pause) => {},
                    }
                }
            }
        }

        summary.resume_after_id = cursor.clone();
        self.write_full_hash_backfill_cursor(cursor).await;
        Ok(summary)
    }

    /// One file: check the skips, hash it, persist it. `Ok(Some(bytes))` when
    /// the file was hashed.
    async fn backfill_one_media_file(
        &self,
        candidate: &MediaFileHashCandidate,
        owned: &OwnedScope,
        options: FullHashBackfillOptions,
        cancellation: &tokio_util::sync::CancellationToken,
        permit: std::sync::Arc<tokio::sync::OwnedMutexGuard<()>>,
    ) -> Result<Option<u64>, BackfillOutcome> {
        let path = stored_path_to_path_buf(&candidate.file_path);

        if !owned.is_empty() && owned.owns(candidate, &path) {
            return Err(BackfillOutcome::Skipped(BackfillSkip::OwnedByOperation));
        }

        // Stat the parent before the file: an unmounted share answers "no such
        // directory" instantly, where opening the file itself can block on a
        // dead network mount.
        if let Some(parent) = path.parent()
            && tokio::fs::metadata(parent).await.is_err()
        {
            return Err(BackfillOutcome::Skipped(BackfillSkip::UnavailableMount));
        }
        let before = tokio::fs::metadata(&path)
            .await
            .map_err(|_| BackfillOutcome::Skipped(BackfillSkip::UnavailableMount))?;

        // The queue predicate already excluded hashed rows, but a copy or an
        // import can hash this file between the page query and its turn. One
        // indexed lookup is nothing against a multi-gigabyte read.
        let current = self
            .services
            .library
            .media_files
            .get_media_file_by_id(&candidate.id)
            .await
            .map_err(|error| BackfillOutcome::Failed(error.to_string()))?;
        let current = match current {
            None => return Err(BackfillOutcome::Skipped(BackfillSkip::AlreadyHashed)),
            Some(file) if file.content_hashes.is_some() => {
                return Err(BackfillOutcome::Skipped(BackfillSkip::AlreadyHashed));
            }
            Some(file) => file,
        };
        if current.file_path != candidate.file_path || current.size_bytes != candidate.size_bytes {
            return Err(BackfillOutcome::Failed(
                "file changed before hashing".into(),
            ));
        }

        let hashes = hash_file_cancellable(
            path.clone(),
            options.chunk_pause,
            cancellation.child_token(),
            Some(permit),
        )
        .await
        .map_err(|error| BackfillOutcome::Failed(error.to_string()))?;
        let bytes = hashes.size_bytes;
        let after = tokio::fs::metadata(&path)
            .await
            .map_err(|error| BackfillOutcome::Failed(error.to_string()))?;
        if !same_file_version(&before, &after) || bytes != current.size_bytes as u64 {
            return Err(BackfillOutcome::Failed("file changed while hashing".into()));
        }

        let published = self
            .services
            .library
            .media_files
            .update_media_file_content_hashes_if_unchanged(
                &current,
                &PersistedContentHashes::from_streamed(&hashes, Utc::now()),
            )
            .await
            .map_err(|error| BackfillOutcome::Failed(error.to_string()))?;
        if !published {
            return Err(BackfillOutcome::Failed(
                "catalog changed while hashing".into(),
            ));
        }

        Ok(Some(bytes))
    }

    /// Resolves every open ownership claim into the shape the per-file check
    /// needs. Runs once per job run.
    async fn full_hash_backfill_owned_scope(&self) -> AppResult<OwnedScope> {
        let claims = self.location_ownership_open_claims().await?;
        let mut scope = OwnedScope::default();
        if claims.is_empty() {
            return Ok(scope);
        }

        let mut owned_root_ids = HashSet::new();
        for claim in claims {
            match claim.entity {
                OwnedEntity::Title(title_id) => {
                    scope.title_ids.insert(title_id);
                }
                OwnedEntity::Root(root_id) => {
                    owned_root_ids.insert(root_id);
                }
            }
        }

        if !owned_root_ids.is_empty() {
            for library in self.services.catalog.libraries.list(None).await? {
                for root in &library.roots {
                    if owned_root_ids.contains(&root.id) {
                        scope.root_paths.push(stored_path_to_path_buf(&root.path));
                    }
                }
            }
        }

        Ok(scope)
    }

    /// A read failure here is not a reason to redo work: falling back to the
    /// start of the queue would re-examine already-hashed files, which the
    /// predicate then skips anyway.
    async fn read_full_hash_backfill_cursor(&self) -> Option<String> {
        match self
            .read_setting_json_value::<FullHashBackfillCursor>(FULL_HASH_BACKFILL_CURSOR_KEY, None)
            .await
        {
            Ok(cursor) => cursor.and_then(|cursor| cursor.after_id),
            Err(error) => {
                tracing::warn!(
                    setting = FULL_HASH_BACKFILL_CURSOR_KEY,
                    "failed to read the full-hash backfill cursor: {error}; starting the sweep over"
                );
                None
            }
        }
    }

    async fn write_full_hash_backfill_cursor(&self, after_id: Option<String>) {
        if let Err(error) = self
            .upsert_system_setting_json(
                FULL_HASH_BACKFILL_CURSOR_KEY,
                &FullHashBackfillCursor { after_id },
                None,
            )
            .await
        {
            tracing::warn!(
                setting = FULL_HASH_BACKFILL_CURSOR_KEY,
                "failed to persist the full-hash backfill cursor: {error}; the next run repeats this page"
            );
        }
    }
}

/// Why a candidate did not produce a hash. Skips are expected states; failures
/// are not, but neither stops the run.
enum BackfillOutcome {
    Skipped(BackfillSkip),
    Failed(String),
}

/// Reads a file end to end through the same hasher a verified copy uses (D2),
/// pausing between chunks so the job never monopolizes the device.
///
/// The CRC costs nothing extra next to the BLAKE3 this job exists to produce
/// (see [`crate::location::verify`]'s measurements), and having it persisted
/// means a later move can compare a destination read-back against a stored
/// value instead of re-reading the source.
pub(super) async fn hash_file_throttled(
    path: PathBuf,
    chunk_pause: Duration,
) -> AppResult<crate::location::model::StreamedContentHashes> {
    hash_file_cancellable(
        path,
        chunk_pause,
        tokio_util::sync::CancellationToken::new(),
        None,
    )
    .await
}

async fn hash_file_cancellable(
    path: PathBuf,
    chunk_pause: Duration,
    cancellation: tokio_util::sync::CancellationToken,
    permit: Option<std::sync::Arc<tokio::sync::OwnedMutexGuard<()>>>,
) -> AppResult<crate::location::model::StreamedContentHashes> {
    let _cancel_on_drop = cancellation.clone().drop_guard();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        use std::io::Read;

        if cancellation.is_cancelled() {
            return Err(AppError::Validation("Full-hash backfill cancelled".into()));
        }
        let mut file = std::fs::File::open(&path).map_err(|error| {
            AppError::Repository(format!(
                "failed to open {} for full-hash backfill: {error}",
                path.display()
            ))
        })?;
        let before = file
            .metadata()
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let mut hasher = StreamedContentHasher::new();
        let mut buffer = vec![0u8; FULL_HASH_BACKFILL_CHUNK_BYTES];
        loop {
            // Cancellation cannot interrupt an OS read already in progress.
            // Check again before every bounded read and before publishing.
            if cancellation.is_cancelled() {
                return Err(AppError::Validation("Full-hash backfill cancelled".into()));
            }
            let read = file.read(&mut buffer).map_err(|error| {
                AppError::Repository(format!(
                    "failed to read {} for full-hash backfill: {error}",
                    path.display()
                ))
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            if !chunk_pause.is_zero() {
                std::thread::sleep(chunk_pause);
            }
        }
        let after =
            std::fs::metadata(&path).map_err(|error| AppError::Repository(error.to_string()))?;
        if cancellation.is_cancelled() {
            return Err(AppError::Validation("Full-hash backfill cancelled".into()));
        }
        if !same_file_version(&before, &after) {
            return Err(AppError::Validation("file changed while hashing".into()));
        }
        Ok(hasher.finalize())
    })
    .await
    .map_err(|error| {
        AppError::Repository(format!("full-hash backfill task failed to join: {error}"))
    })?
}

pub(super) fn same_file_version(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || before.created().ok() != after.created().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return false;
        }
    }
    true
}
