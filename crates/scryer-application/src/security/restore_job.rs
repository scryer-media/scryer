use crate::{
    AppError, AppResult, BackupBundleInspectSummary, RESTORE_WORK_BUDGET, RestoreDeadline,
    restore_refusal,
};
use chrono::{DateTime, Utc};
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, OwnedRwLockReadGuard, RwLock};
use tokio_util::sync::CancellationToken;

pub const RESTORE_JOB_FILENAME: &str = "restore-job.json";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreJobKind {
    Inspect,
    Apply,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreJobPhase {
    Inspecting,
    Draining,
    Restoring,
    Cancelling,
    Restarting,
    Complete,
    Failed,
    Interrupted,
}
impl RestoreJobPhase {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Inspecting
                | Self::Draining
                | Self::Restoring
                | Self::Cancelling
                | Self::Restarting
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestoreJobStatus {
    pub id: String,
    pub upload_id: String,
    pub kind: RestoreJobKind,
    pub phase: RestoreJobPhase,
    pub started_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub reason: Option<String>,
    #[serde(default)]
    pub import_percent: Option<u8>,
    /// The server process that owns the active phase. The desktop wrapper
    /// treats an active record as live only while this process runs.
    #[serde(default)]
    pub pid: Option<u32>,
    /// Restored plugins still need their artifacts recovered from catalogs.
    /// Kept on the committed record so a failed attempt retries next start.
    #[serde(default)]
    pub plugin_recovery_pending: bool,
}

impl RestoreJobStatus {
    /// An Apply whose restart launcher failed after its stage was marked
    /// ready. The stage is still on disk, so a restart can promote it.
    pub fn restart_failed(&self) -> bool {
        self.kind == RestoreJobKind::Apply
            && self.phase == RestoreJobPhase::Failed
            && self.reason.as_deref() == Some(restore_refusal::RESTORE_RESTART_FAILED)
    }

    /// Whether the next boot may promote a ready stage for this record.
    pub fn awaits_promotion(&self) -> bool {
        self.phase == RestoreJobPhase::Restarting || self.restart_failed()
    }
}

/// Only an Apply that is still running, or whose promotion could not be
/// confirmed, owns the application. Every other record leaves it usable.
fn holds_application(status: &RestoreJobStatus) -> bool {
    status.kind == RestoreJobKind::Apply
        && (status.phase.active()
            || (status.phase == RestoreJobPhase::Failed
                && status.reason.as_deref() == Some(restore_refusal::RESTORE_RECOVERY_REQUIRED)))
}

type RestartHook = Arc<dyn Fn() + Send + Sync>;

/// One bounded slot; accepted work and application admission share this owner.
pub struct RestoreCoordinator {
    state: Mutex<Option<RestoreJobStatus>>,
    path: Mutex<Option<PathBuf>>,
    /// Serializes status file writes; never held together with `state`
    /// while writing, so readers never wait on an fsync.
    persist: Mutex<()>,
    summary: Mutex<Option<BackupBundleInspectSummary>>,
    exclusive: AtomicBool,
    pub changed: Notify,
    requests: Arc<RwLock<()>>,
    pub shutdown_workers: CancellationToken,
    workers: tokio_util::task::TaskTracker,
    worker_failed: Arc<AtomicBool>,
    deadline: Mutex<Option<RestoreDeadline>>,
    worker_running: AtomicBool,
    shutting_down: Arc<AtomicBool>,
    failure_restart: Mutex<Option<RestartHook>>,
}
impl Default for RestoreCoordinator {
    fn default() -> Self {
        Self {
            state: Mutex::new(None),
            path: Mutex::new(None),
            persist: Mutex::new(()),
            summary: Mutex::new(None),
            exclusive: AtomicBool::new(false),
            changed: Notify::new(),
            requests: Arc::new(RwLock::new(())),
            shutdown_workers: CancellationToken::new(),
            workers: tokio_util::task::TaskTracker::new(),
            worker_failed: Arc::new(AtomicBool::new(false)),
            deadline: Mutex::new(None),
            worker_running: AtomicBool::new(false),
            shutting_down: Arc::new(AtomicBool::new(false)),
            failure_restart: Mutex::new(None),
        }
    }
}
impl RestoreCoordinator {
    /// Reconstruct ownership before opening the application datastore.
    pub fn recover(data_dir: &Path) -> AppResult<Self> {
        let coordinator = Self::default();
        if let Some(mut status) = read_restore_job(data_dir)? {
            let path = data_dir.join(RESTORE_JOB_FILENAME);
            if status.phase.active() && status.phase != RestoreJobPhase::Restarting {
                status.phase = RestoreJobPhase::Interrupted;
                status.reason = Some(restore_refusal::RESTORE_INTERRUPTED.to_owned());
                persist_status(&path, &status)?;
            } else if status.phase == RestoreJobPhase::Restarting
                && status.pid != Some(std::process::id())
            {
                // The restart may have produced a new process; the record
                // must name the one that now owns the handoff.
                status.pid = Some(std::process::id());
                persist_status(&path, &status)?;
            }
            coordinator
                .exclusive
                .store(holds_application(&status), Ordering::Release);
            coordinator.worker_running.store(
                status.phase == RestoreJobPhase::Restarting,
                Ordering::Release,
            );
            *coordinator.path.lock().expect("restore path lock") = Some(path);
            *coordinator.state.lock().expect("restore status lock") = Some(status);
        }
        Ok(coordinator)
    }

    /// Called by the host once the restored files are promoted. The data is
    /// committed from here on: the restore budget and recovery guard end, and
    /// any later startup failure is an ordinary one.
    pub fn commit_promoted_restore(&self, restored: bool) -> AppResult<()> {
        let committed = {
            let mut state = self.state.lock().expect("restore status lock");
            // A failed handoff is committed only by an actual promotion; with
            // nothing promoted its record stays as it was.
            let Some(current) = state.as_mut().filter(|s| {
                s.phase == RestoreJobPhase::Restarting || (restored && s.restart_failed())
            }) else {
                return Ok(());
            };
            current.phase = RestoreJobPhase::Complete;
            current.reason = None;
            current.plugin_recovery_pending = restored;
            if let Some(deadline) = self.deadline.lock().expect("restore deadline lock").take() {
                current.import_percent = deadline.import_percent().or(current.import_percent);
            }
            self.exclusive
                .store(holds_application(current), Ordering::Release);
            self.worker_running.store(false, Ordering::Release);
            true
        };
        // Persist before waking waiters so they never read a stale record.
        let persisted = if committed {
            self.persist_current()
        } else {
            Ok(())
        };
        self.changed.notify_waiters();
        persisted
    }

    /// Put a failed restart handoff for `upload_id` back into Restarting so
    /// the caller can schedule the restart again. Its stage is already ready,
    /// so nothing is staged twice. Any other record is left untouched.
    pub fn resume_failed_restart(&self, upload_id: &str) -> AppResult<Option<RestoreJobStatus>> {
        let resumed = {
            let mut state = self.state.lock().expect("restore status lock");
            if self.shutting_down.load(Ordering::Acquire) {
                return Ok(None);
            }
            let Some(current) = state
                .as_mut()
                .filter(|s| s.restart_failed() && s.upload_id == upload_id)
            else {
                return Ok(None);
            };
            current.phase = RestoreJobPhase::Restarting;
            current.reason = None;
            current.pid = Some(std::process::id());
            self.exclusive
                .store(holds_application(current), Ordering::Release);
            // As with `transition` into Restarting, no in-process worker owns
            // the handoff; only the scheduled restart does.
            self.worker_running.store(false, Ordering::Release);
            current.clone()
        };
        // Persist before waking waiters so they never read a stale record.
        let persisted = self.persist_current();
        self.changed.notify_waiters();
        persisted.map(|()| Some(resumed))
    }

    /// Whether a committed restore still owes plugin artifact recovery.
    pub fn plugin_recovery_pending(&self) -> bool {
        self.snapshot().is_some_and(|status| {
            status.phase == RestoreJobPhase::Complete && status.plugin_recovery_pending
        })
    }

    /// Record that restored plugins were recovered so no later start repeats it.
    pub fn finish_plugin_recovery(&self) -> AppResult<()> {
        {
            let mut state = self.state.lock().expect("restore status lock");
            match state.as_mut() {
                Some(status) if status.plugin_recovery_pending => {
                    status.plugin_recovery_pending = false;
                }
                _ => return Ok(()),
            }
        }
        self.persist_current()
    }

    pub fn snapshot(&self) -> Option<RestoreJobStatus> {
        let mut status = self.state.lock().expect("restore status lock").clone()?;
        if let Some(deadline) = self
            .deadline
            .lock()
            .expect("restore deadline lock")
            .as_ref()
        {
            status.import_percent = deadline.import_percent().or(status.import_percent);
        }
        Some(status)
    }
    pub fn can_retry(&self) -> bool {
        !self.worker_running.load(Ordering::Acquire)
            && !self.shutting_down.load(Ordering::Acquire)
            && self.snapshot().is_some_and(|status| {
                matches!(
                    status.phase,
                    RestoreJobPhase::Failed | RestoreJobPhase::Interrupted
                ) && status.reason.as_deref() != Some(restore_refusal::RESTORE_RECOVERY_REQUIRED)
            })
    }

    pub fn track_deadline(&self, deadline: &RestoreDeadline) {
        *self.deadline.lock().expect("restore deadline lock") = Some(deadline.clone());
        if self.shutting_down.load(Ordering::Acquire) {
            deadline.cancel();
        }
    }

    /// How to restart the process after an Apply fails in-process. Its drain
    /// already stopped the background workers, so only a restart brings the
    /// application fully back.
    pub fn install_failure_restart(&self, restart: impl Fn() + Send + Sync + 'static) {
        *self.failure_restart.lock().expect("restore restart lock") = Some(Arc::new(restart));
    }

    /// Keep the listener alive while accepted work cancels and rolls back.
    pub async fn drain_for_shutdown(&self) {
        {
            let _state = self.state.lock().expect("restore status lock");
            self.shutting_down.store(true, Ordering::Release);
        }
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(deadline) = self
                .deadline
                .lock()
                .expect("restore deadline lock")
                .as_ref()
            {
                deadline.cancel();
            }
            if !self.worker_running.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
    pub fn blocks_application(&self) -> bool {
        self.exclusive.load(Ordering::Acquire)
    }
    pub fn summary(&self) -> Option<BackupBundleInspectSummary> {
        self.summary.lock().expect("restore summary lock").clone()
    }
    pub fn set_summary(&self, value: BackupBundleInspectSummary) {
        *self.summary.lock().expect("restore summary lock") = Some(value);
    }
    /// Only a panic marks the worker set unclean; cancellation at runtime
    /// shutdown is an ordinary exit.
    pub fn register_worker(&self, worker: tokio::task::JoinHandle<()>) {
        let failed = self.worker_failed.clone();
        self.workers.spawn(async move {
            if worker.await.is_err_and(|error| error.is_panic()) {
                failed.store(true, Ordering::Release);
            }
        });
    }

    /// Track detached work as well as the poller or request which starts it.
    ///
    /// A panic marks the worker set unclean so a restore refuses to proceed.
    /// An intentional `abort()` or shutdown cancellation drops the future at
    /// an await point, which is an ordinary exit and never marks it.
    pub fn spawn_task<F>(&self, work: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let failed = self.worker_failed.clone();
        self.workers.spawn(async move {
            match std::panic::AssertUnwindSafe(work).catch_unwind().await {
                Ok(output) => output,
                Err(panic) => {
                    failed.store(true, Ordering::Release);
                    std::panic::resume_unwind(panic)
                }
            }
        })
    }

    pub fn spawn_worker(
        self: &Arc<Self>,
        work: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let owner = self.clone();
        self.register_worker(tokio::spawn(async move {
            loop {
                let changed = owner.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if owner.shutdown_workers.is_cancelled() {
                    return;
                }
                if !owner.blocks_application() {
                    break;
                }
                tokio::select! {
                    _ = changed => {},
                    _ = owner.shutdown_workers.cancelled() => return,
                }
            }
            work.await;
        }));
    }

    pub async fn admit_request(&self) -> AppResult<OwnedRwLockReadGuard<()>> {
        if self.blocks_application() {
            return Err(Self::maintenance_error());
        }
        let guard = self.requests.clone().read_owned().await;
        if self.blocks_application() {
            return Err(Self::maintenance_error());
        }
        Ok(guard)
    }
    pub fn maintenance_error() -> AppError {
        AppError::validation_refused(
            restore_refusal::RESTORE_MAINTENANCE_ACTIVE,
            "The application is unavailable while restore or recovery is in progress",
        )
    }

    pub fn start(
        &self,
        data_dir: &Path,
        upload_id: &str,
        kind: RestoreJobKind,
    ) -> AppResult<(RestoreJobStatus, bool)> {
        let path = data_dir.join(RESTORE_JOB_FILENAME);
        let (status, previous, previous_summary) = {
            let mut state = self.state.lock().expect("restore status lock");
            if self.shutting_down.load(Ordering::Acquire) {
                return Err(Self::maintenance_error());
            }
            if state.as_ref().is_some_and(|status| {
                status.reason.as_deref() == Some(restore_refusal::RESTORE_RECOVERY_REQUIRED)
            }) {
                return Err(AppError::validation_refused(
                    restore_refusal::RESTORE_RECOVERY_REQUIRED,
                    "Recovery must be completed before another restore can start",
                ));
            }
            if let Some(active) = state.as_ref().filter(|s| s.phase.active()) {
                return if active.upload_id == upload_id && active.kind == kind {
                    Ok((active.clone(), false))
                } else {
                    Err(Self::maintenance_error())
                };
            }
            let started_at = Utc::now();
            let status = RestoreJobStatus {
                id: uuid::Uuid::new_v4().to_string(),
                upload_id: upload_id.to_owned(),
                kind,
                phase: if kind == RestoreJobKind::Inspect {
                    RestoreJobPhase::Inspecting
                } else {
                    RestoreJobPhase::Draining
                },
                started_at,
                expires_at: started_at
                    + chrono::Duration::from_std(RESTORE_WORK_BUDGET)
                        .expect("bounded restore budget"),
                reason: None,
                import_percent: None,
                pid: Some(std::process::id()),
                plugin_recovery_pending: state
                    .as_ref()
                    .is_some_and(|previous| previous.plugin_recovery_pending),
            };
            *self.path.lock().expect("restore path lock") = Some(path.clone());
            let mut summary = self.summary.lock().expect("restore summary lock");
            let previous_summary = summary.clone();
            if kind == RestoreJobKind::Inspect
                || state.as_ref().is_none_or(|s| s.upload_id != upload_id)
            {
                *summary = None;
            }
            drop(summary);
            let previous = state.replace(status.clone());
            self.worker_running.store(true, Ordering::Release);
            *self.deadline.lock().expect("restore deadline lock") = None;
            self.exclusive
                .store(holds_application(&status), Ordering::Release);
            (status, previous, previous_summary)
        };
        if let Err(error) = self.persist_current() {
            // Nothing was handed to a worker; return to the previous record.
            let mut state = self.state.lock().expect("restore status lock");
            if state
                .as_ref()
                .is_some_and(|current| current.id == status.id)
            {
                self.exclusive.store(
                    previous.as_ref().is_some_and(holds_application),
                    Ordering::Release,
                );
                self.worker_running.store(false, Ordering::Release);
                *state = previous;
                *self.summary.lock().expect("restore summary lock") = previous_summary;
            }
            drop(state);
            self.changed.notify_waiters();
            return Err(error);
        }
        self.changed.notify_waiters();
        Ok((status, true))
    }

    pub fn transition(
        &self,
        id: &str,
        phase: RestoreJobPhase,
        reason: Option<&str>,
    ) -> AppResult<()> {
        {
            let mut state = self.state.lock().expect("restore status lock");
            let current = state.as_mut().filter(|s| s.id == id).ok_or_else(|| {
                AppError::validation_refused(
                    restore_refusal::RESTORE_ATTEMPT_NOT_FOUND,
                    "Restore attempt does not exist",
                )
            })?;
            current.phase = phase;
            current.reason = reason.map(str::to_owned);
            if let Some(deadline) = self
                .deadline
                .lock()
                .expect("restore deadline lock")
                .as_ref()
            {
                current.import_percent = deadline.import_percent().or(current.import_percent);
            }
            self.exclusive
                .store(holds_application(current), Ordering::Release);
            if matches!(
                phase,
                RestoreJobPhase::Complete
                    | RestoreJobPhase::Failed
                    | RestoreJobPhase::Interrupted
                    | RestoreJobPhase::Restarting
            ) {
                self.worker_running.store(false, Ordering::Release);
            }
        }
        // Persist before waking waiters so they never read a stale record.
        let persisted = self.persist_current();
        self.changed.notify_waiters();
        persisted
    }

    /// Write the latest record. Concurrent writers serialize here and each
    /// writes the state current at its turn, so the file never regresses.
    fn persist_current(&self) -> AppResult<()> {
        let _writer = self.persist.lock().expect("restore persist lock");
        let Some(path) = self.path.lock().expect("restore path lock").clone() else {
            return Ok(());
        };
        let Some(status) = self.state.lock().expect("restore status lock").clone() else {
            return Ok(());
        };
        persist_status(&path, &status)
    }

    /// A failed promotion cannot certify rollback or a usable restored
    /// datastore. Keep access closed, but record that the startup worker has
    /// terminated. After promotion commits this is a no-op.
    pub fn fail_startup(&self) {
        if self.blocks_application()
            && let Some(status) = self.snapshot()
            && let Err(error) = self.transition(
                &status.id,
                RestoreJobPhase::Failed,
                Some(restore_refusal::RESTORE_RECOVERY_REQUIRED),
            )
        {
            tracing::error!(%error, "could not persist restore recovery status");
        }
    }

    /// Record a failed worker. A failed Apply that did not need recovery left
    /// the live data untouched: it releases the application and, because its
    /// drain stopped the background workers, restarts the process once the
    /// tracked work has finished.
    pub fn fail(&self, id: &str, deadline: &RestoreDeadline, error: AppError) {
        tracing::error!(attempt_id = id, error = %error, "restore worker failed");
        let shutting_down = self.shutting_down.load(Ordering::Acquire);
        let reason =
            if error.validation_reason() == Some(restore_refusal::RESTORE_RECOVERY_REQUIRED) {
                restore_refusal::RESTORE_RECOVERY_REQUIRED
            } else if shutting_down {
                restore_refusal::RESTORE_INTERRUPTED
            } else if deadline.check().is_err() {
                restore_refusal::RESTORE_DEADLINE_EXCEEDED
            } else {
                error
                    .validation_reason()
                    .unwrap_or(restore_refusal::RESTORE_INTERRUPTED)
            };
        let apply = self
            .snapshot()
            .is_some_and(|status| status.id == id && status.kind == RestoreJobKind::Apply);
        if let Err(error) = self.transition(id, RestoreJobPhase::Failed, Some(reason)) {
            tracing::error!(attempt_id = id, error = %error, "could not persist failed restore status");
        }
        if apply
            && reason != restore_refusal::RESTORE_RECOVERY_REQUIRED
            && self.shutdown_workers.is_cancelled()
        {
            self.restart_after_tracked_work();
        }
    }

    fn restart_after_tracked_work(&self) {
        let Some(restart) = self
            .failure_restart
            .lock()
            .expect("restore restart lock")
            .clone()
        else {
            tracing::warn!("restore failed after draining workers; restart Scryer to resume them");
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("restore failed after draining workers; restart Scryer to resume them");
            return;
        };
        let workers = self.workers.clone();
        let shutting_down = self.shutting_down.clone();
        workers.close();
        runtime.spawn(async move {
            // Never cut tracked work short: its return proves it drained.
            workers.wait().await;
            if shutting_down.load(Ordering::Acquire) {
                return;
            }
            tracing::info!("restarting to resume background workers after a failed restore");
            restart();
        });
    }

    /// Expiry requests cancellation but never drops accepted blocking work.
    pub async fn supervise<T>(
        &self,
        id: &str,
        deadline: &RestoreDeadline,
        work: impl std::future::Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        self.track_deadline(deadline);
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = deadline.expired() => {
                let transition = self.transition(id, RestoreJobPhase::Cancelling, Some(restore_refusal::RESTORE_DEADLINE_EXCEEDED));
                if let Err(error) = work.await
                    && error.validation_reason() == Some(restore_refusal::RESTORE_RECOVERY_REQUIRED) {
                    return Err(error);
                }
                transition?;
                Err(RestoreDeadline::expired_error())
            },
            result = &mut work => result,
        }
    }

    /// Stop admission and wait for tracked work, bounded by the restore
    /// deadline. Task handles are never aborted: work still running when the
    /// deadline passes keeps running, and the restore gives up instead of
    /// proceeding past it.
    pub async fn drain(&self, deadline: &RestoreDeadline) -> AppResult<()> {
        self.shutdown_workers.cancel();
        let drained = async {
            let requests = self.requests.write().await;
            self.workers.close();
            self.workers.wait().await;
            drop(requests);
        };
        tokio::select! {
            biased;
            _ = drained => {},
            _ = deadline.expired() => {
                self.workers.close();
                tracing::error!(
                    remaining_tasks = self.workers.len(),
                    "background work did not stop before the restore deadline; the restore will not proceed"
                );
                return Err(RestoreDeadline::expired_error());
            }
        }
        if self.worker_failed.load(Ordering::Acquire) {
            return Err(AppError::validation_refused(
                restore_refusal::RESTORE_INTERRUPTED,
                "A background worker failed while draining; restore cannot safely proceed",
            ));
        }
        deadline.check()
    }
}

pub fn read_restore_job(data_dir: &Path) -> AppResult<Option<RestoreJobStatus>> {
    let path = data_dir.join(RESTORE_JOB_FILENAME);
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| AppError::Repository(format!("invalid restore status record: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::Repository(format!(
            "could not read restore status: {e}"
        ))),
    }
}

fn persist_status(path: &Path, status: &RestoreJobStatus) -> AppResult<()> {
    use std::io::Write;
    let directory = path.parent().expect("restore status parent");
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .map_err(|e| AppError::Repository(format!("could not create restore status: {e}")))?;
    #[cfg(windows)]
    protect_restore_record(file.path())
        .map_err(|e| AppError::Repository(format!("could not protect restore status: {e}")))?;
    serde_json::to_writer(&mut file, status)
        .map_err(|e| AppError::Repository(format!("could not encode restore status: {e}")))?;
    file.flush()
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| AppError::Repository(format!("could not flush restore status: {e}")))?;
    file.persist(path).map_err(|e| {
        AppError::Repository(format!("could not persist restore status: {}", e.error))
    })?;
    Ok(())
}

#[cfg(windows)]
fn protect_restore_record(path: &Path) -> std::io::Result<()> {
    use std::os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Security::{
            ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAce, DACL_SECURITY_INFORMATION,
            GetLengthSid, GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor,
            PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_DESCRIPTOR, SetFileSecurityW,
            SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::FILE_ALL_ACCESS,
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    fn checked(result: i32) -> std::io::Result<()> {
        if result == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    let mut token = std::ptr::null_mut();
    // SAFETY: Windows initializes this out-handle; OwnedHandle closes it on every exit.
    unsafe {
        checked(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        ))?;
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut required = 0;
    // SAFETY: A null buffer asks Windows for the required size only.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut required,
        );
    }
    if required < std::mem::size_of::<TOKEN_USER>() as u32 {
        return Err(std::io::Error::last_os_error());
    }
    let mut user = vec![0_usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: The aligned buffer holds `required` bytes; the SID remains valid
    // until AddAccessAllowedAce copies it into our independently owned ACL.
    unsafe {
        checked(GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            user.as_mut_ptr().cast(),
            required,
            &mut required,
        ))?;
        let sid = (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let acl_size = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            - std::mem::size_of::<u32>()
            + GetLengthSid(sid) as usize;
        let mut acl = vec![0_u32; acl_size.div_ceil(std::mem::size_of::<u32>())];
        let acl_ptr = acl.as_mut_ptr().cast::<ACL>();
        checked(InitializeAcl(acl_ptr, acl_size as u32, ACL_REVISION))?;
        checked(AddAccessAllowedAce(
            acl_ptr,
            ACL_REVISION,
            FILE_ALL_ACCESS,
            sid,
        ))?;
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
        checked(InitializeSecurityDescriptor(descriptor_ptr, 1))?;
        checked(SetSecurityDescriptorDacl(descriptor_ptr, 1, acl_ptr, 0))?;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        checked(SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor_ptr,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn background_restore_refuses_after_child_task_panic() {
        let coordinator = RestoreCoordinator::default();
        let task = coordinator.spawn_task(async { panic!("synthetic worker failure") });
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(60), task)
                .await
                .unwrap()
                .unwrap_err()
                .is_panic()
        );
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            coordinator.drain(&RestoreDeadline::new()),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(
            error.validation_reason(),
            Some(restore_refusal::RESTORE_INTERRUPTED)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_drains_children_registered_after_drain_starts() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(RestoreCoordinator::default());
        let (release_parent, parent_ready) = tokio::sync::oneshot::channel();
        let (release_child, child_ready) = tokio::sync::oneshot::channel();
        let (child_started, started) = tokio::sync::oneshot::channel();
        let parent = coordinator.clone();
        coordinator.spawn_task(async move {
            parent_ready.await.unwrap();
            parent.spawn_task(async move {
                child_ready.await.unwrap();
            });
            child_started.send(()).unwrap();
        });
        coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        let deadline = RestoreDeadline::new();
        let drain = coordinator.drain(&deadline);
        tokio::pin!(drain);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(drain.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        release_parent.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), started)
            .await
            .unwrap()
            .unwrap();
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(drain.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(coordinator.admit_request().await.is_err());
        release_child.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), drain)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_failed_startup_releases_worker_but_keeps_recovery_guard() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        coordinator
            .transition(&job.id, RestoreJobPhase::Restarting, None)
            .unwrap();
        let recovered = RestoreCoordinator::recover(directory.path()).unwrap();
        recovered.fail_startup();
        assert!(recovered.blocks_application());
        assert!(!recovered.can_retry());
        assert!(
            recovered
                .start(directory.path(), "fixture", RestoreJobKind::Apply)
                .is_err()
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            recovered.drain_for_shutdown(),
        )
        .await
        .unwrap();
        assert_eq!(
            read_restore_job(directory.path())
                .unwrap()
                .unwrap()
                .reason
                .as_deref(),
            Some(restore_refusal::RESTORE_RECOVERY_REQUIRED)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_drain_is_bounded_without_aborting_and_restarts_after_work_exits() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(RestoreCoordinator::default());
        let (restarted, restart_seen) = tokio::sync::oneshot::channel();
        let restarted = Mutex::new(Some(restarted));
        coordinator.install_failure_restart(move || {
            if let Some(sender) = restarted.lock().unwrap().take() {
                let _ = sender.send(());
            }
        });
        let lease = coordinator.admit_request().await.unwrap();
        let (status, fresh) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        assert!(fresh);
        assert!(coordinator.admit_request().await.is_err());
        // A tracked task that ignores cancellation must not hold the restore
        // past its deadline, and must not be aborted either.
        let (exiting, exited) = tokio::sync::oneshot::channel::<()>();
        coordinator.register_worker(tokio::spawn(async move {
            let _ = exited.await;
        }));
        let deadline = RestoreDeadline::new();
        let drain = coordinator.drain(&deadline);
        tokio::pin!(drain);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(drain.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(coordinator.blocks_application());
        tokio::time::advance(crate::RESTORE_WORK_BUDGET).await;
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(60), drain)
                .await
                .unwrap()
                .unwrap_err()
                .validation_reason(),
            Some(restore_refusal::RESTORE_DEADLINE_EXCEEDED)
        );
        drop(lease);
        coordinator.fail(&status.id, &deadline, RestoreDeadline::expired_error());
        let failed = read_restore_job(directory.path()).unwrap().unwrap();
        assert_eq!(failed.phase, RestoreJobPhase::Failed);
        assert_eq!(
            failed.reason.as_deref(),
            Some(restore_refusal::RESTORE_DEADLINE_EXCEEDED)
        );
        // Nothing was promoted, so the application is released at once.
        assert!(!coordinator.blocks_application());
        assert!(coordinator.admit_request().await.is_ok());
        // The ignoring task is still alive; the restart waits for it.
        exiting.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), restart_seen)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_abort_is_clean_but_panic_marks_workers_failed() {
        let coordinator = RestoreCoordinator::default();
        let task = coordinator.spawn_task(std::future::pending::<()>());
        task.abort();
        let aborted = tokio::time::timeout(std::time::Duration::from_secs(60), task)
            .await
            .unwrap()
            .unwrap_err();
        assert!(aborted.is_cancelled());
        assert!(!coordinator.worker_failed.load(Ordering::Acquire));

        let (started, ready) = tokio::sync::oneshot::channel();
        let task = coordinator.spawn_task(async move {
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(60), ready)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(60), task)
                .await
                .unwrap()
                .unwrap_err()
                .is_cancelled()
        );
        assert!(!coordinator.worker_failed.load(Ordering::Acquire));

        let task = coordinator.spawn_task(async { panic!("synthetic worker failure") });
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(60), task)
                .await
                .unwrap()
                .unwrap_err()
                .is_panic()
        );
        assert!(coordinator.worker_failed.load(Ordering::Acquire));
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_shutdown_records_interruption_without_restart() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let restarted = Arc::new(AtomicBool::new(false));
        let observed = restarted.clone();
        coordinator.install_failure_restart(move || observed.store(true, Ordering::Release));
        let (job, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        let deadline = RestoreDeadline::new();
        coordinator.track_deadline(&deadline);
        coordinator.shutdown_workers.cancel();
        let shutdown = coordinator.drain_for_shutdown();
        tokio::pin!(shutdown);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(shutdown.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        coordinator.fail(&job.id, &deadline, RestoreDeadline::expired_error());
        tokio::time::timeout(std::time::Duration::from_secs(60), shutdown)
            .await
            .unwrap();
        let status = read_restore_job(directory.path()).unwrap().unwrap();
        assert_eq!(status.phase, RestoreJobPhase::Failed);
        assert_eq!(
            status.reason.as_deref(),
            Some(restore_refusal::RESTORE_INTERRUPTED)
        );
        assert!(!restarted.load(Ordering::Acquire));
    }

    #[test]
    fn background_restore_duplicate_start_retains_attempt_and_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let (original, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        let (duplicate, fresh) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        assert!(!fresh);
        assert_eq!(original.id, duplicate.id);
        assert_eq!(original.expires_at, duplicate.expires_at);
        assert!(
            coordinator
                .start(directory.path(), "other", RestoreJobKind::Apply)
                .is_err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_supervisor_waits_for_cancelled_worker() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        let deadline = RestoreDeadline::new();
        let (release, released) = tokio::sync::oneshot::channel();
        let work = async {
            released.await.unwrap();
            Ok::<_, AppError>(())
        };
        let supervised = coordinator.supervise(&job.id, &deadline, work);
        tokio::pin!(supervised);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(supervised.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        tokio::time::advance(crate::RESTORE_WORK_BUDGET).await;
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(supervised.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert_eq!(
            coordinator.snapshot().unwrap().phase,
            RestoreJobPhase::Cancelling
        );
        assert!(coordinator.blocks_application());
        assert!(
            coordinator
                .start(directory.path(), "other", RestoreJobKind::Apply)
                .is_err()
        );
        release.send(()).unwrap();
        assert_eq!(
            supervised.await.unwrap_err().validation_reason(),
            Some(restore_refusal::RESTORE_DEADLINE_EXCEEDED)
        );
    }

    #[test]
    fn background_restore_restart_retains_deadline_and_interrupts_unhanded_work() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        let recovered = RestoreCoordinator::recover(directory.path()).unwrap();
        assert_eq!(
            recovered.snapshot().unwrap().phase,
            RestoreJobPhase::Interrupted
        );
        // Promotion only happens at startup, so interrupted work left the
        // live data intact and the instance boots normally.
        assert!(!recovered.blocks_application());
        assert!(recovered.can_retry());
        coordinator
            .transition(&job.id, RestoreJobPhase::Restarting, None)
            .unwrap();
        let recovered = RestoreCoordinator::recover(directory.path()).unwrap();
        assert_eq!(recovered.snapshot().unwrap().expires_at, job.expires_at);
        assert_eq!(recovered.snapshot().unwrap().pid, Some(std::process::id()));
        assert!(recovered.blocks_application());
        recovered.commit_promoted_restore(true).unwrap();
        assert!(!recovered.blocks_application());
        let committed = read_restore_job(directory.path()).unwrap().unwrap();
        assert_eq!(committed.phase, RestoreJobPhase::Complete);
        assert!(committed.plugin_recovery_pending);
        assert!(recovered.plugin_recovery_pending());
        recovered.finish_plugin_recovery().unwrap();
        assert!(
            !read_restore_job(directory.path())
                .unwrap()
                .unwrap()
                .plugin_recovery_pending
        );
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_shutdown_after_promotion_never_requires_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        coordinator
            .transition(&job.id, RestoreJobPhase::Restarting, None)
            .unwrap();
        let recovered = RestoreCoordinator::recover(directory.path()).unwrap();
        let deadline = RestoreDeadline::resume(job.expires_at);
        recovered.track_deadline(&deadline);
        recovered.commit_promoted_restore(true).unwrap();
        // Shutdown during the rest of startup, then a failed bootstrap.
        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            recovered.drain_for_shutdown(),
        )
        .await
        .unwrap();
        assert!(deadline.check().is_ok());
        recovered.fail_startup();
        let status = read_restore_job(directory.path()).unwrap().unwrap();
        assert_eq!(status.phase, RestoreJobPhase::Complete);
        assert_eq!(status.reason, None);
        let next_boot = RestoreCoordinator::recover(directory.path()).unwrap();
        assert!(!next_boot.blocks_application());
    }

    #[test]
    fn background_restore_failed_apply_boots_normally_and_allows_new_inspect() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        assert!(coordinator.blocks_application());
        coordinator.fail(
            &job.id,
            &RestoreDeadline::new(),
            AppError::validation_refused("BACKUP_PASSWORD_INVALID", "wrong password"),
        );
        assert!(!coordinator.blocks_application());
        let recovered = RestoreCoordinator::recover(directory.path()).unwrap();
        assert!(!recovered.blocks_application());
        let readable = recovered.snapshot().unwrap();
        assert_eq!(readable.phase, RestoreJobPhase::Failed);
        assert_eq!(readable.reason.as_deref(), Some("BACKUP_PASSWORD_INVALID"));
        let (inspect, fresh) = coordinator
            .start(directory.path(), "other", RestoreJobKind::Inspect)
            .unwrap();
        assert!(fresh);
        assert!(!coordinator.blocks_application());
        coordinator
            .transition(&inspect.id, RestoreJobPhase::Complete, None)
            .unwrap();
        assert!(!coordinator.blocks_application());
        // Only an unconfirmed promotion keeps the guard across restarts.
        let (apply, _) = coordinator
            .start(directory.path(), "other", RestoreJobKind::Apply)
            .unwrap();
        coordinator
            .transition(
                &apply.id,
                RestoreJobPhase::Failed,
                Some(restore_refusal::RESTORE_RECOVERY_REQUIRED),
            )
            .unwrap();
        assert!(coordinator.blocks_application());
        assert!(
            RestoreCoordinator::recover(directory.path())
                .unwrap()
                .blocks_application()
        );
    }

    #[test]
    fn background_restore_failed_restart_handoff_is_committed_by_promotion() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(directory.path(), "fixture", RestoreJobKind::Apply)
            .unwrap();
        coordinator
            .transition(&job.id, RestoreJobPhase::Restarting, None)
            .unwrap();
        coordinator
            .transition(
                &job.id,
                RestoreJobPhase::Failed,
                Some(restore_refusal::RESTORE_RESTART_FAILED),
            )
            .unwrap();
        let recovered = RestoreCoordinator::recover(directory.path()).unwrap();
        assert!(!recovered.blocks_application());
        assert!(recovered.snapshot().unwrap().awaits_promotion());
        // Nothing promoted: the failed handoff record stays as it was.
        recovered.commit_promoted_restore(false).unwrap();
        let unchanged = read_restore_job(directory.path()).unwrap().unwrap();
        assert_eq!(unchanged.phase, RestoreJobPhase::Failed);
        assert_eq!(
            unchanged.reason.as_deref(),
            Some(restore_refusal::RESTORE_RESTART_FAILED)
        );
        // Promoted: the record commits like a Restarting one.
        recovered.commit_promoted_restore(true).unwrap();
        assert!(!recovered.blocks_application());
        let committed = read_restore_job(directory.path()).unwrap().unwrap();
        assert_eq!(committed.phase, RestoreJobPhase::Complete);
        assert_eq!(committed.reason, None);
        assert!(committed.plugin_recovery_pending);
        assert!(!committed.awaits_promotion());
    }

    #[test]
    fn background_restore_failed_start_persist_keeps_previous_record() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = RestoreCoordinator::default();
        let missing = directory.path().join("missing");
        assert!(
            coordinator
                .start(&missing, "fixture", RestoreJobKind::Apply)
                .is_err()
        );
        assert!(coordinator.snapshot().is_none());
        assert!(!coordinator.blocks_application());
        assert!(
            coordinator
                .start(directory.path(), "fixture", RestoreJobKind::Apply)
                .unwrap()
                .1
        );
    }
}
