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
//! - **Bounded.** Sequential reads at background priority, with file and time
//!   limits checked between files. Image maintenance gets admission first.
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

use chrono::{DateTime, Local, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::location::model::PersistedContentHashes;
use crate::location::ownership_guard::OwnedEntity;
use crate::location::verify::StreamedContentHasher;
use crate::settings::keys::FULL_HASH_BACKFILL_CURSOR_KEY;
use crate::stored_paths::stored_path_to_path_buf;
use crate::{AppError, AppResult, AppUseCase, MediaFileHashCandidate};

/// Hashing has its own cadence and lifetime; acquisition, RSS and pending
/// releases never await this worker.
const FULL_HASH_BACKFILL_INTERVAL: Duration = Duration::from_secs(30 * 60);
const FULL_HASH_BACKFILL_WINDOW_START_HOUR: u32 = 2;
const FULL_HASH_BACKFILL_WINDOW_END_HOUR: u32 = 6;

pub(crate) fn full_hash_backfill_window_open<T: TimeZone>(now: &DateTime<T>) -> bool {
    (FULL_HASH_BACKFILL_WINDOW_START_HOUR..FULL_HASH_BACKFILL_WINDOW_END_HOUR).contains(&now.hour())
}

/// Resolves the next local 02:00 window independently so daylight-saving changes
/// never turn the overnight window into a fixed 24-hour delay.
pub(crate) fn next_full_hash_backfill_window<T: TimeZone>(
    now: &DateTime<T>,
    include_current: bool,
) -> Option<DateTime<Utc>> {
    if include_current && full_hash_backfill_window_open(now) {
        return Some(now.with_timezone(&Utc));
    }

    let zone = now.timezone();
    // Before 02:00 today's window is still ahead. Otherwise, resolve tomorrow's
    // opening rather than returning a minute inside the current window.
    let mut date = if now.hour() < FULL_HASH_BACKFILL_WINDOW_START_HOUR {
        now.date_naive()
    } else {
        now.date_naive().succ_opt()?
    };
    for _ in 0..3 {
        // If 02:00 is skipped locally, use the first valid minute in the window.
        for minute in
            FULL_HASH_BACKFILL_WINDOW_START_HOUR * 60..FULL_HASH_BACKFILL_WINDOW_END_HOUR * 60
        {
            let local = date.and_hms_opt(minute / 60, minute % 60, 0)?;
            if let Some(candidate) = zone.from_local_datetime(&local).earliest()
                && candidate > *now
            {
                return Some(candidate.with_timezone(&Utc));
            }
        }
        date = date.succ_opt()?;
    }
    None
}

pub(crate) fn next_full_hash_backfill_run<T: TimeZone>(now: &DateTime<T>) -> Option<DateTime<Utc>> {
    // Stay on local half-hour ticks instead of delaying from completion or
    // immediately catching up on a tick missed while the previous batch ran.
    let elapsed = chrono::Duration::seconds(
        i64::from(now.minute() * 60 + now.second()) % FULL_HASH_BACKFILL_INTERVAL.as_secs() as i64,
    ) + chrono::Duration::nanoseconds(i64::from(now.nanosecond()));
    let next_tick = now.with_timezone(&Utc)
        + chrono::Duration::from_std(FULL_HASH_BACKFILL_INTERVAL)
            .expect("full-hash backfill interval fits chrono duration")
        - elapsed;
    if full_hash_backfill_window_open(&next_tick.with_timezone(&now.timezone())) {
        Some(next_tick)
    } else {
        next_full_hash_backfill_window(now, false)
    }
}

pub async fn start_full_hash_backfill_worker(
    app: AppUseCase,
    shutdown: tokio_util::sync::CancellationToken,
) {
    let mut next_run =
        next_full_hash_backfill_window(&app.runtime.environment.now().with_timezone(&Local), true)
            .expect("a future full-hash backfill window must be available");
    loop {
        app.set_job_next_run_at(crate::JobKey::FullHashBackfill, next_run)
            .await;
        let wait = (next_run - app.runtime.environment.now())
            .to_std()
            .unwrap_or_default();
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(wait) => {},
        }

        let now = app.runtime.environment.now().with_timezone(&Local);
        if !full_hash_backfill_window_open(&now) {
            next_run = next_full_hash_backfill_window(&now, false)
                .expect("a future full-hash backfill window must be available");
            continue;
        }

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
        next_run =
            next_full_hash_backfill_run(&app.runtime.environment.now().with_timezone(&Local))
                .expect("a future full-hash backfill run must be available");
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

/// Bytes read per hashing chunk.
const FULL_HASH_BACKFILL_CHUNK_BYTES: usize = 1024 * 1024;

/// Invocation limits, checked between files without throttling sequential reads.
#[derive(Debug, Clone)]
pub struct FullHashBackfillOptions {
    pub page_size: u32,
    pub files_per_run: usize,
    pub max_duration: Duration,
    pub scheduled: bool,
    #[cfg(test)]
    pub after_file: Option<std::sync::Arc<tokio::sync::Semaphore>>,
}

impl Default for FullHashBackfillOptions {
    fn default() -> Self {
        Self {
            page_size: FULL_HASH_BACKFILL_PAGE_SIZE,
            files_per_run: FULL_HASH_BACKFILL_FILES_PER_RUN,
            max_duration: FULL_HASH_BACKFILL_INTERVAL,
            scheduled: false,
            #[cfg(test)]
            after_file: None,
        }
    }
}

impl FullHashBackfillOptions {
    /// Tests use the same sequential reader and limits as production.
    #[cfg(test)]
    pub(crate) fn unthrottled() -> Self {
        Self::default()
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

/// Stable, structured reason an invocation stopped admitting files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FullHashBackfillStopReason {
    ImageMaintenance,
    FileLimit,
    TimeLimit,
    WindowClosed,
    QueueDrained,
    Cancelled,
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
    /// Includes bytes read from files that later failed validation.
    #[serde(default)]
    pub bytes_read: u64,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default)]
    pub read_ms: u64,
    #[serde(default)]
    pub hash_ms: u64,
    #[serde(default)]
    pub stop_reason: Option<FullHashBackfillStopReason>,
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
    /// The files behind `failed`, in the order they failed, capped at
    /// [`FULL_HASH_BACKFILL_MAX_RECORDED_FAILURES`] so one pathological run
    /// cannot bloat the job row.
    #[serde(default)]
    pub failures: Vec<FullHashBackfillFailure>,
    /// True when more files failed than `failures` lists.
    #[serde(default)]
    pub failures_truncated: bool,
}

/// Most failures one run records by name. `failed` still counts every one.
pub(crate) const FULL_HASH_BACKFILL_MAX_RECORDED_FAILURES: usize = 100;

/// One file the run could not hash. It stays queued for the next sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullHashBackfillFailure {
    pub media_file_id: String,
    pub path: String,
    pub reason: String,
}

impl FullHashBackfillSummary {
    /// Counts a failed file and records it by name while there is room.
    pub(crate) fn record_failure(&mut self, media_file_id: &str, path: &str, reason: &str) {
        self.failed += 1;
        if self.failures.len() < FULL_HASH_BACKFILL_MAX_RECORDED_FAILURES {
            self.failures.push(FullHashBackfillFailure {
                media_file_id: media_file_id.to_string(),
                path: path.to_string(),
                reason: reason.to_string(),
            });
        } else {
            self.failures_truncated = true;
        }
    }

    /// The job run status this summary forces, if any: a cancelled run or a
    /// run with unhashable files finishes as a warning.
    pub(crate) fn run_status_override(&self) -> Option<crate::JobRunStatus> {
        (self.cancelled || self.failed > 0).then_some(crate::JobRunStatus::Warning)
    }

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
    pub(crate) async fn run_full_hash_backfill_job(
        &self,
        trigger: crate::JobTriggerSource,
    ) -> AppResult<FullHashBackfillSummary> {
        self.run_full_hash_backfill_with_options(FullHashBackfillOptions {
            scheduled: trigger != crate::JobTriggerSource::Manual,
            ..Default::default()
        })
        .await
    }

    pub(crate) async fn run_full_hash_backfill_with_options(
        &self,
        options: FullHashBackfillOptions,
    ) -> AppResult<FullHashBackfillSummary> {
        let cancellation = self.runtime.jobs.full_hash_shutdown.child_token();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let app = self.clone();
        // Drain filesystem calls as well as the reader thread if the caller is
        // dropped. Tokio filesystem operations may outlive their awaiter.
        tokio::spawn(async move { app.run_full_hash_backfill_pass(options, cancellation).await })
            .await
            .map_err(|error| {
                AppError::Repository(format!("full-hash invocation failed: {error}"))
            })?
    }

    async fn run_full_hash_backfill_pass(
        &self,
        options: FullHashBackfillOptions,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> AppResult<FullHashBackfillSummary> {
        // The blocking reader retains a share of this permit if its awaiting
        // future is dropped. A replacement run cannot overlap the old I/O.
        let execution = self
            .runtime
            .jobs
            .full_hash_execution_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| {
                AppError::validation_refused(
                    crate::error_reasons::location::FULL_HASH_BACKFILL_ALREADY_RUNNING,
                    "Full-hash backfill is already running",
                )
            })?;
        let Ok(maintenance) = self
            .runtime
            .jobs
            .background_disk_maintenance
            .clone()
            .try_lock_owned()
        else {
            return Ok(FullHashBackfillSummary {
                stop_reason: Some(FullHashBackfillStopReason::ImageMaintenance),
                resume_after_id: self.read_full_hash_backfill_cursor().await,
                ..Default::default()
            });
        };
        let permit = std::sync::Arc::new((execution, maintenance));
        let started = tokio::time::Instant::now();
        let io = std::sync::Arc::new(HashIoStats::default());
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
                if started.elapsed() >= options.max_duration {
                    summary.stop_reason = Some(FullHashBackfillStopReason::TimeLimit);
                    break 'sweep;
                }
                if options.scheduled
                    && !full_hash_backfill_window_open(
                        &self.runtime.environment.now().with_timezone(&Local),
                    )
                {
                    summary.stop_reason = Some(FullHashBackfillStopReason::WindowClosed);
                    break 'sweep;
                }

                match self
                    .backfill_one_media_file(
                        &candidate,
                        &owned,
                        io.clone(),
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
                        summary.record_failure(&candidate.id, &candidate.file_path, &reason);
                        tracing::warn!(
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
                #[cfg(test)]
                if let Some(checkpoint) = &options.after_file {
                    tokio::select! {
                        _ = cancellation.cancelled() => {},
                        permit = checkpoint.acquire() => { if let Ok(permit) = permit { permit.forget(); } },
                    }
                }
            }
        }

        summary.resume_after_id = cursor.clone();
        self.write_full_hash_backfill_cursor(cursor).await;
        summary.stop_reason = Some(if summary.cancelled {
            FullHashBackfillStopReason::Cancelled
        } else if summary.completed_sweep {
            FullHashBackfillStopReason::QueueDrained
        } else {
            summary
                .stop_reason
                .unwrap_or(FullHashBackfillStopReason::FileLimit)
        });
        summary.elapsed_ms = duration_ms(started.elapsed());
        summary.bytes_read = io.bytes.load(std::sync::atomic::Ordering::Relaxed);
        summary.read_ms = io.read_ns.load(std::sync::atomic::Ordering::Relaxed) / 1_000_000;
        summary.hash_ms = io.hash_ns.load(std::sync::atomic::Ordering::Relaxed) / 1_000_000;
        tracing::info!(bytes_read = summary.bytes_read, elapsed_ms = summary.elapsed_ms,
            read_ms = summary.read_ms, hash_ms = summary.hash_ms, stop_reason = ?summary.stop_reason,
            "full-hash backfill invocation finished");
        Ok(summary)
    }

    /// One file: check the skips, hash it, persist it. `Ok(Some(bytes))` when
    /// the file was hashed.
    async fn backfill_one_media_file(
        &self,
        candidate: &MediaFileHashCandidate,
        owned: &OwnedScope,
        io: std::sync::Arc<HashIoStats>,
        cancellation: &tokio_util::sync::CancellationToken,
        permit: std::sync::Arc<BackfillPermits>,
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

        let hashes =
            hash_file_cancellable(path.clone(), io, cancellation.child_token(), Some(permit))
                .await
                .map_err(|error| BackfillOutcome::Failed(error.to_string()))?;
        let bytes = hashes.size_bytes;
        let after = tokio::fs::metadata(&path)
            .await
            .map_err(|error| BackfillOutcome::Failed(error.to_string()))?;
        if !same_file_version(&before, &after) || bytes != after.len() {
            return Err(BackfillOutcome::Failed("file changed while hashing".into()));
        }
        // The file held still, but it is not the size the catalog recorded:
        // the row is stale (an external re-tag, say), not the read.
        if bytes != current.size_bytes as u64 {
            return Err(BackfillOutcome::Failed(format!(
                "size on disk ({bytes} bytes) differs from the catalog ({} bytes); rescan the title",
                current.size_bytes
            )));
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

type BackfillPermits = (
    tokio::sync::OwnedMutexGuard<()>,
    tokio::sync::OwnedMutexGuard<()>,
);

#[derive(Default)]
struct HashIoStats {
    bytes: std::sync::atomic::AtomicU64,
    read_ns: std::sync::atomic::AtomicU64,
    hash_ns: std::sync::atomic::AtomicU64,
}

fn duration_ms(value: Duration) -> u64 {
    value.as_millis().try_into().unwrap_or(u64::MAX)
}

/// Reads a file sequentially on a dedicated background thread.
///
/// The CRC costs nothing extra next to the BLAKE3 this job exists to produce
/// (see [`crate::location::verify`]'s measurements), and having it persisted
/// means a later move can compare a destination read-back against a stored
/// value instead of re-reading the source.
async fn hash_file_cancellable(
    path: PathBuf,
    io: std::sync::Arc<HashIoStats>,
    cancellation: tokio_util::sync::CancellationToken,
    permit: Option<std::sync::Arc<BackfillPermits>>,
) -> AppResult<crate::location::model::StreamedContentHashes> {
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("full-hash-backfill".into())
        .spawn(move || {
            let _permit = permit;
            if let Err(error) = hash_reader_priority() {
                static REPORTED: std::sync::Once = std::sync::Once::new();
                REPORTED.call_once(
                    || tracing::warn!(%error, "full-hash background priority unavailable"),
                );
            }
            let result = (|| {
                use std::io::Read;

                if cancellation.is_cancelled() {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::location::FULL_HASH_BACKFILL_CANCELLED,
                        "Full-hash backfill cancelled",
                    ));
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
                        return Err(AppError::validation_refused(
                            crate::error_reasons::location::FULL_HASH_BACKFILL_CANCELLED,
                            "Full-hash backfill cancelled",
                        ));
                    }
                    let read_started = std::time::Instant::now();
                    let read_result = file.read(&mut buffer);
                    io.read_ns.fetch_add(
                        read_started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    let read = read_result.map_err(|error| {
                        AppError::Repository(format!(
                            "failed to read {} for full-hash backfill: {error}",
                            path.display()
                        ))
                    })?;
                    if read == 0 {
                        break;
                    }
                    io.bytes
                        .fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
                    let hash_started = std::time::Instant::now();
                    hasher.update(&buffer[..read]);
                    io.hash_ns.fetch_add(
                        hash_started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                let after = std::fs::metadata(&path)
                    .map_err(|error| AppError::Repository(error.to_string()))?;
                if cancellation.is_cancelled() {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::location::FULL_HASH_BACKFILL_CANCELLED,
                        "Full-hash backfill cancelled",
                    ));
                }
                if !same_file_version(&before, &after) {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::location::FILE_CHANGED_WHILE_HASHING,
                        "file changed while hashing",
                    ));
                }
                Ok(hasher.finalize())
            })();
            drop(_permit);
            let _ = send.send(result);
        })
        .map_err(|error| {
            AppError::Repository(format!("full-hash reader thread could not start: {error}"))
        })?;
    receive.await.map_err(|error| {
        AppError::Repository(format!("full-hash backfill task failed to join: {error}"))
    })?
}

/// Applied only to this short-lived reader thread, never to a runtime pool.
fn hash_reader_priority() -> std::io::Result<()> {
    crate::background_worker_priority()?;
    #[cfg(target_os = "linux")]
    {
        // Best-effort I/O class, lowest priority. Unsupported schedulers may
        // ignore this hint; it is not an application bandwidth limiter.
        if unsafe { libc::syscall(libc::SYS_ioprio_set, 1, 0, (2 << 13) | 7) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "background I/O priority is unsupported",
    ));
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn full_hash_backfill_window_follows_local_two_to_six() {
        let zone = FixedOffset::west_opt(6 * 60 * 60).unwrap();
        for (hour, open) in [
            (23, false),
            (0, false),
            (1, false),
            (2, true),
            (5, true),
            (6, false),
        ] {
            let now = zone.with_ymd_and_hms(2026, 9, 12, hour, 0, 0).unwrap();
            assert_eq!(full_hash_backfill_window_open(&now), open);

            let next = next_full_hash_backfill_window(&now, true)
                .unwrap()
                .with_timezone(&zone);
            if open {
                assert_eq!(next, now);
            } else {
                assert_eq!(next.hour(), 2);
                assert_eq!(
                    next.date_naive(),
                    now.date_naive() + chrono::Duration::days(i64::from(hour >= 6))
                );
            }
        }
    }

    #[test]
    fn full_hash_backfill_skips_ticks_missed_by_long_batches() {
        let zone = FixedOffset::west_opt(6 * 60 * 60).unwrap();
        let started = zone.with_ymd_and_hms(2026, 9, 12, 2, 0, 0).unwrap();
        for (duration_minutes, next_hour, next_minute) in
            [(5, 2, 30), (30, 3, 0), (45, 3, 0), (95, 4, 0)]
        {
            let completed = started + chrono::Duration::minutes(duration_minutes);
            let next = next_full_hash_backfill_run(&completed)
                .unwrap()
                .with_timezone(&zone);
            assert_eq!(
                next,
                zone.with_ymd_and_hms(2026, 9, 12, next_hour, next_minute, 0)
                    .unwrap()
            );
        }
    }

    #[test]
    fn full_hash_backfill_next_tick_has_no_completion_time_drift() {
        let zone = FixedOffset::east_opt(5 * 3600 + 45 * 60).unwrap();
        let completed = zone
            .with_ymd_and_hms(2026, 9, 12, 2, 45, 17)
            .unwrap()
            .with_nanosecond(123_456_789)
            .unwrap();
        assert_eq!(
            next_full_hash_backfill_run(&completed)
                .unwrap()
                .with_timezone(&zone),
            zone.with_ymd_and_hms(2026, 9, 12, 3, 0, 0).unwrap()
        );
    }

    #[test]
    fn full_hash_backfill_resumes_next_night_after_the_window_closes() {
        let zone = FixedOffset::west_opt(6 * 60 * 60).unwrap();
        let inside_window = zone.with_ymd_and_hms(2026, 9, 12, 2, 0, 0).unwrap();
        assert_eq!(
            next_full_hash_backfill_run(&inside_window)
                .unwrap()
                .with_timezone(&zone),
            zone.with_ymd_and_hms(2026, 9, 12, 2, 30, 0).unwrap()
        );

        for (hour, minute) in [(5, 45), (6, 0), (7, 15)] {
            let completed = zone.with_ymd_and_hms(2026, 9, 12, hour, minute, 0).unwrap();
            assert_eq!(
                next_full_hash_backfill_run(&completed)
                    .unwrap()
                    .with_timezone(&zone),
                zone.with_ymd_and_hms(2026, 9, 13, 2, 0, 0).unwrap()
            );
        }
    }
}
