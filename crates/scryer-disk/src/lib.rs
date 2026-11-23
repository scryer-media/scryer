//! The one owner of every filesystem operation Scryer performs.
//!
//! [`Disk`] holds the only calls to the operating system (behind
//! `DiskBackend`), the short-lived stat cache, single-flight per
//! path, timeouts, and what each configured root's filesystem has shown it can
//! and cannot do. Callers pass a path and a verb and get a generic, typed
//! outcome; they never say or learn what kind of filesystem a path is on.
//! The free-space call lives here too, in [`capacity`].
//!
//! Library roots live on network shares, where every call is a round trip and
//! a dead mount blocks it outright. So reads come in two modes, as capacity
//! readings do:
//!
//! - [`Disk::observed`] never touches the disk. It answers from a reading the
//!   owner took under [`DISK_STAT_REUSE`] ago, or [`PathReading::NotObserved`].
//! - [`Disk::fresh_blocking`] and [`Disk::fresh`] probe now, unless a probe of
//!   the same path started after the caller asked or finished under
//!   [`DISK_STAT_REUSE`] ago. The async form runs off the runtime, bounded by
//!   the root's timeout; a probe that does not answer in time reads
//!   [`PathReading::Unreachable`]. While such a probe is still out, further
//!   async reads under that root answer unreachable at once, but for one
//!   trial at a time, so a dead mount does not collect blocking threads.
//!
//! A check that decides a removal, a replacement, a move or a claim reads
//! with [`Disk::uncached`] or [`Disk::uncached_blocking`]: never
//! [`Disk::observed`], and never [`Disk::fresh`], whose answer can be a reading another process has since
//! outdated.
//!
//! Paths under no configured root (application data: the database, logs,
//! caches) pass straight through: never cached, never bounded by a timeout.
//! The owner works with no roots configured and without an async runtime, so
//! early startup can use it.
//!
//! Every mutation evicts what it can have changed: the path, its parent, and
//! for a directory removal or a move the whole subtree. No mutation here adds
//! deletion of its own: each removes exactly what the caller named, once.
//! The exceptions are files and directories the owner itself created for the
//! caller and that nobody else holds: `Disk::write_atomic_blocking` removes
//! its hidden staging file when it fails, a [`Staged`] write removes its
//! staging file when it is dropped without being committed, and a
//! [`ScratchDir`] removes itself, and everything in it, when it is dropped.
//! Verified copies are not here yet; the verified copier and the
//! copy-and-verify move fallback move behind this owner later.
//!
//! Contents are read whole ([`Disk::read_blocking`]) or through a handle
//! ([`Disk::open_blocking`]). Once a handle is open, reads and writes through
//! it are the caller's: they are not bounded, counted or cached. Opening a
//! file for writing evicts its readings, and [`Disk::finish_blocking`] syncs
//! the handle and evicts them again once the writing is done, so no reading
//! taken while it was written outlives it.

// disk-guard: this crate is the disk service itself, the one place that calls
// the filesystem.
#![allow(clippy::disallowed_methods)]

mod aside;
pub(crate) mod backend;
pub(crate) mod cache;
pub mod capacity;
mod copy;
mod hashed;
mod mark;
mod place;
mod reading;
mod sample;
pub mod stored_paths;
mod stored_version;
mod volume;

use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

use backend::{DiskBackend, SystemBackend};
use cache::{StatCache, key_of};
use volume::Volumes;

pub use aside::{AsideRemoval, PUT_BACK_RETRY_PAUSES, PutBackAside, aside_path_for};
pub use backend::SyncOutcome;
pub use cache::DISK_STAT_REUSE;
#[doc(hidden)]
pub use copy::{CacheBypass, ReadBackHandle, ReadBackOpener, open_cache_bypassed};
pub use copy::{
    Cancel, Check, ContentHasher, ContentHashes, Copied, CopyAdmission, CopyPermit, CrcKind, Depth,
    Evidence, FILE_NAME_MAX_BYTES, HashRefusal, Landing, Phase, Progress, Proof,
    RESUMABLE_STAGING_SUFFIX, StagedCause, StagedCopied, StagedCopy, StagedCopyRefusal, StagedStep,
    Verdict, Watch, resumable_staging_path,
};
pub use hashed::{
    CheckSide, FullHashAnswer, HashAsk, HashRow, HashStore, KnownHash, ReadTimes, StoreFuture,
    StoredHash,
};
pub use mark::{Drift, EntryAlias, Mark, Sameness};
pub use place::{
    CannotLink, Done, Doubt, Fallback, FreeNameMove, Leftover, LinkAfter, LinkProof, LinkProven,
    Method, PlaceRefusal, Placed, Placement, ReadBefore, Refused, Side, Unproven, Verify,
};
pub use reading::{DiskError, EntryKind, FileIdentity, PathReading, reason};
pub(crate) use reading::{DiskVerb, is_cross_device_error};
pub use sample::{CONTENT_SAMPLE_BYTES, ContentSample, SampleStep, sample_reader};
pub use stored_version::{StoredVersion, restore_target_state};
pub use volume::VolumeClass;
#[doc(hidden)]
pub use volume::{configured_volumes, path_is_under_configured_root};

/// The clock readings are stamped and aged by. Tests replace it.
#[doc(hidden)]
pub trait DiskClock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;

    /// Let `duration` pass on the calling blocking thread.
    fn pause(&self, duration: std::time::Duration) {
        std::thread::sleep(duration);
    }
}

#[doc(hidden)]
#[derive(Debug, Default)]
pub struct SystemClock;

impl DiskClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[doc(hidden)]
pub fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[doc(hidden)]
pub fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn hold(flight: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    flight
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether a reading taken at `observed_at` answers a caller who asked at
/// `asked_at`: it was taken after the caller asked, or under `reuse` before.
#[doc(hidden)]
pub fn reusable(
    observed_at: DateTime<Utc>,
    asked_at: DateTime<Utc>,
    reuse: std::time::Duration,
) -> bool {
    let reuse = chrono::Duration::from_std(reuse).unwrap_or_default();
    observed_at >= asked_at || asked_at - observed_at < reuse
}

/// How a job sent off the async runtime ended.
#[doc(hidden)]
pub enum Offloaded<T> {
    Done(T),
    TimedOut,
    /// The blocking task panicked or was cancelled; the join error's words.
    Failed(String),
}

/// Run `job` on a blocking thread, waiting at most `timeout` when one is
/// given. A job that times out keeps running; whatever it records when it
/// finishes is its own business.
#[doc(hidden)]
pub async fn offload<T: Send + 'static>(
    timeout: Option<std::time::Duration>,
    job: impl FnOnce() -> T + Send + 'static,
) -> Offloaded<T> {
    let task = tokio::task::spawn_blocking(job);
    let joined = match timeout {
        Some(timeout) => match tokio::time::timeout(timeout, task).await {
            Ok(joined) => joined,
            Err(_elapsed) => return Offloaded::TimedOut,
        },
        None => task.await,
    };
    match joined {
        Ok(value) => Offloaded::Done(value),
        Err(join) => Offloaded::Failed(join.to_string()),
    }
}

/// How a read sent through [`Disk::bounded`] ended.
enum Bounded<T> {
    Done(T),
    /// Not sent: its path or its root has a read out past its bound.
    Refused,
    TimedOut,
    Failed(String),
}

impl<T> From<Offloaded<T>> for Bounded<T> {
    fn from(outcome: Offloaded<T>) -> Self {
        match outcome {
            Offloaded::Done(value) => Self::Done(value),
            Offloaded::TimedOut => Self::TimedOut,
            Offloaded::Failed(message) => Self::Failed(message),
        }
    }
}

/// Reports an admitted read as answered when its job ends.
struct Answers(Arc<volume::Ticket>);

impl Drop for Answers {
    fn drop(&mut self) {
        self.0.answered();
    }
}

/// What a removal removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remove {
    File,
    /// A file, clearing a Windows read-only attribute only when that refuses
    /// the removal. For the callers that cleared it before the owner existed.
    FileEvenReadOnly,
    /// A directory that must already be empty.
    EmptyDir,
    /// [`Remove::EmptyDir`], clearing a Windows read-only attribute only when
    /// that refuses the removal, as [`Remove::FileEvenReadOnly`] does.
    EmptyDirEvenReadOnly,
    /// A directory and everything in it. A link is removed, never followed.
    Tree,
    /// [`Remove::Tree`], on Windows first clearing the read-only attribute of
    /// everything in the tree that carries one, links left alone. A failure
    /// while clearing removes nothing.
    TreeEvenReadOnly,
}

/// How a removal ended. A path that was already gone is not a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    Removed,
    AlreadyAbsent,
}

/// How a move was done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MoveOutcome {
    Renamed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Hard,
    Symbolic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked,
    /// This destination cannot hold such a link to this source; copy instead.
    NotPossible,
}

/// Whether creating a directory also creates its missing parents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parents {
    /// Create missing parents; an existing directory is success.
    Create,
    /// The parent must exist and the directory must not.
    MustExist,
}

/// Whether a write may replace an existing file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replace {
    Allowed,
    Never,
}

/// Who may read and write a file or directory the owner creates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    /// The process's defaults.
    #[default]
    Default,
    /// Only the account Scryer runs as, where the platform has owner
    /// permissions; elsewhere the defaults. Applied as the file is created,
    /// so its contents are never readable by anyone else first. An existing
    /// file keeps its permissions.
    OwnerOnly,
    /// The permissions another file has, from [`Access::like`]. Applied as
    /// the file is created, as [`Access::OwnerOnly`] is.
    Like(KeptPermissions),
}

/// Permissions taken from an existing file by [`Access::like`]; only the
/// owner applies them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
pub struct KeptPermissions(u32);

impl Access {
    /// Create with the same permissions as a file that has `permissions`,
    /// where the platform has owner, group and other permissions; elsewhere
    /// the defaults.
    pub fn like(permissions: &std::fs::Permissions) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Self::Like(KeptPermissions(permissions.mode() & 0o777))
        }
        #[cfg(not(unix))]
        {
            let _ = permissions;
            Self::Default
        }
    }

    /// The permission bits a creation applies, when it applies any.
    #[cfg(unix)]
    pub(crate) fn mode(self) -> Option<u32> {
        match self {
            Self::Default => None,
            Self::OwnerOnly => Some(0o600),
            Self::Like(kept) => Some(kept.0),
        }
    }
}

/// What [`Disk::restrict_to_owner_blocking`] narrows: a file loses every
/// permission but its owner's read and write, a directory keeps its owner's
/// search permission too, and a program its owner's execute permission.
/// The result does not depend on the process's umask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerOnly {
    File,
    Directory,
    Program,
}

/// How an atomic write names the hidden sibling it stages into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Staging {
    /// A new name on every write, created exclusively and removed when the
    /// write fails.
    Unique,
    /// One name per destination, truncated and reused by every write and
    /// kept when one fails. Only for a destination with a single writer: a
    /// crash leaves at most this sibling, and the next write takes it over.
    /// An existing sibling keeps its permissions, except that
    /// [`Access::OwnerOnly`] narrows them again.
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenMode {
    Read,
    WriteExisting,
    CreateNew,
    Create,
    CreateOrKeep,
    Append,
}

/// How [`Disk::open_blocking`] opens a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Open {
    pub(crate) mode: OpenMode,
    pub(crate) follow_links: bool,
    pub(crate) access: Access,
    pub(crate) read_back: bool,
    pub(crate) sequential: bool,
}

impl Open {
    const fn of(mode: OpenMode) -> Self {
        Self {
            mode,
            follow_links: true,
            access: Access::Default,
            read_back: false,
            sequential: false,
        }
    }

    /// Read an existing file.
    pub const fn read() -> Self {
        Self::of(OpenMode::Read)
    }

    /// Write an existing file from its start, never creating or emptying it.
    pub const fn write_existing() -> Self {
        Self::of(OpenMode::WriteExisting)
    }

    /// Write a new file; an existing one is refused and kept.
    pub const fn create_new() -> Self {
        Self::of(OpenMode::CreateNew)
    }

    /// Write a file, creating it or emptying an existing one.
    pub const fn create() -> Self {
        Self::of(OpenMode::Create)
    }

    /// Read and write a file, creating it when it is missing and keeping
    /// whatever it holds: lock and marker files.
    pub const fn create_or_keep() -> Self {
        Self::of(OpenMode::CreateOrKeep)
    }

    /// Append to a file, creating it when it is missing.
    pub const fn append() -> Self {
        Self::of(OpenMode::Append)
    }

    /// Refuse a link at the path instead of following it
    /// ([`reason::DISK_LINK_NOT_FOLLOWED`]), and anything that is not a
    /// regular file ([`reason::DISK_WRONG_KIND`]).
    pub const fn without_following_links(mut self) -> Self {
        self.follow_links = false;
        self
    }

    /// Who may use a file this open creates.
    pub const fn with_access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// Also allow reading through the handle a writing open returns, so
    /// what was written can be checked through that same handle.
    pub const fn with_read_back(mut self) -> Self {
        self.read_back = true;
        self
    }

    /// Tell the platform the file is read once, start to end, where it
    /// takes the hint. Advisory only.
    pub const fn sequentially(mut self) -> Self {
        self.sequential = true;
        self
    }

    fn writes(self) -> bool {
        self.mode != OpenMode::Read
    }

    fn verb(self) -> DiskVerb {
        if self.writes() {
            DiskVerb::Write
        } else {
            DiskVerb::Read
        }
    }
}

/// Turns "the path is not there" into an answer instead of a failure.
pub trait IfPresent<T> {
    /// `Ok(None)` when the operation failed only because the path, or a
    /// directory above it, does not exist ([`reason::DISK_PATH_MISSING`]).
    /// Every other failure stays a failure.
    fn if_present(self) -> Result<Option<T>, DiskError>;
}

impl<T> IfPresent<T> for Result<T, DiskError> {
    fn if_present(self) -> Result<Option<T>, DiskError> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.reason() == reason::DISK_PATH_MISSING => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// One entry of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    /// What the listing itself said about the entry: [`PathReading::Known`],
    /// possibly without a length, or [`PathReading::NotObserved`] when the
    /// listing could not say.
    pub reading: PathReading,
}

/// One entry of a visit. Its kind is read only when asked: from the listing
/// where it carries one, else with one probe that does not follow a link.
pub struct Visited {
    path: PathBuf,
    raw: Box<dyn backend::RawVisit>,
}

impl Visited {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn into_path(self) -> PathBuf {
        self.path
    }

    pub fn name(&self) -> &std::ffi::OsStr {
        self.path.file_name().unwrap_or_default()
    }

    pub fn kind(&self) -> Result<EntryKind, DiskError> {
        (self.raw.kind()).map_err(|error| DiskError::new(DiskVerb::Read, &self.path, error))
    }
}

/// Pass each entry of the open directory `dir` to `each` until it stops.
fn walk(
    dir: &Path,
    mut entries: backend::DirEntries,
    mut each: impl FnMut(Result<Visited, DiskError>) -> ControlFlow<()>,
) {
    let visited = |raw: Box<dyn backend::RawVisit>| Visited {
        path: dir.join(raw.name()),
        raw,
    };
    let failed = |error| DiskError::new(DiskVerb::List, dir, error);
    let _ = entries.try_for_each(|entry| each(entry.map(visited).map_err(failed)));
}

/// Keep `entry` in `kept` when `keep` accepts it; an entry the listing failed
/// on replaces `kept` with its error and stops.
fn gather(
    kept: &mut Result<Vec<Visited>, DiskError>,
    entry: Result<Visited, DiskError>,
    keep: fn(&Visited) -> bool,
) -> ControlFlow<()> {
    match (entry, kept.as_mut()) {
        (Ok(entry), Ok(kept)) => {
            if keep(&entry) {
                kept.push(entry);
            }
            ControlFlow::Continue(())
        }
        (Err(error), _) => {
            *kept = Err(error);
            ControlFlow::Break(())
        }
        (Ok(_), Err(_)) => ControlFlow::Break(()),
    }
}

struct Inner {
    backend: Arc<dyn DiskBackend>,
    clock: Arc<dyn DiskClock>,
    volumes: RwLock<Volumes>,
    cache: StatCache,
    copies: std::sync::OnceLock<CopyAdmission>,
    /// The table of full hashes, once installed.
    hash_store: std::sync::OnceLock<Arc<dyn HashStore>>,
    /// Whole-hash asks, one per path at a time, and the whole reads they run.
    hash_turns: hashed::HashTurns,
    /// Whole-file reads by path, once a test asked for the count.
    whole_reads: std::sync::OnceLock<Mutex<std::collections::HashMap<PathBuf, usize>>>,
    #[cfg(test)]
    calls: Mutex<std::collections::HashMap<&'static str, u64>>,
    /// Every call by name, in the order made.
    #[cfg(test)]
    call_order: Mutex<Vec<&'static str>>,
    #[cfg(test)]
    probe_timeout: Mutex<Option<std::time::Duration>>,
    /// Where the next steps of a copy fail, in order, and the raw error code
    /// each fails with.
    #[cfg(test)]
    copy_faults: Mutex<std::collections::VecDeque<(CopyFault, i32)>>,
}

/// A step of a copy a test makes fail.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyFault {
    /// Reading the source's name.
    SourceName,
    /// Not a failure: the source's name reads as owned by the user the code
    /// names.
    SourceOwner,
    Write,
    Sync,
    /// The promotion of a staged copy, which leaves an empty reservation at
    /// the destination name and says so.
    Promote,
}

/// The filesystem owner. Cheap to clone; clones share one cache.
#[derive(Clone)]
pub struct Disk {
    inner: Arc<Inner>,
    /// A substitute for opening full read-backs; production has none.
    read_back: Option<copy::ReadBackOpener>,
}

impl std::fmt::Debug for Disk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Disk").finish_non_exhaustive()
    }
}

impl Disk {
    /// The process's owner, on the real filesystem and clock.
    pub fn process() -> Self {
        static PROCESS: std::sync::OnceLock<Disk> = std::sync::OnceLock::new();
        PROCESS
            .get_or_init(|| Self::with_seams(Arc::new(SystemBackend), Arc::new(SystemClock)))
            .clone()
    }

    pub(crate) fn with_seams(backend: Arc<dyn DiskBackend>, clock: Arc<dyn DiskClock>) -> Self {
        Self {
            inner: Arc::new(Inner {
                backend,
                clock,
                volumes: RwLock::new(Volumes::default()),
                cache: StatCache::default(),
                copies: std::sync::OnceLock::new(),
                hash_store: std::sync::OnceLock::new(),
                hash_turns: hashed::HashTurns::default(),
                whole_reads: std::sync::OnceLock::new(),
                #[cfg(test)]
                calls: Mutex::default(),
                #[cfg(test)]
                call_order: Mutex::default(),
                #[cfg(test)]
                probe_timeout: Mutex::default(),
                #[cfg(test)]
                copy_faults: Mutex::default(),
            }),
            read_back: None,
        }
    }

    /// Install the configured roots. Readings already held stay until they
    /// age out or are evicted.
    pub fn set_volumes(&self, roots: impl IntoIterator<Item = (PathBuf, VolumeClass)>) {
        let mut volumes = write(&self.inner.volumes);
        *volumes = volumes.replaced(roots.into_iter().collect());
    }

    /// What is held about `path`, without touching the disk.
    pub fn observed(&self, path: &Path) -> PathReading {
        if !self.cacheable(path) {
            return PathReading::NotObserved;
        }
        self.inner
            .cache
            .current(&key_of(path), self.inner.clock.now())
            .unwrap_or(PathReading::NotObserved)
    }

    /// What is at `path` now, without following a link. Blocks on the disk:
    /// call it only from a blocking thread.
    pub fn fresh_blocking(&self, path: &Path) -> PathReading {
        if !self.cacheable(path) {
            return self.lstat(path);
        }
        let asked_at = self.inner.clock.now();
        let key = key_of(path);
        let flight = self.inner.cache.flight(&key);
        let _out = hold(&flight);
        if let Some(reading) = self.inner.cache.reusable(&key, asked_at) {
            return reading;
        }
        let epoch = self.inner.cache.epoch();
        let reading = self.lstat(path);
        self.inner.cache.store(&key, reading, epoch);
        reading
    }

    /// [`Self::fresh_blocking`] that always asks the disk and remembers
    /// nothing: for a check that decides a removal, a replacement, a move or
    /// a claim, and for a fact another name of the same file can change, its
    /// link count.
    pub fn uncached_blocking(&self, path: &Path) -> PathReading {
        self.lstat(path)
    }

    /// [`Self::fresh_blocking`] off the async runtime, bounded by the root's
    /// timeout. A probe that does not answer in time reads unreachable, and
    /// nothing is kept for it; when it answers later, its reading is kept.
    ///
    /// While a read of the same path, or of anything under the same root, is
    /// out past its bound, this answers unreachable at once without probing
    /// (see `Disk::bounded`); a reading still inside the reuse window is
    /// answered as usual.
    pub async fn fresh(&self, path: impl Into<PathBuf>) -> PathReading {
        let path = path.into();
        let traits = self.traits(&path);
        if traits.cacheable
            && let Some(reading) = self
                .inner
                .cache
                .reusable(&key_of(&path), self.inner.clock.now())
        {
            return reading;
        }
        let owner = self.clone();
        let probed = path.clone();
        let unreachable = |cause| PathReading::Unreachable {
            observed_at: self.inner.clock.now(),
            cause,
        };
        match self
            .bounded(&path, traits.probe_timeout, move || {
                owner.fresh_blocking(&probed)
            })
            .await
        {
            Bounded::Done(reading) => reading,
            Bounded::TimedOut => unreachable(reason::DISK_TIMED_OUT),
            Bounded::Refused | Bounded::Failed(_) => unreachable(reason::DISK_PATH_UNREACHABLE),
        }
    }

    /// What `path` resolves to through any links. Never cached: a link's
    /// target can be anywhere.
    pub fn follow_blocking(&self, path: &Path) -> PathReading {
        let result = self.call("stat", || self.inner.backend.stat(path));
        PathReading::from_result(&result, self.inner.clock.now())
    }

    pub async fn follow(&self, path: impl Into<PathBuf>) -> PathReading {
        self.probe_now(path.into(), Self::follow_blocking).await
    }

    /// [`Self::uncached_blocking`] off the runtime, bounded as
    /// [`Self::follow`] is: for a check that must see the disk as it is now.
    pub async fn uncached(&self, path: impl Into<PathBuf>) -> PathReading {
        self.probe_now(path.into(), Self::uncached_blocking).await
    }

    async fn probe_now(&self, path: PathBuf, read: fn(&Self, &Path) -> PathReading) -> PathReading {
        self.probe_with(path, read, |unreachable| unreachable).await
    }

    /// One bounded read off the runtime; a read that did not answer is
    /// `unanswered` of an unreachable reading.
    async fn probe_with<T: Send + 'static>(
        &self,
        path: PathBuf,
        read: fn(&Self, &Path) -> T,
        unanswered: fn(PathReading) -> T,
    ) -> T {
        let timeout = self.traits(&path).probe_timeout;
        let owner = self.clone();
        let probed = path.clone();
        match self
            .bounded(&path, timeout, move || read(&owner, &probed))
            .await
        {
            Bounded::Done(read) => read,
            outcome => unanswered(PathReading::Unreachable {
                observed_at: self.inner.clock.now(),
                cause: match outcome {
                    Bounded::TimedOut => reason::DISK_TIMED_OUT,
                    _ => reason::DISK_PATH_UNREACHABLE,
                },
            }),
        }
    }

    /// A [`Mark`] of what is at `path` itself, a link not followed: one
    /// probe, never cached. `Err` is the reading that says why there is no
    /// mark: missing or unreachable, never the two confused.
    pub fn fingerprint_blocking(&self, path: &Path) -> Result<Mark, PathReading> {
        let result = self.call("lstat", || self.inner.backend.lstat(path));
        self.mark_of(result)
    }

    /// `Disk::fingerprint_blocking` of what `path` resolves to through any
    /// links: one probe.
    pub fn fingerprint_followed_blocking(&self, path: &Path) -> Result<Mark, PathReading> {
        let result = self.call("stat", || self.inner.backend.stat(path));
        self.mark_of(result)
    }

    fn mark_of(&self, result: io::Result<backend::RawStat>) -> Result<Mark, PathReading> {
        match &result {
            Ok(stat) => Ok(Mark::of_stat(stat)),
            Err(_) => Err(PathReading::from_result(&result, self.inner.clock.now())),
        }
    }

    /// `Disk::fingerprint_blocking` off the runtime, bounded as
    /// [`Self::uncached`] is.
    pub async fn fingerprint(&self, path: impl Into<PathBuf>) -> Result<Mark, PathReading> {
        self.probe_with(path.into(), Self::fingerprint_blocking, Err)
            .await
    }

    /// [`Self::fingerprint_followed_blocking`] off the runtime, bounded.
    pub async fn fingerprint_followed(
        &self,
        path: impl Into<PathBuf>,
    ) -> Result<Mark, PathReading> {
        self.probe_with(path.into(), Self::fingerprint_followed_blocking, Err)
            .await
    }

    /// The Windows volume serial and file index of what is at `path` itself,
    /// a link not followed, as the platform gives them: a zero serial and an
    /// index the volume does not keep included. Only for text stored before
    /// marks carried them, which must keep its form; anything else asks a
    /// [`Mark`]. `None` when the path reads without them. One probe.
    #[cfg(windows)]
    #[doc(hidden)]
    pub fn windows_file_numbers_blocking(&self, path: &Path) -> Option<(u64, u64)> {
        let stat = self.call("lstat", || self.inner.backend.lstat(path)).ok()?;
        Some((stat.extra.serial.unwrap_or(0), stat.extra.inode?))
    }

    /// [`Self::windows_file_numbers_blocking`] off the runtime, bounded.
    #[cfg(windows)]
    #[doc(hidden)]
    pub async fn windows_file_numbers(&self, path: impl Into<PathBuf>) -> Option<(u64, u64)> {
        self.probe_with(path.into(), Self::windows_file_numbers_blocking, |_| None)
            .await
    }

    /// The entries of `dir`. On a configured root the listing seeds a reading
    /// for each entry and forgets cached entries it no longer holds.
    pub fn list_blocking(&self, dir: &Path) -> Result<Vec<Listed>, DiskError> {
        let (epoch, raw) = self.open_listing_blocking(dir)?;
        self.read_listing_blocking(dir, epoch, raw)
    }

    /// Open `dir` for a listing, with the cache epoch from before the open.
    fn open_listing_blocking(&self, dir: &Path) -> Result<(u64, backend::Listing), DiskError> {
        let epoch = self.inner.cache.epoch();
        self.call("list", || self.inner.backend.list(dir))
            .map(|raw| (epoch, raw))
            .map_err(|error| DiskError::new(DiskVerb::List, dir, error))
    }

    fn read_listing_blocking(
        &self,
        dir: &Path,
        epoch: u64,
        raw: backend::Listing,
    ) -> Result<Vec<Listed>, DiskError> {
        let raw = raw
            .collect::<io::Result<Vec<_>>>()
            .map_err(|error| DiskError::new(DiskVerb::List, dir, error))?;
        let observed_at = self.inner.clock.now();
        let listed: Vec<Listed> = raw
            .into_iter()
            .map(|entry| {
                let reading = match (entry.stat, entry.kind) {
                    (Some(stat), _) => stat.reading(observed_at),
                    (None, Some(kind)) => PathReading::Known {
                        kind,
                        len: None,
                        modified: None,
                        identity: None,
                        links: None,
                        observed_at,
                    },
                    (None, None) => PathReading::NotObserved,
                };
                Listed {
                    path: dir.join(entry.name),
                    reading,
                }
            })
            .collect();
        if self.cacheable(dir) {
            let seeded: Vec<(PathBuf, PathReading)> = listed
                .iter()
                .map(|entry| (key_of(&entry.path), entry.reading))
                .collect();
            self.inner.cache.seed_listing(&key_of(dir), &seeded, epoch);
        }
        Ok(listed)
    }

    /// [`Self::list_blocking`] off the runtime. Under a configured root only
    /// opening the directory is bounded, as [`Self::visit`] bounds it: a
    /// large directory is never refused for taking long to read.
    pub async fn list(&self, dir: impl Into<PathBuf>) -> Result<Vec<Listed>, DiskError> {
        let dir = dir.into();
        let timeout = self.traits(&dir).probe_timeout;
        let (epoch, raw) = self
            .run(DiskVerb::List, dir.clone(), timeout, |disk, dir| {
                disk.open_listing_blocking(dir)
            })
            .await?;
        self.run(DiskVerb::List, dir, None, move |disk, dir| {
            disk.read_listing_blocking(dir, epoch, raw)
        })
        .await
    }

    /// Whether the directory `dir` holds any entry. Reads at most one entry,
    /// so a large directory costs no more than an empty one.
    pub fn has_entries_blocking(&self, dir: &Path) -> Result<bool, DiskError> {
        self.call("has_entries", || self.inner.backend.has_entries(dir))
            .map_err(|error| DiskError::new(DiskVerb::List, dir, error))
    }

    pub async fn has_entries(&self, dir: impl Into<PathBuf>) -> Result<bool, DiskError> {
        let dir = dir.into();
        let timeout = self.traits(&dir).probe_timeout;
        self.run(DiskVerb::List, dir, timeout, |disk, dir| {
            disk.has_entries_blocking(dir)
        })
        .await
    }

    /// Each entry of `dir` in listing order until `each` stops. An entry the
    /// listing failed on is passed as its error, for the caller to skip or
    /// stop on. Kinds are read only when asked, and nothing is cached.
    pub fn visit_blocking(
        &self,
        dir: &Path,
        each: impl FnMut(Result<Visited, DiskError>) -> ControlFlow<()>,
    ) -> Result<(), DiskError> {
        walk(dir, self.open_dir_blocking(dir)?, each);
        Ok(())
    }

    fn open_dir_blocking(&self, dir: &Path) -> Result<backend::DirEntries, DiskError> {
        self.call("visit", || self.inner.backend.open_dir(dir))
            .map_err(|error| DiskError::new(DiskVerb::List, dir, error))
    }

    /// [`Self::visit_blocking`] off the runtime, `each` folding into `state`.
    /// Under a configured root only opening the directory is bounded, as
    /// [`Self::read`] bounds only the open: a long walk neither fails nor
    /// marks the root as stuck.
    pub async fn visit<T: Send + 'static>(
        &self,
        dir: impl Into<PathBuf>,
        mut state: T,
        mut each: impl FnMut(&mut T, Result<Visited, DiskError>) -> ControlFlow<()> + Send + 'static,
    ) -> Result<T, DiskError> {
        let dir = dir.into();
        let timeout = self.traits(&dir).probe_timeout;
        let entries = self
            .run(DiskVerb::List, dir.clone(), timeout, |disk, dir| {
                disk.open_dir_blocking(dir)
            })
            .await?;
        self.run(DiskVerb::List, dir, None, move |_, dir| {
            walk(dir, entries, |entry| each(&mut state, entry));
            Ok(state)
        })
        .await
    }

    /// The entries of `dir` that `keep` accepts, kinds unread unless `keep`
    /// reads them. An entry the listing failed on fails the call.
    pub fn entries_blocking(
        &self,
        dir: &Path,
        keep: fn(&Visited) -> bool,
    ) -> Result<Vec<Visited>, DiskError> {
        let mut kept = Ok(Vec::new());
        self.visit_blocking(dir, |entry| gather(&mut kept, entry, keep))?;
        kept
    }

    pub async fn entries(
        &self,
        dir: impl Into<PathBuf>,
        keep: fn(&Visited) -> bool,
    ) -> Result<Vec<Visited>, DiskError> {
        let found = self.visit(dir, Ok(Vec::new()), move |kept, entry| {
            gather(kept, entry, keep)
        });
        found.await?
    }

    /// The absolute path with every link resolved.
    pub fn canonicalize_blocking(&self, path: &Path) -> Result<PathBuf, DiskError> {
        self.call("canonicalize", || self.inner.backend.canonicalize(path))
            .map_err(|error| DiskError::new(DiskVerb::Resolve, path, error))
    }

    pub async fn canonicalize(&self, path: impl Into<PathBuf>) -> Result<PathBuf, DiskError> {
        let path = path.into();
        let timeout = self.traits(&path).probe_timeout;
        self.run(DiskVerb::Resolve, path, timeout, |disk, path| {
            disk.canonicalize_blocking(path)
        })
        .await
    }

    /// Where the link at `path` points.
    pub fn read_link_blocking(&self, path: &Path) -> Result<PathBuf, DiskError> {
        self.call("read_link", || self.inner.backend.read_link(path))
            .map_err(|error| DiskError::new(DiskVerb::Resolve, path, error))
    }

    pub async fn read_link(&self, path: impl Into<PathBuf>) -> Result<PathBuf, DiskError> {
        let path = path.into();
        let timeout = self.traits(&path).probe_timeout;
        self.run(DiskVerb::Resolve, path, timeout, |disk, path| {
            disk.read_link_blocking(path)
        })
        .await
    }

    /// Remove exactly `path`, as `what` says. A path already gone is
    /// [`Removal::AlreadyAbsent`], not an error.
    pub fn remove_blocking(&self, path: &Path, what: Remove) -> Result<Removal, DiskError> {
        let backend = &self.inner.backend;
        let result = match what {
            Remove::File => self.call("remove_file", || backend.remove_file(path)),
            Remove::FileEvenReadOnly => self.call("remove_file", || {
                backend.remove_file_clearing_readonly(path)
            }),
            Remove::EmptyDir => self.call("remove_dir", || backend.remove_dir(path)),
            Remove::EmptyDirEvenReadOnly => {
                self.call("remove_dir", || backend.remove_dir_clearing_readonly(path))
            }
            Remove::Tree => self.call("remove_tree", || backend.remove_tree(path)),
            Remove::TreeEvenReadOnly => self.call("remove_tree", || {
                backend.remove_tree_clearing_readonly(path)
            }),
        };
        // Evicted whatever the outcome: a failed tree removal can have
        // removed part of the tree.
        match what {
            Remove::File | Remove::FileEvenReadOnly => self.inner.cache.evict(&key_of(path)),
            _ => self.inner.cache.evict_subtree(&key_of(path)),
        }
        match result {
            Ok(()) => Ok(Removal::Removed),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Removal::AlreadyAbsent),
            Err(error) => Err(DiskError::new(DiskVerb::Remove, path, error)),
        }
    }

    pub async fn remove(
        &self,
        path: impl Into<PathBuf>,
        what: Remove,
    ) -> Result<Removal, DiskError> {
        self.run(DiskVerb::Remove, path.into(), None, move |disk, path| {
            disk.remove_blocking(path, what)
        })
        .await
    }

    /// Move `source` to `dest` by rename, never replacing anything at `dest`.
    /// A move that a rename cannot make fails with
    /// [`reason::DISK_MOVE_NEEDS_COPY`] and leaves both paths as they were.
    pub fn move_exclusive_blocking(
        &self,
        source: &Path,
        dest: &Path,
    ) -> Result<MoveOutcome, DiskError> {
        let result = self.rename_exclusive(source, dest);
        self.inner.cache.evict_subtree(&key_of(source));
        self.inner.cache.evict_subtree(&key_of(dest));
        result.map(|()| MoveOutcome::Renamed).map_err(|unmoved| {
            DiskError::new(DiskVerb::Move, source, unmoved.error).leaving(unmoved.left)
        })
    }

    pub async fn move_exclusive(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
    ) -> Result<MoveOutcome, DiskError> {
        let dest = dest.into();
        self.run(DiskVerb::Move, source.into(), None, move |disk, source| {
            disk.move_exclusive_blocking(source, &dest)
        })
        .await
    }

    /// Move `source` to `dest` by rename, replacing the file at `dest` if
    /// there is one: for a destination the caller itself holds, such as a
    /// reservation or rollback copy it made. Anything else moves with
    /// [`Self::move_exclusive_blocking`]. A move that a rename cannot make
    /// fails with [`reason::DISK_MOVE_NEEDS_COPY`] and leaves both paths as
    /// they were.
    pub fn move_replacing_blocking(
        &self,
        source: &Path,
        dest: &Path,
    ) -> Result<MoveOutcome, DiskError> {
        let result = self.call("rename", || self.inner.backend.rename(source, dest));
        self.inner.cache.evict_subtree(&key_of(source));
        self.inner.cache.evict_subtree(&key_of(dest));
        result
            .map(|()| MoveOutcome::Renamed)
            .map_err(|error| DiskError::new(DiskVerb::Move, source, error))
    }

    pub async fn move_replacing(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
    ) -> Result<MoveOutcome, DiskError> {
        let dest = dest.into();
        self.run(DiskVerb::Move, source.into(), None, move |disk, source| {
            disk.move_replacing_blocking(source, &dest)
        })
        .await
    }

    /// Move `source` to `dest` with a rename that refuses an existing
    /// destination in the same call. `Ok(false)`, with nothing changed, where
    /// the filesystem has no such rename, for a caller with its own way on.
    pub(crate) fn rename_no_replace_blocking(
        &self,
        source: &Path,
        dest: &Path,
    ) -> Result<bool, DiskError> {
        let result = self.try_rename_no_replace(source, dest);
        self.inner.cache.evict_subtree(&key_of(source));
        self.inner.cache.evict_subtree(&key_of(dest));
        result
            .transpose()
            .map(|renamed| renamed.is_some())
            .map_err(|error| DiskError::new(DiskVerb::Move, source, error))
    }

    /// The failure a test queued for this step of a copy, when it is next.
    #[cfg(test)]
    pub(crate) fn fault(&self, at: CopyFault) -> Option<io::Error> {
        let mut faults = self.inner.copy_faults.lock().unwrap();
        let (_, code) = faults.pop_front_if(|(step, _)| *step == at)?;
        Some(io::Error::from_raw_os_error(code))
    }

    /// Move `source` to `dest` by reserving `dest` with an exclusive create
    /// and renaming over the reservation, which is removed again if the
    /// rename fails and it still reads as empty. For a caller that reserves the name whatever the
    /// filesystem offers; anything else uses [`Self::move_exclusive_blocking`].
    /// A failure that left something at `dest` says so in [`DiskError::left`].
    pub fn move_claimed_blocking(
        &self,
        source: &Path,
        dest: &Path,
    ) -> Result<MoveOutcome, DiskError> {
        let result = self.call("rename_claimed", || {
            self.inner.backend.rename_claimed(source, dest)
        });
        self.inner.cache.evict_subtree(&key_of(source));
        self.inner.cache.evict_subtree(&key_of(dest));
        result.map(|()| MoveOutcome::Renamed).map_err(|failed| {
            let left = failed
                .reservation_left
                .then(|| Leftover::AtReservedName { at: dest.into() });
            DiskError::new(DiskVerb::Move, source, failed.error).leaving(left)
        })
    }

    /// Give `dest` the permissions `source` has, following links, where the
    /// platform has owner, group and other permissions; elsewhere nothing
    /// changes.
    pub(crate) fn copy_permissions_blocking(
        &self,
        source: &Path,
        dest: &Path,
    ) -> Result<(), DiskError> {
        let result = self.call("copy_permissions", || {
            self.inner.backend.copy_permissions(source, dest)
        });
        self.inner.cache.evict(&key_of(dest));
        result.map_err(|error| DiskError::new(DiskVerb::Change, dest, error))
    }

    /// Link `dest` to `source`. A destination that cannot hold the link
    /// answers [`LinkOutcome::NotPossible`] so the caller can copy instead.
    pub fn link_blocking(
        &self,
        source: &Path,
        dest: &Path,
        link: Link,
    ) -> Result<LinkOutcome, DiskError> {
        let backend = &self.inner.backend;
        let result = match link {
            Link::Hard => self.call("hard_link", || backend.hard_link(source, dest)),
            Link::Symbolic => self.call("symlink", || backend.symlink(source, dest)),
        };
        self.inner.cache.evict(&key_of(dest));
        if link == Link::Hard && result.is_ok() {
            // A new hard link changed the source's link count.
            self.inner.cache.evict(&key_of(source));
        }
        match result {
            Ok(()) => Ok(LinkOutcome::Linked),
            Err(error) if link_not_possible(&error) => Ok(LinkOutcome::NotPossible),
            Err(error) => Err(DiskError::new(DiskVerb::Link, dest, error)),
        }
    }

    pub async fn link(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
        link: Link,
    ) -> Result<LinkOutcome, DiskError> {
        let source = source.into();
        self.run(DiskVerb::Link, dest.into(), None, move |disk, dest| {
            disk.link_blocking(&source, dest, link)
        })
        .await
    }

    pub fn create_dir_blocking(&self, path: &Path, parents: Parents) -> Result<(), DiskError> {
        self.create_dir_as_blocking(path, parents, Access::Default)
    }

    pub async fn create_dir(
        &self,
        path: impl Into<PathBuf>,
        parents: Parents,
    ) -> Result<(), DiskError> {
        self.create_dir_as(path, parents, Access::Default).await
    }

    /// [`Self::create_dir_blocking`], with `access` applied to every
    /// directory this call creates. Directories already there keep theirs.
    pub fn create_dir_as_blocking(
        &self,
        path: &Path,
        parents: Parents,
        access: Access,
    ) -> Result<(), DiskError> {
        let backend = &self.inner.backend;
        let result = match parents {
            Parents::Create => self.call("create_dir_all", || backend.create_dir_all(path, access)),
            Parents::MustExist => self.call("create_dir", || backend.create_dir(path, access)),
        };
        // Any ancestor may have been created, and a cached "missing" for it
        // would now be wrong.
        self.inner.cache.evict_ancestors(&key_of(path));
        result.map_err(|error| DiskError::new(DiskVerb::CreateDir, path, error))
    }

    async fn create_dir_as(
        &self,
        path: impl Into<PathBuf>,
        parents: Parents,
        access: Access,
    ) -> Result<(), DiskError> {
        self.run(DiskVerb::CreateDir, path.into(), None, move |disk, path| {
            disk.create_dir_as_blocking(path, parents, access)
        })
        .await
    }

    /// Write `bytes` to `path` so a reader sees the old file or the whole new
    /// one, never part of either: missing parents are created, the bytes go to
    /// a hidden sibling and are synced, then the sibling is renamed into
    /// place. With [`Replace::Never`] an existing `path` is refused and kept.
    /// `access` applies to the new file.
    ///
    /// When this call created the hidden sibling and then the write, the
    /// sync or the rename fails, it removes that one sibling, once, best
    /// effort; the caller gets the original error either way. A sibling it
    /// did not create (the exclusive create was refused) is left alone, and
    /// it never removes `path` or a parent directory it created.
    pub(crate) fn write_atomic_blocking(
        &self,
        path: &Path,
        bytes: &[u8],
        replace: Replace,
        access: Access,
    ) -> Result<SyncOutcome, DiskError> {
        self.write_staged_blocking(path, bytes, replace, access, Staging::Unique, false)
    }

    /// `Disk::write_atomic_blocking` for contents that must be durable
    /// before they replace anything: when the sync is unconfirmed, `path` is
    /// left as it was and the write fails with [`io::ErrorKind::Unsupported`].
    pub fn write_durable_blocking(
        &self,
        path: &Path,
        bytes: &[u8],
        replace: Replace,
        access: Access,
    ) -> Result<(), DiskError> {
        self.write_staged_blocking(path, bytes, replace, access, Staging::Unique, true)
            .map(drop)
    }

    /// `Disk::write_atomic_blocking` staged as `staging` says, and, when
    /// `durable`, refused as [`Self::write_durable_blocking`] refuses an
    /// unconfirmed sync.
    pub fn write_staged_blocking(
        &self,
        path: &Path,
        bytes: &[u8],
        replace: Replace,
        access: Access,
        staging: Staging,
        durable: bool,
    ) -> Result<SyncOutcome, DiskError> {
        self.create_parent(path)?;
        let staged = match staging {
            Staging::Unique => staging_path_for(path),
            Staging::Fixed => fixed_staging_path_for(path),
        };
        let written = match staging {
            Staging::Unique => self.call("write_new", || {
                self.inner.backend.write_new(&staged, bytes, access)
            }),
            Staging::Fixed => self
                .call("write_fixed", || {
                    self.overwrite_synced(&staged, bytes, access)
                })
                .map_err(backend::WriteFailure::not_created),
        };
        let mut left = None;
        let placed = written.and_then(|synced| {
            if durable {
                synced.required().map_err(backend::WriteFailure::created)?;
            }
            self.land(&staged, path, replace)
                .map(|()| synced)
                .map_err(|unmoved| {
                    left = unmoved.left;
                    backend::WriteFailure::created(unmoved.error)
                })
        });
        let result = placed.map_err(|failure| {
            if failure.created && staging == Staging::Unique {
                // Best effort: an error here, or a sibling already gone,
                // leaves the caller's error as it is.
                let _ = self.call("remove_file", || self.inner.backend.remove_file(&staged));
            }
            DiskError::new(DiskVerb::Write, path, failure.error).leaving(left)
        });
        self.inner.cache.evict(&key_of(path));
        self.inner.cache.evict(&key_of(&staged));
        result
    }

    pub async fn write_staged(
        &self,
        path: impl Into<PathBuf>,
        bytes: Vec<u8>,
        replace: Replace,
        access: Access,
        staging: Staging,
        durable: bool,
    ) -> Result<SyncOutcome, DiskError> {
        self.run(DiskVerb::Write, path.into(), None, move |disk, path| {
            disk.write_staged_blocking(path, &bytes, replace, access, staging, durable)
        })
        .await
    }

    /// Create or truncate `staged`, write `bytes` and sync them.
    fn overwrite_synced(
        &self,
        staged: &Path,
        bytes: &[u8],
        access: Access,
    ) -> io::Result<SyncOutcome> {
        let backend = &self.inner.backend;
        // A link or anything but a regular file at the sibling is refused
        // and kept; only a regular file is emptied and reused.
        let open = Open::create_or_keep().without_following_links();
        // The open refuses links and, on Windows, any other reparse point.
        let mut file = backend.open(staged, open.with_access(access))?;
        if matches!(access, Access::OwnerOnly) {
            backend.restrict_handle_to_owner(&file, OwnerOnly::File)?;
        }
        file.set_len(0)?;
        io::Write::write_all(&mut file, bytes)?;
        backend.sync_handle(&file)
    }

    pub async fn write_atomic(
        &self,
        path: impl Into<PathBuf>,
        bytes: Vec<u8>,
        replace: Replace,
        access: Access,
    ) -> Result<SyncOutcome, DiskError> {
        self.run(DiskVerb::Write, path.into(), None, move |disk, path| {
            disk.write_atomic_blocking(path, &bytes, replace, access)
        })
        .await
    }

    /// Make an existing file's contents durable.
    pub fn sync_blocking(&self, path: &Path) -> Result<SyncOutcome, DiskError> {
        self.call("sync_file", || self.inner.backend.sync_file(path))
            .map_err(|error| DiskError::new(DiskVerb::Sync, path, error))
    }

    pub async fn sync(&self, path: impl Into<PathBuf>) -> Result<SyncOutcome, DiskError> {
        self.run(DiskVerb::Sync, path.into(), None, |disk, path| {
            disk.sync_blocking(path)
        })
        .await
    }

    /// The whole contents of `path`, following links.
    pub fn read_blocking(&self, path: &Path) -> Result<Vec<u8>, DiskError> {
        self.call("read", || self.inner.backend.read(path))
            .map_err(|error| DiskError::new(DiskVerb::Read, path, error))
    }

    /// [`Self::read_blocking`] off the runtime. Under a configured root only
    /// reaching the file is bounded, as every read is: the open waits at most
    /// the root's bound, so a dead mount still cannot collect threads, and the
    /// contents are then read through the handle with no timeout, so a large
    /// file on a slow share neither fails nor marks the root as stuck.
    pub async fn read(&self, path: impl Into<PathBuf>) -> Result<Vec<u8>, DiskError> {
        self.read_whole(path.into(), |bytes, _| Ok(bytes), Self::read_blocking)
            .await
    }

    /// The whole contents of `path` as text. Contents that are not UTF-8 fail
    /// with [`reason::DISK_OPERATION_FAILED`].
    pub fn read_to_string_blocking(&self, path: &Path) -> Result<String, DiskError> {
        text_of(path, self.read_blocking(path)?)
    }

    /// [`Self::read_to_string_blocking`] off the runtime, bounded as
    /// [`Self::read`] is.
    pub async fn read_to_string(&self, path: impl Into<PathBuf>) -> Result<String, DiskError> {
        self.read_whole(
            path.into(),
            |bytes, path| text_of(path, bytes),
            Self::read_to_string_blocking,
        )
        .await
    }

    /// A whole read off the runtime. Application data is one unbounded call,
    /// as `whole` makes it. Under a root the open is bounded and the contents
    /// are read through the handle, then turned into the answer by `then`,
    /// with no bound.
    async fn read_whole<T: Send + 'static>(
        &self,
        path: PathBuf,
        then: fn(Vec<u8>, &Path) -> Result<T, DiskError>,
        whole: fn(&Disk, &Path) -> Result<T, DiskError>,
    ) -> Result<T, DiskError> {
        let Some(timeout) = self.traits(&path).probe_timeout else {
            return self.run(DiskVerb::Read, path, None, whole).await;
        };
        let file = self
            .run(DiskVerb::Read, path.clone(), Some(timeout), |disk, path| {
                disk.open_blocking(path, Open::read())
            })
            .await?;
        self.run(DiskVerb::Read, path, None, move |disk, path| {
            let bytes = disk
                .call("read_from", || disk.inner.backend.read_from(file))
                .map_err(|error| DiskError::new(DiskVerb::Read, path, error))?;
            then(bytes, path)
        })
        .await
    }

    /// A handle on `path`, opened as `open` says, for contents too large or
    /// too long-lived for one buffer. Opening for writing evicts `path` and
    /// its parent; call [`Self::finish_blocking`] when done writing.
    pub fn open_blocking(&self, path: &Path, open: Open) -> Result<std::fs::File, DiskError> {
        let result = self.call("open", || self.inner.backend.open(path, open));
        if open.writes() {
            self.inner.cache.evict(&key_of(path));
        }
        result.map_err(|error| DiskError::new(open.verb(), path, error))
    }

    /// Reads the permission bits of an open handle without looking up its path.
    #[cfg(unix)]
    pub fn opened_file_mode_blocking(&self, file: &std::fs::File) -> Option<u32> {
        use std::os::unix::fs::PermissionsExt;

        self.call("handle_stat", || file.metadata())
            .ok()
            .map(|metadata| metadata.permissions().mode() & 0o777)
    }

    /// [`Self::open_blocking`] off the runtime. Opening to read is bounded
    /// under a configured root; opening to write never is. Reads and writes
    /// through the handle afterwards are not bounded.
    pub async fn open(
        &self,
        path: impl Into<PathBuf>,
        open: Open,
    ) -> Result<tokio::fs::File, DiskError> {
        let path = path.into();
        let timeout = if open.writes() {
            None
        } else {
            self.traits(&path).probe_timeout
        };
        self.run(open.verb(), path, timeout, move |disk, path| {
            disk.open_blocking(path, open)
        })
        .await
        .map(tokio::fs::File::from_std)
    }

    /// Write all of `bytes` through `file`, a handle on `path` from
    /// [`Self::open`], and flush them out of the handle.
    pub async fn write_through(
        &self,
        path: &Path,
        file: &mut tokio::fs::File,
        bytes: &[u8],
    ) -> Result<(), DiskError> {
        use tokio::io::AsyncWriteExt;
        let failed = |error| DiskError::new(DiskVerb::Write, path, error);
        file.write_all(bytes).await.map_err(failed)?;
        file.flush().await.map_err(failed)
    }

    /// Sync what was written through `file`, a handle on `path`, close it,
    /// and evict `path` and its parent so no reading taken while it was
    /// written remains.
    pub fn finish_blocking(
        &self,
        path: &Path,
        file: std::fs::File,
    ) -> Result<SyncOutcome, DiskError> {
        self.finish_marking_blocking(path, file, false).0
    }

    /// [`Self::finish_blocking`], and when `mark` asks, the mark of the file
    /// read through its handle after the sync and before the close, as late
    /// as the handle allows. A handle that cannot say gives no mark.
    pub(crate) fn finish_marking_blocking(
        &self,
        path: &Path,
        file: std::fs::File,
        mark: bool,
    ) -> (Result<SyncOutcome, DiskError>, Option<Mark>) {
        let result = self.call("sync_handle", || self.inner.backend.sync_handle(&file));
        #[cfg(test)]
        let result = self.fault(CopyFault::Sync).map_or(result, Err);
        // std's metadata of the handle: not a stat of the name, so the mark
        // never proves a link.
        let marked = match mark && result.is_ok() {
            true => self
                .call("handle_stat", || file.metadata())
                .ok()
                .map(|metadata| Mark::of(&metadata)),
            false => None,
        };
        drop(file);
        self.inner.cache.evict(&key_of(path));
        (
            result.map_err(|error| DiskError::new(DiskVerb::Sync, path, error)),
            marked,
        )
    }

    /// [`Self::finish_blocking`] for a handle from [`Self::open`]. Writes
    /// still buffered in the handle are flushed first.
    pub async fn finish(
        &self,
        path: impl Into<PathBuf>,
        mut file: tokio::fs::File,
    ) -> Result<SyncOutcome, DiskError> {
        use tokio::io::AsyncWriteExt;
        let path = path.into();
        if let Err(error) = file.flush().await {
            self.inner.cache.evict(&key_of(&path));
            return Err(DiskError::new(DiskVerb::Write, &path, error));
        }
        let file = file.into_std().await;
        self.run(DiskVerb::Sync, path, None, move |disk, path| {
            disk.finish_blocking(path, file)
        })
        .await
    }

    /// Write `bytes` to `path` in place: created, or emptied and rewritten
    /// when `replace` allows. Neither atomic nor synced, and parents are not
    /// created: for scratch files and files nobody reads while they are
    /// written. A failure can leave part of the bytes behind.
    /// `Disk::write_atomic_blocking` is the write for anything else.
    pub fn write_blocking(
        &self,
        path: &Path,
        bytes: &[u8],
        replace: Replace,
        access: Access,
    ) -> Result<(), DiskError> {
        let result = self.call("write", || {
            self.inner.backend.write(path, bytes, replace, access)
        });
        self.inner.cache.evict(&key_of(path));
        result.map_err(|error| DiskError::new(DiskVerb::Write, path, error))
    }

    pub async fn write(
        &self,
        path: impl Into<PathBuf>,
        bytes: Vec<u8>,
        replace: Replace,
        access: Access,
    ) -> Result<(), DiskError> {
        self.run(DiskVerb::Write, path.into(), None, move |disk, path| {
            disk.write_blocking(path, &bytes, replace, access)
        })
        .await
    }

    /// Start a streamed write that replaces `dest` all at once: the caller
    /// writes into a hidden sibling, then [`Staged::commit_blocking`] syncs
    /// it and renames it into place. Missing parents are created when
    /// `parents` says so. The sibling is removed if the write is dropped
    /// without a commit or the commit fails.
    pub fn stage_blocking(
        &self,
        dest: &Path,
        parents: Parents,
        access: Access,
    ) -> Result<Staged, DiskError> {
        if parents == Parents::Create {
            self.create_parent(dest)?;
        }
        let staged = staging_path_for(dest);
        let open = Open::create_new().with_access(access);
        let result = self.call("open", || self.inner.backend.open(&staged, open));
        self.inner.cache.evict(&key_of(&staged));
        let file = result.map_err(|error| DiskError::new(DiskVerb::Write, dest, error))?;
        Ok(Staged {
            disk: self.clone(),
            staged,
            dest: dest.to_path_buf(),
            file: Some(file),
            committed: false,
        })
    }

    pub async fn stage(
        &self,
        dest: impl Into<PathBuf>,
        parents: Parents,
        access: Access,
    ) -> Result<Staged, DiskError> {
        self.run(DiskVerb::Write, dest.into(), None, move |disk, dest| {
            disk.stage_blocking(dest, parents, access)
        })
        .await
    }

    /// Make the entries of `dir` durable, such as a rename into it. Where a
    /// directory cannot be synced this answers [`SyncOutcome::Unconfirmed`]
    /// without touching the disk.
    pub fn sync_dir_blocking(&self, dir: &Path) -> Result<SyncOutcome, DiskError> {
        self.call("sync_dir", || self.inner.backend.sync_dir(dir))
            .map_err(|error| DiskError::new(DiskVerb::Sync, dir, error))
    }

    pub async fn sync_dir(&self, dir: impl Into<PathBuf>) -> Result<SyncOutcome, DiskError> {
        self.run(DiskVerb::Sync, dir.into(), None, |disk, dir| {
            disk.sync_dir_blocking(dir)
        })
        .await
    }

    /// A new, empty directory under the system's temporary directory, named
    /// `prefix` and a random suffix, that only Scryer's account can use. It is
    /// removed, with everything in it, when the returned handle is dropped.
    pub fn scratch_dir_blocking(&self, prefix: &str) -> Result<ScratchDir, DiskError> {
        let base = std::env::temp_dir();
        let mut taken = None;
        for _ in 0..SCRATCH_NAME_TRIES {
            let path = base.join(format!("{prefix}{}", unique_suffix(&base)));
            let result = self.call("create_dir", || {
                self.inner.backend.create_dir(&path, Access::OwnerOnly)
            });
            self.inner.cache.evict(&key_of(&path));
            match result {
                Ok(()) => {
                    return Ok(ScratchDir {
                        disk: self.clone(),
                        path,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => taken = Some(error),
                Err(error) => return Err(DiskError::new(DiskVerb::CreateDir, &path, error)),
            }
        }
        Err(DiskError::new(
            DiskVerb::CreateDir,
            &base,
            taken.unwrap_or_else(|| io::ErrorKind::AlreadyExists.into()),
        ))
    }

    pub async fn scratch_dir(&self, prefix: impl Into<String>) -> Result<ScratchDir, DiskError> {
        let prefix = prefix.into();
        self.run(
            DiskVerb::CreateDir,
            std::env::temp_dir(),
            None,
            move |disk, _| disk.scratch_dir_blocking(&prefix),
        )
        .await
    }

    /// Narrow the permissions of the existing `path` to its owner, following
    /// a link. Where the platform has no owner permissions, nothing changes.
    pub fn restrict_to_owner_blocking(
        &self,
        path: &Path,
        what: OwnerOnly,
    ) -> Result<(), DiskError> {
        let result = self.call("restrict_to_owner", || {
            self.inner.backend.restrict_to_owner(path, what)
        });
        self.inner.cache.evict(&key_of(path));
        result.map_err(|error| DiskError::new(DiskVerb::Change, path, error))
    }

    pub async fn restrict_to_owner(
        &self,
        path: impl Into<PathBuf>,
        what: OwnerOnly,
    ) -> Result<(), DiskError> {
        self.run(DiskVerb::Change, path.into(), None, move |disk, path| {
            disk.restrict_to_owner_blocking(path, what)
        })
        .await
    }

    /// Set the modification time of the existing file at `path`, following a
    /// link.
    pub(crate) fn set_modified_blocking(
        &self,
        path: &Path,
        at: SystemTime,
    ) -> Result<(), DiskError> {
        let result = self.call("set_modified", || self.inner.backend.set_modified(path, at));
        self.inner.cache.evict(&key_of(path));
        result.map_err(|error| DiskError::new(DiskVerb::Change, path, error))
    }

    pub async fn set_modified(
        &self,
        path: impl Into<PathBuf>,
        at: SystemTime,
    ) -> Result<(), DiskError> {
        self.run(DiskVerb::Change, path.into(), None, move |disk, path| {
            disk.set_modified_blocking(path, at)
        })
        .await
    }

    fn create_parent(&self, path: &Path) -> Result<(), DiskError> {
        match path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            Some(parent) => self.create_dir_blocking(parent, Parents::Create),
            None => Ok(()),
        }
    }

    /// Rename a staging file this owner created onto `dest`, as `replace`
    /// says.
    fn land(&self, staged: &Path, dest: &Path, replace: Replace) -> Result<(), Unmoved> {
        match replace {
            Replace::Allowed => {
                Ok(self.call("rename", || self.inner.backend.rename(staged, dest))?)
            }
            Replace::Never => self.rename_exclusive(staged, dest),
        }
    }

    /// A hard link with the platform's own refusal, for the one caller that
    /// tells a different volume from a filesystem without links.
    fn hard_link_unclassified(&self, source: &Path, dest: &Path) -> io::Result<()> {
        let result = self.call("hard_link", || self.inner.backend.hard_link(source, dest));
        self.inner.cache.evict(&key_of(dest));
        if result.is_ok() {
            // A new hard link changed the source's link count.
            self.inner.cache.evict(&key_of(source));
        }
        result
    }

    /// An exclusive rename where the filesystem offers one, else a claim of
    /// the destination and a rename over it.
    fn rename_exclusive(&self, source: &Path, dest: &Path) -> Result<(), Unmoved> {
        match self.try_rename_no_replace(source, dest) {
            Some(result) => Ok(result?),
            None => self
                .call("rename_claimed", || {
                    self.inner.backend.rename_claimed(source, dest)
                })
                .map_err(|failed| Unmoved {
                    error: failed.error,
                    left: failed
                        .reservation_left
                        .then(|| Leftover::AtReservedName { at: dest.into() }),
                }),
        }
    }

    /// The exclusive rename, or `None` where the filesystem has none. A root
    /// whose filesystem plainly refused it is not asked again; a refusal these
    /// paths alone may have drawn falls back for this rename only.
    fn try_rename_no_replace(&self, source: &Path, dest: &Path) -> Option<io::Result<()>> {
        if read(&self.inner.volumes).lacks_exclusive_rename(dest) {
            return None;
        }
        match self.call("rename_no_replace", || {
            self.inner.backend.rename_no_replace(source, dest)
        }) {
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                if !backend::AmbiguousRefusal::is(&error) {
                    read(&self.inner.volumes).note_no_exclusive_rename(dest);
                }
                None
            }
            other => Some(other),
        }
    }

    fn traits(&self, path: &Path) -> volume::VolumeTraits {
        #[allow(unused_mut)]
        let mut traits = read(&self.inner.volumes).class_of(path).traits();
        #[cfg(test)]
        if traits.probe_timeout.is_some()
            && let Some(timeout) = *self.inner.probe_timeout.lock().unwrap()
        {
            traits.probe_timeout = Some(timeout);
        }
        traits
    }

    /// Bound the async reads of configured roots by `timeout`, for tests.
    #[cfg(test)]
    pub(crate) fn set_probe_timeout_for_test(&self, timeout: std::time::Duration) {
        *self.inner.probe_timeout.lock().unwrap() = Some(timeout);
    }

    fn cacheable(&self, path: &Path) -> bool {
        self.traits(path).cacheable && !cache::climbs(path)
    }

    fn lstat(&self, path: &Path) -> PathReading {
        let result = self.call("lstat", || self.inner.backend.lstat(path));
        PathReading::from_result(&result, self.inner.clock.now())
    }

    /// Every backend call goes through here, so tests can count them.
    fn call<T>(&self, operation: &'static str, call: impl FnOnce() -> T) -> T {
        #[cfg(test)]
        {
            let mut calls = self
                .inner
                .calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *calls.entry(operation).or_default() += 1;
            self.inner
                .call_order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(operation);
        }
        #[cfg(not(test))]
        let _ = operation;
        call()
    }

    /// Run a blocking verb off the async runtime. A read passes its root's
    /// timeout and is admitted as [`Self::bounded`] says; a refused read fails
    /// as a timed-out one does. Mutations pass no timeout and always run.
    async fn run<T: Send + 'static>(
        &self,
        verb: DiskVerb,
        path: PathBuf,
        timeout: Option<std::time::Duration>,
        job: impl FnOnce(&Disk, &Path) -> Result<T, DiskError> + Send + 'static,
    ) -> Result<T, DiskError> {
        let owner = self.clone();
        let target = path.clone();
        match self
            .bounded(&path, timeout, move || job(&owner, &target))
            .await
        {
            Bounded::Done(result) => result,
            Bounded::Refused | Bounded::TimedOut => Err(DiskError::timed_out(
                verb,
                &path,
                timeout.unwrap_or_default(),
            )),
            Bounded::Failed(message) => Err(DiskError::new(verb, &path, io::Error::other(message))),
        }
    }

    /// Run `job` on a blocking thread, waiting at most `timeout`.
    ///
    /// A timed-out job keeps its thread until the disk answers, so on a dead
    /// mount every further call would hold one more. A read that timed out and
    /// has not answered therefore marks its path and its root: further reads
    /// of that path are refused outright, and reads of anything under that
    /// root are refused but for one trial at a time. Once any read under the
    /// root answers, however late, reads go out again. Without a timeout
    /// (application data, mutations) the job always runs.
    async fn bounded<T: Send + 'static>(
        &self,
        path: &Path,
        timeout: Option<std::time::Duration>,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Bounded<T> {
        let learned = timeout.and_then(|_| read(&self.inner.volumes).learned(path));
        let Some(learned) = learned else {
            return offload(timeout, job).await.into();
        };
        let key = key_of(path);
        if learned.is_stuck(&key) {
            return Bounded::Refused;
        }
        let Some(trial) = learned.admit() else {
            return Bounded::Refused;
        };
        let ticket = volume::Ticket::new(learned, key, trial);
        let answers = Answers(ticket.clone());
        let outcome = offload(timeout, move || {
            // Dropped when the job returns or unwinds, however late.
            let _answers = answers;
            job()
        })
        .await;
        if matches!(outcome, Offloaded::TimedOut) {
            ticket.timed_out();
        }
        outcome.into()
    }

    /// Backend calls of `operation` this owner has made, for tests.
    #[cfg(test)]
    pub(crate) fn calls(&self, operation: &str) -> u64 {
        self.inner
            .calls
            .lock()
            .unwrap()
            .get(operation)
            .copied()
            .unwrap_or(0)
    }

    /// The calls this owner made of `operations`, in order, for tests.
    #[cfg(test)]
    pub(crate) fn calls_in_order(&self, operations: &[&str]) -> Vec<&'static str> {
        let order = self.inner.call_order.lock().unwrap();
        order
            .iter()
            .copied()
            .filter(|operation| operations.contains(operation))
            .collect()
    }

    /// Every backend call this owner has made, for tests.
    #[cfg(test)]
    pub(crate) fn total_calls(&self) -> u64 {
        self.inner.calls.lock().unwrap().values().sum()
    }

    /// The class of the root `path` is under, for tests.
    #[doc(hidden)]
    pub fn class_of(&self, path: &Path) -> VolumeClass {
        read(&self.inner.volumes).class_of(path)
    }
}

/// A link the destination cannot hold, as opposed to one that failed: the
/// two paths are on different filesystems, or the filesystem has no such
/// links or no more room for them.
fn link_not_possible(error: &io::Error) -> bool {
    if is_cross_device_error(error) || error.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(
            error.raw_os_error(),
            Some(code) if code == libc::EPERM
                || code == libc::ENOTSUP
                || code == libc::EOPNOTSUPP
                || code == libc::EMLINK
        )
    }
    #[cfg(windows)]
    {
        // ERROR_INVALID_FUNCTION (no links on this volume) and
        // ERROR_PRIVILEGE_NOT_HELD (symbolic links not permitted).
        matches!(error.raw_os_error(), Some(1) | Some(1314))
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// `bytes` read from `path` as text.
fn text_of(path: &Path, bytes: Vec<u8>) -> Result<String, DiskError> {
    String::from_utf8(bytes).map_err(|error| {
        DiskError::new(
            DiskVerb::Read,
            path,
            io::Error::new(io::ErrorKind::InvalidData, error),
        )
    })
}

/// A streamed write into a hidden sibling of its destination, from
/// [`Disk::stage_blocking`]. Write into it through [`std::io::Write`] or
/// [`Self::file`], then commit it. Dropped without a commit, it removes its
/// staging file, which nobody else was given, and nothing else; dropping it
/// blocks on that one removal.
#[derive(Debug)]
pub struct Staged {
    disk: Disk,
    staged: PathBuf,
    dest: PathBuf,
    file: Option<std::fs::File>,
    committed: bool,
}

impl Staged {
    /// The staging file, to read back what was written before committing.
    pub fn path(&self) -> &Path {
        &self.staged
    }

    pub fn destination(&self) -> &Path {
        &self.dest
    }

    pub fn file(&mut self) -> &mut std::fs::File {
        self.file
            .as_mut()
            .expect("a staged write holds its file until it is committed")
    }

    /// Sync the staging file and rename it onto the destination as `replace`
    /// says. With [`Replace::Never`] an existing destination is refused
    /// ([`reason::DISK_DESTINATION_EXISTS`]) and kept. On any failure the
    /// staging file is removed and the destination left as it was.
    pub fn commit_blocking(self, replace: Replace) -> Result<SyncOutcome, DiskError> {
        self.commit_inner(replace, false)
    }

    /// [`Self::commit_blocking`] for contents that must be durable before
    /// they replace anything: when the sync is unconfirmed, the destination
    /// is left as it was and the commit fails with
    /// [`io::ErrorKind::Unsupported`].
    pub fn commit_durable_blocking(self, replace: Replace) -> Result<(), DiskError> {
        self.commit_inner(replace, true).map(drop)
    }

    fn commit_inner(mut self, replace: Replace, durable: bool) -> Result<SyncOutcome, DiskError> {
        let disk = self.disk.clone();
        let file = self
            .file
            .take()
            .expect("a staged write holds its file until it is committed");
        let synced = disk.call("sync_handle", || disk.inner.backend.sync_handle(&file));
        drop(file);
        let mut left = None;
        let result = synced.and_then(|outcome| {
            if durable {
                outcome.required()?;
            }
            disk.land(&self.staged, &self.dest, replace)
                .map(|()| outcome)
                .map_err(|unmoved| {
                    left = unmoved.left;
                    unmoved.error
                })
        });
        disk.inner.cache.evict(&key_of(&self.dest));
        disk.inner.cache.evict(&key_of(&self.staged));
        match result {
            Ok(outcome) => {
                self.committed = true;
                Ok(outcome)
            }
            Err(error) => Err(DiskError::new(DiskVerb::Write, &self.dest, error).leaving(left)),
        }
    }

    /// [`Self::commit_blocking`] off the runtime, never timed out.
    pub async fn commit(self, replace: Replace) -> Result<SyncOutcome, DiskError> {
        let dest = self.dest.clone();
        match offload(None, move || self.commit_blocking(replace)).await {
            Offloaded::Done(result) => result,
            Offloaded::TimedOut => Err(DiskError::timed_out(
                DiskVerb::Write,
                &dest,
                std::time::Duration::ZERO,
            )),
            Offloaded::Failed(message) => Err(DiskError::new(
                DiskVerb::Write,
                &dest,
                io::Error::other(message),
            )),
        }
    }
}

impl io::Write for Staged {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file().write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file().flush()
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Closed first: some platforms refuse to remove an open file.
        drop(self.file.take());
        let backend = &self.disk.inner.backend;
        // Best effort, as the caller has nothing to do about it.
        let _ = self
            .disk
            .call("remove_file", || backend.remove_file(&self.staged));
        self.disk.inner.cache.evict(&key_of(&self.staged));
    }
}

/// A private temporary directory from [`Disk::scratch_dir_blocking`]. When
/// it is dropped it removes itself and everything in it, best effort,
/// blocking on that removal; nothing outside it is touched.
#[derive(Debug)]
pub struct ScratchDir {
    disk: Disk,
    path: PathBuf,
}

impl ScratchDir {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = self.disk.remove_blocking(&self.path, Remove::Tree);
    }
}

/// Names tried for a scratch directory before giving up; a collision of
/// random names is already unlikely once.
const SCRATCH_NAME_TRIES: usize = 8;

/// Nine hex digits that differ on every call.
fn unique_suffix(near: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(near.as_os_str().as_encoded_bytes());
    hasher.update(uuid::Uuid::new_v4().to_string().as_bytes());
    hasher.finalize().to_hex()[..9].to_string()
}

/// A hidden sibling of `dest` that no scanner takes for media. A short digest
/// keeps the name 25 bytes however long the destination's name is.
#[doc(hidden)]
pub fn staging_path_for(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(".scryer-{}.partial", unique_suffix(dest)))
}

/// The one hidden sibling [`Staging::Fixed`] reuses for `dest`.
fn fixed_staging_path_for(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let digest = blake3::hash(dest.as_os_str().as_encoded_bytes()).to_hex();
    parent.join(format!(".scry-sf-{}.partial", &digest[..9]))
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support {
    use super::*;

    /// An owner of its own on the real filesystem and clock, for a test that
    /// installs its own hash table or counts its own reads.
    pub fn standalone() -> Disk {
        Disk::with_seams(Arc::new(SystemBackend), Arc::new(SystemClock))
    }

    /// A clock tests move by hand.
    #[derive(Default)]
    pub struct ManualClock(Mutex<Option<DateTime<Utc>>>);

    impl ManualClock {
        pub fn at(instant: DateTime<Utc>) -> Arc<Self> {
            Arc::new(Self(Mutex::new(Some(instant))))
        }
        pub fn set(&self, instant: DateTime<Utc>) {
            *self.0.lock().unwrap() = Some(instant);
        }
    }

    /// A configured root, or one path under it, held as one that does not
    /// answer; see [`silence_root`] and [`silence_path`].
    pub struct SilencedRoot(Arc<volume::Ticket>);

    /// Holds the configured root `path` is under as one whose reads do not
    /// answer, the state a read that outlived its bound leaves: until the
    /// guard drops, every bounded read under that root is refused at once.
    /// Nothing is read. `None` when `path` is under no configured root.
    pub fn silence_root(disk: &Disk, path: &Path) -> Option<SilencedRoot> {
        let learned = read(&disk.inner.volumes).learned(path)?;
        let ticket = volume::Ticket::new(learned.clone(), key_of(path), true);
        ticket.timed_out();
        // The root's one trial read is taken as well, and never sent.
        learned.admit();
        Some(SilencedRoot(ticket))
    }

    /// Holds `path` alone as a path whose read went out and has not
    /// answered: until the guard drops, a bounded read of `path` itself is
    /// refused at once, and reads of other paths under its root go out one
    /// at a time. Nothing is read. `None` when `path` is under no configured
    /// root.
    pub fn silence_path(disk: &Disk, path: &Path) -> Option<SilencedRoot> {
        let learned = read(&disk.inner.volumes).learned(path)?;
        let ticket = volume::Ticket::new(learned, key_of(path), false);
        ticket.timed_out();
        Some(SilencedRoot(ticket))
    }

    impl Drop for SilencedRoot {
        fn drop(&mut self) {
            self.0.answered();
        }
    }

    /// The roots a test installed on a disk, in place of the ones it had;
    /// see [`install_volumes`].
    pub struct InstalledVolumes {
        disk: Disk,
        before: Option<Volumes>,
    }

    /// Installs `roots` on `disk` as [`Disk::set_volumes`] does, and puts
    /// back the roots it had, with what was learned about them, when the
    /// guard drops. For a test that configures the process's owner.
    pub fn install_volumes(
        disk: &Disk,
        roots: impl IntoIterator<Item = (PathBuf, VolumeClass)>,
    ) -> InstalledVolumes {
        let mut volumes = write(&disk.inner.volumes);
        let installed = volumes.replaced(roots.into_iter().collect());
        let before = Some(std::mem::replace(&mut *volumes, installed));
        InstalledVolumes {
            disk: disk.clone(),
            before,
        }
    }

    impl Drop for InstalledVolumes {
        fn drop(&mut self) {
            if let Some(before) = self.before.take() {
                *write(&self.disk.inner.volumes) = before;
            }
        }
    }

    impl DiskClock for ManualClock {
        fn now(&self) -> DateTime<Utc> {
            self.0.lock().unwrap().expect("clock set")
        }

        /// The pause passes at once, and shows on the clock.
        fn pause(&self, duration: std::time::Duration) {
            if let Some(now) = self.0.lock().unwrap().as_mut() {
                *now += chrono::Duration::from_std(duration).unwrap_or_default();
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// A rename that did not happen, and what it left at the destination name.
struct Unmoved {
    error: io::Error,
    left: Option<Leftover>,
}

impl From<io::Error> for Unmoved {
    fn from(error: io::Error) -> Self {
        Self { error, left: None }
    }
}
