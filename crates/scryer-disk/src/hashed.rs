//! Whole-file hashes: the one read that computes them, and the one answer to
//! what a file's full hash is.
//!
//! A caller asks [`Disk::full_hash`] with a [`HashAsk`]: the path, a mark of
//! it the caller read in this unit of work after its last write to the path,
//! the catalog row the file has, if any, and whether a hash recorded earlier
//! may answer ([`Fresh::MayReuse`]) or the bytes must be read now
//! ([`Fresh::MustRead`]). The answer is typed: from the in-process cache,
//! from the stored table, read now, or why not.
//!
//! [`Fresh::MayReuse`] asks the cache, then the table, then reads. A
//! recorded hash answers only while [`Mark::content_unchanged_since`] holds
//! between the caller's mark and the mark it was recorded against. A call
//! with no mark always reads: nothing is reused on a guess, and the service
//! never probes for a mark itself. [`Fresh::MustRead`] always reads; it is for
//! a check that decides a removal, which bytes and not metadata must decide.
//!
//! Every read that completes is recorded: in the cache, and, when the ask
//! names a row and a store is installed, in the table. A read that saw the
//! file change, failed or was cancelled records nothing. The table is
//! written only from [`Disk::full_hash`] and [`Disk::record_full_hash`],
//! which run on the async runtime, never on a blocking thread, and the
//! caller must not hold the database's writer while it awaits them.
//!
//! The cache is bounded and in memory. It is keyed by path, holds the mark
//! each hash was recorded against, and forgets a path whenever the owner
//! changes it, as the stat cache does. It has no time-based reuse.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::cache::{climbs, key_of, slot_of};
use super::copy::{CHUNK_BYTES, ContentHasher, HashRefusal};
use super::reading::DiskVerb;
use super::{Cancel, Disk, DiskError, Mark, Open, Phase, Progress, reason};

/// Whether a hash recorded earlier may answer an ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fresh {
    /// A recorded hash answers while the file is still the file it was
    /// recorded of. For a caller that does not decide a removal.
    MayReuse,
    /// Read the bytes now, whatever is recorded, and record what was read.
    MustRead,
}

/// The catalog row a file's hash is stored on. There is no lookup by path
/// alone: nothing indexes it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HashRow {
    /// The media file row's id.
    Id(String),
    /// The title's row for the file at this stored path.
    TitlePath { title_id: String, path: String },
}

/// A whole-file hash and the mark of the file it describes.
#[derive(Clone, Debug)]
pub struct KnownHash {
    pub size_bytes: u64,
    /// The whole-file BLAKE3, in hex.
    pub blake3: String,
    /// The whole-file CRC-64/NVME, where it was taken or stored.
    pub crc: Option<u64>,
    /// The file the hash was taken of, read with links followed.
    pub mark: Mark,
}

impl KnownHash {
    /// Whether this hash still describes the file `now` was read of. See
    /// [`Mark::content_unchanged_since`].
    pub fn describes(&self, now: &Mark) -> bool {
        now.content_unchanged_since(&self.mark) && now.is_len(self.size_bytes)
    }
}

/// What [`Disk::full_hash`] answered.
#[derive(Debug)]
pub enum FullHashAnswer {
    /// Recorded in this process, of the file the caller's mark describes.
    Cached(KnownHash),
    /// Stored on the row, of the file the caller's mark describes.
    Stored(KnownHash),
    /// Read end to end now, and recorded.
    Read(KnownHash),
    /// The file changed while it was read; nothing was recorded.
    Changed,
    /// The file could not be read end to end.
    Unreadable(HashRefusal),
    /// The ask was cancelled before the read finished.
    Cancelled,
}

impl FullHashAnswer {
    /// The hash, however it was answered.
    pub fn known(&self) -> Option<&KnownHash> {
        match self {
            Self::Cached(known) | Self::Stored(known) | Self::Read(known) => Some(known),
            Self::Changed | Self::Unreadable(_) | Self::Cancelled => None,
        }
    }

    /// [`Self::known`], owned.
    pub fn into_known(self) -> Option<KnownHash> {
        match self {
            Self::Cached(known) | Self::Stored(known) | Self::Read(known) => Some(known),
            Self::Changed | Self::Unreadable(_) | Self::Cancelled => None,
        }
    }
}

/// A future a [`HashStore`] returns.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The persisted table of full hashes, which the application keeps on its
/// catalog rows. Installed once with [`Disk::install_hash_store`].
pub trait HashStore: Send + Sync {
    /// The hash stored on `row` and the mark it was stored against. `None`
    /// when there is none or the table could not say; a failure is never an
    /// answer.
    fn stored<'a>(&'a self, row: &'a HashRow) -> StoreFuture<'a, Option<KnownHash>>;
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
    mark: Option<Mark>,
    row: Option<HashRow>,
    fresh: Fresh,
    cancel: Cancel,
    progress: Progress,
    background: Option<fn()>,
    times: Option<Arc<ReadTimes>>,
}

impl HashAsk {
    /// Ask for `path`'s hash. `mark` is a read of the path the caller took in
    /// this unit of work after its last write to it, with links followed;
    /// without one, the file is read.
    pub fn new(path: impl Into<PathBuf>, mark: Option<Mark>, fresh: Fresh) -> Self {
        Self {
            path: path.into(),
            mark,
            row: None,
            fresh,
            cancel: Cancel::never(),
            progress: Progress::none(),
            background: None,
            times: None,
        }
    }

    /// The catalog row the file's hash is stored on.
    pub fn of_row(mut self, row: HashRow) -> Self {
        self.row = Some(row);
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

    /// Background work: the read runs on a thread of its own, which
    /// `lower_priority` lowers before it reads.
    pub fn in_background(mut self, lower_priority: fn()) -> Self {
        self.background = Some(lower_priority);
        self
    }

    pub fn timed(mut self, times: Arc<ReadTimes>) -> Self {
        self.times = Some(times);
        self
    }
}

// ── The cache ────────────────────────────────────────────────────────────────

/// Paths kept at most. Past that, the oldest are dropped down to three
/// quarters of this, so trimming is not paid per insert.
pub(crate) const HASH_CACHE_MAX: usize = 4096;

#[derive(Debug, Default)]
struct Held {
    /// Keyed by the slot of the spelling; the spelling and the hash.
    entries: BTreeMap<PathBuf, (PathBuf, KnownHash)>,
    /// Slots in the order they were recorded, oldest first; a slot recorded
    /// again or forgotten may still appear and is skipped.
    order: VecDeque<PathBuf>,
}

/// Whole-file hashes recorded in this process. See the module text.
#[derive(Debug, Default)]
pub(crate) struct HashCache {
    held: Mutex<Held>,
}

impl HashCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The hash recorded for exactly `path` that still describes `now`.
    fn answer(&self, path: &Path, now: &Mark) -> Option<KnownHash> {
        let key = key_of(path);
        let held = self.lock();
        let (spelling, known) = held.entries.get(&slot_of(&key))?;
        (*spelling == key && known.describes(now)).then(|| known.clone())
    }

    pub(crate) fn record(&self, path: &Path, known: KnownHash) {
        let key = key_of(path);
        if climbs(&key) {
            return;
        }
        let slot = slot_of(&key);
        let mut held = self.lock();
        held.entries.insert(slot.clone(), (key, known));
        held.order.push_back(slot);
        if held.entries.len() > HASH_CACHE_MAX {
            let keep = HASH_CACHE_MAX * 3 / 4;
            while held.entries.len() > keep {
                let Some(oldest) = held.order.pop_front() else {
                    break;
                };
                held.entries.remove(&oldest);
            }
        }
        if held.order.len() > HASH_CACHE_MAX * 2 {
            let Held { entries, order } = &mut *held;
            order.retain(|slot| entries.contains_key(slot));
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

// ── The read ─────────────────────────────────────────────────────────────────

/// How a read ended short of a hash.
enum Short {
    Changed,
    Unreadable(HashRefusal),
    Cancelled,
}

impl Disk {
    /// Install the table [`Self::full_hash`] consults and records to. Only
    /// the first call has an effect. With none installed, the table step is
    /// skipped.
    pub fn install_hash_store(&self, store: Arc<dyn HashStore>) {
        let _ = self.inner.hash_store.set(store);
    }

    fn hash_store(&self) -> Option<&Arc<dyn HashStore>> {
        self.inner.hash_store.get()
    }

    /// The full hash of `ask`'s path. See the module text. Reads run off the
    /// async runtime.
    pub async fn full_hash(&self, ask: HashAsk) -> FullHashAnswer {
        if let Some(answer) = self.recorded(&ask) {
            return answer;
        }
        if ask.fresh == Fresh::MayReuse
            && let (Some(now), Some(row), Some(store)) = (&ask.mark, &ask.row, self.hash_store())
            && let Some(stored) = store.stored(row).await
            && stored.describes(now)
        {
            self.inner.cache.hashes.record(&ask.path, stored.clone());
            return FullHashAnswer::Stored(stored);
        }
        let row = ask.row.clone();
        let disk = self.clone();
        let answer = match ask.background {
            Some(lower_priority) => {
                let (send, receive) = tokio::sync::oneshot::channel();
                let spawned = std::thread::Builder::new()
                    .name("full-hash-read".into())
                    .spawn(move || {
                        lower_priority();
                        let _ = send.send(disk.read_and_record_blocking(&ask));
                    });
                match spawned {
                    Ok(_) => receive.await.ok(),
                    Err(_) => None,
                }
            }
            None => tokio::task::spawn_blocking(move || disk.read_and_record_blocking(&ask))
                .await
                .ok(),
        };
        let Some(answer) = answer else {
            return FullHashAnswer::Unreadable(HashRefusal {
                reason: reason::DISK_OPERATION_FAILED,
                sentence: "the full-hash read could not run".to_owned(),
            });
        };
        if let (FullHashAnswer::Read(known), Some(row)) = (&answer, &row) {
            self.record_full_hash(row, known).await;
        }
        answer
    }

    /// [`Self::full_hash`] on a blocking thread: the cache, then a read. The
    /// table is neither consulted nor written; an async caller that holds the
    /// row stores a read with [`Self::record_full_hash`].
    pub fn full_hash_blocking(&self, ask: &HashAsk) -> FullHashAnswer {
        self.recorded(ask)
            .unwrap_or_else(|| self.read_and_record_blocking(ask))
    }

    /// Store `known`, read now, on `row`, when a store is installed. The
    /// caller must not hold the database's writer.
    pub async fn record_full_hash(&self, row: &HashRow, known: &KnownHash) {
        if let Some(store) = self.hash_store() {
            store.record(row, known).await;
        }
    }

    /// Record `known`, a hash of `path` taken by a whole read the caller ran
    /// through this owner, in the cache.
    pub fn note_full_hash(&self, path: &Path, known: KnownHash) {
        self.inner.cache.hashes.record(path, known);
    }

    /// The cached answer `ask` may take.
    fn recorded(&self, ask: &HashAsk) -> Option<FullHashAnswer> {
        if ask.fresh != Fresh::MayReuse {
            return None;
        }
        let now = ask.mark.as_ref()?;
        self.inner
            .cache
            .hashes
            .answer(&ask.path, now)
            .map(FullHashAnswer::Cached)
    }

    fn read_and_record_blocking(&self, ask: &HashAsk) -> FullHashAnswer {
        let epoch = self.inner.cache.epoch();
        match self.read_whole_blocking(ask) {
            Ok(known) => {
                if self.inner.cache.epoch() == epoch {
                    self.inner.cache.hashes.record(&ask.path, known.clone());
                }
                FullHashAnswer::Read(known)
            }
            Err(Short::Changed) => FullHashAnswer::Changed,
            Err(Short::Unreadable(refusal)) => FullHashAnswer::Unreadable(refusal),
            Err(Short::Cancelled) => FullHashAnswer::Cancelled,
        }
    }

    /// Read the file end to end once, hashing every byte.
    ///
    /// A mark of the handle is taken as it opens. With no mark from the
    /// caller, one followed stat of the name after the read must show the
    /// same file untouched: it binds the name to the bytes read, and is the
    /// mark the hash is recorded against. A caller that passed a mark
    /// brackets the read of the name itself: the opened handle must still
    /// hold the content the caller's mark describes, a second mark of the
    /// handle at the end must show it untouched, and the hash is recorded
    /// against the caller's mark, with no stat of the name.
    ///
    /// Probes: one open and one stat of the handle; then one followed stat
    /// of the name with no mark from the caller, or a second stat of the
    /// handle with one.
    fn read_whole_blocking(&self, ask: &HashAsk) -> Result<KnownHash, Short> {
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
        if let Some(seen) = &ask.mark
            && !opened.content_unchanged_since(seen)
        {
            return Err(Short::Changed);
        }
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
        let mark = match &ask.mark {
            Some(seen) => {
                let closing = handle_mark(&file)?;
                if !opened.compare(&closing).is_untouched() {
                    return Err(Short::Changed);
                }
                seen.clone()
            }
            None => {
                drop(file);
                let after = self
                    .fingerprint_followed_blocking(path)
                    .map_err(|_| Short::Changed)?;
                if !opened.compare(&after).is_untouched() {
                    return Err(Short::Changed);
                }
                after
            }
        };
        if ask.cancel.is_cancelled() {
            return Err(Short::Cancelled);
        }
        if !mark.is_len(hashes.size_bytes) {
            return Err(Short::Changed);
        }
        Ok(KnownHash {
            size_bytes: hashes.size_bytes,
            blake3: hashes.blake3,
            crc: Some(hashes.crc),
            mark,
        })
    }
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}
