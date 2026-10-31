//! The running side of one configured HTTP downloader: its queue in memory,
//! the bounded scheduler, and the workers that transfer each job.
//!
//! The queue document on disk is the record. Every state change is written
//! there before it is reported; byte progress goes to the job's own progress
//! file, written only after the bytes are synced. A restart (or a
//! configuration change) reloads both and picks up whatever was queued or in
//! flight.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{AppError, AppResult};
use tokio::sync::{Mutex, OnceCell, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};

use super::selection::media_final_path;
use super::state::{
    HttpFileRole, HttpJob, HttpJobFile, HttpJobProgress, HttpJobState, HttpQueueDocument,
    HttpStateStore, RefusedVariant,
};
use super::transfer::{
    BandwidthLimiter, CHECKPOINT_BYTES, CHECKPOINT_INTERVAL, Checkpoint, CheckpointCadence, Clock,
    Egress, MediaCheck, MediaProbe, NativeMediaProbe, TransferContext, TransferError, system_clock,
    transfer_file,
};

/// Jobs that may wait or run at once. Submissions past this are refused.
pub(crate) const MAX_ACTIVE_JOBS: usize = 1000;
/// Finished and failed jobs kept for history; the oldest beyond this are
/// dropped from the queue document (their files stay where they are).
pub(crate) const MAX_RETAINED_FINISHED_JOBS: usize = 2000;
/// The first retry's wait; later retries double it.
const DEFAULT_BACKOFF_UNIT: Duration = Duration::from_secs(1);

/// Operator-tunable limits, adjustable without restarting transfers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HttpEngineLimits {
    pub max_concurrent_downloads: usize,
    pub max_connections_per_host: usize,
    pub bandwidth_limit_kbps: Option<u64>,
}

#[derive(Default)]
struct EngineInner {
    document: HttpQueueDocument,
    running: HashMap<String, RunningJob>,
}

struct RunningJob {
    cancel: CancellationToken,
    live: Arc<LiveProgress>,
    admission_host: Option<String>,
}

/// In-memory progress of a running job, for queue listings only.
pub(crate) struct LiveProgress {
    pub file_index: AtomicU64,
    pub current_file_bytes: Arc<AtomicU64>,
    started_at: Instant,
    started_bytes: AtomicU64,
}

pub(crate) struct HttpEngine {
    pub(crate) client_id: String,
    pub(crate) download_dir: PathBuf,
    store: HttpStateStore,
    limits: std::sync::Mutex<HttpEngineLimits>,
    egress: std::sync::Mutex<Egress>,
    /// What the egress was built from; an unchanged key keeps the client and
    /// its connection pool.
    egress_key: std::sync::Mutex<String>,
    #[cfg(test)]
    egress_builds: AtomicU64,
    limiter: Arc<BandwidthLimiter>,
    host_limiter: super::transfer::HostLimiter,
    backoff_unit: std::sync::Mutex<Duration>,
    media_probe: std::sync::Mutex<Arc<dyn MediaProbe>>,
    checkpoint_cadence: std::sync::Mutex<CheckpointCadence>,
    clock: std::sync::Mutex<Clock>,
    inner: Mutex<EngineInner>,
    /// Document writes requested so far. A write covers every request made
    /// before its snapshot was taken.
    persist_requests: AtomicU64,
    /// Held by the one writer; holds the request count its last write
    /// covered.
    save_lock: Mutex<u64>,
    #[cfg(test)]
    document_writes: AtomicU64,
    #[cfg(test)]
    progress_writes: AtomicU64,
    started: OnceCell<()>,
    /// Whether the client is enabled. A disabled engine starts no transfer,
    /// and a running one stops after the file it is on.
    enabled: AtomicBool,
    /// The workers of the engine this one replaced, which must stop before
    /// this one reads the queue they write.
    predecessor: std::sync::Mutex<Option<TaskTracker>>,
    shutdown: CancellationToken,
    tasks: TaskTracker,
    revision: watch::Sender<u64>,
}

impl HttpEngine {
    pub(crate) fn new(
        client_id: String,
        download_dir: PathBuf,
        limits: HttpEngineLimits,
        egress: Egress,
        egress_key: String,
        predecessor: Option<TaskTracker>,
    ) -> Arc<Self> {
        let store = HttpStateStore::for_client(&download_dir, &client_id);
        Arc::new(Self {
            client_id,
            download_dir,
            store,
            host_limiter: super::transfer::HostLimiter::new(limits.max_connections_per_host),
            limiter: Arc::new(BandwidthLimiter::new(bytes_per_second(
                limits.bandwidth_limit_kbps,
            ))),
            limits: std::sync::Mutex::new(limits),
            egress: std::sync::Mutex::new(egress),
            egress_key: std::sync::Mutex::new(egress_key),
            #[cfg(test)]
            egress_builds: AtomicU64::new(1),
            backoff_unit: std::sync::Mutex::new(DEFAULT_BACKOFF_UNIT),
            media_probe: std::sync::Mutex::new(Arc::new(NativeMediaProbe)),
            checkpoint_cadence: std::sync::Mutex::new(CheckpointCadence {
                bytes: CHECKPOINT_BYTES,
                interval: CHECKPOINT_INTERVAL,
            }),
            clock: std::sync::Mutex::new(system_clock()),
            inner: Mutex::new(EngineInner::default()),
            persist_requests: AtomicU64::new(0),
            save_lock: Mutex::new(0),
            #[cfg(test)]
            document_writes: AtomicU64::new(0),
            #[cfg(test)]
            progress_writes: AtomicU64::new(0),
            started: OnceCell::new(),
            enabled: AtomicBool::new(true),
            predecessor: std::sync::Mutex::new(predecessor),
            shutdown: CancellationToken::new(),
            tasks: TaskTracker::new(),
            revision: watch::channel(0).0,
        })
    }

    /// Apply new limits to transfers that start from now on.
    pub(crate) fn set_limits(&self, limits: HttpEngineLimits) {
        *self.limits.lock().unwrap_or_else(|p| p.into_inner()) = limits;
        self.host_limiter.set_limit(limits.max_connections_per_host);
        self.limiter
            .set_rate(bytes_per_second(limits.bandwidth_limit_kbps));
    }

    /// Whether the egress in use was built from `key`.
    pub(crate) fn has_egress_key(&self, key: &str) -> bool {
        *self.egress_key.lock().unwrap_or_else(|p| p.into_inner()) == key
    }

    /// Replace the egress for transfers that start from now on.
    pub(crate) fn set_egress(&self, egress: Egress, key: String) {
        *self.egress.lock().unwrap_or_else(|p| p.into_inner()) = egress;
        *self.egress_key.lock().unwrap_or_else(|p| p.into_inner()) = key;
        #[cfg(test)]
        self.egress_builds.fetch_add(1, Ordering::Relaxed);
    }

    /// Egress clients built for this engine, including its first.
    #[cfg(test)]
    pub(crate) fn egress_builds(&self) -> u64 {
        self.egress_builds.load(Ordering::Relaxed)
    }

    fn limits(&self) -> HttpEngineLimits {
        *self.limits.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn egress(&self) -> Egress {
        self.egress
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn backoff_unit(&self) -> Duration {
        *self.backoff_unit.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Shorten retry waits so tests drive retries without waiting on the
    /// clock.
    #[cfg(test)]
    pub(crate) fn set_backoff_unit(&self, unit: Duration) {
        *self.backoff_unit.lock().unwrap_or_else(|p| p.into_inner()) = unit;
    }

    fn media_probe(&self) -> Arc<dyn MediaProbe> {
        Arc::clone(&self.media_probe.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Replace the probe that checks finished media files.
    #[cfg(test)]
    pub(crate) fn set_media_probe(&self, probe: Arc<dyn MediaProbe>) {
        *self.media_probe.lock().unwrap_or_else(|p| p.into_inner()) = probe;
    }

    fn checkpoint_cadence(&self) -> CheckpointCadence {
        *self
            .checkpoint_cadence
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Checkpoint after fewer bytes, so tests see several checkpoints in a
    /// small body.
    #[cfg(test)]
    pub(crate) fn set_checkpoint_cadence(&self, bytes: u64, interval: Duration) {
        *self
            .checkpoint_cadence
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = CheckpointCadence { bytes, interval };
    }

    fn clock(&self) -> Clock {
        Arc::clone(&self.clock.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Replace the clock that decides whether a resource URL has expired.
    #[cfg(test)]
    pub(crate) fn set_clock(&self, clock: Clock) {
        *self.clock.lock().unwrap_or_else(|p| p.into_inner()) = clock;
    }

    /// Queue documents written since the engine started.
    #[cfg(test)]
    pub(crate) fn document_writes(&self) -> u64 {
        self.document_writes.load(Ordering::Relaxed)
    }

    /// Progress files written since the engine started.
    #[cfg(test)]
    pub(crate) fn progress_writes(&self) -> u64 {
        self.progress_writes.load(Ordering::Relaxed)
    }

    /// Stop scheduling and ask every worker to checkpoint and stop. Their
    /// jobs stay recorded as they were, for the successor to resume.
    pub(crate) fn begin_shutdown(&self) {
        self.shutdown.cancel();
        self.tasks.close();
    }

    /// The engine's workers, for a successor to wait on.
    pub(crate) fn workers(&self) -> TaskTracker {
        self.tasks.clone()
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Enable or disable the client. Disabling starts no new transfer and
    /// lets a running one finish its current file before the job goes back
    /// to the queue. Enabling schedules the queue again.
    pub(crate) fn set_enabled(self: &Arc<Self>, enabled: bool) {
        let was = self.enabled.swap(enabled, Ordering::AcqRel);
        if enabled
            && !was
            && !self.shutdown.is_cancelled()
            && tokio::runtime::Handle::try_current().is_ok()
        {
            let engine = Arc::clone(self);
            self.tasks.spawn(async move { engine.schedule().await });
        }
    }

    #[cfg(test)]
    pub(crate) fn is_shut_down(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    /// A receiver that changes whenever the queue document does.
    #[cfg(test)]
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// Load the queue document once, after any predecessor for the same
    /// client has stopped writing, and resume what was queued or running.
    pub(crate) async fn ensure_started(self: &Arc<Self>) -> AppResult<()> {
        if self.shutdown.is_cancelled() {
            return Err(AppError::download_submit_unavailable(
                "the HTTP downloader's configuration changed; try again",
            ));
        }
        self.started
            .get_or_try_init(|| async {
                let predecessor = self
                    .predecessor
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                if let Some(predecessor) = predecessor {
                    predecessor.wait().await;
                }
                let mut document = self.store.load().await?;
                let mut changed = false;
                for job in &mut document.jobs {
                    if job.state.is_terminal() {
                        continue;
                    }
                    // Checkpoints after the document's last write live in
                    // the job's progress file.
                    if let Some(progress) = self.store.load_progress(&job.id).await
                        && progress.apply_if_newer(job)
                    {
                        changed = true;
                    }
                    // Nothing runs before this point, so a job recorded as
                    // downloading was interrupted and goes back to the queue.
                    if job.state == HttpJobState::Downloading {
                        job.state = HttpJobState::Queued;
                        changed = true;
                    }
                }
                self.inner.lock().await.document = document;
                if changed {
                    self.persist().await?;
                }
                Ok::<(), AppError>(())
            })
            .await?;
        self.schedule().await;
        Ok(())
    }

    /// Write the current document.
    ///
    /// One writer at a time, and a write covers every request made before it
    /// took its snapshot: callers change the document first and request the
    /// write after, so a burst of requests waiting behind one write is served
    /// by the next single write of the newest state.
    async fn persist(&self) -> AppResult<()> {
        let requested = self.persist_requests.fetch_add(1, Ordering::AcqRel) + 1;
        let mut covered = self.save_lock.lock().await;
        if *covered >= requested {
            return Ok(());
        }
        let covers = self.persist_requests.load(Ordering::Acquire);
        // Copy under the lock, serialize and write after releasing it.
        let snapshot = self.inner.lock().await.document.clone();
        self.store.save(&snapshot).await?;
        drop(snapshot);
        *covered = covers;
        #[cfg(test)]
        self.document_writes.fetch_add(1, Ordering::Relaxed);
        self.revision.send_modify(|revision| *revision += 1);
        Ok(())
    }

    pub(crate) async fn add_job(self: &Arc<Self>, job: HttpJob) -> AppResult<()> {
        self.ensure_started().await?;
        {
            let mut inner = self.inner.lock().await;
            let active = inner
                .document
                .jobs
                .iter()
                .filter(|job| !job.state.is_terminal())
                .count();
            if active >= MAX_ACTIVE_JOBS {
                return Err(AppError::download_submit_unavailable(format!(
                    "the HTTP downloader already holds {MAX_ACTIVE_JOBS} unfinished downloads"
                )));
            }
            if inner.document.jobs.iter().any(|existing| {
                existing.download_id.is_some() && existing.download_id == job.download_id
            }) {
                return Err(AppError::validation_refused(
                    crate::error_reasons::ACQUISITION_HTTP_DOWNLOAD_ALREADY_SUBMITTED,
                    "this download was already submitted to the HTTP downloader",
                ));
            }
            inner.document.jobs.push(job);
            prune_finished(&mut inner.document);
        }
        self.persist().await?;
        self.schedule().await;
        Ok(())
    }

    /// One job and, while it runs, its live progress. Only that job is cloned.
    pub(crate) async fn job_with_live(
        self: &Arc<Self>,
        id: &str,
    ) -> AppResult<Option<(HttpJob, Option<LiveSnapshot>)>> {
        self.ensure_started().await?;
        let inner = self.inner.lock().await;
        Ok(inner
            .document
            .jobs
            .iter()
            .find(|job| job.id == id)
            .map(|job| (job.clone(), live_of(&inner, job))))
    }

    /// The jobs `matches` accepts, with live progress for those running.
    /// Jobs it rejects are never cloned.
    pub(crate) async fn jobs_where(
        self: &Arc<Self>,
        matches: impl Fn(&HttpJob) -> bool,
    ) -> AppResult<Vec<(HttpJob, Option<LiveSnapshot>)>> {
        self.ensure_started().await?;
        let inner = self.inner.lock().await;
        Ok(inner
            .document
            .jobs
            .iter()
            .filter(|job| matches(job))
            .map(|job| (job.clone(), live_of(&inner, job)))
            .collect())
    }

    #[cfg(test)]
    pub(crate) async fn job(self: &Arc<Self>, id: &str) -> AppResult<Option<HttpJob>> {
        self.ensure_started().await?;
        let inner = self.inner.lock().await;
        Ok(inner.document.jobs.iter().find(|job| job.id == id).cloned())
    }

    pub(crate) async fn pause(self: &Arc<Self>, id: &str) -> AppResult<()> {
        self.ensure_started().await?;
        {
            let mut inner = self.inner.lock().await;
            let job = find_job_mut(&mut inner.document, id)?;
            match job.state {
                HttpJobState::Queued | HttpJobState::Downloading => {
                    job.state = HttpJobState::Paused;
                    job.updated_at = Utc::now();
                }
                HttpJobState::Paused => return Ok(()),
                state => {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::ACQUISITION_DOWNLOAD_NOT_PAUSABLE,
                        format!("a {} download cannot be paused", state_label(state)),
                    ));
                }
            }
            if let Some(running) = inner.running.get(id) {
                running.cancel.cancel();
            }
        }
        self.persist().await
    }

    pub(crate) async fn resume(self: &Arc<Self>, id: &str) -> AppResult<()> {
        self.ensure_started().await?;
        {
            let mut inner = self.inner.lock().await;
            let job = find_job_mut(&mut inner.document, id)?;
            match job.state {
                HttpJobState::Paused => {
                    job.state = HttpJobState::Queued;
                    job.updated_at = Utc::now();
                }
                HttpJobState::Queued | HttpJobState::Downloading => return Ok(()),
                state => {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::ACQUISITION_DOWNLOAD_NOT_RESUMABLE,
                        format!("a {} download cannot be resumed", state_label(state)),
                    ));
                }
            }
        }
        self.persist().await?;
        self.schedule().await;
        Ok(())
    }

    /// Drop a job from the queue. Its files, finished or partial, stay on
    /// disk: this client never deletes anything it wrote.
    pub(crate) async fn remove(self: &Arc<Self>, id: &str, remove_data: bool) -> AppResult<()> {
        self.ensure_started().await?;
        let removed = {
            let mut inner = self.inner.lock().await;
            if let Some(running) = inner.running.get(id) {
                running.cancel.cancel();
            }
            let before = inner.document.jobs.len();
            inner.document.jobs.retain(|job| job.id != id);
            before != inner.document.jobs.len()
        };
        if !removed {
            return Err(AppError::not_found_reason(
                crate::error_reasons::ACQUISITION_HTTP_DOWNLOAD_NOT_FOUND,
                format!("the HTTP downloader has no download {id}"),
            ));
        }
        if remove_data {
            warn!(
                client_id = self.client_id.as_str(),
                item_id = id,
                "the HTTP downloader does not remove downloaded data; the files were left in place"
            );
        }
        self.persist().await?;
        self.schedule().await;
        Ok(())
    }

    /// Start queued jobs up to the concurrency and per-host limits.
    ///
    /// Boxed with an explicit `Send` bound: a finishing worker calls back in
    /// here, and the recursion through the spawned worker would otherwise
    /// leave the compiler unable to prove the future `Send`.
    fn schedule(self: &Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.schedule_inner())
    }

    async fn schedule_inner(self: &Arc<Self>) {
        if self.shutdown.is_cancelled() || self.started.get().is_none() || !self.is_enabled() {
            return;
        }
        let limits = self.limits();
        let mut inner = self.inner.lock().await;
        let candidates: Vec<String> = inner
            .document
            .jobs
            .iter()
            .filter(|job| job.state == HttpJobState::Queued && !inner.running.contains_key(&job.id))
            .map(|job| job.id.clone())
            .collect();
        for id in candidates {
            if inner.running.len() >= limits.max_concurrent_downloads.max(1) {
                break;
            }
            let admission_host = inner
                .document
                .jobs
                .iter()
                .find(|job| job.id == id)
                .and_then(|job| {
                    job.files
                        .iter()
                        .find(|file| !file.complete && file.skipped_error.is_none())
                })
                .and_then(|file| url::Url::parse(&file.resource.url).ok())
                .and_then(|url| {
                    url.host_str()
                        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
                });
            if admission_host.as_ref().is_some_and(|host| {
                inner
                    .running
                    .values()
                    .filter(|job| job.admission_host.as_ref() == Some(host))
                    .count()
                    >= limits.max_connections_per_host.max(1)
            }) {
                continue;
            }
            let cancel = self.shutdown.child_token();
            let live = Arc::new(LiveProgress {
                file_index: AtomicU64::new(0),
                current_file_bytes: Arc::new(AtomicU64::new(0)),
                started_at: Instant::now(),
                started_bytes: AtomicU64::new(0),
            });
            if let Some(job) = inner.document.jobs.iter_mut().find(|job| job.id == id) {
                live.started_bytes
                    .store(job.durable_bytes(), Ordering::Relaxed);
                job.state = HttpJobState::Downloading;
                job.updated_at = Utc::now();
            }
            inner.running.insert(
                id.clone(),
                RunningJob {
                    cancel: cancel.clone(),
                    live: Arc::clone(&live),
                    admission_host,
                },
            );
            let engine = Arc::clone(self);
            self.tasks.spawn(async move {
                engine.run_job(id, cancel, live).await;
            });
        }
    }

    async fn run_job(
        self: Arc<Self>,
        id: String,
        cancel: CancellationToken,
        live: Arc<LiveProgress>,
    ) {
        if let Err(error) = self.persist().await {
            warn!(client_id = self.client_id.as_str(), error = %error, "HTTP downloader could not record a started download");
        }
        let outcome = self.transfer_job(&id, &cancel, &live).await;
        let finished = {
            let mut inner = self.inner.lock().await;
            inner.running.remove(&id);
            match inner.document.jobs.iter_mut().find(|job| job.id == id) {
                None => false,
                Some(job) => {
                    match &outcome {
                        Ok(()) => {
                            job.state = HttpJobState::Completed;
                            job.error = None;
                            job.error_reason = None;
                            job.completed_at = Some(Utc::now());
                            info!(item_id = id.as_str(), "HTTP download completed");
                        }
                        // Paused, removed or shutting down: the control
                        // operation already recorded the job's state.
                        Err(TransferError::Cancelled) => {}
                        Err(error) => {
                            let message = error.message();
                            warn!(
                                item_id = id.as_str(),
                                error = message.as_str(),
                                "HTTP download failed"
                            );
                            job.state = HttpJobState::Failed;
                            job.error = Some(message);
                            job.error_reason = error.reason().map(str::to_owned);
                        }
                    }
                    if job.state.is_terminal() {
                        // Never tried again; only the refusals stay, for the
                        // record.
                        job.media_fallbacks = Vec::new();
                    }
                    job.updated_at = Utc::now();
                    true
                }
            }
        };
        if finished
            && !self.shutdown.is_cancelled()
            && let Err(error) = self.persist().await
        {
            warn!(client_id = self.client_id.as_str(), error = %error, "HTTP downloader could not record a finished download");
        }
        self.schedule().await;
    }

    async fn transfer_job(
        self: &Arc<Self>,
        id: &str,
        cancel: &CancellationToken,
        live: &Arc<LiveProgress>,
    ) -> Result<(), TransferError> {
        let (file_count, output_dir) = {
            let inner = self.inner.lock().await;
            let job = inner
                .document
                .jobs
                .iter()
                .find(|job| job.id == id)
                .ok_or(TransferError::Cancelled)?;
            (job.files.len(), job.output_dir.clone())
        };
        // A slow or hung filesystem must not hold every other caller of the
        // engine behind the lock.
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|error| {
                TransferError::Permanent(format!(
                    "could not create {}: {error}",
                    output_dir.display()
                ))
            })?;
        let mut index = 0;
        while index < file_count {
            let (mut file, expected_duration_seconds, verify, subtitle_required) = {
                let inner = self.inner.lock().await;
                let job = inner
                    .document
                    .jobs
                    .iter()
                    .find(|job| job.id == id)
                    .ok_or(TransferError::Cancelled)?;
                (
                    job.files[index].clone(),
                    job.expected_duration_seconds,
                    job.verify.clone(),
                    job.subtitle_required,
                )
            };
            if file.complete || file.skipped_error.is_some() {
                index += 1;
                continue;
            }
            if !self.is_enabled() {
                // Disabled between files: the finished ones stay finished
                // and the job waits in the queue for the client to return.
                self.requeue(id).await;
                return Err(TransferError::Cancelled);
            }
            live.file_index.store(index as u64, Ordering::Relaxed);
            live.current_file_bytes
                .store(file.bytes_done, Ordering::Relaxed);
            let probe = self.media_probe();
            let checkpoint = JobCheckpoint {
                engine: self,
                job_id: id,
                file_index: index,
            };
            let context = TransferContext {
                egress: self.egress(),
                checkpoint: &checkpoint,
                cancel,
                live_bytes: &live.current_file_bytes,
                limiter: Some(self.limiter.as_ref()),
                host_limiter: Some(&self.host_limiter),
                backoff_unit: self.backoff_unit(),
                cadence: self.checkpoint_cadence(),
                clock: self.clock(),
                media_check: (file.role == HttpFileRole::Media).then(|| MediaCheck {
                    probe: probe.as_ref(),
                    expected_duration_seconds,
                    verify,
                }),
            };
            match transfer_file(&context, &mut file).await {
                Ok(()) => index += 1,
                Err(error @ (TransferError::Forbidden(_) | TransferError::Rejected(_)))
                    if file.role == HttpFileRole::Media =>
                {
                    // The same file slot is tried again with the next
                    // candidate, from zero bytes.
                    self.advance_media_candidate(id, index, &error).await?;
                    live.current_file_bytes.store(0, Ordering::Relaxed);
                }
                Err(TransferError::Cancelled) => return Err(TransferError::Cancelled),
                Err(error) if file.role == HttpFileRole::Subtitle => {
                    let language = file.language.as_deref().unwrap_or("unlabelled");
                    if subtitle_required {
                        let message = format!(
                            "the required {language} subtitle could not be downloaded: {}; the media file is on disk",
                            error.message()
                        );
                        return Err(match error.reason() {
                            Some(reason) => TransferError::Refused(reason, message),
                            None => TransferError::Permanent(message),
                        });
                    }
                    warn!(
                        item_id = id,
                        language,
                        error = error.message().as_str(),
                        "HTTP download skipping a subtitle it could not fetch"
                    );
                    self.skip_file(id, index, &error).await?;
                    index += 1;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Put a job that stopped early back in the queue, unless a control
    /// operation already moved it elsewhere.
    async fn requeue(&self, id: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(job) = inner.document.jobs.iter_mut().find(|job| job.id == id)
            && job.state == HttpJobState::Downloading
        {
            job.state = HttpJobState::Queued;
            job.updated_at = Utc::now();
            info!(
                item_id = id,
                "HTTP download waiting while its client is disabled"
            );
        }
    }

    /// Record that an optional file was given up on, so it is neither tried
    /// again nor reported as delivered.
    async fn skip_file(
        &self,
        id: &str,
        index: usize,
        error: &TransferError,
    ) -> Result<(), TransferError> {
        {
            let mut inner = self.inner.lock().await;
            let job = inner
                .document
                .jobs
                .iter_mut()
                .find(|job| job.id == id)
                .ok_or(TransferError::Cancelled)?;
            job.files[index].skipped_error = Some(error.message());
            job.files[index].skipped_reason = error.reason().map(str::to_owned);
            job.progress_revision = job.progress_revision.saturating_add(1);
            job.updated_at = Utc::now();
        }
        self.persist()
            .await
            .map_err(|error| TransferError::Permanent(error.to_string()))
    }

    /// Give up on the active media resource and switch the job to its next
    /// fallback, or fail it when none is left. The new candidate writes a
    /// fresh attempt file; whatever the earlier one wrote stays on disk.
    async fn advance_media_candidate(
        &self,
        id: &str,
        index: usize,
        error: &TransferError,
    ) -> Result<(), TransferError> {
        let outcome = {
            let mut inner = self.inner.lock().await;
            let job = inner
                .document
                .jobs
                .iter_mut()
                .find(|job| job.id == id)
                .ok_or(TransferError::Cancelled)?;
            job.refused_variants.push(RefusedVariant {
                forbidden: matches!(error, TransferError::Forbidden(_)),
                error: error.message(),
            });
            job.updated_at = Utc::now();
            let next = usize::try_from(job.candidate_index)
                .ok()
                .and_then(|position| job.media_fallbacks.get(position))
                .cloned();
            match next {
                Some(resource) => {
                    job.candidate_index = job.candidate_index.saturating_add(1);
                    let final_path =
                        media_final_path(&job.output_dir, &job.output_basename, &resource);
                    let candidate = job.candidate_index;
                    let slot = &mut job.files[index];
                    let attempt = slot.attempt.saturating_add(1);
                    *slot = HttpJobFile {
                        role: HttpFileRole::Media,
                        resource,
                        final_path,
                        partial_path: None,
                        publication: None,
                        attempt,
                        bytes_done: 0,
                        total_bytes: None,
                        validator: None,
                        complete: false,
                        language: slot.language.clone(),
                        forbidden_answers: 0,
                        skipped_error: None,
                        skipped_reason: None,
                    };
                    info!(
                        item_id = id,
                        candidate,
                        reason = error.message().as_str(),
                        "HTTP download moving to the next media variant"
                    );
                    Ok(())
                }
                None => Err(TransferError::Permanent(job.exhausted_message())),
            }
        };
        self.persist()
            .await
            .map_err(|error| TransferError::Permanent(error.to_string()))?;
        outcome
    }

    /// Record one file's durable progress: in the job's progress file while
    /// the file transfers, in the document once the file is finished.
    async fn record_file(&self, job_id: &str, index: usize, file: &HttpJobFile) -> AppResult<()> {
        let progress = {
            let mut inner = self.inner.lock().await;
            let Some(job) = inner.document.jobs.iter_mut().find(|job| job.id == job_id) else {
                // Removed while it ran; nothing left to record against.
                return Ok(());
            };
            let finishes = file.complete && job.files.get(index).is_some_and(|slot| !slot.complete);
            // The progress file carries bytes only; a new final name goes
            // into the document.
            let renamed = job.files.get(index).is_some_and(|slot| {
                slot.final_path != file.final_path
                    || slot.partial_path != file.partial_path
                    || slot.publication != file.publication
            });
            if let Some(slot) = job.files.get_mut(index) {
                *slot = file.clone();
            }
            job.progress_revision = job.progress_revision.saturating_add(1);
            job.updated_at = Utc::now();
            (!finishes && !renamed).then(|| HttpJobProgress::of(job))
        };
        // A successor loads the queue only after every worker here has
        // stopped, so a final checkpoint during shutdown still belongs here.
        match progress {
            Some(progress) => {
                self.store.save_progress(job_id, &progress).await?;
                #[cfg(test)]
                self.progress_writes.fetch_add(1, Ordering::Relaxed);
                self.revision.send_modify(|revision| *revision += 1);
                Ok(())
            }
            None => self.persist().await,
        }
    }

    /// Record a `403 Forbidden` answer against the file and the job.
    async fn record_forbidden(
        &self,
        job_id: &str,
        index: usize,
        file: &HttpJobFile,
    ) -> AppResult<()> {
        {
            let mut inner = self.inner.lock().await;
            let Some(job) = inner.document.jobs.iter_mut().find(|job| job.id == job_id) else {
                return Ok(());
            };
            if let Some(slot) = job.files.get_mut(index) {
                *slot = file.clone();
            }
            job.forbidden_requests = job.forbidden_requests.saturating_add(1);
            job.updated_at = Utc::now();
        }
        self.persist().await
    }

    /// Prove the download directory can hold this client's queue.
    pub(crate) async fn check_writable(&self) -> AppResult<()> {
        self.store.initialize().await
    }
}

struct JobCheckpoint<'a> {
    engine: &'a Arc<HttpEngine>,
    job_id: &'a str,
    file_index: usize,
}

#[async_trait]
impl Checkpoint for JobCheckpoint<'_> {
    async fn record(&self, file: &HttpJobFile) -> AppResult<()> {
        self.engine
            .record_file(self.job_id, self.file_index, file)
            .await
    }

    async fn record_forbidden(&self, file: &HttpJobFile) -> AppResult<()> {
        self.engine
            .record_forbidden(self.job_id, self.file_index, file)
            .await
    }
}

/// A queue listing's view of a running job.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LiveSnapshot {
    pub bytes_done: u64,
    pub bytes_per_second: Option<f64>,
}

impl LiveSnapshot {
    fn of(job: &HttpJob, live: &LiveProgress) -> Self {
        let index = live.file_index.load(Ordering::Relaxed) as usize;
        let finished: u64 = job
            .files
            .iter()
            .take(index)
            .map(|file| file.total_bytes.unwrap_or(file.bytes_done))
            .sum();
        let current = live.current_file_bytes.load(Ordering::Relaxed);
        let bytes_done = finished.saturating_add(current).max(job.durable_bytes());
        let elapsed = live.started_at.elapsed().as_secs_f64();
        let moved = bytes_done.saturating_sub(live.started_bytes.load(Ordering::Relaxed));
        let bytes_per_second = (elapsed >= 1.0 && moved > 0).then(|| moved as f64 / elapsed);
        Self {
            bytes_done,
            bytes_per_second,
        }
    }
}

fn live_of(inner: &EngineInner, job: &HttpJob) -> Option<LiveSnapshot> {
    inner
        .running
        .get(&job.id)
        .map(|running| LiveSnapshot::of(job, &running.live))
}

fn find_job_mut<'a>(document: &'a mut HttpQueueDocument, id: &str) -> AppResult<&'a mut HttpJob> {
    document
        .jobs
        .iter_mut()
        .find(|job| job.id == id)
        .ok_or_else(|| {
            AppError::not_found_reason(
                crate::error_reasons::ACQUISITION_HTTP_DOWNLOAD_NOT_FOUND,
                format!("the HTTP downloader has no download {id}"),
            )
        })
}

/// Keep the newest finished jobs within the retention bound. Dropping a job
/// from the document leaves its files untouched.
fn prune_finished(document: &mut HttpQueueDocument) {
    let finished = document
        .jobs
        .iter()
        .filter(|job| job.state.is_terminal())
        .count();
    if finished <= MAX_RETAINED_FINISHED_JOBS {
        return;
    }
    let excess = finished - MAX_RETAINED_FINISHED_JOBS;
    let mut by_age: Vec<(chrono::DateTime<Utc>, String)> = document
        .jobs
        .iter()
        .filter(|job| job.state.is_terminal())
        .map(|job| (job.updated_at, job.id.clone()))
        .collect();
    by_age.sort();
    let drop: std::collections::HashSet<String> =
        by_age.into_iter().take(excess).map(|(_, id)| id).collect();
    document.jobs.retain(|job| !drop.contains(&job.id));
}

fn bytes_per_second(kbps: Option<u64>) -> u64 {
    kbps.filter(|kbps| *kbps > 0)
        .map(|kbps| kbps.saturating_mul(1024))
        .unwrap_or(0)
}

pub(crate) fn state_label(state: HttpJobState) -> &'static str {
    match state {
        HttpJobState::Queued => "queued",
        HttpJobState::Downloading => "downloading",
        HttpJobState::Paused => "paused",
        HttpJobState::Completed => "completed",
        HttpJobState::Failed => "failed",
    }
}
