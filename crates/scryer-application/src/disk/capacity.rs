//! The one place Scryer asks a filesystem how much room it has.
//!
//! Every free-space figure in Scryer comes from [`StorageCapacity`]: the
//! health check, the dashboard, request and lifecycle rules, the import and
//! sidecar guards, archive extraction limits, move previews, the application
//! upgrade check and the diagnostics bundle. It owns when the disk is called
//! (the call itself, the only `statfs` / `statvfs` / `GetDiskFreeSpaceExW` for
//! capacity, is `scryer_disk::capacity::probe`), the timeout around it, the
//! background cadence over library roots, the per-path cache and its staleness
//! rule, and single-flight, so callers asking about one path at once share one
//! call.
//!
//! Library roots live on network shares, where each call is a round trip and a
//! dead mount blocks it outright. So there are two ways to read:
//!
//! - [`StorageCapacity::reading`] never touches the disk. It answers from the
//!   last sample, as a typed [`StorageCapacityReading`]. Anything that runs
//!   while a person waits and only reports a figure uses it.
//! - [`StorageCapacity::refresh`] (async, bounded by a timeout, off the async
//!   runtime) and [`StorageCapacity::refresh_blocking`] (for code already on a
//!   blocking thread) take a figure now, unless a call for the same path started
//!   after the caller asked or finished under [`STORAGE_CAPACITY_REFRESH_REUSE`]
//!   ago. Guards that stop a copy, move or extraction for lack of space use it.
//!
//! [`start_storage_capacity_sampler`] samples every library root once every
//! [`STORAGE_CAPACITY_SAMPLE_INTERVAL`]. Nothing here is persisted and nothing
//! writes to the database: a restart starts empty.
//!
//! The shape follows Weaver's storage capacity owner: background sampling of
//! known roots, readers that take the last completed reading, a reading older
//! than two intervals counted as not current, on-demand looks single-flighted
//! per path with a bounded wait, a bounded cache for other paths, and a test
//! seam that counts disk calls.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Duration, Utc};
use tracing::{info, warn};

use super::{DiskClock, Offloaded, SystemClock, offload, read, reusable, write};
use crate::AppUseCase;
use crate::helpers::FilesystemSpace;
pub(crate) use scryer_disk::capacity::CapacityProbeReading;

/// How often each library root is sampled. A root's free space moves when an
/// import lands or a file is removed, a scale of minutes, and every guard that
/// acts on free space takes its own figure through [`StorageCapacity::refresh`].
/// Five minutes keeps background traffic to a network share at twelve calls per
/// filesystem per hour while a rule's view of a root stays minutes old.
/// Weaver samples every five seconds because a download writes continuously;
/// a library root does not.
pub const STORAGE_CAPACITY_SAMPLE_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(5 * 60);

/// How old a sample may be and still be read: two sampling intervals, as in
/// Weaver, so one late or failed pass does not make every root unknown while a
/// stopped sampler makes them unknown within one more interval.
pub fn storage_sample_max_age() -> Duration {
    Duration::from_std(STORAGE_CAPACITY_SAMPLE_INTERVAL * 2).unwrap_or(Duration::minutes(10))
}

pub use scryer_disk::capacity::{STORAGE_CAPACITY_REFRESH_REUSE, STORAGE_CAPACITY_SAMPLE_TIMEOUT};

/// Every this many passes the sampler calls every root, even those it
/// believes share a filesystem, so a remount or retarget is noticed within an
/// hour at the default interval.
const FULL_PASS_EVERY: u64 = 12;

/// Paths other than library roots kept in the cache at most. Past that, the
/// least recently sampled one with no call outstanding is dropped.
const OTHER_PATHS_MAX: usize = 256;

/// Reasons a reading is not a figure. Stable API: rule inputs carry them.
pub mod reason {
    /// No sample of this root has been taken since startup.
    pub const STORAGE_NOT_SAMPLED: &str = "storage_not_sampled";
    /// The newest sample is older than [`super::storage_sample_max_age`].
    pub const STORAGE_SAMPLE_STALE: &str = "storage_sample_stale";
    /// The newest sample could not read the filesystem: it failed, timed
    /// out, or reported figures that cannot be right (a zero total, or more
    /// available than total). The same code lifecycle storage facts use.
    pub const STORAGE_CAPACITY_UNAVAILABLE: &str = "storage_capacity_unavailable";

    // Read by the distinctness test and by the lifecycle switch.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: &[&str] = &[
        STORAGE_NOT_SAMPLED,
        STORAGE_SAMPLE_STALE,
        STORAGE_CAPACITY_UNAVAILABLE,
    ];
}

/// The disk seam: the capacity call itself. Tests replace it.
pub(crate) trait CapacityDisk: Send + Sync + 'static {
    fn probe(&self, path: &Path) -> io::Result<CapacityProbeReading>;
}

#[derive(Debug, Default)]
struct SystemDisk;

impl CapacityDisk for SystemDisk {
    // disk-guard: the capacity owner is the one caller of the platform's
    // free-space call; it bounds, shares and caches it.
    #[allow(clippy::disallowed_methods)]
    fn probe(&self, path: &Path) -> io::Result<CapacityProbeReading> {
        scryer_disk::capacity::probe(path)
    }
}

/// What may be said about a path's capacity at a given instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageCapacityReading {
    /// No sample of this path has been taken since startup.
    NotSampled,
    /// The newest sample is older than [`storage_sample_max_age`].
    Stale { sampled_at: DateTime<Utc> },
    /// The newest sample could not read the filesystem.
    Unreachable { sampled_at: DateTime<Utc> },
    Known {
        sampled_at: DateTime<Utc>,
        available_bytes: i64,
        total_bytes: i64,
    },
}

impl StorageCapacityReading {
    /// The [`reason`] code for a reading that is not a figure.
    pub fn unknown_reason(self) -> Option<&'static str> {
        match self {
            Self::NotSampled => Some(reason::STORAGE_NOT_SAMPLED),
            Self::Stale { .. } => Some(reason::STORAGE_SAMPLE_STALE),
            Self::Unreachable { .. } => Some(reason::STORAGE_CAPACITY_UNAVAILABLE),
            Self::Known { .. } => None,
        }
    }

    /// Available as a percentage of total, the figure lifecycle rules read.
    pub fn available_percent(self) -> Option<f64> {
        match self {
            Self::Known {
                available_bytes,
                total_bytes,
                ..
            } => Some((available_bytes as f64 * 100.0) / total_bytes as f64),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn sampled_at(self) -> Option<DateTime<Utc>> {
        match self {
            Self::NotSampled => None,
            Self::Stale { sampled_at }
            | Self::Unreachable { sampled_at }
            | Self::Known { sampled_at, .. } => Some(sampled_at),
        }
    }
}

#[derive(Clone, Debug)]
struct ProbeFailure {
    kind: io::ErrorKind,
    message: String,
}

impl ProbeFailure {
    fn from_error(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }

    fn to_error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

#[derive(Clone, Debug)]
struct Sample {
    sampled_at: DateTime<Utc>,
    outcome: Result<CapacityProbeReading, ProbeFailure>,
}

impl Sample {
    fn space(&self) -> io::Result<FilesystemSpace> {
        match &self.outcome {
            Ok(reading) => Ok(reading.space),
            Err(failure) => Err(failure.to_error()),
        }
    }

    /// The figures, when they can be right.
    fn values(&self) -> Option<(i64, i64)> {
        self.outcome
            .as_ref()
            .ok()
            .and_then(|reading| capacity_values(reading.space))
    }

    /// What sampling may group this path's filesystem by.
    fn group(&self) -> Option<(u64, u64)> {
        let reading = self.outcome.as_ref().ok()?;
        capacity_values(reading.space)?;
        Some((reading.filesystem_id?, reading.space.total_bytes))
    }
}

#[derive(Debug, Default)]
struct Entry {
    sample: Option<Sample>,
    /// Held while a call for this path is out. Callers that find it held
    /// wait on it, then answer from the sample the call left.
    flight: Arc<Mutex<()>>,
    /// The path is a library root the sampler keeps current.
    root: bool,
    /// The original spelling of a root, which the sampler calls with.
    path: PathBuf,
}

struct Inner {
    disk: Arc<dyn CapacityDisk>,
    clock: Arc<dyn DiskClock>,
    entries: RwLock<HashMap<String, Entry>>,
    passes: AtomicU64,
    #[cfg(test)]
    calls: AtomicU64,
}

/// The canonical capacity owner. Cheap to clone; clones share one cache.
#[derive(Clone)]
pub struct StorageCapacity {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for StorageCapacity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = read(&self.inner.entries);
        f.debug_struct("StorageCapacity")
            .field("paths", &entries.len())
            .finish_non_exhaustive()
    }
}

fn key_of(path: &Path) -> String {
    scryer_domain::normalize_library_root_path(&path.to_string_lossy())
}

/// The validity rule lifecycle storage facts apply: a positive total and an
/// available figure within it, both representable as `i64`.
fn capacity_values(space: FilesystemSpace) -> Option<(i64, i64)> {
    let total = i64::try_from(space.total_bytes).ok()?;
    let available = i64::try_from(space.available_bytes).ok()?;
    (total > 0 && available >= 0 && available <= total).then_some((available, total))
}

impl StorageCapacity {
    /// The process's capacity owner, on the real filesystem and clock. The
    /// runtime holds this instance, and code that runs below any service
    /// handle (import checks, archive limits, the upgrade check) reaches the
    /// same one here, so every caller shares one cache and one flight per path.
    pub fn process() -> Self {
        static PROCESS: std::sync::OnceLock<StorageCapacity> = std::sync::OnceLock::new();
        PROCESS
            .get_or_init(|| Self::with_seams(Arc::new(SystemDisk), Arc::new(SystemClock)))
            .clone()
    }

    pub(crate) fn with_seams(disk: Arc<dyn CapacityDisk>, clock: Arc<dyn DiskClock>) -> Self {
        Self {
            inner: Arc::new(Inner {
                disk,
                clock,
                entries: RwLock::new(HashMap::new()),
                passes: AtomicU64::new(0),
                #[cfg(test)]
                calls: AtomicU64::new(0),
            }),
        }
    }

    /// Disk calls this owner has made, for tests.
    #[cfg(test)]
    pub(crate) fn disk_calls(&self) -> u64 {
        self.inner.calls.load(Ordering::SeqCst)
    }

    /// Put a sample in the cache as if a call had made it, for tests of
    /// callers that only read.
    #[cfg(test)]
    pub(crate) fn record_for_test(
        &self,
        path: &str,
        sampled_at: DateTime<Utc>,
        space: Option<FilesystemSpace>,
    ) {
        let outcome = space
            .map(|space| CapacityProbeReading {
                space,
                filesystem_id: None,
            })
            .ok_or_else(|| ProbeFailure {
                kind: io::ErrorKind::Other,
                message: "scripted failure".into(),
            });
        self.store(
            &key_of(Path::new(path)),
            Sample {
                sampled_at,
                outcome,
            },
        );
    }

    /// What may be said about `path` at `now`, from the cache alone.
    pub fn reading(&self, path: &str, now: DateTime<Utc>) -> StorageCapacityReading {
        self.reading_with_max_age(path, now, storage_sample_max_age())
    }

    pub fn reading_with_max_age(
        &self,
        path: &str,
        now: DateTime<Utc>,
        max_age: Duration,
    ) -> StorageCapacityReading {
        let key = key_of(Path::new(path));
        let sample = read(&self.inner.entries)
            .get(&key)
            .and_then(|entry| entry.sample.clone());
        let Some(sample) = sample else {
            return StorageCapacityReading::NotSampled;
        };
        let sampled_at = sample.sampled_at;
        if now - sampled_at > max_age {
            return StorageCapacityReading::Stale { sampled_at };
        }
        match sample.values() {
            Some((available_bytes, total_bytes)) => StorageCapacityReading::Known {
                sampled_at,
                available_bytes,
                total_bytes,
            },
            None => StorageCapacityReading::Unreachable { sampled_at },
        }
    }

    /// A current figure for `path`, taken now unless one for this path started
    /// after the caller asked or is under [`STORAGE_CAPACITY_REFRESH_REUSE`]
    /// old. Blocks on the disk call: call it only from a blocking thread.
    pub fn refresh_blocking(&self, path: &Path) -> io::Result<FilesystemSpace> {
        let key = key_of(path);
        let asked_at = self.inner.clock.now();
        let flight = self.flight(&key, path);
        let _out = flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(sample) = self.sample(&key)
            && reusable(sample.sampled_at, asked_at, STORAGE_CAPACITY_REFRESH_REUSE)
        {
            return sample.space();
        }
        let sample = self.call(path);
        let answer = sample.space();
        self.store(&key, sample);
        answer
    }

    /// [`Self::refresh_blocking`] off the async runtime, waiting at most
    /// `timeout`. A call that does not answer in time is recorded as
    /// unreachable; when it does answer later, its sample replaces that.
    pub async fn refresh(
        &self,
        path: impl Into<PathBuf>,
        timeout: std::time::Duration,
    ) -> io::Result<FilesystemSpace> {
        let path = path.into();
        let owner = self.clone();
        let probed = path.clone();
        match offload(Some(timeout), move || owner.refresh_blocking(&probed)).await {
            Offloaded::Done(answer) => answer,
            Offloaded::Failed(join) => Err(io::Error::other(join)),
            Offloaded::TimedOut => {
                let failure = ProbeFailure {
                    kind: io::ErrorKind::TimedOut,
                    message: format!(
                        "the filesystem did not answer within {} ms",
                        timeout.as_millis()
                    ),
                };
                let error = failure.to_error();
                self.store(
                    &key_of(&path),
                    Sample {
                        sampled_at: self.inner.clock.now(),
                        outcome: Err(failure),
                    },
                );
                Err(error)
            }
        }
    }

    /// The cached figure while it is current, otherwise a refresh: for
    /// readers that show a figure (the health check, the dashboard) and should
    /// not show nothing before the first pass. An unreachable sample is
    /// answered from the cache, so a dead mount is not called again on every
    /// page open; the next pass tries it.
    pub async fn current_or_refresh(
        &self,
        path: &str,
        timeout: std::time::Duration,
    ) -> Option<FilesystemSpace> {
        match self.reading(path, self.inner.clock.now()) {
            StorageCapacityReading::Known {
                available_bytes,
                total_bytes,
                ..
            } => Some(FilesystemSpace {
                total_bytes: total_bytes as u64,
                available_bytes: available_bytes as u64,
            }),
            StorageCapacityReading::Unreachable { .. } => None,
            StorageCapacityReading::NotSampled | StorageCapacityReading::Stale { .. } => self
                .refresh(PathBuf::from(path), timeout)
                .await
                .ok()
                .filter(|space| capacity_values(*space).is_some()),
        }
    }

    /// The newest current figure for the volume holding `path`, from the cache
    /// alone: the nearest of `path` and its ancestors with a known reading.
    /// For a guard whose own call for `path` just failed, so it can still go
    /// by the figure the sampler took of the library root above it. Never
    /// touches the disk.
    #[cfg_attr(not(feature = "runtime-archives"), allow(dead_code))]
    pub(crate) fn last_known_space_above(&self, path: &Path) -> Option<FilesystemSpace> {
        let now = self.inner.clock.now();
        path.ancestors().find_map(
            |ancestor| match self.reading(&ancestor.to_string_lossy(), now) {
                StorageCapacityReading::Known {
                    available_bytes,
                    total_bytes,
                    ..
                } => Some(FilesystemSpace {
                    total_bytes: total_bytes as u64,
                    available_bytes: available_bytes as u64,
                }),
                _ => None,
            },
        )
    }

    /// Make `roots` the library roots the sampler keeps current. A root no
    /// longer listed becomes an ordinary cached path, dropped when the cache
    /// needs room.
    pub(crate) fn set_roots(&self, roots: &[String]) {
        let wanted: HashMap<String, PathBuf> = roots
            .iter()
            .map(|root| (key_of(Path::new(root)), PathBuf::from(root)))
            .filter(|(key, _)| !key.is_empty())
            .collect();
        let mut entries = write(&self.inner.entries);
        for (key, entry) in entries.iter_mut() {
            entry.root = wanted.contains_key(key);
        }
        for (key, path) in wanted {
            let entry = entries.entry(key).or_default();
            entry.root = true;
            entry.path = path;
        }
        drop(entries);
        self.trim();
    }

    /// One background pass over the current roots. Roots whose last calls
    /// agree on filesystem id and total cost one call between full passes.
    /// A root whose previous call is still out is not called again; it reads
    /// unreachable until that call returns. Returns the calls made.
    pub(crate) async fn sample_roots(&self) -> usize {
        let pass = self.inner.passes.fetch_add(1, Ordering::SeqCst);
        let full = pass % FULL_PASS_EVERY == 0;
        let roots: Vec<(String, PathBuf, Option<(u64, u64)>, Arc<Mutex<()>>)> =
            read(&self.inner.entries)
                .iter()
                .filter(|(_, entry)| entry.root)
                .map(|(key, entry)| {
                    (
                        key.clone(),
                        entry.path.clone(),
                        entry.sample.as_ref().and_then(Sample::group),
                        entry.flight.clone(),
                    )
                })
                .collect();

        let mut groups: Vec<(Option<(u64, u64)>, Vec<(String, PathBuf)>)> = Vec::new();
        for (key, path, group, flight) in roots {
            if flight.try_lock().is_err() {
                self.store(
                    &key,
                    Sample {
                        sampled_at: self.inner.clock.now(),
                        outcome: Err(ProbeFailure {
                            kind: io::ErrorKind::TimedOut,
                            message: "the previous capacity call has not returned".into(),
                        }),
                    },
                );
                continue;
            }
            let shared = if full { None } else { group };
            match shared.and_then(|group| groups.iter_mut().find(|(g, _)| *g == Some(group))) {
                Some((_, members)) => members.push((key, path)),
                None => groups.push((shared, vec![(key, path)])),
            }
        }

        let mut calls = 0;
        let mut pending = tokio::task::JoinSet::new();
        for (_, members) in groups {
            let owner = self.clone();
            pending.spawn(async move { owner.sample_group(members).await });
        }
        while let Some(joined) = pending.join_next().await {
            calls += joined.unwrap_or(0);
        }
        calls
    }

    async fn sample_group(&self, members: Vec<(String, PathBuf)>) -> usize {
        let mut members = members.into_iter();
        let Some((lead_key, lead_path)) = members.next() else {
            return 0;
        };
        let before = self.sample(&lead_key).map(|sample| sample.sampled_at);
        let _ = self
            .refresh(lead_path, STORAGE_CAPACITY_SAMPLE_TIMEOUT)
            .await;
        let mut calls = 1;
        let lead = self.sample(&lead_key);
        let rest: Vec<_> = members.collect();
        match lead {
            Some(sample) if sample.values().is_some() && Some(sample.sampled_at) != before => {
                for (key, _) in rest {
                    self.store(&key, sample.clone());
                }
            }
            _ => {
                for (_, path) in rest {
                    let _ = self.refresh(path, STORAGE_CAPACITY_SAMPLE_TIMEOUT).await;
                    calls += 1;
                }
            }
        }
        calls
    }

    fn call(&self, path: &Path) -> Sample {
        #[cfg(test)]
        self.inner.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .inner
            .disk
            .probe(path)
            .map_err(|error| ProbeFailure::from_error(&error));
        Sample {
            sampled_at: self.inner.clock.now(),
            outcome,
        }
    }

    fn sample(&self, key: &str) -> Option<Sample> {
        read(&self.inner.entries)
            .get(key)
            .and_then(|entry| entry.sample.clone())
    }

    fn flight(&self, key: &str, path: &Path) -> Arc<Mutex<()>> {
        if let Some(entry) = read(&self.inner.entries).get(key) {
            return entry.flight.clone();
        }
        let flight = {
            let mut entries = write(&self.inner.entries);
            let entry = entries.entry(key.to_string()).or_insert_with(|| Entry {
                path: path.to_path_buf(),
                ..Entry::default()
            });
            entry.flight.clone()
        };
        self.trim();
        flight
    }

    /// Store `sample` unless the path already holds a newer one, so a slow
    /// call finishing late cannot roll a path back.
    fn store(&self, key: &str, sample: Sample) {
        if key.is_empty() {
            return;
        }
        let mut entries = write(&self.inner.entries);
        let entry = entries.entry(key.to_string()).or_default();
        if entry
            .sample
            .as_ref()
            .is_some_and(|existing| existing.sampled_at > sample.sampled_at)
        {
            return;
        }
        let recovered = matches!(&entry.sample, Some(previous) if previous.values().is_none())
            && sample.values().is_some();
        let failed = matches!(&entry.sample, Some(previous) if previous.values().is_some())
            && sample.values().is_none();
        if entry.root && recovered {
            info!(root = %key, "library root capacity readings recovered");
        }
        if entry.root && failed {
            if let Err(failure) = &sample.outcome {
                warn!(root = %key, error = %failure.message, "library root capacity reading unavailable");
            }
        }
        entry.sample = Some(sample);
    }

    /// Keep at most [`OTHER_PATHS_MAX`] non-root paths, dropping the least
    /// recently sampled with no call out.
    fn trim(&self) {
        let mut entries = write(&self.inner.entries);
        let others = entries.values().filter(|entry| !entry.root).count();
        if others <= OTHER_PATHS_MAX {
            return;
        }
        let mut idle: Vec<(Option<DateTime<Utc>>, String)> = entries
            .iter()
            .filter(|(_, entry)| !entry.root && Arc::strong_count(&entry.flight) == 1)
            .map(|(key, entry)| (entry.sample.as_ref().map(|s| s.sampled_at), key.clone()))
            .collect();
        idle.sort();
        for (_, key) in idle.into_iter().take(others - OTHER_PATHS_MAX) {
            entries.remove(&key);
        }
    }
}

impl AppUseCase {
    /// Every configured library root path, for the sampler and the owner's
    /// roots.
    pub(super) async fn storage_capacity_roots(&self) -> crate::AppResult<Vec<String>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        Ok(libraries
            .into_iter()
            .flat_map(|library| library.roots)
            .map(|root| root.path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect())
    }
}

/// Keep every library root's capacity sample current. Each pass reads the
/// configured roots (one library read, no writes), so a root added, removed or
/// retargeted is picked up by the next pass, then samples them. Stops when
/// `token` is cancelled.
pub async fn start_storage_capacity_sampler(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    info!("storage capacity sampler started");
    let capacity = app.runtime.health.storage_capacity.clone();
    loop {
        match app.storage_capacity_roots().await {
            Ok(roots) => {
                capacity.set_roots(&roots);
                capacity.sample_roots().await;
            }
            Err(error) => {
                warn!(error = %error, "storage capacity sampler could not read library roots");
            }
        }
        tokio::select! {
            _ = token.cancelled() => {
                info!("storage capacity sampler shutting down");
                return;
            }
            _ = tokio::time::sleep(STORAGE_CAPACITY_SAMPLE_INTERVAL) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::test_support::ManualClock;
    use std::collections::VecDeque;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000 + seconds, 0).expect("instant")
    }

    fn space(available: u64, total: u64) -> FilesystemSpace {
        FilesystemSpace {
            total_bytes: total,
            available_bytes: available,
        }
    }

    #[test]
    fn the_capacity_owner_is_the_process_one() {
        use crate::disk::DiskCapacity;

        let disk = crate::disk::Disk::process();
        let path = "/srv/fixture-disk-owner-capacity";
        disk.capacity().record_for_test(
            path,
            at(0),
            Some(crate::helpers::FilesystemSpace {
                total_bytes: 100,
                available_bytes: 40,
            }),
        );
        assert!(matches!(
            StorageCapacity::process().reading(path, at(0)),
            StorageCapacityReading::Known {
                available_bytes: 40,
                ..
            }
        ));
    }

    type Answer = io::Result<CapacityProbeReading>;

    /// Answers each path from a script; a path with no script fails. A gate,
    /// when set, holds every call until the test opens it.
    #[derive(Default)]
    struct ScriptedDisk {
        answers: Mutex<HashMap<PathBuf, VecDeque<Answer>>>,
        fallback: Mutex<HashMap<PathBuf, CapacityProbeReading>>,
        calls: Mutex<Vec<PathBuf>>,
        gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    }

    impl ScriptedDisk {
        fn always(&self, path: &str, space: FilesystemSpace, filesystem_id: Option<u64>) {
            self.fallback.lock().unwrap().insert(
                PathBuf::from(path),
                CapacityProbeReading {
                    space,
                    filesystem_id,
                },
            );
        }
        fn then(&self, path: &str, answer: Answer) {
            self.answers
                .lock()
                .unwrap()
                .entry(PathBuf::from(path))
                .or_default()
                .push_back(answer);
        }
        fn calls(&self) -> Vec<PathBuf> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CapacityDisk for ScriptedDisk {
        fn probe(&self, path: &Path) -> Answer {
            self.calls.lock().unwrap().push(path.to_path_buf());
            if let Some(entered) = self.entered.lock().unwrap().as_ref() {
                let _ = entered.send(());
            }
            if let Some(gate) = self.gate.lock().unwrap().as_ref() {
                let _ = gate.recv();
            }
            if let Some(answer) = self
                .answers
                .lock()
                .unwrap()
                .get_mut(path)
                .and_then(VecDeque::pop_front)
            {
                return answer;
            }
            self.fallback
                .lock()
                .unwrap()
                .get(path)
                .copied()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such fixture path"))
        }
    }

    fn owner(clock: &Arc<ManualClock>) -> (StorageCapacity, Arc<ScriptedDisk>) {
        let disk = Arc::new(ScriptedDisk::default());
        let capacity = StorageCapacity::with_seams(disk.clone(), clock.clone());
        (capacity, disk)
    }

    #[test]
    fn reason_codes_are_distinct_and_listed() {
        let mut seen = std::collections::HashSet::new();
        for code in reason::ALL {
            assert!(seen.insert(*code), "{code} repeated");
        }
        for reading in [
            StorageCapacityReading::NotSampled,
            StorageCapacityReading::Stale { sampled_at: at(0) },
            StorageCapacityReading::Unreachable { sampled_at: at(0) },
        ] {
            assert!(reason::ALL.contains(&reading.unknown_reason().unwrap()));
        }
    }

    #[test]
    fn the_bound_is_two_sampling_intervals() {
        assert_eq!(storage_sample_max_age(), Duration::minutes(10));
    }

    #[test]
    fn a_cached_read_never_calls_the_disk() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.always("/srv/fixture-media", space(25, 100), None);
        assert_eq!(
            capacity.reading("/srv/fixture-media", at(0)),
            StorageCapacityReading::NotSampled
        );
        capacity
            .refresh_blocking(Path::new("/srv/fixture-media/"))
            .expect("fixture answers");
        let calls = capacity.disk_calls();
        for offset in 0..5 {
            assert_eq!(
                capacity.reading("/srv/fixture-media", at(offset)),
                StorageCapacityReading::Known {
                    sampled_at: at(0),
                    available_bytes: 25,
                    total_bytes: 100,
                }
            );
        }
        assert_eq!(capacity.disk_calls(), calls);
        assert_eq!(disk.calls().len(), 1);
    }

    #[test]
    fn the_last_known_space_above_a_path_comes_from_its_nearest_known_ancestor() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        let output = Path::new("/srv/fixture-library/.scry-ex/fixture-download/out");
        assert_eq!(capacity.last_known_space_above(output), None);

        capacity.record_for_test("/srv/fixture-library", at(0), Some(space(25, 100)));
        // The output folder's own call failed; that sample is not a figure.
        capacity.record_for_test(&output.to_string_lossy(), at(0), None);
        assert_eq!(
            capacity.last_known_space_above(output),
            Some(space(25, 100))
        );

        // A nearer known ancestor wins over the library root.
        capacity.record_for_test("/srv/fixture-library/.scry-ex", at(0), Some(space(10, 100)));
        assert_eq!(
            capacity.last_known_space_above(output),
            Some(space(10, 100))
        );

        // A figure past the staleness bound is not used.
        clock.set(at(1) + storage_sample_max_age());
        assert_eq!(capacity.last_known_space_above(output), None);
        assert!(disk.calls().is_empty(), "the lookup never calls the disk");
    }

    #[test]
    fn a_sample_past_the_bound_is_stale() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.always("/srv/fixture-media", space(25, 100), None);
        capacity
            .refresh_blocking(Path::new("/srv/fixture-media"))
            .unwrap();
        let bound = storage_sample_max_age();
        assert!(matches!(
            capacity.reading("/srv/fixture-media", at(0) + bound),
            StorageCapacityReading::Known { .. }
        ));
        assert_eq!(
            capacity.reading("/srv/fixture-media", at(1) + bound),
            StorageCapacityReading::Stale { sampled_at: at(0) }
        );
    }

    #[test]
    fn an_unreachable_or_degenerate_root_is_unknown_never_zero() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.then(
            "/srv/fixture-failed",
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "share did not answer",
            )),
        );
        disk.always("/srv/fixture-degenerate", space(0, 0), None);
        disk.always("/srv/fixture-inverted", space(200, 100), None);
        for root in [
            "/srv/fixture-failed",
            "/srv/fixture-degenerate",
            "/srv/fixture-inverted",
        ] {
            let _ = capacity.refresh_blocking(Path::new(root));
            let reading = capacity.reading(root, at(1));
            assert_eq!(
                reading,
                StorageCapacityReading::Unreachable { sampled_at: at(0) },
                "{root}"
            );
            assert_eq!(
                reading.unknown_reason(),
                Some(reason::STORAGE_CAPACITY_UNAVAILABLE)
            );
        }
        let error = capacity
            .refresh_blocking(Path::new("/srv/fixture-failed"))
            .expect_err("reused failure");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn a_refresh_reuses_a_figure_under_a_second_old_and_calls_again_after() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.then("/srv/fixture-media", Ok(reading(40, 100, None)));
        disk.then("/srv/fixture-media", Ok(reading(30, 100, None)));
        let path = Path::new("/srv/fixture-media");
        assert_eq!(capacity.refresh_blocking(path).unwrap(), space(40, 100));
        assert_eq!(capacity.refresh_blocking(path).unwrap(), space(40, 100));
        assert_eq!(capacity.disk_calls(), 1);
        clock.set(at(1));
        assert_eq!(capacity.refresh_blocking(path).unwrap(), space(30, 100));
        assert_eq!(capacity.disk_calls(), 2);
    }

    fn reading(available: u64, total: u64, id: Option<u64>) -> CapacityProbeReading {
        CapacityProbeReading {
            space: space(available, total),
            filesystem_id: id,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_refreshes_of_one_path_share_one_call() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.always("/srv/fixture-media", space(40, 100), None);
        let (open, gate) = std::sync::mpsc::channel();
        let (entered_tx, entered) = std::sync::mpsc::channel();
        *disk.gate.lock().unwrap() = Some(gate);
        *disk.entered.lock().unwrap() = Some(entered_tx);

        let first = {
            let capacity = capacity.clone();
            tokio::spawn(async move {
                capacity
                    .refresh("/srv/fixture-media", std::time::Duration::from_secs(86_400))
                    .await
            })
        };
        // The first call is inside the disk; everyone else now has to wait on it.
        tokio::task::spawn_blocking(move || entered.recv().expect("first call entered"))
            .await
            .unwrap();
        let rest: Vec<_> = (0..8)
            .map(|_| {
                let capacity = capacity.clone();
                tokio::spawn(async move {
                    capacity
                        .refresh("/srv/fixture-media", std::time::Duration::from_secs(86_400))
                        .await
                })
            })
            .collect();
        open.send(()).unwrap();
        drop(open);
        assert_eq!(first.await.unwrap().unwrap(), space(40, 100));
        for waiter in rest {
            assert_eq!(waiter.await.unwrap().unwrap(), space(40, 100));
        }
        assert_eq!(disk.calls().len(), 1);
    }

    #[test]
    fn an_older_sample_never_replaces_a_newer_one() {
        let clock = ManualClock::at(at(10));
        let (capacity, disk) = owner(&clock);
        disk.always("/srv/fixture-media", space(40, 100), None);
        capacity
            .refresh_blocking(Path::new("/srv/fixture-media"))
            .unwrap();
        capacity.record_for_test("/srv/fixture-media", at(5), None);
        assert!(matches!(
            capacity.reading("/srv/fixture-media", at(11)),
            StorageCapacityReading::Known { sampled_at, .. } if sampled_at == at(10)
        ));
    }

    #[tokio::test]
    async fn sampling_follows_the_root_set() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.always("/srv/fixture-a", space(10, 100), None);
        disk.always("/srv/fixture-b", space(20, 100), None);
        capacity.set_roots(&["/srv/fixture-a".into()]);
        assert_eq!(capacity.sample_roots().await, 1);
        assert!(matches!(
            capacity.reading("/srv/fixture-a", at(0)),
            StorageCapacityReading::Known { .. }
        ));
        assert_eq!(
            capacity.reading("/srv/fixture-b", at(0)),
            StorageCapacityReading::NotSampled
        );

        clock.set(at(300));
        capacity.set_roots(&["/srv/fixture-b".into()]);
        assert_eq!(capacity.sample_roots().await, 1);
        assert_eq!(
            disk.calls(),
            vec![
                PathBuf::from("/srv/fixture-a"),
                PathBuf::from("/srv/fixture-b")
            ]
        );
        assert!(matches!(
            capacity.reading("/srv/fixture-b", at(300)),
            StorageCapacityReading::Known {
                available_bytes: 20,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn roots_on_one_filesystem_cost_one_call_between_full_passes() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.always("/srv/fixture-share/movies", space(50, 1_000), Some(7));
        disk.always("/srv/fixture-share/series", space(50, 1_000), Some(7));
        disk.always("/srv/fixture-other", space(5, 10), Some(9));
        capacity.set_roots(&[
            "/srv/fixture-share/movies".into(),
            "/srv/fixture-share/series".into(),
            "/srv/fixture-other".into(),
        ]);
        // The first pass is full: every root is called and its filesystem learned.
        assert_eq!(capacity.sample_roots().await, 3);
        clock.set(at(300));
        assert_eq!(capacity.sample_roots().await, 2);
        for root in ["/srv/fixture-share/movies", "/srv/fixture-share/series"] {
            assert_eq!(
                capacity.reading(root, at(300)).sampled_at(),
                Some(at(300)),
                "{root}"
            );
        }
    }

    #[tokio::test]
    async fn a_root_whose_call_is_still_out_reads_unreachable_and_is_not_called_again() {
        let clock = ManualClock::at(at(0));
        let (capacity, disk) = owner(&clock);
        disk.always("/srv/fixture-hung", space(5, 10), None);
        capacity.set_roots(&["/srv/fixture-hung".into()]);
        let flight = capacity.flight("/srv/fixture-hung", Path::new("/srv/fixture-hung"));
        let held = flight.lock().unwrap();
        assert_eq!(capacity.sample_roots().await, 0);
        drop(held);
        assert!(disk.calls().is_empty());
        assert_eq!(
            capacity.reading("/srv/fixture-hung", at(0)),
            StorageCapacityReading::Unreachable { sampled_at: at(0) }
        );
    }

    #[test]
    fn other_paths_are_bounded() {
        let clock = ManualClock::at(at(0));
        let (capacity, _disk) = owner(&clock);
        capacity.set_roots(&["/srv/fixture-root".into()]);
        for index in 0..(OTHER_PATHS_MAX + 10) {
            let _ = capacity.refresh_blocking(Path::new(&format!("/srv/fixture-dir-{index}")));
        }
        let entries = read(&capacity.inner.entries);
        assert_eq!(
            entries.values().filter(|entry| !entry.root).count(),
            OTHER_PATHS_MAX
        );
        assert!(entries.contains_key("/srv/fixture-root"));
    }

    #[test]
    fn available_percent_matches_the_lifecycle_formula() {
        let reading = StorageCapacityReading::Known {
            sampled_at: at(0),
            available_bytes: 25,
            total_bytes: 200,
        };
        assert_eq!(reading.available_percent(), Some(12.5));
        assert_eq!(StorageCapacityReading::NotSampled.available_percent(), None);
    }
}
