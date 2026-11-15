//! The one owner of every filesystem operation Scryer performs.
//!
//! [`Disk`] holds the only calls to the operating system (behind
//! [`backend::DiskBackend`]), the short-lived stat cache, single-flight per
//! path, timeouts, and what each configured root's filesystem has shown it can
//! and cannot do. Callers pass a path and a verb and get a generic, typed
//! outcome; they never say or learn what kind of filesystem a path is on.
//! Free space lives here too, in [`capacity`].
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
//! Deletion guards always use a fresh read, never [`Disk::observed`] and never
//! a [`DiskPass`]'s evidence.
//!
//! Paths under no configured root (application data: the database, logs,
//! caches) pass straight through: never cached, never bounded by a timeout.
//! The owner works with no roots configured and without an async runtime, so
//! early startup can use it.
//!
//! Every mutation evicts what it can have changed: the path, its parent, and
//! for a directory removal or a move the whole subtree. No mutation here adds
//! deletion of its own: each removes exactly what the caller named, once. The
//! one exception is [`Disk::write_atomic_blocking`]: when it fails, it removes
//! the hidden staging file that the same call created, and nothing else.
//! Verified copies are not here yet; the verified copier and the
//! copy-and-verify move fallback move behind this owner later.

pub(crate) mod backend;
pub(crate) mod cache;
pub(crate) mod capacity;
mod pass;
mod reading;
mod volume;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Utc};

use backend::{DiskBackend, SystemBackend};
use cache::{StatCache, key_of};
use volume::Volumes;

pub use backend::SyncOutcome;
pub use cache::DISK_STAT_REUSE;
pub use pass::DiskPass;
pub use reading::{DiskError, DiskVerb, EntryKind, FileIdentity, PathReading, reason};
pub use volume::VolumeClass;

/// The clock readings are stamped and aged by. Tests replace it.
pub(crate) trait DiskClock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default)]
pub(crate) struct SystemClock;

impl DiskClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub(crate) fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
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
pub(crate) fn reusable(
    observed_at: DateTime<Utc>,
    asked_at: DateTime<Utc>,
    reuse: std::time::Duration,
) -> bool {
    let reuse = chrono::Duration::from_std(reuse).unwrap_or_default();
    observed_at >= asked_at || asked_at - observed_at < reuse
}

/// How a job sent off the async runtime ended.
pub(crate) enum Offloaded<T> {
    Done(T),
    TimedOut,
    /// The blocking task panicked or was cancelled; the join error's words.
    Failed(String),
}

/// Run `job` on a blocking thread, waiting at most `timeout` when one is
/// given. A job that times out keeps running; whatever it records when it
/// finishes is its own business.
pub(crate) async fn offload<T: Send + 'static>(
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
    /// A directory that must already be empty.
    EmptyDir,
    /// A directory and everything in it. A link is removed, never followed.
    Tree,
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

/// One entry of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    /// What the listing itself said about the entry: [`PathReading::Known`],
    /// possibly without a length, or [`PathReading::NotObserved`] when the
    /// listing could not say.
    pub reading: PathReading,
}

struct Inner {
    backend: Arc<dyn DiskBackend>,
    clock: Arc<dyn DiskClock>,
    volumes: RwLock<Volumes>,
    cache: StatCache,
    #[cfg(test)]
    calls: Mutex<std::collections::HashMap<&'static str, u64>>,
    #[cfg(test)]
    probe_timeout: Mutex<Option<std::time::Duration>>,
}

/// The filesystem owner. Cheap to clone; clones share one cache.
#[derive(Clone)]
pub struct Disk {
    inner: Arc<Inner>,
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
                #[cfg(test)]
                calls: Mutex::default(),
                #[cfg(test)]
                probe_timeout: Mutex::default(),
            }),
        }
    }

    /// The free-space owner. Callers still reach it through
    /// `StorageCapacity::process` until they move onto this owner.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn capacity(&self) -> capacity::StorageCapacity {
        capacity::StorageCapacity::process()
    }

    /// Install the configured roots. Readings already held stay until they
    /// age out or are evicted.
    pub fn set_volumes(&self, roots: impl IntoIterator<Item = (PathBuf, VolumeClass)>) {
        let mut volumes = write(&self.inner.volumes);
        *volumes = volumes.replaced(roots.into_iter().collect());
    }

    /// Start a unit of work that keeps its own evidence.
    pub fn pass(&self) -> DiskPass {
        DiskPass::new(self.clone())
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

    /// [`Self::fresh_blocking`] off the async runtime, bounded by the root's
    /// timeout. A probe that does not answer in time is recorded as
    /// unreachable; when it answers later, its reading replaces that.
    ///
    /// While a read of the same path, or of anything under the same root, is
    /// out past its bound, this answers unreachable at once without probing
    /// (see [`Self::bounded`]); a reading still inside the reuse window is
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
        let unreachable = PathReading::Unreachable {
            observed_at: self.inner.clock.now(),
        };
        match self
            .bounded(&path, traits.probe_timeout, move || {
                owner.fresh_blocking(&probed)
            })
            .await
        {
            Bounded::Done(reading) => reading,
            Bounded::TimedOut => {
                let cache = &self.inner.cache;
                cache.store(&key_of(&path), unreachable, cache.epoch());
                unreachable
            }
            Bounded::Refused | Bounded::Failed(_) => unreachable,
        }
    }

    /// What `path` resolves to through any links. Never cached: a link's
    /// target can be anywhere.
    pub fn follow_blocking(&self, path: &Path) -> PathReading {
        let result = self.call("stat", || self.inner.backend.stat(path));
        PathReading::from_result(&result, self.inner.clock.now())
    }

    pub async fn follow(&self, path: impl Into<PathBuf>) -> PathReading {
        let path = path.into();
        let timeout = self.traits(&path).probe_timeout;
        let owner = self.clone();
        let followed = path.clone();
        match self
            .bounded(&path, timeout, move || owner.follow_blocking(&followed))
            .await
        {
            Bounded::Done(reading) => reading,
            Bounded::Refused | Bounded::TimedOut | Bounded::Failed(_) => PathReading::Unreachable {
                observed_at: self.inner.clock.now(),
            },
        }
    }

    /// The entries of `dir`. On a configured root the listing seeds a reading
    /// for each entry and forgets cached entries it no longer holds.
    pub fn list_blocking(&self, dir: &Path) -> Result<Vec<Listed>, DiskError> {
        let epoch = self.inner.cache.epoch();
        let raw = self
            .call("list", || self.inner.backend.list(dir))
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

    pub async fn list(&self, dir: impl Into<PathBuf>) -> Result<Vec<Listed>, DiskError> {
        let dir = dir.into();
        let timeout = self.traits(&dir).probe_timeout;
        self.run(DiskVerb::List, dir, timeout, |disk, dir| {
            disk.list_blocking(dir)
        })
        .await
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
            Remove::EmptyDir => self.call("remove_dir", || backend.remove_dir(path)),
            Remove::Tree => self.call("remove_tree", || backend.remove_tree(path)),
        };
        // Evicted whatever the outcome: a failed tree removal can have
        // removed part of the tree.
        match what {
            Remove::File => self.inner.cache.evict(&key_of(path)),
            Remove::EmptyDir | Remove::Tree => self.inner.cache.evict_subtree(&key_of(path)),
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
        result
            .map(|()| MoveOutcome::Renamed)
            .map_err(|error| DiskError::new(DiskVerb::Move, source, error))
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
        let backend = &self.inner.backend;
        let result = match parents {
            Parents::Create => self.call("create_dir_all", || backend.create_dir_all(path)),
            Parents::MustExist => self.call("create_dir", || backend.create_dir(path)),
        };
        // Any ancestor may have been created, and a cached "missing" for it
        // would now be wrong.
        self.inner.cache.evict_ancestors(&key_of(path));
        result.map_err(|error| DiskError::new(DiskVerb::CreateDir, path, error))
    }

    pub async fn create_dir(
        &self,
        path: impl Into<PathBuf>,
        parents: Parents,
    ) -> Result<(), DiskError> {
        self.run(DiskVerb::CreateDir, path.into(), None, move |disk, path| {
            disk.create_dir_blocking(path, parents)
        })
        .await
    }

    /// Write `bytes` to `path` so a reader sees the old file or the whole new
    /// one, never part of either: missing parents are created, the bytes go to
    /// a hidden sibling and are synced, then the sibling is renamed into
    /// place. With [`Replace::Never`] an existing `path` is refused and kept.
    ///
    /// When this call created the hidden sibling and then the write, the
    /// sync or the rename fails, it removes that one sibling, once, best
    /// effort; the caller gets the original error either way. A sibling it
    /// did not create (the exclusive create was refused) is left alone, and
    /// it never removes `path` or a parent directory it created.
    pub fn write_atomic_blocking(
        &self,
        path: &Path,
        bytes: &[u8],
        replace: Replace,
    ) -> Result<SyncOutcome, DiskError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            self.create_dir_blocking(parent, Parents::Create)?;
        }
        let staged = staging_path_for(path);
        let written = self.call("write_new", || self.inner.backend.write_new(&staged, bytes));
        let placed = written.and_then(|synced| {
            let renamed = match replace {
                Replace::Allowed => {
                    self.call("rename", || self.inner.backend.rename(&staged, path))
                }
                Replace::Never => self.rename_exclusive(&staged, path),
            };
            renamed
                .map(|()| synced)
                .map_err(backend::WriteFailure::created)
        });
        let result = placed.map_err(|failure| {
            if failure.created {
                // Best effort: an error here, or a sibling already gone,
                // leaves the caller's error as it is.
                let _ = self.call("remove_file", || self.inner.backend.remove_file(&staged));
            }
            DiskError::new(DiskVerb::Write, path, failure.error)
        });
        self.inner.cache.evict(&key_of(path));
        self.inner.cache.evict(&key_of(&staged));
        result
    }

    pub async fn write_atomic(
        &self,
        path: impl Into<PathBuf>,
        bytes: Vec<u8>,
        replace: Replace,
    ) -> Result<SyncOutcome, DiskError> {
        self.run(DiskVerb::Write, path.into(), None, move |disk, path| {
            disk.write_atomic_blocking(path, &bytes, replace)
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

    /// An exclusive rename where the filesystem offers one, else a claim of
    /// the destination and a rename over it. A root whose filesystem refused
    /// the exclusive rename goes straight to the claim from then on.
    fn rename_exclusive(&self, source: &Path, dest: &Path) -> io::Result<()> {
        let backend = &self.inner.backend;
        if !read(&self.inner.volumes).lacks_exclusive_rename(dest) {
            match self.call("rename_no_replace", || {
                backend.rename_no_replace(source, dest)
            }) {
                Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                    read(&self.inner.volumes).note_no_exclusive_rename(dest);
                }
                other => return other,
            }
        }
        self.call("rename_claimed", || backend.rename_claimed(source, dest))
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
        self.traits(path).cacheable
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

    /// Every backend call this owner has made, for tests.
    #[cfg(test)]
    pub(crate) fn total_calls(&self) -> u64 {
        self.inner.calls.lock().unwrap().values().sum()
    }
}

/// A link the destination cannot hold, as opposed to one that failed: the
/// two paths are on different filesystems, or the filesystem has no such
/// links or no more room for them.
fn link_not_possible(error: &io::Error) -> bool {
    if crate::fs_safety::is_cross_device_error(error) || error.kind() == io::ErrorKind::Unsupported
    {
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

/// A hidden sibling of `dest` that no scanner takes for media. A short digest
/// keeps the name 25 bytes however long the destination's name is.
fn staging_path_for(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let mut hasher = blake3::Hasher::new();
    hasher.update(dest.as_os_str().as_encoded_bytes());
    hasher.update(crate::Id::new().0.as_bytes());
    let digest = hasher.finalize().to_hex();
    parent.join(format!(".scryer-{}.partial", &digest[..9]))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A clock tests move by hand.
    #[derive(Default)]
    pub(crate) struct ManualClock(Mutex<Option<DateTime<Utc>>>);

    impl ManualClock {
        pub(crate) fn at(instant: DateTime<Utc>) -> Arc<Self> {
            Arc::new(Self(Mutex::new(Some(instant))))
        }
        pub(crate) fn set(&self, instant: DateTime<Utc>) {
            *self.0.lock().unwrap() = Some(instant);
        }
    }

    impl DiskClock for ManualClock {
        fn now(&self) -> DateTime<Utc> {
            self.0.lock().unwrap().expect("clock set")
        }
    }
}

#[cfg(test)]
mod tests;
