//! Whole-file hashes: the one read that computes them, and the one answer to
//! what a file's full hash is.
//!
//! [`Disk::full_hash`] answers with a hash recorded earlier while the file's
//! length and modification time now equal the length and modification time
//! recorded when that hash was taken, and otherwise reads the whole file and
//! records what it read with the length and modification time it read them
//! at. One function, [`still_fits`], holds that test; the file number, the
//! change time and the volume are not part of it.
//!
//! A file with a catalog row has its hash recorded on that row, in the
//! installed [`HashStore`], which only loads and saves: a stored hash with no
//! mark, or a mark this build cannot read, is read once and recorded. A file
//! with no row, or any file while no store is installed, has its hash kept in
//! a bounded in-process cache, judged by the same test.
//!
//! Asks of one path take turns, so a second ask waits for the first read and
//! answers from what it recorded. At most [`HASH_READS_AT_ONCE`] whole reads
//! run at a time, each on a blocking thread.
//!
//! Probes: one followed stat of the path when a recorded hash is there to be
//! judged; a read adds one open and two stats of its handle.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::cache::{climbs, key_of, slot_of};
use super::copy::{CHUNK_BYTES, Check, ContentHasher, Depth, HashRefusal, judged};
use super::reading::DiskVerb;
use super::{Cancel, Disk, DiskError, EntryKind, Mark, Open, Phase, Progress, reason};

/// Whole reads that run at once, across every ask.
pub(crate) const HASH_READS_AT_ONCE: usize = 2;

/// The catalog row a file's hash is stored on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HashRow {
    /// The media file row's id.
    Id(String),
    /// The title's row for the file at this stored path.
    TitlePath { title_id: String, path: String },
}

/// A whole-file hash and the mark of the file it was taken of.
#[derive(Clone, Debug)]
pub struct KnownHash {
    pub size_bytes: u64,
    /// The whole-file BLAKE3, in hex.
    pub blake3: String,
    /// The whole-file CRC-64/NVME, where it was taken or stored.
    pub crc: Option<u64>,
    /// The file the hash was taken of, read through the handle that read it.
    pub mark: Mark,
}

/// What a row holds, as the store loaded it.
#[derive(Clone, Debug)]
pub struct StoredHash {
    /// The whole-file BLAKE3, in hex.
    pub blake3: String,
    pub crc: Option<u64>,
    /// The mark the hash was stored with; `None` when it was stored without
    /// one or with text this build cannot read.
    pub mark: Option<Mark>,
}

/// What [`Disk::full_hash`] answered.
#[derive(Debug)]
pub enum FullHashAnswer {
    /// Kept in this process, of a file the path still fits.
    Cached(KnownHash),
    /// Stored on the row, of a file the path still fits.
    Stored(KnownHash),
    /// Read end to end now, and recorded.
    Read(KnownHash),
    /// The file changed while it was read; nothing was recorded.
    Changed,
    /// The file could not be read end to end.
    Unreadable(HashRefusal),
    /// The ask was cancelled before the read finished.
    Cancelled,
    /// Asked with [`HashAsk::recorded_only`]: nothing recorded fits the file
    /// now, and nothing was read.
    Unrecorded,
}

impl FullHashAnswer {
    /// The hash, however it was answered.
    pub fn known(&self) -> Option<&KnownHash> {
        match self {
            Self::Cached(known) | Self::Stored(known) | Self::Read(known) => Some(known),
            Self::Changed | Self::Unreadable(_) | Self::Cancelled | Self::Unrecorded => None,
        }
    }

    /// [`Self::known`], owned.
    pub fn into_known(self) -> Option<KnownHash> {
        match self {
            Self::Cached(known) | Self::Stored(known) | Self::Read(known) => Some(known),
            Self::Changed | Self::Unreadable(_) | Self::Cancelled | Self::Unrecorded => None,
        }
    }
}

/// A future a [`HashStore`] returns.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The persisted table of full hashes, which the application keeps on its
/// catalog rows. Installed once with [`Disk::install_hash_store`]. It loads
/// and saves; whether a stored hash answers is decided here, never there.
pub trait HashStore: Send + Sync {
    /// What `row` holds. `None` when it holds no hash or the table could not
    /// say; a failure is never an answer.
    fn stored<'a>(&'a self, row: &'a HashRow) -> StoreFuture<'a, Option<StoredHash>>;
    /// Store `hash`, read now, on `row`. A failure is logged by the store and
    /// never fails the ask.
    fn record<'a>(&'a self, row: &'a HashRow, hash: &'a KnownHash) -> StoreFuture<'a, ()>;
}

/// Running totals of whole reads, for a caller that reports them.
#[derive(Debug, Default)]
pub struct ReadTimes {
    pub bytes: AtomicU64,
    pub read_ns: AtomicU64,
    pub hash_ns: AtomicU64,
}

/// One ask of [`Disk::full_hash`].
#[derive(Clone)]
pub struct HashAsk {
    path: PathBuf,
    row: Option<HashRow>,
    recorded: Option<StoredHash>,
    recorded_only: bool,
    fresh: bool,
    cancel: Cancel,
    progress: Progress,
    times: Option<Arc<ReadTimes>>,
}

impl HashAsk {
    /// Ask for `path`'s hash.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            row: None,
            recorded: None,
            recorded_only: false,
            fresh: false,
            cancel: Cancel::never(),
            progress: Progress::none(),
            times: None,
        }
    }

    /// The catalog row the file's hash is stored on.
    pub fn of_row(mut self, row: HashRow) -> Self {
        self.row = Some(row);
        self
    }

    /// What the caller already holds of the file's recorded hash, as its
    /// catalog row stored it, judged here as a stored row would be; the table
    /// is not asked. A read is recorded on the row, where there is one.
    pub fn recorded(mut self, stored: StoredHash) -> Self {
        self.recorded = Some(stored);
        self
    }

    /// Answer only from a recorded hash that fits the file now, and read
    /// nothing otherwise: [`FullHashAnswer::Unrecorded`].
    pub fn recorded_only(mut self) -> Self {
        self.recorded_only = true;
        self
    }

    /// Read the file now, whatever is recorded or cached of it: for a proof
    /// a removal rests on, where a rewrite that kept the length and the
    /// modification time must still be seen. What is read is recorded as
    /// any read is.
    pub fn fresh(mut self) -> Self {
        self.fresh = true;
        self
    }

    pub fn cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    /// Bytes read so far are reported as [`Phase::Verifying`].
    pub fn progress(mut self, progress: Progress) -> Self {
        self.progress = progress;
        self
    }

    pub fn timed(mut self, times: Arc<ReadTimes>) -> Self {
        self.times = Some(times);
        self
    }
}

/// Whether a hash taken of the file `recorded` marks still describes the file
/// `now` marks: both regular files of one length and one modification time.
/// Nothing else decides it. No probe.
fn still_fits(recorded: &Mark, now: &Mark) -> bool {
    recorded.kind() == Some(EntryKind::File)
        && now.kind() == Some(EntryKind::File)
        && recorded.len().is_some()
        && recorded.len() == now.len()
        && now.modified().is_some()
        && recorded.modified() == now.modified()
}

// ── The cache ────────────────────────────────────────────────────────────────

/// Paths kept at most. Past that, the oldest are dropped down to three
/// quarters of this, so trimming is not paid per insert.
pub(crate) const HASH_CACHE_MAX: usize = 4096;

#[derive(Debug, Default)]
struct Held {
    /// Keyed by the slot of the spelling; the spelling, the hash, and when it
    /// was recorded.
    entries: BTreeMap<PathBuf, (PathBuf, KnownHash, u64)>,
    /// Slots in the order they were recorded, oldest first, with when. An
    /// entry whose slot was recorded again later or forgotten is skipped.
    order: VecDeque<(PathBuf, u64)>,
    next: u64,
}

/// Whole-file hashes of files with no row, kept in this process. See the
/// module text.
#[derive(Debug, Default)]
pub(crate) struct HashCache {
    held: Mutex<Held>,
}

impl HashCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The hash kept for exactly `path`.
    pub(crate) fn kept(&self, path: &Path) -> Option<KnownHash> {
        let key = key_of(path);
        let held = self.lock();
        let (spelling, known, _) = held.entries.get(&slot_of(&key))?;
        (*spelling == key).then(|| known.clone())
    }

    /// Keep `known` for `path` when `current` still holds once this cache's
    /// lock is taken. An eviction changes what `current` reads before it
    /// forgets under the same lock, so a read that began before it is never
    /// kept after it.
    pub(crate) fn record_if(&self, path: &Path, known: KnownHash, current: impl FnOnce() -> bool) {
        let key = key_of(path);
        if climbs(&key) {
            return;
        }
        let slot = slot_of(&key);
        let mut held = self.lock();
        if !current() {
            return;
        }
        let at = held.next;
        held.next += 1;
        held.entries.insert(slot.clone(), (key, known, at));
        held.order.push_back((slot, at));
        if held.entries.len() > HASH_CACHE_MAX {
            let keep = HASH_CACHE_MAX * 3 / 4;
            while held.entries.len() > keep {
                let Some((oldest, at)) = held.order.pop_front() else {
                    break;
                };
                if held.entries.get(&oldest).is_some_and(|entry| entry.2 == at) {
                    held.entries.remove(&oldest);
                }
            }
        }
        if held.order.len() > HASH_CACHE_MAX * 2 {
            let Held { entries, order, .. } = &mut *held;
            order.retain(|(slot, at)| entries.get(slot).is_some_and(|entry| entry.2 == *at));
        }
    }

    /// Forget `key`, and everything under it when `subtree`. A key that
    /// climbs through `..` can name any path, so it forgets every hash.
    pub(crate) fn forget(&self, key: &Path, subtree: bool) {
        let mut held = self.lock();
        if held.entries.is_empty() {
            return;
        }
        if climbs(key) {
            held.entries.clear();
            held.order.clear();
            return;
        }
        let slot = slot_of(key);
        if !subtree {
            held.entries.remove(&slot);
            return;
        }
        let doomed: Vec<PathBuf> = held
            .entries
            .range::<Path, _>((
                std::ops::Bound::Included(slot.as_path()),
                std::ops::Bound::Unbounded,
            ))
            .map(|(path, _)| path)
            .take_while(|path| path.starts_with(&slot))
            .cloned()
            .collect();
        for path in doomed {
            held.entries.remove(&path);
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().entries.len()
    }
}

// ── Turns and the read budget ────────────────────────────────────────────────

/// The asks of each path, one at a time, and the whole reads, a few at a
/// time. An entry lives only while an ask of its path is out.
#[derive(Debug)]
pub(crate) struct HashTurns {
    paths: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    reads: tokio::sync::Semaphore,
}

impl Default for HashTurns {
    fn default() -> Self {
        Self {
            paths: Mutex::default(),
            reads: tokio::sync::Semaphore::new(HASH_READS_AT_ONCE),
        }
    }
}

/// An ask's turn at its path; the path's entry is dropped with the last one.
struct Turn<'a> {
    turns: &'a HashTurns,
    slot: PathBuf,
    flight: Option<Arc<tokio::sync::Mutex<()>>>,
    held: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl HashTurns {
    fn paths(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>> {
        self.paths.lock().unwrap_or_else(|error| error.into_inner())
    }

    async fn take(&self, path: &Path) -> Turn<'_> {
        let slot = slot_of(&key_of(path));
        let flight = self.paths().entry(slot.clone()).or_default().clone();
        // Made before the wait, so an ask dropped while it waits still lets
        // go of its path's entry.
        let mut turn = Turn {
            turns: self,
            slot,
            flight: Some(flight.clone()),
            held: None,
        };
        turn.held = Some(flight.lock_owned().await);
        turn
    }

    #[cfg(test)]
    pub(crate) fn paths_out(&self) -> usize {
        self.paths().len()
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        drop(self.held.take());
        let flight = self.flight.take();
        let mut paths = self.turns.paths();
        // The map's own handle and this one: no other ask holds or waits,
        // and another ask takes its handle only under this lock.
        if flight.is_some_and(|flight| Arc::strong_count(&flight) == 2) {
            paths.remove(&self.slot);
        }
    }
}

// ── The read ─────────────────────────────────────────────────────────────────

/// How a read ended short of a hash.
pub(crate) enum Short {
    Changed,
    Unreadable(HashRefusal),
    Cancelled,
}

impl Short {
    /// What a content check says of a file that could not be hashed.
    pub(crate) fn into_check(self) -> Check {
        match self {
            Self::Changed => Check::Changed,
            Self::Unreadable(refusal) => Check::Unread(refusal.to_string()),
            Self::Cancelled => Check::Unread("the read was cancelled".to_owned()),
        }
    }
}

/// One file a check reads: its path, and the catalog row its hash is stored
/// on where it has one.
#[derive(Clone, Debug)]
pub struct CheckSide {
    path: PathBuf,
    row: Option<HashRow>,
    fresh: bool,
}

impl CheckSide {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            row: None,
            fresh: false,
        }
    }

    /// At full depth, read the file now: [`HashAsk::fresh`].
    pub fn fresh(mut self) -> Self {
        self.fresh = true;
        self
    }

    /// The catalog row the file's hash is stored on.
    pub fn of_row(mut self, row: HashRow) -> Self {
        self.row = Some(row);
        self
    }

    /// [`Self::of_row`] where there is one.
    pub fn of_row_if(mut self, row: Option<HashRow>) -> Self {
        self.row = row;
        self
    }

    fn ask(self) -> HashAsk {
        HashAsk {
            row: self.row,
            fresh: self.fresh,
            ..HashAsk::new(self.path)
        }
    }
}

impl From<&Path> for CheckSide {
    fn from(path: &Path) -> Self {
        Self::new(path)
    }
}

impl From<PathBuf> for CheckSide {
    fn from(path: PathBuf) -> Self {
        Self::new(path)
    }
}

/// What a full-hash answer says to a content check.
fn hashed_for_check(answer: FullHashAnswer) -> Result<KnownHash, Check> {
    match answer {
        FullHashAnswer::Cached(known)
        | FullHashAnswer::Stored(known)
        | FullHashAnswer::Read(known) => Ok(known),
        FullHashAnswer::Changed => Err(Check::Changed),
        FullHashAnswer::Unreadable(refusal) => Err(Check::Unread(refusal.to_string())),
        FullHashAnswer::Cancelled => Err(Check::Unread("the read was cancelled".to_owned())),
        FullHashAnswer::Unrecorded => Err(Check::Unread("nothing recorded fits".to_owned())),
    }
}

impl Disk {
    /// Install the table [`Self::full_hash`] loads from and saves to. Only
    /// the first call has an effect. With none installed, every hash is kept
    /// in the in-process cache.
    pub fn install_hash_store(&self, store: Arc<dyn HashStore>) {
        let _ = self.inner.hash_store.set(store);
    }

    /// Whether two files hold the same content, at `depth`, the user's
    /// setting. At [`Depth::Full`] each file's [`Self::full_hash`] answers
    /// and the two are compared; at any other depth both are sampled live,
    /// length and head and tail, nothing stored taking part. A file that
    /// changed while it was read is [`Check::Changed`]. Nothing is changed.
    ///
    /// Probes: at full depth, [`Self::full_hash`]'s for each file; otherwise
    /// one open and two stats of the handle for each, and its two windows.
    pub async fn check(
        &self,
        left: impl Into<CheckSide>,
        right: impl Into<CheckSide>,
        depth: Depth,
    ) -> Check {
        let (left, right) = (left.into(), right.into());
        if depth != Depth::Full {
            let disk = self.clone();
            return tokio::task::spawn_blocking(move || {
                disk.samples_checked(&left.path, &right.path)
            })
            .await
            .unwrap_or_else(|error| Check::Unread(error.to_string()));
        }
        let left = match hashed_for_check(self.full_hash(left.ask()).await) {
            Ok(known) => known,
            Err(short) => return short,
        };
        let right = match hashed_for_check(self.full_hash(right.ask()).await) {
            Ok(known) => known,
            Err(short) => return short,
        };
        judged(left.size_bytes == right.size_bytes && left.blake3 == right.blake3)
    }

    /// Whether one file still holds the content its catalog recorded as
    /// `recorded`, the whole-file BLAKE3: [`Self::full_hash`] answers and the
    /// two are compared. With nothing recorded there is nothing to differ
    /// from: the hash is recorded on the side's row and the file is
    /// [`Check::Same`]. This is the full-depth check of one file; at quick
    /// depth one file alone has nothing to be compared with.
    ///
    /// Probes: [`Self::full_hash`]'s.
    pub async fn check_against(&self, side: impl Into<CheckSide>, recorded: Option<&str>) -> Check {
        match hashed_for_check(self.full_hash(side.into().ask()).await) {
            Ok(known) => judged(recorded.is_none_or(|recorded| recorded == known.blake3)),
            Err(short) => short,
        }
    }

    /// The full hash of `ask`'s path. See the module text. The caller must
    /// not hold the database's writer while it awaits this.
    pub async fn full_hash(&self, ask: HashAsk) -> FullHashAnswer {
        let _turn = self.inner.hash_turns.take(&ask.path).await;
        let store = ask.row.clone().zip(self.inner.hash_store.get().cloned());

        let as_stored = |stored: StoredHash| {
            let mark = stored.mark?;
            Some(FullHashAnswer::Stored(KnownHash {
                size_bytes: mark.len()?,
                blake3: stored.blake3,
                crc: stored.crc,
                mark,
            }))
        };
        let recorded = match (&ask.recorded, &store) {
            _ if ask.fresh => None,
            (Some(stored), _) => as_stored(stored.clone()),
            (None, Some((row, store))) => store.stored(row).await.and_then(as_stored),
            (None, None) => self
                .inner
                .cache
                .hashes
                .kept(&ask.path)
                .map(FullHashAnswer::Cached),
        };
        if let Some(recorded) = recorded
            && let Ok(now) = self.fingerprint_followed(ask.path.clone()).await
            && recorded
                .known()
                .is_some_and(|known| still_fits(&known.mark, &now))
        {
            return recorded;
        }
        if ask.recorded_only {
            return FullHashAnswer::Unrecorded;
        }

        let Ok(_read) = self.inner.hash_turns.reads.acquire().await else {
            return could_not_run();
        };
        let epoch = self.inner.cache.epoch();
        let disk = self.clone();
        let path = ask.path.clone();
        let Ok(read) = tokio::task::spawn_blocking(move || disk.read_whole_blocking(&ask)).await
        else {
            return could_not_run();
        };
        let known = match read {
            Ok(known) => known,
            Err(Short::Changed) => return FullHashAnswer::Changed,
            Err(Short::Unreadable(refusal)) => return FullHashAnswer::Unreadable(refusal),
            Err(Short::Cancelled) => return FullHashAnswer::Cancelled,
        };
        match &store {
            Some((row, store)) => store.record(row, &known).await,
            None => self.inner.cache.record_hash(&path, known.clone(), epoch),
        }
        FullHashAnswer::Read(known)
    }

    /// Read the file end to end once, hashing every byte, and mark it through
    /// the handle as it opens and again at the end. The two marks must show
    /// the file untouched and of the length read; the hash is recorded
    /// against the first.
    ///
    /// Probes: one open and two stats of the handle.
    pub(crate) fn read_whole_blocking(&self, ask: &HashAsk) -> Result<KnownHash, Short> {
        let path = ask.path.as_path();
        if ask.cancel.is_cancelled() {
            return Err(Short::Cancelled);
        }
        let refusal = |what: &str, error: DiskError| {
            Short::Unreadable(HashRefusal {
                reason: error.reason(),
                sentence: format!(
                    "failed to {what} {} for its full hash: {}",
                    path.display(),
                    io::Error::from(error)
                ),
            })
        };
        let mut file = self
            .open_blocking(path, Open::read().sequentially())
            .map_err(|error| refusal("open", error))?;
        self.note_whole_read(path);
        let handle_mark = |file: &std::fs::File| {
            self.call("handle_stat", || file.metadata())
                .map(|metadata| Mark::of(&metadata))
                .map_err(|error| refusal("read", DiskError::new(DiskVerb::Read, path, error)))
        };
        let opened = handle_mark(&file)?;
        let mut hasher = ContentHasher::new();
        let mut buffer = vec![0u8; CHUNK_BYTES];
        loop {
            if ask.cancel.is_cancelled() {
                return Err(Short::Cancelled);
            }
            let started = std::time::Instant::now();
            let read = file
                .read(&mut buffer)
                .map_err(|error| refusal("read", DiskError::new(DiskVerb::Read, path, error)))?;
            if let Some(times) = &ask.times {
                times
                    .read_ns
                    .fetch_add(elapsed_ns(started), Ordering::Relaxed);
                times.bytes.fetch_add(read as u64, Ordering::Relaxed);
            }
            if read == 0 {
                break;
            }
            let started = std::time::Instant::now();
            hasher.update(&buffer[..read]);
            if let Some(times) = &ask.times {
                times
                    .hash_ns
                    .fetch_add(elapsed_ns(started), Ordering::Relaxed);
            }
            ask.progress.report(Phase::Verifying, hasher.size_bytes());
        }
        let hashes = hasher.finalize();
        let closing = handle_mark(&file)?;
        if !opened.compare(&closing).is_untouched() || !opened.is_len(hashes.size_bytes) {
            return Err(Short::Changed);
        }
        if ask.cancel.is_cancelled() {
            return Err(Short::Cancelled);
        }
        Ok(KnownHash {
            size_bytes: hashes.size_bytes,
            blake3: hashes.blake3,
            crc: Some(hashes.crc),
            mark: opened,
        })
    }
}

fn could_not_run() -> FullHashAnswer {
    FullHashAnswer::Unreadable(HashRefusal {
        reason: reason::DISK_OPERATION_FAILED,
        sentence: "the full-hash read could not run".to_owned(),
    })
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
pub(crate) fn fits_for_test(recorded: &Mark, now: &Mark) -> bool {
    still_fits(recorded, now)
}
