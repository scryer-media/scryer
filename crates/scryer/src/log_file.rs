use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogFilePolicy {
    pub max_bytes: u64,
    pub max_files: usize,
}

impl LogFilePolicy {
    pub(crate) fn parse(size: Option<&str>, count: Option<&str>) -> Result<Self, String> {
        fn positive<T: std::str::FromStr>(value: &str, name: &str) -> Result<T, String> {
            if value.is_empty()
                || !value.bytes().all(|b| b.is_ascii_digit())
                || value.bytes().all(|b| b == b'0')
            {
                return Err(format!("{name} must be a positive integer"));
            }
            value.parse().map_err(|_| format!("{name} is out of range"))
        }
        Ok(Self {
            max_bytes: size
                .map(|v| positive(v, "SCRYER_LOG_MAX_SIZE"))
                .transpose()?
                .unwrap_or(10 * 1024 * 1024),
            max_files: count
                .map(|v| positive(v, "SCRYER_LOG_MAX_FILES"))
                .transpose()?
                .unwrap_or(5),
        })
    }

    pub(crate) fn from_env() -> Result<Self, String> {
        fn value(name: &str) -> Result<Option<String>, String> {
            match std::env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(_) => Err(format!("{name} must be a positive integer")),
            }
        }
        Self::parse(
            value("SCRYER_LOG_MAX_SIZE")?.as_deref(),
            value("SCRYER_LOG_MAX_FILES")?.as_deref(),
        )
    }
}

type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

// Multiple of max_bytes at which rollover proceeds despite a pending segment.
const ROTATION_CEILING_FACTOR: u64 = 4;

struct Active {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    permissions: fs::Permissions,
    day: NaiveDate,
    policy: LogFilePolicy,
    clock: Clock,
    work: Arc<Work>,
    retry_at: Option<Instant>,
    next_sequence: u64,
}

#[derive(Default)]
struct Queue {
    // More than one entry is possible only when recovering previous runs or
    // when the active file reached the rotation ceiling during compression.
    jobs: VecDeque<Job>,
    shutdown: bool,
    // Queue length at which the head job parked after a failure that will not
    // pass on its own; it is retried once a rollover changes the queue.
    parked: Option<usize>,
}

struct Job {
    source: Option<PathBuf>,
}

#[derive(Default)]
struct Faults {
    #[cfg(test)]
    fail: Mutex<Option<&'static str>>,
    // Fails the step with a permission error on every attempt until cleared.
    #[cfg(test)]
    deny: Mutex<Option<&'static str>>,
    #[cfg(test)]
    denied: std::sync::atomic::AtomicUsize,
}

impl Faults {
    fn check(&self, _step: &str) -> io::Result<()> {
        #[cfg(test)]
        {
            let mut fail = self.fail.lock().unwrap();
            if fail.as_ref().is_some_and(|step| *step == _step) {
                *fail = None;
                return Err(io::Error::other(format!("injected {_step} failure")));
            }
            if self.deny.lock().unwrap().is_some_and(|step| step == _step) {
                self.denied
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
        }
        Ok(())
    }
}

// A module-detected condition that retrying on a timer cannot resolve. The
// io::Error wrapping it keeps its original kind and message.
#[derive(Debug)]
struct Unrecoverable(&'static str);

impl std::fmt::Display for Unrecoverable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Unrecoverable {}

fn unrecoverable(message: &'static str) -> io::Error {
    io::Error::other(Unrecoverable(message))
}

// Failures that will not pass on their own park the job until the queue
// changes or the process restarts instead of retrying on the backoff timer.
fn waits_for_queue_change(error: &io::Error) -> bool {
    use io::ErrorKind::*;
    matches!(
        error.kind(),
        PermissionDenied | NotFound | InvalidData | UnexpectedEof | ReadOnlyFilesystem
    ) || error
        .get_ref()
        .is_some_and(|inner| inner.is::<Unrecoverable>())
}

struct Work {
    queue: Mutex<Queue>,
    changed: Condvar,
    active_path: PathBuf,
    max_files: usize,
    faults: Faults,
}

#[derive(Clone)]
pub(crate) struct LogFileWriter(Arc<Mutex<Active>>);

// The global tracing subscriber outlives main, so worker lifetime must be owned
// by a separate guard held by main rather than by the subscriber's writer.
pub(crate) struct LogFileGuard {
    writer: LogFileWriter,
    worker: Option<JoinHandle<()>>,
}

impl Drop for LogFileGuard {
    fn drop(&mut self) {
        let active = self.writer.0.lock().unwrap();
        if let Some(file) = &active.file
            && let Err(error) = file.sync_all()
        {
            report("flush", &error);
        }
        let work = active.work.clone();
        work.queue.lock().unwrap().shutdown = true;
        work.changed.notify_all();
        drop(active);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            eprintln!("Scryer log compression worker panicked; sealed logs preserved");
        }
    }
}

pub(crate) fn open_log_file(
    path: &Path,
    policy: LogFilePolicy,
) -> io::Result<(LogFileWriter, LogFileGuard)> {
    open_with_clock(path, policy, Arc::new(Utc::now))
}

fn open_with_clock(
    path: &Path,
    policy: LogFilePolicy,
    clock: Clock,
) -> io::Result<(LogFileWriter, LogFileGuard)> {
    let path = std::path::absolute(path)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let file = open_active(&path)?;
    let metadata = file.metadata()?;
    let day = metadata
        .modified()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| clock())
        .date_naive();
    let sealed = managed_files(&path, "sealed")?;
    let archives = managed_files(&path, "gz")?;
    let next_sequence = sealed
        .iter()
        .chain(&archives)
        .map(|p| sequence(p))
        .max()
        .map(|n| {
            n.checked_add(1)
                .ok_or_else(|| io::Error::other("log sequence exhausted"))
        })
        .transpose()?
        .unwrap_or(0);
    let mut jobs: VecDeque<_> = sealed
        .into_iter()
        // An interruption before rename can leave an empty exclusive reservation.
        // Preserve it; it has no events to recover and must not become an archive.
        .filter(|p| fs::symlink_metadata(p).map_or(true, |m| m.len() != 0))
        .map(|p| Job { source: Some(p) })
        .collect();
    if archives.len() > policy.max_files {
        jobs.push_back(Job { source: None });
    }
    let work = Arc::new(Work {
        queue: Mutex::new(Queue {
            jobs,
            shutdown: false,
            parked: None,
        }),
        changed: Condvar::new(),
        active_path: path.clone(),
        max_files: policy.max_files,
        faults: Faults::default(),
    });
    let writer = LogFileWriter(Arc::new(Mutex::new(Active {
        path,
        bytes: metadata.len(),
        permissions: metadata.permissions(),
        file: Some(file),
        day,
        policy,
        clock,
        work: work.clone(),
        retry_at: None,
        next_sequence,
    })));
    let worker = std::thread::Builder::new()
        .name("log-compression".into())
        .spawn(move || run_worker(work))?;
    let guard = LogFileGuard {
        writer: writer.clone(),
        worker: Some(worker),
    };
    Ok((writer, guard))
}

fn report(operation: &str, error: &io::Error) {
    eprintln!("Scryer file log {operation} failed: {error}; recoverable log contents preserved");
}

fn regular(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(unrecoverable("log segment is not a regular file"));
    }
    Ok(())
}

fn no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
}

fn open_regular(path: &Path) -> io::Result<File> {
    regular(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(unrecoverable("not a regular log file"));
    }
    Ok(file)
}

fn open_active(path: &Path) -> io::Result<File> {
    open_active_with_permissions(path, None)
}

fn preserve_create_mode(options: &mut OpenOptions, permissions: Option<&fs::Permissions>) {
    #[cfg(not(unix))]
    let _ = (options, permissions);
    #[cfg(unix)]
    if let Some(permissions) = permissions {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // Set the mode at creation, before any log contents become visible.
        options.mode(permissions.mode() & 0o777);
    }
}

fn open_active_with_permissions(
    path: &Path,
    permissions: Option<&fs::Permissions>,
) -> io::Result<File> {
    if fs::symlink_metadata(path).is_ok() {
        regular(path)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    preserve_create_mode(&mut options, permissions);
    no_follow(&mut options);
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(io::Error::other("not a regular active log"));
    }
    Ok(file)
}

fn prefix(path: &Path) -> std::ffi::OsString {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".scryer-rotate-");
    name
}

fn managed_files(path: &Path, extension: &str) -> io::Result<Vec<PathBuf>> {
    let prefix = prefix(path);
    let suffix = format!(".{extension}");
    let mut paths = Vec::new();
    for entry in fs::read_dir(path.parent().unwrap())? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .as_encoded_bytes()
            .strip_prefix(prefix.as_encoded_bytes())
            .and_then(|s| s.strip_suffix(suffix.as_bytes()))
            .and_then(|s| std::str::from_utf8(s).ok())
        else {
            continue;
        };
        let Some((stamp, sequence)) = id.split_once('-') else {
            continue;
        };
        if stamp.len() != 16
            || sequence.len() != 20
            || !sequence.bytes().all(|b| b.is_ascii_digit())
            || sequence.parse::<u64>().is_err()
            || chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ").is_err()
        {
            continue;
        }
        if entry.file_type()?.is_file() {
            paths.push(entry.path());
        }
    }
    paths.sort_by_key(|path| (sequence(path), path.clone()));
    Ok(paths)
}

fn sequence(path: &Path) -> u64 {
    let stem = path.file_stem().unwrap().as_encoded_bytes();
    std::str::from_utf8(&stem[stem.len() - 20..])
        .unwrap()
        .parse()
        .unwrap()
}

impl Active {
    fn rotate(&mut self, now: DateTime<Utc>) -> io::Result<()> {
        let work = self.work.clone();
        let mut queue = work.queue.lock().unwrap();
        // A pending segment defers rollover, but only up to a hard ceiling so a
        // slow or failing worker cannot let the active file grow without bound.
        let ceiling = self
            .policy
            .max_bytes
            .saturating_mul(ROTATION_CEILING_FACTOR);
        if queue.shutdown || (!queue.jobs.is_empty() && self.bytes < ceiling) {
            return Ok(());
        }
        if let Some(file) = &self.file {
            self.permissions = file.metadata()?.permissions();
        }
        // Reserve an exclusive destination before closing the Windows handle.
        let sealed = loop {
            let sequence = self.next_sequence;
            self.next_sequence = sequence
                .checked_add(1)
                .ok_or_else(|| io::Error::other("log sequence exhausted"))?;
            let mut name = prefix(&self.path);
            name.push(format!(
                "{}-{sequence:020}.sealed",
                now.format("%Y%m%dT%H%M%SZ")
            ));
            let candidate = self.path.with_file_name(name);
            if fs::symlink_metadata(candidate.with_extension("gz")).is_ok() {
                continue;
            }
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(reservation) => {
                    drop(reservation);
                    break candidate;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        };
        if let Some(file) = self.file.as_mut()
            && let Err(error) = file.flush().and_then(|_| file.sync_all())
        {
            let _ = fs::remove_file(&sealed); // Our exclusive empty reservation.
            return Err(error);
        }
        drop(self.file.take());
        if let Err(error) = work
            .faults
            .check("rename")
            .and_then(|_| fs::rename(&self.path, &sealed))
        {
            let _ = fs::remove_file(&sealed);
            self.file = open_active_with_permissions(&self.path, Some(&self.permissions)).ok();
            return Err(error);
        }
        queue.jobs.push_back(Job {
            source: Some(sealed),
        });
        work.changed.notify_all();
        self.bytes = 0;
        self.day = now.date_naive();
        let reopened = work
            .faults
            .check("reopen")
            .and_then(|_| open_active_with_permissions(&self.path, Some(&self.permissions)));
        match reopened {
            Ok(file) => self.file = Some(file),
            Err(error) => {
                // A transient reopen failure must not unnecessarily lose this event.
                self.file = open_active_with_permissions(&self.path, Some(&self.permissions)).ok();
                return Err(error);
            }
        }
        Ok(())
    }

    fn prepare_event(&mut self) {
        if self
            .retry_at
            .is_some_and(|deadline| Instant::now() < deadline)
        {
            return;
        }
        let now = (self.clock)();
        let result = (|| {
            if self.file.is_none() {
                let file = open_active_with_permissions(&self.path, Some(&self.permissions))?;
                self.bytes = file.metadata()?.len();
                self.file = Some(file);
            }
            if self.bytes > 0
                && (self.bytes >= self.policy.max_bytes || now.date_naive() != self.day)
            {
                self.rotate(now)?;
            } else if self.bytes == 0 {
                self.day = now.date_naive();
            }
            Ok(())
        })();
        if let Err(error) = result {
            report("rotation", &error);
            self.retry_at = Some(Instant::now() + Duration::from_secs(30));
        } else {
            self.retry_at = None;
        }
    }
}

pub(crate) struct LogFileWriteHandle<'a> {
    active: MutexGuard<'a, Active>,
    started: bool,
}

impl Write for LogFileWriteHandle<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if !self.started {
            self.active.prepare_event();
            self.started = true;
        }
        let file = self
            .active
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("active log unavailable"))?;
        let written = file.write(bytes)?;
        self.active.bytes = self.active.bytes.saturating_add(written as u64);
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.active
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("active log unavailable"))?
            .flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFileWriter {
    type Writer = LogFileWriteHandle<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        LogFileWriteHandle {
            active: self.0.lock().unwrap(),
            started: false,
        }
    }
}

fn same_contents(source: &Path, archive: &Path) -> io::Result<bool> {
    let mut source = open_regular(source)?;
    let mut archive = flate2::read::MultiGzDecoder::new(open_regular(archive)?);
    let mut a = [0u8; 8192];
    let mut b = [0u8; 8192];
    loop {
        let n = source.read(&mut a)?;
        if n == 0 {
            return Ok(archive.read(&mut b[..1])? == 0);
        }
        archive.read_exact(&mut b[..n])?;
        if a[..n] != b[..n] {
            return Ok(false);
        }
    }
}

fn publish(source: &Path, faults: &Faults) -> io::Result<()> {
    let destination = source.with_extension("gz");
    if fs::symlink_metadata(&destination).is_ok() {
        if same_contents(source, &destination)? {
            // This namespace is published only after syncing the gzip. Recovery
            // verifies its entire contents, including the checksum, before unlinking.
            return sync_directory(source.parent().unwrap());
        }
        return Err(unrecoverable("archive collision; existing file preserved"));
    }
    let temporary = source.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut input = open_regular(source)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    preserve_create_mode(&mut options, Some(&input.metadata()?.permissions()));
    let output = options.open(&temporary)?;
    let result = (|| {
        let mut encoder = flate2::GzBuilder::new()
            .comment(ROTATION_MARKER)
            .write(output, flate2::Compression::default());
        faults.check("compression")?;
        io::copy(&mut input, &mut encoder)?;
        encoder.finish()?.sync_all()?;
        faults.check("publication")?;
        let linked = faults
            .check("hard_link")
            .and_then(|_| fs::hard_link(&temporary, &destination));
        if let Err(error) = linked {
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(error);
            }
            // FAT/exFAT cannot hard-link. Native exclusive rename retains both
            // atomic publication and collision safety on those filesystems.
            publish_platform::rename_exclusive(&temporary, &destination)?;
        }
        sync_directory(source.parent().unwrap())
    })();
    // Only this invocation's exclusively created temporary file is eligible.
    if let Err(error) = fs::remove_file(&temporary)
        && error.kind() != io::ErrorKind::NotFound
    {
        report("temporary cleanup", &error);
    }
    result
}

/// Written as the gzip comment of every archive this rotation publishes, so
/// retention can tell its own archives from files that only share the name
/// pattern.
const ROTATION_MARKER: &str = "scryer-log-rotation";

/// Whether `archive` is a regular file whose gzip header carries
/// [`ROTATION_MARKER`]. Anything that cannot be read counts as not ours.
fn rotated_here(archive: &Path) -> bool {
    let Ok(file) = open_regular(archive) else {
        return false;
    };
    flate2::read::GzDecoder::new(file)
        .header()
        .and_then(|header| header.comment())
        == Some(ROTATION_MARKER.as_bytes())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn process_job(job: &mut Job, work: &Work) -> io::Result<()> {
    if let Some(source) = &job.source {
        publish(source, &work.faults)?;
        regular(source)?;
        fs::remove_file(source)?;
        job.source = None;
    }
    // Only archives that carry this rotation's mark count toward retention
    // or are ever removed. A file that merely follows the naming pattern, or
    // whose mark cannot be read, is left alone.
    let archives = managed_files(&work.active_path, "gz")?
        .into_iter()
        .filter(|archive| rotated_here(archive))
        .collect::<Vec<_>>();
    let expired = archives.len().saturating_sub(work.max_files);
    for archive in archives.into_iter().take(expired) {
        regular(&archive)?;
        work.faults.check("retention")?;
        fs::remove_file(archive)?;
    }
    Ok(())
}

fn run_worker(work: Arc<Work>) {
    let mut delay = Duration::from_secs(1);
    loop {
        let mut queue = work.queue.lock().unwrap();
        while queue.jobs.is_empty() && !queue.shutdown {
            queue = work.changed.wait(queue).unwrap();
        }
        if queue.jobs.is_empty() {
            return;
        }
        // Leave a placeholder queued to defer another rollover during compression.
        let mut job = Job {
            source: queue.jobs.front_mut().unwrap().source.take(),
        };
        drop(queue);
        let result = process_job(&mut job, &work);
        let mut queue = work.queue.lock().unwrap();
        match result {
            Ok(()) => {
                queue.jobs.pop_front();
                work.changed.notify_all();
                delay = Duration::from_secs(1);
            }
            Err(error) => {
                queue.jobs.front_mut().unwrap().source = job.source;
                report("compression/retention", &error);
                if queue.shutdown {
                    return;
                }
                if waits_for_queue_change(&error) {
                    // No timer: only a rollover pushing a job or shutdown wakes it.
                    queue.parked = Some(queue.jobs.len());
                    work.changed.notify_all();
                    let mut queue = work
                        .changed
                        .wait_while(queue, |q| !q.shutdown && q.parked == Some(q.jobs.len()))
                        .unwrap();
                    queue.parked = None;
                    continue;
                }
                let _ = work
                    .changed
                    .wait_timeout_while(queue, delay, |q| !q.shutdown)
                    .unwrap();
                delay = (delay * 2).min(Duration::from_secs(60));
            }
        }
    }
}

#[cfg(test)]
#[path = "log_file_tests.rs"]
mod tests;

#[path = "log_file_publish.rs"]
mod publish_platform;
