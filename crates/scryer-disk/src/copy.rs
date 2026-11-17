//! Copying a file's bytes, and proving the copy.
//!
//! Every byte a recorded copy moves is read once at the source and fed to two
//! hashers: a streaming CRC, which a full-depth read-back of the destination
//! is compared against, and a whole-file BLAKE3, which the caller keeps as
//! the file's identity so nothing has to read it again.
//!
//! # The CRC: CRC-64/NVME
//!
//! Measured with a throwaway harness (256 MiB in-memory buffer, 1 MiB update
//! chunks, best of 7 passes, `opt-level = 3`) on an Apple M5 Max under heavy
//! concurrent build load, so these are contended lower bounds and only the
//! relative order is load-independent:
//!
//! | Candidate | Throughput |
//! |---|---|
//! | CRC-64/NVME | 0.36 – 0.58 GiB/s |
//! | CRC-64/GO-ISO | 0.40 – 0.59 GiB/s |
//! | CRC-64/XZ (ECMA-182) | 0.33 – 0.54 GiB/s |
//! | CRC-32/ISCSI (Castagnoli) | 0.88 – 1.09 GiB/s |
//! | CRC-32/ISO-HDLC | 0.52 – 1.08 GiB/s |
//! | BLAKE3 (single-threaded, reference) | 1.27 – 2.31 GiB/s |
//! | Combined pass (CRC-64/NVME + BLAKE3) | 0.26 – 0.46 GiB/s |
//!
//! CRC-64/NVME is the fastest 64-bit variant on this hardware; the 32-bit
//! ones are about twice as fast but too narrow for multi-gigabyte media
//! files. The extra cost is not on the critical path: a copy is bounded by
//! the device and the network, and the combined pass by BLAKE3. Re-measure if
//! the default ever moves; [`CrcKind`] exists so a stored CRC stays
//! interpretable across such a change.
//!
//! # Depth
//!
//! **Full** re-reads the whole destination with the platform's cache bypass
//! engaged (`F_NOCACHE` on macOS, `POSIX_FADV_DONTNEED` on Linux) and compares
//! the CRC and byte count against what the copy streamed. **Quick** compares
//! the destination's sampled first and last windows against the source's,
//! plus its size against the streamed size. Quick is the floor: when the full
//! read-back cannot run, the proof is the quick one, marked as a fallback
//! with the reason. It is never a pass and never something weaker.
//!
//! # The staged name
//!
//! A staged recorded copy is written beside its destination under
//! [`resumable_staging_path`] and takes the real name only once complete, so
//! after a crash, a full disk or a dropped mount the destination either does
//! not exist or holds a complete file. The name is deterministic so the
//! caller that journaled the attempt can find what it left. A name already
//! occupied refuses the copy: only that caller can tell its own interrupted
//! attempt from a file that happens to carry the suffix.
//!
//! A recorded copy removes nothing but a staged file this call created and
//! could not finish. A destination that does not prove is kept exactly as
//! written, and the source is never touched.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::mark::UNKNOWN_LANE;
use super::place::{Fail, Job, Leftover, Refused, Side};
use super::reading::reason;
use super::sample::{ContentSample, SampleStep, sample_reader};
use super::{
    Disk, DiskError, DiskVerb, Mark, Open, PathReading, Remove, Replace, Sameness, SyncOutcome,
};

/// Bytes moved per copy iteration, and per read-back iteration.
pub(crate) const CHUNK_BYTES: usize = 1024 * 1024;

/// The CRC a stored value was computed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrcKind {
    Crc64Nvme,
}

impl CrcKind {
    fn algorithm(self) -> crc_turbo::CrcAlgorithm {
        match self {
            Self::Crc64Nvme => crc_turbo::CrcAlgorithm::Crc64Nvme,
        }
    }
}

/// What one pass over a file's bytes produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentHashes {
    pub size_bytes: u64,
    pub crc_kind: CrcKind,
    pub crc: u64,
    /// The whole-file BLAKE3, in hex.
    pub blake3: String,
}

/// Feeds every buffer to both hashers, so the bytes are read exactly once.
pub struct ContentHasher {
    crc: crc_turbo::Digest,
    blake3: blake3::Hasher,
    size_bytes: u64,
}

impl ContentHasher {
    pub fn new() -> Self {
        Self {
            crc: crc_turbo::Digest::new(CrcKind::Crc64Nvme.algorithm()),
            blake3: blake3::Hasher::new(),
            size_bytes: 0,
        }
    }

    pub fn update(&mut self, buffer: &[u8]) {
        self.crc.update(buffer);
        self.blake3.update(buffer);
        self.size_bytes = self.size_bytes.saturating_add(buffer.len() as u64);
    }

    /// Bytes absorbed so far.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn finalize(self) -> ContentHashes {
        ContentHashes {
            size_bytes: self.size_bytes,
            crc_kind: CrcKind::Crc64Nvme,
            crc: self.crc.finalize(),
            blake3: self.blake3.finalize().to_hex().to_string(),
        }
    }
}

impl Default for ContentHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// How hard a recorded copy is proven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// Not at all: the caller proves the copy from its hashes.
    Unproven,
    /// Size, and the sampled first and last windows against the source's.
    Quick,
    /// The whole destination read back against the streamed CRC.
    Full,
}

/// Where a recorded copy writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// Beside the destination under [`resumable_staging_path`], taking the
    /// real name once complete.
    Staged,
    /// Under the destination name itself. A policy that does not replace
    /// creates the name and fails if it is held; one that replaces writes
    /// over the file the caller holds there. Nothing is removed on failure.
    Direct,
}

/// The evidence a recorded copy gathers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub depth: Depth,
    pub landing: Landing,
}

/// What a proof concluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The destination matched at the depth applied.
    Verified,
    /// The destination did not match.
    Mismatch,
    /// The proof could not run, so nothing is proven.
    Unavailable,
}

/// One destination's proof against the hashes streamed while it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// The depth asked for: [`Depth::Quick`] or [`Depth::Full`].
    pub asked: Depth,
    /// True when the full read-back could not run and the quick proof stood
    /// in for it.
    pub fell_back: bool,
    pub verdict: Verdict,
    /// Why the verdict is not a plain pass, or why the depth fell back.
    pub detail: Option<String>,
}

/// What a recorded copy produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copied {
    pub hashes: ContentHashes,
    /// Absent at [`Depth::Unproven`].
    pub proof: Option<Proof>,
}

/// What a watched copy is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for its turn on the destination's volume.
    Waiting,
    Copying,
    /// Every byte is written; the flush, the sync and the naming remain.
    /// Reported only by a copy asked to report its start and end.
    Finishing,
    Verifying,
}

/// Where a copy reports how far it is: the phase, and the bytes written or
/// read back in it so far. Called on the copy's own thread once per chunk, so
/// it must do nothing but bookkeeping.
#[derive(Clone, Default)]
pub struct Progress(Option<Arc<dyn Fn(Phase, u64) + Send + Sync>>);

impl Progress {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn to(sink: impl Fn(Phase, u64) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(sink)))
    }

    pub(crate) fn report(&self, phase: Phase, bytes: u64) {
        if let Some(sink) = &self.0 {
            sink(phase, bytes);
        }
    }
}

/// Asked before every chunk whether the copy should stop.
#[derive(Clone)]
pub struct Cancel(Option<Arc<dyn Fn() -> bool + Send + Sync>>);

impl Cancel {
    pub fn never() -> Self {
        Self(None)
    }

    pub fn when(asked: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(asked)))
    }

    fn is_cancelled(&self) -> bool {
        self.0.as_ref().is_some_and(|asked| asked())
    }
}

/// Who is watching a placement, and who may stop it.
#[derive(Clone)]
pub struct Watch {
    pub progress: Progress,
    pub cancel: Cancel,
}

impl Watch {
    pub(crate) fn nobody() -> Self {
        Self {
            progress: Progress::none(),
            cancel: Cancel::never(),
        }
    }

    fn is_nobody(&self) -> bool {
        self.progress.0.is_none() && self.cancel.0.is_none()
    }
}

/// A file that could not be read end to end. It displays the sentence for a
/// person; [`Self::reason`] is the code.
#[derive(Debug, thiserror::Error)]
#[error("{sentence}")]
pub struct HashRefusal {
    reason: &'static str,
    sentence: String,
}

impl HashRefusal {
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

// ── Admission ────────────────────────────────────────────────────────────────

/// How many copies may write to one volume at a time. Proving a copy holds
/// no turn.
#[derive(Clone)]
pub struct CopyAdmission {
    volumes: Arc<Mutex<HashMap<String, Weak<Semaphore>>>>,
    capacity: usize,
    #[cfg(test)]
    asking: Arc<Asking>,
}

/// One copy's turn on a volume, given back when dropped.
pub struct CopyPermit(#[allow(dead_code)] OwnedSemaphorePermit);

/// How many callers are inside [`CopyAdmission::acquire_keyed`] right now,
/// for a test to wait on.
#[cfg(test)]
#[derive(Default)]
struct Asking {
    count: std::sync::atomic::AtomicUsize,
    changed: tokio::sync::Notify,
}

/// One caller asking for a turn, counted until its ask ends or is dropped.
#[cfg(test)]
struct AskingOne(Arc<Asking>);

#[cfg(test)]
impl Drop for AskingOne {
    fn drop(&mut self) {
        let count = &self.0.count;
        count.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

impl CopyAdmission {
    pub fn new(capacity: usize) -> Self {
        Self {
            volumes: Arc::new(Mutex::new(HashMap::new())),
            capacity: capacity.clamp(1, 8),
            #[cfg(test)]
            asking: Arc::default(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Waits until exactly `count` callers are asking for a turn. The caller
    /// bounds the wait.
    #[cfg(all(test, unix))]
    pub(crate) async fn asking_is(&self, count: usize) {
        loop {
            let mut changed = std::pin::pin!(self.asking.changed.notified());
            changed.as_mut().enable();
            if self.asking.count.load(std::sync::atomic::Ordering::SeqCst) == count {
                return;
            }
            changed.await;
        }
    }

    /// A turn on the volume a mark names, without a probe. A mark that
    /// carries no volume, as a plain Windows read does not, shares the
    /// unknown lane.
    pub async fn acquire_on(&self, volume: &Mark) -> CopyPermit {
        self.acquire_keyed(&volume.lane()).await
    }

    /// A turn in a named lane.
    pub(crate) async fn acquire_keyed(&self, key: &str) -> CopyPermit {
        #[cfg(test)]
        let _asking = {
            let count = &self.asking.count;
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.asking.changed.notify_waiters();
            AskingOne(self.asking.clone())
        };
        let semaphore = {
            let mut volumes = self.volumes.lock().unwrap_or_else(|e| e.into_inner());
            volumes.retain(|_, value| value.strong_count() > 0);
            if let Some(semaphore) = volumes.get(key).and_then(Weak::upgrade) {
                semaphore
            } else {
                let semaphore = Arc::new(Semaphore::new(self.capacity));
                volumes.insert(key.to_owned(), Arc::downgrade(&semaphore));
                semaphore
            }
        };
        CopyPermit(
            semaphore
                .acquire_owned()
                .await
                .expect("copy admission is never closed"),
        )
    }
}

/// Waits for `turn` on this thread, parked between wake-ups. The turn's
/// future needs no runtime, so this works on a blocking thread of one and on
/// a thread that has none.
fn wait_parked<T>(turn: impl std::future::Future<Output = T>) -> T {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut turn = std::pin::pin!(turn);
    loop {
        match turn.as_mut().poll(&mut context) {
            std::task::Poll::Ready(turn) => return turn,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

// ── The read-back seam ───────────────────────────────────────────────────────

/// How the destination was opened for a full-depth read-back.
#[doc(hidden)]
pub enum ReadBackHandle {
    Ready {
        file: File,
        bypass: CacheBypass,
    },
    /// The read-back cannot run here; the proof falls back to the quick one
    /// with this reason.
    Unsupported(String),
    /// The destination could not be opened, so nothing can be proven.
    Unavailable(String),
}

/// Whether the read-back is bypassing the page cache.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheBypass {
    Engaged,
    /// This platform offers no bypass. The full read-back still runs, and
    /// the proof's detail says so.
    NotAvailableOnPlatform,
    /// The filesystem rejected the bypass for this file. A cached full
    /// read-back is still stronger than the quick proof, so it proceeds and
    /// the detail says so.
    Rejected(String),
}

/// Opens a destination for the full-depth read-back. Tests substitute one to
/// reach the fallback without an exotic filesystem.
#[doc(hidden)]
pub type ReadBackOpener = Arc<dyn Fn(&Path) -> ReadBackHandle + Send + Sync>;

/// Opens `path` on the process's owner with the platform's page-cache bypass
/// engaged.
#[doc(hidden)]
pub fn open_cache_bypassed(path: &Path) -> ReadBackHandle {
    Disk::process().open_read_back(path)
}

// macOS: `F_NOCACHE` keeps this read out of the unified buffer cache. Pages
// already resident from the write may still be served, so it is a strong hint
// rather than a guarantee of a device read.
#[cfg(target_os = "macos")]
fn engage_cache_bypass(file: File) -> ReadBackHandle {
    use std::os::unix::io::AsRawFd;

    // SAFETY: `file` owns the descriptor for the duration of the call.
    let code = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) };
    if code == -1 {
        return ReadBackHandle::Ready {
            file,
            bypass: CacheBypass::Rejected(format!(
                "F_NOCACHE was rejected for this file: {}",
                io::Error::last_os_error()
            )),
        };
    }
    ReadBackHandle::Ready {
        file,
        bypass: CacheBypass::Engaged,
    }
}

// Linux: `POSIX_FADV_DONTNEED` over the whole file, issued after the copy's
// fsync so the clean pages can be dropped and the read-back goes to the
// device. `O_DIRECT` is deliberately not used: it demands block-aligned
// buffers and offsets and is unsupported on several filesystems media
// libraries live on, which would turn most full proofs into quick ones.
#[cfg(target_os = "linux")]
fn engage_cache_bypass(file: File) -> ReadBackHandle {
    use std::os::unix::io::AsRawFd;

    // SAFETY: `file` owns the descriptor for the duration of the call. A
    // length of 0 means "to end of file".
    let code = unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
    // posix_fadvise reports errors by return value, not errno.
    if code != 0 {
        return ReadBackHandle::Ready {
            file,
            bypass: CacheBypass::Rejected(format!(
                "POSIX_FADV_DONTNEED was rejected for this file: {}",
                io::Error::from_raw_os_error(code)
            )),
        };
    }
    ReadBackHandle::Ready {
        file,
        bypass: CacheBypass::Engaged,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn engage_cache_bypass(file: File) -> ReadBackHandle {
    ReadBackHandle::Ready {
        file,
        bypass: CacheBypass::NotAvailableOnPlatform,
    }
}

// ── The staged name ──────────────────────────────────────────────────────────

/// Suffix a staged recorded copy carries until it takes the real name.
pub const RESUMABLE_STAGING_SUFFIX: &str = ".scryer-partial";

/// The longest file name, in bytes, the filesystems Scryer writes to accept.
pub const FILE_NAME_MAX_BYTES: usize = 255;

/// Where a staged recorded copy of `destination` is written: a sibling in the
/// same folder, so taking the real name is a rename inside one filesystem.
///
/// A name too long to carry the suffix is shortened and given a hash of the
/// whole name, so the staged name stays deterministic and two long names that
/// share a prefix still stage apart.
pub fn resumable_staging_path(destination: &Path) -> PathBuf {
    let name = destination.file_name().unwrap_or_default();
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if name.as_encoded_bytes().len() + RESUMABLE_STAGING_SUFFIX.len() <= FILE_NAME_MAX_BYTES {
        let mut partial = name.to_os_string();
        partial.push(RESUMABLE_STAGING_SUFFIX);
        return parent.join(partial);
    }
    let hash = blake3::hash(name.as_encoded_bytes()).to_hex();
    let hash = &hash.as_str()[..16];
    let Some(name) = name.to_str() else {
        return parent.join(format!("partial-{hash}{RESUMABLE_STAGING_SUFFIX}"));
    };
    let budget = FILE_NAME_MAX_BYTES - RESUMABLE_STAGING_SUFFIX.len() - hash.len() - 1;
    let mut kept = 0;
    for (index, ch) in name.char_indices() {
        if index + ch.len_utf8() > budget {
            break;
        }
        kept = index + ch.len_utf8();
    }
    parent.join(format!(
        "{}-{hash}{RESUMABLE_STAGING_SUFFIX}",
        &name[..kept]
    ))
}

// ── Writing ──────────────────────────────────────────────────────────────────

/// How the file a copy writes into is opened.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opening {
    /// Created by this copy, which fails if the name is held.
    Claim,
    /// Already held by the caller. Opening without creating means the
    /// reservation, not this call, decided the file may be written.
    Held,
    /// Created, or emptied when it is there: for a staged name the caller
    /// chose and this copy has just cleared.
    Fresh,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    OpenSource,
    /// Reading the open source's own facts.
    SourceHandle,
    /// The open source is not the file the caller read.
    SourceChanged,
    /// The open source is not a regular file at all.
    SourceNotFile,
    OpenTarget,
    Read,
    Write,
    /// Reading and writing in one call that does not say which failed.
    Transfer,
    Flush,
    Sync,
    Cancelled,
}

pub(crate) enum Cause {
    Disk(DiskError),
    Io(io::Error),
}

/// A copy that did not finish, and whether this attempt opened the target:
/// the only case in which what sits there is its own.
pub(crate) struct WriteFail {
    pub(crate) step: Step,
    pub(crate) cause: Cause,
    pub(crate) claimed: bool,
}

pub(crate) struct Written {
    pub(crate) hashes: Option<ContentHashes>,
    pub(crate) sync: SyncOutcome,
}

/// What a copy does beyond moving the bytes.
pub(crate) struct Writing<'a> {
    pub(crate) opening: Opening,
    pub(crate) hash: bool,
    pub(crate) sync_folder: bool,
    /// Give the target the source's permissions, so a copy does not silently
    /// change access; permissions the caller applies afterwards take
    /// precedence.
    pub(crate) carry_permissions: bool,
    pub(crate) chunk_bytes: usize,
    /// The source as the caller read it. The file this copy opens must still
    /// be that one, or nothing is written.
    pub(crate) seen: Option<&'a Mark>,
    /// Report [`Phase::Copying`] at nothing once the target is open, and
    /// [`Phase::Finishing`] once the last byte is written.
    pub(crate) phases: bool,
    pub(crate) watch: &'a Watch,
}

/// The one copy loop. The target is fsynced.
pub(crate) fn write_copy(
    disk: &Disk,
    source: &Path,
    target: &Path,
    writing: Writing<'_>,
) -> Result<Written, WriteFail> {
    let fail = |step, cause, claimed| WriteFail {
        step,
        cause,
        claimed,
    };
    let mut input = disk
        .open_blocking(source, Open::read())
        .map_err(|error| fail(Step::OpenSource, Cause::Disk(error), false))?;
    if let Some(seen) = writing.seen {
        let opened = input
            .metadata()
            .map_err(|error| fail(Step::SourceHandle, Cause::Io(error), false))?;
        if !is_still(seen, &Mark::of(&opened)) {
            let changed = io::Error::other("the source changed before the copy");
            let step = match opened.is_file() {
                true => Step::SourceChanged,
                false => Step::SourceNotFile,
            };
            return Err(fail(step, Cause::Io(changed), false));
        }
    }
    let mut output = match writing.opening {
        Opening::Fresh => disk
            .open_blocking(target, Open::create())
            .map_err(|error| fail(Step::OpenTarget, Cause::Disk(error), false))?,
        Opening::Claim => disk
            .open_blocking(target, Open::create_new())
            .map_err(|error| fail(Step::OpenTarget, Cause::Disk(error), false))?,
        Opening::Held => {
            // A link at the held name is refused, never written through or
            // emptied: the reservation covers the name, not what it points at.
            let file = disk
                .open_blocking(target, Open::write_existing().without_following_links())
                .map_err(|error| fail(Step::OpenTarget, Cause::Disk(error), false))?;
            file.set_len(0)
                .map_err(|error| fail(Step::OpenTarget, Cause::Io(error), false))?;
            file
        }
    };
    if writing.phases {
        writing.watch.progress.report(Phase::Copying, 0);
    }

    let hashes = if !writing.hash && writing.watch.is_nobody() {
        io::copy(&mut input, &mut output)
            .map_err(|error| fail(Step::Transfer, Cause::Io(error), true))?;
        None
    } else {
        let mut hasher = writing.hash.then(ContentHasher::new);
        let mut written = 0u64;
        let mut buffer = vec![0u8; writing.chunk_bytes];
        loop {
            if writing.watch.cancel.is_cancelled() {
                let stopped = io::Error::new(io::ErrorKind::Interrupted, "the copy was cancelled");
                return Err(fail(Step::Cancelled, Cause::Io(stopped), true));
            }
            let read = input
                .read(&mut buffer)
                .map_err(|error| fail(Step::Read, Cause::Io(error), true))?;
            if read == 0 {
                break;
            }
            if let Some(hasher) = &mut hasher {
                hasher.update(&buffer[..read]);
            }
            #[cfg(test)]
            if let Some(error) = disk.fault(crate::CopyFault::Write) {
                return Err(fail(Step::Write, Cause::Io(error), true));
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| fail(Step::Write, Cause::Io(error), true))?;
            written = written.saturating_add(read as u64);
            writing.watch.progress.report(Phase::Copying, written);
        }
        if writing.phases {
            writing.watch.progress.report(Phase::Finishing, written);
        }
        hasher.map(ContentHasher::finalize)
    };

    output
        .flush()
        .map_err(|error| fail(Step::Flush, Cause::Io(error), true))?;
    let sync = disk
        .finish_blocking(target, output)
        .map_err(|error| fail(Step::Sync, Cause::Disk(error), true))?;
    if writing.sync_folder {
        if sync == SyncOutcome::Unconfirmed {
            tracing::debug!(
                destination = %target.display(),
                "copy destination filesystem does not support fsync; relying on copy verification"
            );
        }
        // The file's own fsync does not promise its directory entry survives
        // a crash.
        sync_folder_best_effort(disk, target);
    }
    if writing.carry_permissions {
        let _ = disk.copy_permissions_blocking(source, target);
    }
    Ok(Written { hashes, sync })
}

/// Whether `now` is still the regular file `seen` was read as: not shown to
/// be another file, and its length, modification time and owner held.
fn is_still(seen: &Mark, now: &Mark) -> bool {
    now.kind() == Some(super::EntryKind::File) && seen.compare(now).is_unwritten_by_its_owner()
}

/// Best effort: a filesystem that rejects a directory fsync is no reason to
/// fail a copy that otherwise succeeded.
fn sync_folder_best_effort(disk: &Disk, destination: &Path) {
    let Some(parent) = destination.parent() else {
        return;
    };
    if parent.as_os_str().is_empty() {
        return;
    }
    if let Err(error) = disk.sync_dir_blocking(parent) {
        tracing::debug!(
            parent = %parent.display(),
            %error,
            "could not fsync the destination directory after a verified copy"
        );
    }
}

impl WriteFail {
    /// The target's exclusive create found something there, so this attempt
    /// wrote nothing.
    fn found_the_target_taken(&self) -> bool {
        self.step == Step::OpenTarget
            && !self.claimed
            && match &self.cause {
                Cause::Disk(error) => error.reason() == reason::DISK_DESTINATION_EXISTS,
                Cause::Io(error) => error.kind() == io::ErrorKind::AlreadyExists,
            }
    }

    /// The failure as a recorded copy reports it: classified from what the
    /// disk said, worded for a person.
    fn recorded(self, source: &Path, target: &Path) -> Fail {
        if self.step == Step::Cancelled {
            return Fail::new(
                Refused::Cancelled,
                io::ErrorKind::Interrupted,
                format!("the copy to {} was cancelled", target.display()),
            );
        }
        let (what, path, side) = match self.step {
            Step::OpenSource => ("open copy source", source, Side::Source),
            Step::Read => ("read copy source", source, Side::Source),
            Step::OpenTarget => ("open copy destination", target, Side::Destination),
            Step::Flush => ("flush copy destination", target, Side::Destination),
            Step::Sync => ("fsync copy destination", target, Side::Destination),
            _ => ("write copy destination", target, Side::Destination),
        };
        let error = match self.cause {
            Cause::Disk(error) => error,
            Cause::Io(error) => DiskError::new(DiskVerb::Write, path, error),
        };
        worded(error, side, |said| {
            format!("failed to {what}: {}: {said}", path.display())
        })
    }
}

/// Classifies `error`, then replaces the system's words with a sentence that
/// carries them.
fn worded(error: DiskError, side: Side, sentence: impl FnOnce(&io::Error) -> String) -> Fail {
    let mut fail = Fail::of(error, side);
    fail.detail = io::Error::new(fail.detail.kind(), sentence(&fail.detail));
    fail
}

impl Job<'_> {
    /// A copy that leaves its source alone and reports what it streamed and
    /// what the destination proved to be.
    pub(crate) fn recorded_copy(&self, evidence: Evidence) -> Result<(), Fail> {
        let (disk, source, dest) = (self.disk, self.source, self.dest);
        let staged = (evidence.landing == Landing::Staged).then(|| resumable_staging_path(dest));
        let target = staged.as_deref().unwrap_or(dest);
        let replaces = self.policy.replace == Replace::Allowed;
        if let Some(staged) = &staged
            && !self.staged_free
        {
            staged_name_is_free(staged, disk.uncached_blocking(staged))?;
        }
        // Every recorded copy writes inside its volume's budget. A caller
        // that could wait off this thread already holds the turn.
        if self.permit.borrow().is_none() {
            self.watch.progress.report(Phase::Waiting, 0);
            let lane = disk.copy_lane(dest);
            let turn = wait_parked(disk.copy_admission().acquire_keyed(&lane));
            self.permit.replace(Some(turn));
        }

        self.watch.progress.report(Phase::Copying, 0);
        let writing = Writing {
            opening: match staged.is_none() && replaces {
                true => Opening::Held,
                false => Opening::Claim,
            },
            hash: true,
            sync_folder: true,
            carry_permissions: true,
            chunk_bytes: CHUNK_BYTES,
            seen: None,
            phases: false,
            watch: self.watch,
        };
        let hashes = match write_copy(disk, source, target, writing) {
            Ok(written) => written.hashes.expect("a recorded copy hashes"),
            Err(fail) => {
                // The source is untouched, so an incomplete staged file this
                // attempt created is worth nothing to anyone. One it never
                // opened belongs to someone else and stays.
                if staged.is_some() && fail.claimed {
                    remove_staged_best_effort(disk, target);
                }
                // The claim, not the earlier read, decides whether the
                // staged name was free: something can take it while the copy
                // waits for its turn.
                if staged.is_some() && fail.found_the_target_taken() {
                    return Err(staging_occupied(target));
                }
                // Landing directly, the name opened is the destination's.
                self.wrote_dest.set(staged.is_none() && fail.claimed);
                return Err(fail.recorded(source, target));
            }
        };

        if let Some(staged) = &staged {
            let promoted = match replaces {
                true => disk.move_replacing_blocking(staged, dest).map(|_| ()),
                false => disk.promote_staged_blocking(staged, dest),
            };
            if let Err(error) = promoted {
                remove_staged_best_effort(disk, staged);
                return Err(worded(error, Side::Destination, |said| {
                    format!(
                        "failed to place the completed copy at {}: {said}",
                        dest.display()
                    )
                }));
            }
            // Taking the name is a second directory-entry change.
            sync_folder_best_effort(disk, dest);
        }
        // Proving the copy holds no turn on the volume.
        self.permit.take();

        let proof = (evidence.depth != Depth::Unproven).then(|| {
            disk.prove_copy_blocking(source, dest, &hashes, evidence.depth, &self.watch.progress)
        });
        self.copied.replace(Some(Copied { hashes, proof }));
        Ok(())
    }
}

// ── A copy staged under the caller's name ────────────────────────────────────

/// The raw code a file handle the system dropped under a copy fails with.
const BAD_HANDLE: i32 = 9;

/// How [`Disk::copy_staged_blocking`] lands a copy. Every choice is the
/// caller's; none has a default.
#[derive(Clone, Copy, Debug)]
pub struct StagedCopy<'a> {
    /// The name the bytes are written under, in the destination's folder,
    /// until the copy is whole. It is the caller's own: a file there is
    /// removed before each attempt and after a failed one, and the name is
    /// created or emptied.
    pub staged: &'a Path,
    /// The source as the caller read it. Its name is read again before each
    /// attempt and before the staged file takes the destination name, and
    /// the file each attempt opens is read through its handle; a source that
    /// is no longer that regular file, at that length and modification time
    /// and under that owner, stops the copy.
    pub seen: &'a Mark,
    /// Fsync the folder once the staged file is written.
    pub sync_folder: bool,
    /// Give the copy the source's permissions.
    pub carry_permissions: bool,
    /// An attempt that fails on a dropped file handle is made again after
    /// each of these pauses in turn, and no more. A failure to read the
    /// source's name is never tried again.
    pub bad_handle_pauses: &'a [Duration],
    pub chunk_bytes: usize,
}

/// Where a staged copy stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StagedStep {
    ClearStaged,
    /// The source's name, read before the attempt.
    SourceBefore,
    OpenSource,
    /// The open source's own facts.
    SourceHandle,
    OpenStaged,
    Transfer,
    Flush,
    Sync,
    /// The source's name, read once the staged file was whole.
    SourceAfter,
    /// Taking the destination name.
    Promote,
}

/// Why a staged copy stopped.
#[derive(Debug)]
pub enum StagedCause {
    Io(io::Error),
    /// The source is not the regular file the caller read.
    SourceChanged,
    /// The source, opened, is not a regular file at all.
    SourceNotAFile,
    Cancelled,
}

/// A staged copy that landed.
#[derive(Debug)]
pub struct StagedCopied {
    /// What the copy streamed.
    pub hashes: ContentHashes,
    /// Whether the fsync of the copy was confirmed. A caller that relies on
    /// it reads this; the copy lands either way.
    pub sync: SyncOutcome,
}

/// A staged copy that did not land. The staged name holds nothing of it
/// unless the cleanup itself failed, and the destination name was not taken
/// unless [`Self::left`] says what is there.
#[derive(Debug)]
pub struct StagedCopyRefusal {
    pub step: StagedStep,
    pub cause: StagedCause,
    /// Attempts made, this one included.
    pub attempts: usize,
    /// What a failed promotion left at the destination name it reserved.
    pub left: Option<Leftover>,
}

impl Disk {
    /// Copies `source` to `dest` through `how.staged`, as `how` says, and
    /// returns what it streamed and how its fsync went, which it does not
    /// log. The staged file takes the destination name
    /// with [`Self::move_claimed_blocking`]. Takes no turn on the volume and
    /// proves nothing: both are the caller's. Reports [`Phase::Copying`] at
    /// nothing once the staged file is open, and [`Phase::Finishing`] once
    /// its last byte is written. Blocks on the disk.
    pub fn copy_staged_blocking(
        &self,
        source: &Path,
        dest: &Path,
        how: &StagedCopy<'_>,
        watch: &Watch,
    ) -> Result<StagedCopied, StagedCopyRefusal> {
        let mut attempts = 1;
        loop {
            let mut left = None;
            let refused = |step, cause, left| StagedCopyRefusal {
                step,
                cause,
                attempts,
                left,
            };
            if let Err(error) = self.remove_blocking(how.staged, Remove::File) {
                return Err(refused(
                    StagedStep::ClearStaged,
                    StagedCause::Io(error.into()),
                    None,
                ));
            }
            let (step, cause) = match self.copy_staged_once(source, dest, how, watch, &mut left) {
                Ok(copied) => return Ok(copied),
                Err(failed) => failed,
            };
            let _ = self.remove_blocking(how.staged, Remove::File);
            let named = matches!(step, StagedStep::SourceBefore | StagedStep::SourceAfter);
            let dropped_handle = !named
                && matches!(
                    &cause,
                    StagedCause::Io(error) if error.raw_os_error() == Some(BAD_HANDLE)
                );
            match how.bad_handle_pauses.get(attempts - 1) {
                Some(pause) if dropped_handle => self.inner.clock.pause(*pause),
                _ => return Err(refused(step, cause, left)),
            }
            attempts += 1;
        }
    }

    /// One attempt.
    fn copy_staged_once(
        &self,
        source: &Path,
        dest: &Path,
        how: &StagedCopy<'_>,
        watch: &Watch,
        left: &mut Option<Leftover>,
    ) -> Result<StagedCopied, (StagedStep, StagedCause)> {
        if watch.cancel.is_cancelled() {
            return Err((StagedStep::SourceBefore, StagedCause::Cancelled));
        }
        let still = |step| {
            let read = self.call("lstat", || self.inner.backend.lstat(source));
            #[cfg(test)]
            let read = self.fault(crate::CopyFault::SourceName).map_or(read, Err);
            #[cfg(test)]
            let read = read.map(|mut stat| {
                if let Some(to) = self.fault(crate::CopyFault::SourceOwner) {
                    stat.extra.owner = to.raw_os_error().map(|owner| owner as u32);
                }
                stat
            });
            match read.map(|stat| is_still(how.seen, &Mark::of_stat(&stat))) {
                Ok(true) => Ok(()),
                Ok(false) => Err((step, StagedCause::SourceChanged)),
                Err(error) => Err((step, StagedCause::Io(error))),
            }
        };
        still(StagedStep::SourceBefore)?;
        let writing = Writing {
            opening: Opening::Fresh,
            hash: true,
            sync_folder: how.sync_folder,
            carry_permissions: how.carry_permissions,
            chunk_bytes: how.chunk_bytes,
            seen: Some(how.seen),
            phases: true,
            watch,
        };
        let written = write_copy(self, source, how.staged, writing).map_err(|fail| {
            let step = match fail.step {
                Step::OpenSource => StagedStep::OpenSource,
                Step::SourceHandle | Step::SourceChanged | Step::SourceNotFile => {
                    StagedStep::SourceHandle
                }
                Step::OpenTarget => StagedStep::OpenStaged,
                Step::Flush => StagedStep::Flush,
                Step::Sync => StagedStep::Sync,
                Step::Read | Step::Write | Step::Transfer | Step::Cancelled => StagedStep::Transfer,
            };
            let cause = match (fail.step, fail.cause) {
                (Step::Cancelled, _) => StagedCause::Cancelled,
                (Step::SourceChanged, _) => StagedCause::SourceChanged,
                (Step::SourceNotFile, _) => StagedCause::SourceNotAFile,
                (_, Cause::Disk(error)) => StagedCause::Io(error.into()),
                (_, Cause::Io(error)) => StagedCause::Io(error),
            };
            (step, cause)
        })?;
        still(StagedStep::SourceAfter)?;
        self.move_claimed_blocking(how.staged, dest).map_err(|error| {
            *left = error.left().cloned();
            (StagedStep::Promote, StagedCause::Io(error.into()))
        })?;
        Ok(StagedCopied {
            hashes: written.hashes.expect("a staged copy hashes"),
            sync: written.sync,
        })
    }
}

/// Refuses a copy whose staged name, read as `reading`, is occupied, rather
/// than removing whatever occupies it.
pub(crate) fn staged_name_is_free(staged: &Path, reading: PathReading) -> Result<(), Fail> {
    match reading.exists() {
        Some(false) => Ok(()),
        None => Err(Fail::unread(
            reading,
            Side::Destination,
            format!(
                "failed to inspect the destination staging path {}: {}",
                staged.display(),
                reading.describe_absence()
            ),
        )),
        Some(true) => Err(staging_occupied(staged)),
    }
}

fn staging_occupied(staged: &Path) -> Fail {
    Fail::new(
        Refused::StagingOccupied,
        io::ErrorKind::AlreadyExists,
        format!(
            "The destination staging path {} is occupied. Nothing was copied and the source was preserved; move or remove that file and try again.",
            staged.display()
        ),
    )
}

/// Drops an incomplete staged file after a failed attempt. A stray one costs
/// a refusal the next time, never correctness.
fn remove_staged_best_effort(disk: &Disk, staged: &Path) {
    if let Err(error) = disk.remove_blocking(staged, Remove::File) {
        tracing::warn!(
            partial = %staged.display(),
            %error,
            "could not remove an incomplete partial copy; the next attempt will clear it"
        );
    }
}

// ── Hashing and proving ──────────────────────────────────────────────────────

impl Disk {
    /// The same owner, opening full read-backs through `opener`.
    #[doc(hidden)]
    pub fn with_read_back_opener(&self, opener: ReadBackOpener) -> Self {
        Self {
            inner: self.inner.clone(),
            read_back: Some(opener),
        }
    }

    /// Sets how many copies may write to one volume at a time. Only the
    /// first call, made before any copy asks for a turn, has an effect.
    pub fn install_copy_workers(&self, per_volume: usize) {
        let _ = self.inner.copies.set(CopyAdmission::new(per_volume));
    }

    /// The owner's copy admission: two per volume unless installed otherwise.
    pub fn copy_admission(&self) -> CopyAdmission {
        self.inner
            .copies
            .get_or_init(|| CopyAdmission::new(2))
            .clone()
    }

    /// Starts counting, on this owner, how often each path is read end to
    /// end by a hash or a full read-back. For tests that prove a file was
    /// not read twice.
    #[doc(hidden)]
    pub fn count_whole_reads(&self) {
        self.inner.whole_reads.get_or_init(Mutex::default);
    }

    /// How many times `path` was read end to end on this owner since
    /// [`Self::count_whole_reads`], which must have been called.
    #[doc(hidden)]
    pub fn whole_reads_of(&self, path: &Path) -> usize {
        self.inner
            .whole_reads
            .get()
            .expect("whole reads are counted only after count_whole_reads")
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    fn note_whole_read(&self, path: &Path) {
        if let Some(reads) = self.inner.whole_reads.get() {
            *reads
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .entry(path.to_path_buf())
                .or_default() += 1;
        }
    }

    fn open_read_back(&self, path: &Path) -> ReadBackHandle {
        if let Some(opener) = &self.read_back {
            return opener(path);
        }
        match self.open_blocking(path, Open::read()) {
            Ok(file) => engage_cache_bypass(file),
            Err(error) => ReadBackHandle::Unavailable(format!(
                "could not open the destination for verification: {}: {}",
                path.display(),
                io::Error::from(error)
            )),
        }
    }

    /// Read one file end to end and produce its content hashes, reporting
    /// the bytes read as [`Phase::Verifying`].
    pub(crate) fn hash_file_blocking(
        &self,
        path: &Path,
        progress: &Progress,
    ) -> Result<ContentHashes, HashRefusal> {
        self.note_whole_read(path);
        let refusal = |what: &str, error: DiskError| HashRefusal {
            reason: error.reason(),
            sentence: format!(
                "failed to {what} copy source: {}: {}",
                path.display(),
                io::Error::from(error)
            ),
        };
        let mut input = self
            .open_blocking(path, Open::read())
            .map_err(|error| refusal("open", error))?;
        let mut hasher = ContentHasher::new();
        let mut buffer = vec![0u8; CHUNK_BYTES];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| refusal("read", DiskError::new(DiskVerb::Read, path, error)))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            progress.report(Phase::Verifying, hasher.size_bytes());
        }
        Ok(hasher.finalize())
    }

    /// `Disk::hash_file_blocking` off the async runtime. `None` when the
    /// read did not run to its end; the words say why.
    pub async fn hash_file(
        &self,
        path: impl Into<PathBuf>,
        progress: Progress,
    ) -> Result<Result<ContentHashes, HashRefusal>, String> {
        let (disk, path) = (self.clone(), path.into());
        tokio::task::spawn_blocking(move || disk.hash_file_blocking(&path, &progress))
            .await
            .map_err(|error| error.to_string())
    }

    /// Prove `destination` against the hashes streamed while it was written
    /// from `source`, at `depth`. [`Depth::Unproven`] is proven as quick.
    /// Nothing is changed whatever the verdict.
    pub fn prove_copy_blocking(
        &self,
        source: &Path,
        destination: &Path,
        hashes: &ContentHashes,
        depth: Depth,
        progress: &Progress,
    ) -> Proof {
        progress.report(Phase::Verifying, 0);
        if depth != Depth::Full {
            let (verdict, detail) = self.quick_proof(source, destination, hashes);
            return Proof {
                asked: Depth::Quick,
                fell_back: false,
                verdict,
                detail,
            };
        }
        let full = |verdict, detail| Proof {
            asked: Depth::Full,
            fell_back: false,
            verdict,
            detail,
        };
        match self.open_read_back(destination) {
            ReadBackHandle::Unavailable(reason) => full(Verdict::Unavailable, Some(reason)),
            ReadBackHandle::Unsupported(reason) => {
                self.quick_floor(source, destination, hashes, &reason)
            }
            ReadBackHandle::Ready { file, bypass } => {
                self.note_whole_read(destination);
                match full_read_back(file, hashes, progress) {
                    Ok((verdict, detail)) => {
                        full(verdict, join_details(bypass_note(bypass), detail))
                    }
                    Err(reason) => self.quick_floor(source, destination, hashes, &reason),
                }
            }
        }
    }

    /// [`Self::prove_copy_blocking`] off the async runtime.
    pub async fn prove_copy(
        &self,
        source: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
        hashes: ContentHashes,
        depth: Depth,
        progress: Progress,
    ) -> Result<Proof, String> {
        let (disk, source, destination) = (self.clone(), source.into(), destination.into());
        tokio::task::spawn_blocking(move || {
            disk.prove_copy_blocking(&source, &destination, &hashes, depth, &progress)
        })
        .await
        .map_err(|error| error.to_string())
    }

    /// Whatever stopped the full read-back, the file still gets the quick
    /// proof, marked as a fallback with the reason.
    fn quick_floor(
        &self,
        source: &Path,
        destination: &Path,
        hashes: &ContentHashes,
        reason: &str,
    ) -> Proof {
        let (verdict, detail) = self.quick_proof(source, destination, hashes);
        Proof {
            asked: Depth::Full,
            fell_back: true,
            verdict,
            detail: join_details(
                Some(format!(
                    "full verification fell back to the quick check: {reason}"
                )),
                detail,
            ),
        }
    }

    /// Size against the streamed size, then the destination's sample against
    /// the source's. Content between the two windows is not covered.
    fn quick_proof(
        &self,
        source: &Path,
        destination: &Path,
        hashes: &ContentHashes,
    ) -> (Verdict, Option<String>) {
        let reading = self.follow_blocking(destination);
        let Some(destination_size) = reading.size() else {
            return (
                Verdict::Unavailable,
                Some(format!(
                    "could not stat the destination: {}: {}",
                    destination.display(),
                    reading.describe_absence()
                )),
            );
        };
        if destination_size != hashes.size_bytes {
            return (
                Verdict::Mismatch,
                Some(format!(
                    "destination is {destination_size} bytes; {} bytes were copied",
                    hashes.size_bytes
                )),
            );
        }

        let copied = match self.sample_of(destination) {
            Ok(sample) => sample,
            Err(error) => {
                return (
                    Verdict::Unavailable,
                    Some(format!("could not read the destination: {error}")),
                );
            }
        };
        // The source is still there: nothing touches it until this passes.
        // If it cannot be read, nothing is proven.
        let original = match self.sample_of(source) {
            Ok(sample) => sample,
            Err(error) => {
                return (
                    Verdict::Unavailable,
                    Some(format!(
                        "could not re-read the source for the quick check: {error}"
                    )),
                );
            }
        };
        if original == copied {
            return (Verdict::Verified, None);
        }
        let describe = |path: &Path, sample: &ContentSample| {
            format!(
                "{} (size={} sample_bytes={} sample_blake3={})",
                path.display(),
                sample.size_bytes,
                sample.sample_bytes,
                sample.sample_blake3
            )
        };
        (
            Verdict::Mismatch,
            Some(format!(
                "repository: copy verification failed: {} is not identical to {}",
                describe(source, &original),
                describe(destination, &copied)
            )),
        )
    }

    /// One file's sample, or the sentence for why it could not be taken. The
    /// wording is stored with verification records, so it does not change.
    fn sample_of(&self, path: &Path) -> Result<ContentSample, String> {
        let failed = |step: &str, error: io::Error| {
            format!(
                "repository: failed to {step} import content proof path: {}: {error}",
                path.display()
            )
        };
        let mut file = self
            .open_blocking(path, Open::read())
            .map_err(|error| failed("open", error.into()))?;
        let size_bytes = file
            .metadata()
            .map_err(|error| failed("stat", error))?
            .len();
        sample_reader(&mut file, size_bytes).map_err(|(step, error)| match step {
            SampleStep::Seek => failed("seek", error),
            SampleStep::Read => failed("read", error),
        })
    }

    /// The nearest ancestor of `path`, itself included, that exists, a link
    /// followed: where something written at `path` would land. One probe for
    /// each ancestor tried.
    pub fn nearest_existing_blocking<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.ancestors()
            .find(|at| self.follow_blocking(at).exists() == Some(true))
    }

    /// The volume `path` is on or will land on: the one its nearest ancestor
    /// that answers resolves to, links followed. `None` when none answers.
    /// One probe for each ancestor tried.
    pub fn landing_volume_blocking(&self, path: &Path) -> Option<Mark> {
        path.ancestors().find_map(|at| self.volume_at(at))
    }

    #[cfg(not(windows))]
    fn volume_at(&self, path: &Path) -> Option<Mark> {
        Some(self.fingerprint_followed_blocking(path).ok()?.volume())
    }

    #[cfg(windows)]
    fn volume_at(&self, path: &Path) -> Option<Mark> {
        let serial = self.call("volume_serial", || self.inner.backend.volume_serial(path))?;
        Some(Mark::on_volume(serial))
    }

    /// The copy lane of the volume `dest` will land on; the unknown lane when
    /// no volume answers.
    pub(crate) fn copy_lane(&self, dest: &Path) -> String {
        self.landing_volume_blocking(dest)
            .map_or_else(|| UNKNOWN_LANE.to_owned(), |volume| volume.lane())
    }

    /// A turn on the volume `dest` will land on. The volume is read within
    /// the root's bound; when it does not answer there is no turn and the
    /// copy is refused as one whose destination could not be read.
    pub(crate) async fn copy_turn(&self, dest: &Path) -> Result<CopyPermit, Fail> {
        let read = |disk: &Self, dest: &Path| Ok(disk.copy_lane(dest));
        let lane = self.probe_with(dest.to_path_buf(), read, Err).await;
        let lane = lane.map_err(|reading| {
            let why = reading.describe_absence();
            let detail = format!("failed to inspect the volume of {}: {why}", dest.display());
            Fail::unread(reading, Side::Destination, detail)
        })?;
        Ok(self.copy_admission().acquire_keyed(&lane).await)
    }

    /// Whether a move from `source` to `destination` can be a rename: both
    /// are on one filesystem. An unknown answer is `false`, which only ever
    /// costs a copy. The destination usually does not exist yet, so its
    /// nearest existing ancestor is what is compared.
    ///
    /// Only unix answers: a move on any other platform copies, so it reads
    /// nothing here.
    pub async fn renames_between(&self, source: &Path, destination: &Path) -> bool {
        if !cfg!(unix) {
            return false;
        }
        let (disk, source, destination) = (
            self.clone(),
            source.to_path_buf(),
            destination.to_path_buf(),
        );
        tokio::task::spawn_blocking(move || disk.same_filesystem(&source, &destination))
            .await
            .unwrap_or(false)
    }

    /// Whether `source` and the volume `destination` lands on are one
    /// volume, read within the destination root's bound; unknown is `false`.
    /// On Windows both must name the same drive or share and answer the same
    /// non-zero volume serial. Nothing asks this on the way to a move: see
    /// [`Self::renames_between`].
    pub async fn shares_volume(&self, source: &Path, destination: &Path) -> bool {
        let (disk, source_path, landing) = (
            self.clone(),
            source.to_path_buf(),
            destination.to_path_buf(),
        );
        let timeout = self.traits(destination).probe_timeout;
        match self
            .bounded(destination, timeout, move || {
                disk.same_filesystem(&source_path, &landing)
            })
            .await
        {
            super::Bounded::Done(same) => same,
            _ => false,
        }
    }

    fn same_filesystem(&self, source_path: &Path, destination: &Path) -> bool {
        // Only unix and Windows name a volume.
        if !cfg!(any(unix, windows)) {
            return false;
        }
        // Two drives or shares are two volumes, whatever serials they answer.
        #[cfg(windows)]
        if volume_prefix(source_path) != volume_prefix(destination) {
            return false;
        }
        // A symlinked source lives on the filesystem holding the link, not
        // the one holding its target.
        let Ok(source) = self.fingerprint_blocking(source_path) else {
            return false;
        };
        // A plain read names no volume on Windows. The volume comes from a
        // handle, which follows a link, so a linked source is left to the
        // copy rather than judged by its target's volume. A serial of zero
        // is not one.
        #[cfg(windows)]
        let source = match source.kind() {
            Some(super::EntryKind::Symlink) | None => return false,
            Some(_) => match self.volume_at(source_path) {
                Some(volume) if volume.same_volume(&Mark::on_volume(0)) == Sameness::Different => {
                    volume
                }
                _ => return false,
            },
        };
        let mut candidate = Some(destination);
        while let Some(current) = candidate {
            if current.as_os_str().is_empty() {
                return false;
            }
            if self.fresh_blocking(current).exists() == Some(true) {
                return self
                    .volume_at(current)
                    .is_some_and(|volume| volume.same_volume(&source) == Sameness::Same);
            }
            candidate = current.parent();
        }
        false
    }
}

/// The drive or share a Windows path names, upper-cased: `C:` for `C:\x` and
/// `\\?\C:\x`, `\\SERVER\SHARE` for `\\server\share\x` and its verbatim
/// form, the prefix's own text otherwise, and `None` for a path with none.
#[cfg(windows)]
fn volume_prefix(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    Some(match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            format!("{}:", char::from(letter).to_ascii_uppercase())
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
            "\\\\{}\\{}",
            server.to_string_lossy().to_uppercase(),
            share.to_string_lossy().to_uppercase()
        ),
        _ => prefix.as_os_str().to_string_lossy().to_uppercase(),
    })
}

/// Reads the whole destination back and compares it against what the copy
/// streamed. `Err` means the read-back could not be completed, which is the
/// trigger for the quick fallback, not a verdict on the content.
fn full_read_back(
    mut file: File,
    hashes: &ContentHashes,
    progress: &Progress,
) -> Result<(Verdict, Option<String>), String> {
    let mut digest = crc_turbo::Digest::new(hashes.crc_kind.algorithm());
    let mut read_bytes: u64 = 0;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("destination read-back failed: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        read_bytes = read_bytes.saturating_add(read as u64);
        progress.report(Phase::Verifying, read_bytes);
    }

    if read_bytes != hashes.size_bytes {
        return Ok((
            Verdict::Mismatch,
            Some(format!(
                "destination is {read_bytes} bytes; {} bytes were copied",
                hashes.size_bytes
            )),
        ));
    }

    let read_back_crc = digest.finalize();
    if read_back_crc != hashes.crc {
        tracing::warn!(
            algorithm = ?hashes.crc_kind,
            expected = hashes.crc,
            actual = read_back_crc,
            "copy read-back mismatch"
        );
        return Ok((
            Verdict::Mismatch,
            Some("The copied file did not match the source. The source was preserved; retry the transfer.".into()),
        ));
    }
    Ok((Verdict::Verified, None))
}

fn bypass_note(bypass: CacheBypass) -> Option<String> {
    match bypass {
        CacheBypass::Engaged => None,
        CacheBypass::NotAvailableOnPlatform => Some(
            "the destination read-back was not cache-bypassed: this platform offers no bypass mechanism"
                .to_string(),
        ),
        CacheBypass::Rejected(reason) => Some(format!(
            "the destination read-back was not cache-bypassed: {reason}"
        )),
    }
}

fn join_details(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}
