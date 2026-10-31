//! Restore work owns its deadline independently of any transport future.
use crate::{AppError, AppResult};
use chrono::{DateTime, Utc};
use std::{
    cell::RefCell,
    future::Future,
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// One accepted inspection or application gets this total work budget.
pub const RESTORE_WORK_BUDGET: Duration = Duration::from_secs(3_600);

pub mod refusal {
    /// The restore exhausted its total work budget.
    pub const RESTORE_DEADLINE_EXCEEDED: &str = "RESTORE_DEADLINE_EXCEEDED";
    /// Restore owns the application until its worker and recovery finish.
    pub const RESTORE_MAINTENANCE_ACTIVE: &str = "RESTORE_MAINTENANCE_ACTIVE";
    /// The previous process stopped before the restart handoff was committed.
    pub const RESTORE_INTERRUPTED: &str = "RESTORE_INTERRUPTED";
    /// The requested restore attempt does not exist.
    pub const RESTORE_ATTEMPT_NOT_FOUND: &str = "RESTORE_ATTEMPT_NOT_FOUND";
    /// Rollback or promotion could not be confirmed safe for another attempt.
    pub const RESTORE_RECOVERY_REQUIRED: &str = "RESTORE_RECOVERY_REQUIRED";
    /// Restore control cannot be mixed with ordinary or ambiguous operations.
    pub const RESTORE_OPERATION_INVALID: &str = "RESTORE_OPERATION_INVALID";
    /// The prepared restore could not launch its restart handoff.
    pub const RESTORE_RESTART_FAILED: &str = "RESTORE_RESTART_FAILED";
    pub const ALL: &[&str] = &[
        RESTORE_DEADLINE_EXCEEDED,
        RESTORE_MAINTENANCE_ACTIVE,
        RESTORE_INTERRUPTED,
        RESTORE_ATTEMPT_NOT_FOUND,
        RESTORE_RECOVERY_REQUIRED,
        RESTORE_OPERATION_INVALID,
        RESTORE_RESTART_FAILED,
    ];
}

#[derive(Clone, Debug)]
pub struct RestoreDeadline {
    expires: Instant,
    expires_at: DateTime<Utc>,
    cancellation: CancellationToken,
    import_progress: Arc<RestoreImportProgress>,
}

/// Rough row-weighted import progress; no datastore queries or per-row I/O.
#[derive(Debug, Default)]
struct RestoreImportProgress {
    started: AtomicBool,
    complete: AtomicBool,
    total: AtomicU64,
    imported: AtomicU64,
}

tokio::task_local! { static ASYNC_DEADLINE: RestoreDeadline; }
thread_local! { static BLOCKING_DEADLINE: RefCell<Option<RestoreDeadline>> = const { RefCell::new(None) }; }

impl RestoreDeadline {
    pub fn new() -> Self {
        Self::with_budget(RESTORE_WORK_BUDGET)
    }

    pub fn with_budget(budget: Duration) -> Self {
        Self {
            expires: Instant::now() + budget,
            expires_at: Utc::now()
                + chrono::Duration::from_std(budget).expect("bounded restore budget"),
            cancellation: CancellationToken::new(),
            import_progress: Arc::default(),
        }
    }

    /// Restart consumes the original budget; it never grants a fresh hour.
    pub fn resume(expires_at: DateTime<Utc>) -> Self {
        let remaining = (expires_at - Utc::now())
            .to_std()
            .unwrap_or_default()
            .min(RESTORE_WORK_BUDGET);
        Self {
            expires: Instant::now() + remaining,
            expires_at,
            cancellation: CancellationToken::new(),
            import_progress: Arc::default(),
        }
    }

    pub fn begin_import(&self, total_rows: u64) {
        self.import_progress
            .total
            .store(total_rows, Ordering::Relaxed);
        self.import_progress.imported.store(0, Ordering::Relaxed);
        self.import_progress
            .complete
            .store(false, Ordering::Relaxed);
        self.import_progress.started.store(true, Ordering::Release);
    }
    pub fn imported_row(&self) {
        self.import_progress
            .imported
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn finish_import(&self) {
        self.import_progress.complete.store(true, Ordering::Release);
    }
    pub fn import_percent(&self) -> Option<u8> {
        if !self.import_progress.started.load(Ordering::Acquire) {
            return None;
        }
        if self.import_progress.complete.load(Ordering::Acquire) {
            return Some(100);
        }
        let total = self.import_progress.total.load(Ordering::Relaxed);
        let imported = self.import_progress.imported.load(Ordering::Relaxed);
        Some(if total == 0 {
            0
        } else {
            ((u128::from(imported) * 100 / u128::from(total)).min(99)) as u8
        })
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
    pub fn remaining(&self) -> Duration {
        self.expires.saturating_duration_since(Instant::now())
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn check(&self) -> AppResult<()> {
        if self.cancellation.is_cancelled() || Instant::now() >= self.expires {
            self.cancel();
            return Err(Self::expired_error());
        }
        Ok(())
    }
    pub fn expired_error() -> AppError {
        AppError::validation_refused(
            refusal::RESTORE_DEADLINE_EXCEEDED,
            "Restore exceeded its one-hour work budget; waiting for safe rollback and cleanup",
        )
    }
    pub async fn scope<F: Future>(&self, future: F) -> F::Output {
        ASYNC_DEADLINE.scope(self.clone(), future).await
    }
    pub async fn expired(&self) {
        tokio::select! {
            _ = self.cancellation.cancelled() => {},
            _ = tokio::time::sleep_until(self.expires) => self.cancel(),
        }
    }
    pub fn scope_blocking<T>(&self, work: impl FnOnce() -> T) -> T {
        struct Reset(Option<RestoreDeadline>);
        impl Drop for Reset {
            fn drop(&mut self) {
                BLOCKING_DEADLINE.with(|slot| *slot.borrow_mut() = self.0.take());
            }
        }
        let _reset = Reset(BLOCKING_DEADLINE.with(|slot| slot.replace(Some(self.clone()))));
        work()
    }
    pub async fn run<F, T>(&self, future: F) -> AppResult<T>
    where
        F: Future<Output = AppResult<T>>,
    {
        self.check()?;
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(Self::expired_error()),
            _ = tokio::time::sleep_until(self.expires) => { self.cancel(); Err(Self::expired_error()) },
            result = future => result,
        }
    }
}
impl Default for RestoreDeadline {
    fn default() -> Self {
        Self::new()
    }
}

pub fn current_restore_deadline() -> Option<RestoreDeadline> {
    ASYNC_DEADLINE
        .try_with(Clone::clone)
        .ok()
        .or_else(|| BLOCKING_DEADLINE.with(|slot| slot.borrow().clone()))
}
pub fn check_restore_deadline() -> AppResult<()> {
    current_restore_deadline().map_or(Ok(()), |deadline| deadline.check())
}

/// Bounds synchronous decoder/extractor work without abandoning its worker.
pub struct RestoreReader<R> {
    inner: R,
    deadline: Option<RestoreDeadline>,
}
impl<R> RestoreReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            deadline: current_restore_deadline(),
        }
    }
}
impl<R: Read> Read for RestoreReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if let Some(deadline) = &self.deadline {
            deadline
                .check()
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::TimedOut, error))?;
        }
        let length = bytes.len().min(64 * 1024);
        self.inner.read(&mut bytes[..length])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn background_restore_import_percentage_is_row_weighted_and_shared() {
        let deadline = RestoreDeadline::new();
        assert_eq!(deadline.import_percent(), None);
        deadline.begin_import(200);
        let worker = deadline.clone();
        for _ in 0..50 {
            worker.imported_row();
        }
        assert_eq!(deadline.import_percent(), Some(25));
        for _ in 0..150 {
            worker.imported_row();
        }
        assert_eq!(deadline.import_percent(), Some(99));
        worker.finish_import();
        assert_eq!(deadline.import_percent(), Some(100));
        deadline.begin_import(0);
        assert_eq!(deadline.import_percent(), Some(0));
        deadline.finish_import();
        assert_eq!(deadline.import_percent(), Some(100));
    }
    #[tokio::test(start_paused = true)]
    async fn background_restore_gets_one_hour_not_the_graphql_budget() {
        let deadline = RestoreDeadline::new();
        tokio::time::advance(Duration::from_secs(310)).await;
        assert!(deadline.check().is_ok());
        tokio::time::advance(Duration::from_secs(3_290)).await;
        assert_eq!(
            deadline.check().unwrap_err().validation_reason(),
            Some(refusal::RESTORE_DEADLINE_EXCEEDED)
        );
    }
    #[test]
    fn background_restore_reasons_are_distinct_and_prefixed() {
        let unique: std::collections::HashSet<_> = refusal::ALL.iter().collect();
        assert_eq!(unique.len(), refusal::ALL.len());
        assert!(
            refusal::ALL.iter().all(|r| r.starts_with("RESTORE_")
                && r.bytes().all(|c| c.is_ascii_uppercase() || c == b'_'))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn background_restore_reader_checks_expiry_between_bounded_chunks() {
        let deadline = RestoreDeadline::new();
        let mut reader = deadline.scope_blocking(|| RestoreReader::new(std::io::repeat(7)));
        let mut output = vec![0; 128 * 1024];
        assert_eq!(reader.read(&mut output).unwrap(), 64 * 1024);
        tokio::time::advance(RESTORE_WORK_BUDGET).await;
        assert_eq!(
            reader.read(&mut output).unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
    }
}
