use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::mpsc;

use super::backend::{RawEntry, RawStat, WriteFailure};
use super::test_support::ManualClock;
use super::*;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000 + seconds, 0).expect("instant")
}

const ROOT: &str = "/srv/fixture-media";

fn file(len: u64) -> RawStat {
    RawStat {
        kind: EntryKind::File,
        len,
        modified: None,
        identity: FileIdentity::from_raw(1, len + 100),
        links: Some(1),
    }
}

fn dir() -> RawStat {
    RawStat {
        kind: EntryKind::Directory,
        len: 0,
        modified: None,
        identity: None,
        links: None,
    }
}

/// Answers stat calls from a script (a path with no script is not found),
/// lists from a script, and lets every mutation succeed unless a failure is
/// scripted for that operation and path, or for that operation on any path.
/// `write_body` scripts a write that fails after its file was created.
/// Records every call. A gate, when set, holds every stat call (or only those
/// under `gate_under`) until the test opens it. Reads answer from `contents`.
/// Opens record what was asked and hand back a handle on a real scratch file,
/// so the handle can be written and synced; an open to read is missing unless
/// `contents` holds the path, and its handle then reads those contents.
/// `contents_gate`, when set, holds every read through a handle until the
/// test opens it, after signalling `contents_entered`.
#[derive(Default)]
struct Scripted {
    stats: Mutex<HashMap<PathBuf, Result<RawStat, io::ErrorKind>>>,
    listings: Mutex<HashMap<PathBuf, Vec<RawEntry>>>,
    failures: Mutex<HashMap<(&'static str, PathBuf), io::ErrorKind>>,
    any_path_failures: Mutex<HashMap<&'static str, io::ErrorKind>>,
    calls: Mutex<Vec<(&'static str, PathBuf)>>,
    gate: Mutex<Option<mpsc::Receiver<()>>>,
    gate_under: Mutex<Option<PathBuf>>,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    contents: Mutex<HashMap<PathBuf, Vec<u8>>>,
    opened: Mutex<Vec<Open>>,
    handles: Mutex<Option<tempfile::TempDir>>,
    /// Access asked for by creating calls, in order.
    accesses: Mutex<Vec<(&'static str, Access)>>,
    contents_gate: Mutex<Option<mpsc::Receiver<()>>>,
    contents_entered: Mutex<Option<mpsc::Sender<()>>>,
}

impl Scripted {
    fn stat_is(&self, path: &str, answer: Result<RawStat, io::ErrorKind>) {
        self.stats
            .lock()
            .unwrap()
            .insert(PathBuf::from(path), answer);
    }
    fn lists(&self, path: &str, entries: &[(&str, Option<EntryKind>)]) {
        self.listings.lock().unwrap().insert(
            PathBuf::from(path),
            entries
                .iter()
                .map(|(name, kind)| RawEntry {
                    name: OsString::from(name),
                    kind: *kind,
                    stat: None,
                })
                .collect(),
        );
    }
    fn holds(&self, path: &str, bytes: &[u8]) {
        self.contents
            .lock()
            .unwrap()
            .insert(PathBuf::from(path), bytes.to_vec());
    }
    fn accesses(&self) -> Vec<(&'static str, Access)> {
        self.accesses.lock().unwrap().clone()
    }
    fn fails(&self, operation: &'static str, path: &str, kind: io::ErrorKind) {
        self.failures
            .lock()
            .unwrap()
            .insert((operation, PathBuf::from(path)), kind);
    }
    fn always_fails(&self, operation: &'static str, kind: io::ErrorKind) {
        self.any_path_failures
            .lock()
            .unwrap()
            .insert(operation, kind);
    }
    fn any_path_failure(&self, operation: &str) -> Option<io::ErrorKind> {
        self.any_path_failures
            .lock()
            .unwrap()
            .get(operation)
            .copied()
    }
    /// Paths `operation` was called on, in order.
    fn paths_of(&self, operation: &str) -> Vec<PathBuf> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, _)| *op == operation)
            .map(|(_, path)| path.clone())
            .collect()
    }
    fn calls_of(&self, operation: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, _)| *op == operation)
            .count()
    }
    fn record(&self, operation: &'static str, path: &Path) -> io::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push((operation, path.to_path_buf()));
        match self
            .failures
            .lock()
            .unwrap()
            .get(&(operation, path.to_path_buf()))
        {
            Some(kind) => Err((*kind).into()),
            None => match self.any_path_failure(operation) {
                Some(kind) => Err(kind.into()),
                None => Ok(()),
            },
        }
    }
    fn stat_answer(&self, operation: &'static str, path: &Path) -> io::Result<RawStat> {
        self.record(operation, path)?;
        if let Some(entered) = self.entered.lock().unwrap().as_ref() {
            let _ = entered.send(());
        }
        let gated = self
            .gate_under
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|under| path.starts_with(under));
        if gated && let Some(gate) = self.gate.lock().unwrap().as_ref() {
            let _ = gate.recv();
        }
        match self.stats.lock().unwrap().get(path) {
            Some(Ok(stat)) => Ok(*stat),
            Some(Err(kind)) => Err((*kind).into()),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }
}

impl DiskBackend for Scripted {
    fn stat(&self, path: &Path) -> io::Result<RawStat> {
        self.stat_answer("stat", path)
    }
    fn lstat(&self, path: &Path) -> io::Result<RawStat> {
        self.stat_answer("lstat", path)
    }
    fn list(&self, path: &Path) -> io::Result<Vec<RawEntry>> {
        self.record("list", path)?;
        self.listings
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.record("canonicalize", path)
            .map(|()| path.to_path_buf())
    }
    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        self.record("read_link", path).map(|()| path.to_path_buf())
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.record("remove_file", path)
    }
    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        self.record("remove_dir", path)
    }
    fn remove_tree(&self, path: &Path) -> io::Result<()> {
        self.record("remove_tree", path)
    }
    fn rename(&self, source: &Path, _dest: &Path) -> io::Result<()> {
        self.record("rename", source)
    }
    fn rename_no_replace(&self, source: &Path, _dest: &Path) -> io::Result<()> {
        self.record("rename_no_replace", source)
    }
    fn rename_claimed(&self, source: &Path, _dest: &Path) -> io::Result<()> {
        self.record("rename_claimed", source)
    }
    fn hard_link(&self, _source: &Path, dest: &Path) -> io::Result<()> {
        self.record("hard_link", dest)
    }
    fn symlink(&self, _target: &Path, at: &Path) -> io::Result<()> {
        self.record("symlink", at)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.record("create_dir_all", path)
    }
    fn create_dir(&self, path: &Path, access: Access) -> io::Result<()> {
        self.accesses.lock().unwrap().push(("create_dir", access));
        self.record("create_dir", path)
    }
    fn write_new(
        &self,
        path: &Path,
        _bytes: &[u8],
        access: Access,
    ) -> Result<SyncOutcome, WriteFailure> {
        self.accesses.lock().unwrap().push(("write_new", access));
        self.record("write_new", path)
            .map_err(WriteFailure::not_created)?;
        match self.any_path_failure("write_body") {
            Some(kind) => Err(WriteFailure::created(kind.into())),
            None => Ok(SyncOutcome::Durable),
        }
    }
    fn sync_file(&self, path: &Path) -> io::Result<SyncOutcome> {
        self.record("sync_file", path)
            .map(|()| SyncOutcome::Unconfirmed)
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.record("read", path)?;
        self.contents
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn open(&self, path: &Path, open: Open) -> io::Result<std::fs::File> {
        self.opened.lock().unwrap().push(open);
        self.accesses.lock().unwrap().push(("open", open.access));
        self.record("open", path)?;
        let mut handles = self.handles.lock().unwrap();
        let dir = handles.get_or_insert_with(|| tempfile::tempdir().expect("handle dir"));
        let handle = dir.path().join(format!("handle-{}", self.calls_of("open")));
        if open.mode != OpenMode::Read {
            return SystemBackend.open(&handle, Open::create_or_keep());
        }
        let bytes = self
            .contents
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or(io::ErrorKind::NotFound)?;
        SystemBackend.write(&handle, &bytes, Replace::Never, Access::Default)?;
        SystemBackend.open(&handle, Open::read())
    }
    fn read_from(&self, file: std::fs::File) -> io::Result<Vec<u8>> {
        self.record("read_from", Path::new(""))?;
        if let Some(entered) = self.contents_entered.lock().unwrap().as_ref() {
            let _ = entered.send(());
        }
        if let Some(gate) = self.contents_gate.lock().unwrap().as_ref() {
            let _ = gate.recv();
        }
        SystemBackend.read_from(file)
    }
    fn write(
        &self,
        path: &Path,
        _bytes: &[u8],
        _replace: Replace,
        access: Access,
    ) -> io::Result<()> {
        self.accesses.lock().unwrap().push(("write", access));
        self.record("write", path)
    }
    fn sync_handle(&self, _file: &std::fs::File) -> io::Result<SyncOutcome> {
        self.record("sync_handle", Path::new(""))
            .map(|()| SyncOutcome::Durable)
    }
    fn sync_dir(&self, path: &Path) -> io::Result<SyncOutcome> {
        self.record("sync_dir", path).map(|()| SyncOutcome::Durable)
    }
    fn restrict_to_owner(&self, path: &Path, _what: OwnerOnly) -> io::Result<()> {
        self.record("restrict_to_owner", path)
    }
    fn set_modified(&self, path: &Path, _at: SystemTime) -> io::Result<()> {
        self.record("set_modified", path)
    }
}

fn owner(clock: &Arc<ManualClock>) -> (Disk, Arc<Scripted>) {
    let backend = Arc::new(Scripted::default());
    let disk = Disk::with_seams(backend.clone(), clock.clone());
    disk.set_volumes([(PathBuf::from(ROOT), VolumeClass::LibraryRoot)]);
    (disk, backend)
}

fn p(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[test]
fn an_observed_read_never_calls_the_backend() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    assert_eq!(
        disk.observed(&p("/srv/fixture-media/a.mkv")),
        PathReading::NotObserved
    );
    assert_eq!(disk.total_calls(), 0);

    disk.fresh_blocking(&p("/srv/fixture-media/a.mkv"));
    let calls = disk.total_calls();
    for offset in 0..5 {
        clock.set(at(0) + chrono::Duration::milliseconds(offset * 200));
        assert_eq!(
            disk.observed(&p("/srv/fixture-media/a.mkv")).kind(),
            Some(EntryKind::File)
        );
    }
    // A cache-only read trusts a reading for the reuse window and no longer.
    clock.set(at(1));
    assert_eq!(
        disk.observed(&p("/srv/fixture-media/a.mkv")),
        PathReading::NotObserved
    );
    assert_eq!(disk.total_calls(), calls);
    assert_eq!(backend.calls_of("lstat"), 1);
}

#[test]
fn a_fresh_read_reuses_a_reading_under_a_second_old_and_probes_after() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    let path = p("/srv/fixture-media/a.mkv");
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    assert_eq!(disk.fresh_blocking(&path).exists(), Some(true));
    assert_eq!(disk.fresh_blocking(&path).exists(), Some(true));
    assert_eq!(disk.calls("lstat"), 1);
    clock.set(at(1));
    backend.stat_is("/srv/fixture-media/a.mkv", Err(io::ErrorKind::NotFound));
    assert_eq!(
        disk.fresh_blocking(&path),
        PathReading::Missing { observed_at: at(1) }
    );
    assert_eq!(disk.calls("lstat"), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_fresh_reads_of_one_path_share_one_call() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    let (open, gate) = mpsc::channel();
    let (entered_tx, entered) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    *backend.entered.lock().unwrap() = Some(entered_tx);

    let first = {
        let disk = disk.clone();
        tokio::spawn(async move { disk.fresh("/srv/fixture-media/a.mkv").await })
    };
    // The first probe is inside the backend; everyone else waits on it.
    tokio::task::spawn_blocking(move || entered.recv().expect("first probe entered"))
        .await
        .unwrap();
    let rest: Vec<_> = (0..8)
        .map(|_| {
            let disk = disk.clone();
            tokio::spawn(async move { disk.fresh("/srv/fixture-media/a.mkv").await })
        })
        .collect();
    open.send(()).unwrap();
    drop(open);
    assert_eq!(first.await.unwrap().exists(), Some(true));
    for waiter in rest {
        assert_eq!(waiter.await.unwrap().exists(), Some(true));
    }
    assert_eq!(backend.calls_of("lstat"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_probe_that_does_not_answer_in_time_reads_unreachable_until_it_does() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    disk.set_probe_timeout_for_test(std::time::Duration::from_millis(1));
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);

    // The probe is held in the backend until the gate opens, so the bound
    // always elapses first.
    let reading = disk.fresh("/srv/fixture-media/a.mkv").await;
    assert_eq!(reading, PathReading::Unreachable { observed_at: at(0) });
    assert_eq!(
        disk.observed(&p("/srv/fixture-media/a.mkv")),
        PathReading::Unreachable { observed_at: at(0) }
    );

    // When the held probe returns, its newer reading replaces the timeout.
    clock.set(at(1));
    let (done_tx, done) = mpsc::channel();
    {
        let disk = disk.clone();
        let flight = disk
            .inner
            .cache
            .flight(&key_of(&p("/srv/fixture-media/a.mkv")));
        std::thread::spawn(move || {
            open.send(()).unwrap();
            // Taking the flight waits for the held probe to finish storing.
            drop(hold(&flight));
            done_tx.send(()).unwrap();
        });
    }
    tokio::task::spawn_blocking(move || done.recv().unwrap())
        .await
        .unwrap();
    assert_eq!(
        disk.observed(&p("/srv/fixture-media/a.mkv")).kind(),
        Some(EntryKind::File)
    );
}

#[test]
fn only_not_found_is_missing() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is("/srv/fixture-media/gone", Err(io::ErrorKind::NotFound));
    backend.stat_is(
        "/srv/fixture-media/denied",
        Err(io::ErrorKind::PermissionDenied),
    );
    backend.stat_is("/srv/fixture-media/hung", Err(io::ErrorKind::TimedOut));
    backend.stat_is(
        "/srv/fixture-media/stale",
        Err(io::ErrorKind::StaleNetworkFileHandle),
    );
    assert_eq!(
        disk.fresh_blocking(&p("/srv/fixture-media/gone")),
        PathReading::Missing { observed_at: at(0) }
    );
    for path in [
        "/srv/fixture-media/denied",
        "/srv/fixture-media/hung",
        "/srv/fixture-media/stale",
    ] {
        let reading = disk.fresh_blocking(&p(path));
        assert_eq!(
            reading,
            PathReading::Unreachable { observed_at: at(0) },
            "{path}"
        );
        assert_eq!(
            reading.unknown_reason(),
            Some(reason::DISK_PATH_UNREACHABLE)
        );
    }
}

#[test]
fn application_data_is_never_cached() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is("/var/lib/invented-app/scryer.db", Ok(file(9)));
    let path = p("/var/lib/invented-app/scryer.db");
    assert_eq!(disk.fresh_blocking(&path).exists(), Some(true));
    assert_eq!(disk.fresh_blocking(&path).exists(), Some(true));
    assert_eq!(backend.calls_of("lstat"), 2);
    assert_eq!(disk.observed(&path), PathReading::NotObserved);
    backend.lists(
        "/var/lib/invented-app",
        &[("scryer.db", Some(EntryKind::File))],
    );
    disk.list_blocking(&p("/var/lib/invented-app")).unwrap();
    assert_eq!(disk.observed(&path), PathReading::NotObserved);
    assert_eq!(disk.inner.cache.len(), 0);
}

#[test]
fn the_owner_works_with_no_roots_and_no_runtime() {
    let backend = Arc::new(Scripted::default());
    let disk = Disk::with_seams(backend.clone(), ManualClock::at(at(0)));
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    assert_eq!(
        disk.fresh_blocking(&p("/srv/fixture-media/a.mkv")).exists(),
        Some(true)
    );
    assert_eq!(
        disk.fresh_blocking(&p("/srv/fixture-media/a.mkv")).exists(),
        Some(true)
    );
    assert_eq!(backend.calls_of("lstat"), 2);
    assert_eq!(
        disk.remove_blocking(&p("/srv/fixture-media/a.mkv"), Remove::File)
            .unwrap(),
        Removal::Removed
    );
    assert_eq!(disk.inner.cache.len(), 0);
}

#[test]
fn a_listing_seeds_its_children_and_forgets_ones_it_no_longer_holds() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is("/srv/fixture-media/Show/old.mkv", Ok(file(3)));
    disk.fresh_blocking(&p("/srv/fixture-media/Show/old.mkv"));
    backend.stat_is("/srv/fixture-media/Show/Season 01/x.mkv", Ok(file(3)));
    disk.fresh_blocking(&p("/srv/fixture-media/Show/Season 01/x.mkv"));
    backend.lists(
        "/srv/fixture-media/Show",
        &[
            ("a.mkv", Some(EntryKind::File)),
            ("Season 01", Some(EntryKind::Directory)),
            ("odd", None),
        ],
    );
    let listed = disk.list_blocking(&p("/srv/fixture-media/Show")).unwrap();
    assert_eq!(listed.len(), 3);
    let lstats = backend.calls_of("lstat");

    assert_eq!(
        disk.observed(&p("/srv/fixture-media/Show/a.mkv")).kind(),
        Some(EntryKind::File)
    );
    assert_eq!(
        disk.observed(&p("/srv/fixture-media/Show/Season 01"))
            .kind(),
        Some(EntryKind::Directory)
    );
    assert_eq!(
        disk.observed(&p("/srv/fixture-media/Show/odd")),
        PathReading::NotObserved
    );
    // A child the listing did not return is forgotten; a grandchild is not
    // the listing's to judge.
    assert_eq!(
        disk.observed(&p("/srv/fixture-media/Show/old.mkv")),
        PathReading::NotObserved
    );
    assert!(
        disk.observed(&p("/srv/fixture-media/Show/Season 01/x.mkv"))
            .exists()
            .is_some()
    );
    assert_eq!(backend.calls_of("lstat"), lstats);

    // A listing proves kind, not size: a probing read still probes.
    backend.stat_is("/srv/fixture-media/Show/a.mkv", Ok(file(7)));
    assert!(matches!(
        disk.fresh_blocking(&p("/srv/fixture-media/Show/a.mkv")),
        PathReading::Known { len: Some(7), .. }
    ));
    assert_eq!(backend.calls_of("lstat"), lstats + 1);
}

/// Seed readings for `paths` with one probe each.
fn seed(disk: &Disk, backend: &Scripted, paths: &[&str]) {
    for path in paths {
        backend.stat_is(path, Ok(dir()));
        disk.fresh_blocking(&p(path));
        assert!(disk.observed(&p(path)).exists().is_some(), "{path} seeded");
    }
}

fn evicted(disk: &Disk, paths: &[&str]) {
    for path in paths {
        assert_eq!(
            disk.observed(&p(path)),
            PathReading::NotObserved,
            "{path} evicted"
        );
    }
}

fn kept(disk: &Disk, paths: &[&str]) {
    for path in paths {
        assert!(disk.observed(&p(path)).exists().is_some(), "{path} kept");
    }
}

const TREE: &[&str] = &[
    "/srv/fixture-media",
    "/srv/fixture-media/Show",
    "/srv/fixture-media/Show/S01",
    "/srv/fixture-media/Show/S01/e1.mkv",
    "/srv/fixture-media/Other",
];

#[test]
fn removing_a_file_evicts_it_and_its_parent() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(&disk, &backend, TREE);
    disk.remove_blocking(&p("/srv/fixture-media/Show/S01/e1.mkv"), Remove::File)
        .unwrap();
    evicted(
        &disk,
        &[
            "/srv/fixture-media/Show/S01/e1.mkv",
            "/srv/fixture-media/Show/S01",
        ],
    );
    kept(
        &disk,
        &["/srv/fixture-media/Show", "/srv/fixture-media/Other"],
    );
}

#[test]
fn removing_a_directory_evicts_its_subtree_and_parent() {
    for what in [Remove::EmptyDir, Remove::Tree] {
        let clock = ManualClock::at(at(0));
        let (disk, backend) = owner(&clock);
        seed(&disk, &backend, TREE);
        disk.remove_blocking(&p("/srv/fixture-media/Show"), what)
            .unwrap();
        evicted(
            &disk,
            &[
                "/srv/fixture-media",
                "/srv/fixture-media/Show",
                "/srv/fixture-media/Show/S01",
                "/srv/fixture-media/Show/S01/e1.mkv",
            ],
        );
        kept(&disk, &["/srv/fixture-media/Other"]);
    }
}

#[test]
fn a_failed_removal_still_evicts() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(&disk, &backend, TREE);
    backend.fails(
        "remove_tree",
        "/srv/fixture-media/Show",
        io::ErrorKind::PermissionDenied,
    );
    let error = disk
        .remove_blocking(&p("/srv/fixture-media/Show"), Remove::Tree)
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_PERMISSION_DENIED);
    evicted(&disk, &["/srv/fixture-media/Show/S01/e1.mkv"]);
}

#[test]
fn removing_what_is_already_gone_is_not_a_failure() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.fails(
        "remove_file",
        "/srv/fixture-media/gone.mkv",
        io::ErrorKind::NotFound,
    );
    assert_eq!(
        disk.remove_blocking(&p("/srv/fixture-media/gone.mkv"), Remove::File)
            .unwrap(),
        Removal::AlreadyAbsent
    );
    // One call: no retry, nothing else removed.
    assert_eq!(disk.total_calls(), 1);
}

#[test]
fn a_move_evicts_both_subtrees() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(&disk, &backend, TREE);
    seed(&disk, &backend, &["/srv/fixture-media/Other/S01"]);
    assert_eq!(
        disk.move_exclusive_blocking(
            &p("/srv/fixture-media/Show/S01"),
            &p("/srv/fixture-media/Other/S01")
        )
        .unwrap(),
        MoveOutcome::Renamed
    );
    evicted(
        &disk,
        &[
            "/srv/fixture-media/Show",
            "/srv/fixture-media/Show/S01",
            "/srv/fixture-media/Show/S01/e1.mkv",
            "/srv/fixture-media/Other",
            "/srv/fixture-media/Other/S01",
        ],
    );
}

#[test]
fn a_root_without_exclusive_rename_is_learned_once() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.fails(
        "rename_no_replace",
        "/srv/fixture-media/a",
        io::ErrorKind::Unsupported,
    );
    backend.fails(
        "rename_no_replace",
        "/srv/fixture-media/b",
        io::ErrorKind::Unsupported,
    );
    disk.move_exclusive_blocking(&p("/srv/fixture-media/a"), &p("/srv/fixture-media/a2"))
        .unwrap();
    disk.move_exclusive_blocking(&p("/srv/fixture-media/b"), &p("/srv/fixture-media/b2"))
        .unwrap();
    assert_eq!(disk.calls("rename_no_replace"), 1);
    assert_eq!(disk.calls("rename_claimed"), 2);
    // Application data keeps trying: there is no root to remember it on.
    backend.fails(
        "rename_no_replace",
        "/tmp/invented-a",
        io::ErrorKind::Unsupported,
    );
    backend.fails(
        "rename_no_replace",
        "/tmp/invented-b",
        io::ErrorKind::Unsupported,
    );
    disk.move_exclusive_blocking(&p("/tmp/invented-a"), &p("/tmp/invented-a2"))
        .unwrap();
    disk.move_exclusive_blocking(&p("/tmp/invented-b"), &p("/tmp/invented-b2"))
        .unwrap();
    assert_eq!(disk.calls("rename_no_replace"), 3);
}

#[test]
fn a_move_that_needs_a_copy_or_meets_an_existing_destination_is_refused() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.fails(
        "rename_no_replace",
        "/srv/fixture-media/a",
        io::ErrorKind::CrossesDevices,
    );
    backend.fails(
        "rename_no_replace",
        "/srv/fixture-media/b",
        io::ErrorKind::AlreadyExists,
    );
    let error = disk
        .move_exclusive_blocking(&p("/srv/fixture-media/a"), &p("/srv/fixture-downloads/a"))
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_MOVE_NEEDS_COPY);
    let error = disk
        .move_exclusive_blocking(&p("/srv/fixture-media/b"), &p("/srv/fixture-media/c"))
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
    assert_eq!(disk.calls("rename_claimed"), 0);
}

#[test]
fn a_link_evicts_its_destination_and_reports_when_it_cannot_be_made() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(&disk, &backend, TREE);
    seed(&disk, &backend, &["/srv/fixture-media/Other/e1.mkv"]);
    backend.fails(
        "hard_link",
        "/srv/fixture-media/Other/e1.mkv",
        io::ErrorKind::CrossesDevices,
    );
    assert_eq!(
        disk.link_blocking(
            &p("/srv/fixture-media/Show/S01/e1.mkv"),
            &p("/srv/fixture-media/Other/e1.mkv"),
            Link::Hard
        )
        .unwrap(),
        LinkOutcome::NotPossible
    );
    evicted(
        &disk,
        &[
            "/srv/fixture-media/Other/e1.mkv",
            "/srv/fixture-media/Other",
        ],
    );
    kept(&disk, &["/srv/fixture-media/Show/S01/e1.mkv"]);
    assert_eq!(
        disk.link_blocking(
            &p("/srv/fixture-media/Show/S01/e1.mkv"),
            &p("/srv/fixture-media/Other/e2.mkv"),
            Link::Symbolic
        )
        .unwrap(),
        LinkOutcome::Linked
    );
    backend.fails(
        "symlink",
        "/srv/fixture-media/Other/e3.mkv",
        io::ErrorKind::AlreadyExists,
    );
    let error = disk
        .link_blocking(
            &p("/srv/fixture-media/Show/S01/e1.mkv"),
            &p("/srv/fixture-media/Other/e3.mkv"),
            Link::Symbolic,
        )
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
}

#[test]
fn creating_directories_evicts_every_ancestor_a_missing_reading_could_hold() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    for path in ["/srv/fixture-media/New", "/srv/fixture-media/New/S01"] {
        disk.fresh_blocking(&p(path));
        assert_eq!(disk.observed(&p(path)).exists(), Some(false));
    }
    seed(&disk, &backend, &["/srv/fixture-media/Other"]);
    disk.create_dir_blocking(&p("/srv/fixture-media/New/S01"), Parents::Create)
        .unwrap();
    evicted(
        &disk,
        &["/srv/fixture-media/New", "/srv/fixture-media/New/S01"],
    );
    kept(&disk, &["/srv/fixture-media/Other"]);
}

#[test]
fn an_atomic_write_creates_parents_stages_and_evicts_the_destination() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    disk.fresh_blocking(&p("/srv/fixture-media/Show/show.nfo"));
    assert_eq!(
        disk.write_atomic_blocking(
            &p("/srv/fixture-media/Show/show.nfo"),
            b"x",
            Replace::Never,
            Access::Default
        )
        .unwrap(),
        SyncOutcome::Durable
    );
    evicted(&disk, &["/srv/fixture-media/Show/show.nfo"]);
    assert_eq!(backend.calls_of("create_dir_all"), 1);
    let staged = backend
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|(op, _)| *op == "write_new")
        .map(|(_, path)| path.clone())
        .unwrap();
    assert_eq!(staged.parent(), Some(Path::new("/srv/fixture-media/Show")));
    assert!(
        staged
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".scryer-")
    );
    assert_eq!(disk.calls("rename_no_replace"), 1);
    assert_eq!(disk.calls("rename"), 0);
    disk.write_atomic_blocking(
        &p("/srv/fixture-media/Show/show.nfo"),
        b"y",
        Replace::Allowed,
        Access::Default,
    )
    .unwrap();
    assert_eq!(disk.calls("rename"), 1);
}

#[test]
fn a_pass_probes_each_path_once_and_uses_its_listings() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is("/srv/fixture-media/Show", Ok(dir()));
    backend.lists(
        "/srv/fixture-media/Show",
        &[("a.mkv", Some(EntryKind::File))],
    );
    let mut pass = disk.pass();
    for second in 0..3 {
        // Well past the reuse window: only the pass's evidence prevents a probe.
        clock.set(at(second * 10));
        assert_eq!(
            pass.reading(&p("/srv/fixture-media/Show")).exists(),
            Some(true)
        );
        assert_eq!(pass.list(&p("/srv/fixture-media/Show")).unwrap().len(), 1);
        assert_eq!(
            pass.reading(&p("/srv/fixture-media/Show/a.mkv")).kind(),
            Some(EntryKind::File)
        );
    }
    assert_eq!(backend.calls_of("lstat"), 1);
    assert_eq!(backend.calls_of("list"), 1);

    pass.forget(&p("/srv/fixture-media/Show/a.mkv"));
    clock.set(at(100));
    pass.reading(&p("/srv/fixture-media/Show"));
    assert_eq!(backend.calls_of("lstat"), 2, "the parent is forgotten too");
    drop(pass);
    // The next pass starts with no evidence of its own.
    let mut next = disk.pass();
    clock.set(at(200));
    next.reading(&p("/srv/fixture-media/Show"));
    assert_eq!(backend.calls_of("lstat"), 3);
}

#[test]
fn the_capacity_owner_is_the_process_one() {
    let disk = Disk::with_seams(Arc::new(Scripted::default()), ManualClock::at(at(0)));
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
        capacity::StorageCapacity::process().reading(path, at(0)),
        capacity::StorageCapacityReading::Known {
            available_bytes: 40,
            ..
        }
    ));
}

#[tokio::test]
async fn async_verbs_run_off_the_runtime_and_report_like_their_blocking_forms() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.fails(
        "remove_file",
        "/srv/fixture-media/gone",
        io::ErrorKind::NotFound,
    );
    assert_eq!(
        disk.remove("/srv/fixture-media/gone", Remove::File)
            .await
            .unwrap(),
        Removal::AlreadyAbsent
    );
    assert_eq!(
        disk.sync("/srv/fixture-media/a").await.unwrap(),
        SyncOutcome::Unconfirmed
    );
    backend.fails(
        "create_dir",
        "/srv/fixture-media/x",
        io::ErrorKind::AlreadyExists,
    );
    assert_eq!(
        disk.create_dir("/srv/fixture-media/x", Parents::MustExist)
            .await
            .unwrap_err()
            .reason(),
        reason::DISK_DESTINATION_EXISTS
    );
    assert!(disk.list("/srv/fixture-media/none").await.is_err());
    assert_eq!(
        disk.canonicalize("/srv/fixture-media/a").await.unwrap(),
        p("/srv/fixture-media/a")
    );
}

const OTHER_ROOT: &str = "/srv/fixture-other";

/// An owner with two library roots, whose async reads of `ROOT` are held in
/// the backend until the returned sender is dropped.
fn two_roots_one_held(
    clock: &Arc<ManualClock>,
    held: &str,
) -> (Disk, Arc<Scripted>, mpsc::Sender<()>) {
    let (disk, backend) = owner(clock);
    disk.set_volumes([
        (p(ROOT), VolumeClass::LibraryRoot),
        (p(OTHER_ROOT), VolumeClass::LibraryRoot),
    ]);
    disk.set_probe_timeout_for_test(std::time::Duration::from_millis(1));
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    *backend.gate_under.lock().unwrap() = Some(p(held));
    (disk, backend, open)
}

fn lstats_under(backend: &Scripted, root: &str) -> usize {
    backend
        .paths_of("lstat")
        .iter()
        .filter(|path| path.starts_with(root))
        .count()
}

/// Wait, within a generous failure bound, until every read under the root of
/// `path` has answered.
async fn until_clear(disk: &Disk, path: &str) {
    let learned = read(&disk.inner.volumes).learned(&p(path)).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while !learned.is_clear() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("every held read answered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_root_holds_one_probe_and_one_trial_and_answers_the_rest_at_once() {
    let clock = ManualClock::at(at(0));
    let (disk, backend, open) = two_roots_one_held(&clock, ROOT);
    for path in [
        "/srv/fixture-media/a.mkv",
        "/srv/fixture-media/b.mkv",
        "/srv/fixture-media/c.mkv",
        "/srv/fixture-other/x.mkv",
        "/var/lib/invented-app/scryer.db",
    ] {
        backend.stat_is(path, Ok(file(5)));
    }

    // The first probe outlives its bound; the root is now suspect, and the
    // next read under it goes out as the one trial, which is held too.
    assert_eq!(
        disk.fresh("/srv/fixture-media/a.mkv").await,
        PathReading::Unreachable { observed_at: at(0) }
    );
    assert_eq!(
        disk.fresh("/srv/fixture-media/b.mkv").await,
        PathReading::Unreachable { observed_at: at(0) }
    );
    assert_eq!(lstats_under(&backend, ROOT), 2);

    // Past the reuse window, with a bound no test would wait out: every
    // further read under the root, of the stuck paths or a new sibling,
    // answers at once without reaching the backend.
    clock.set(at(5));
    disk.set_probe_timeout_for_test(std::time::Duration::from_secs(3600));
    let burst = async {
        for _ in 0..8 {
            for path in [
                "/srv/fixture-media/a.mkv",
                "/srv/fixture-media/b.mkv",
                "/srv/fixture-media/c.mkv",
            ] {
                assert_eq!(
                    disk.fresh(path).await,
                    PathReading::Unreachable { observed_at: at(5) },
                    "{path}"
                );
            }
            assert_eq!(
                disk.follow("/srv/fixture-media/c.mkv").await,
                PathReading::Unreachable { observed_at: at(5) }
            );
            for error in [
                disk.list("/srv/fixture-media").await.map(|_| ()),
                disk.canonicalize("/srv/fixture-media/c.mkv")
                    .await
                    .map(|_| ()),
                disk.read_link("/srv/fixture-media/c.mkv").await.map(|_| ()),
            ] {
                assert_eq!(error.unwrap_err().reason(), reason::DISK_TIMED_OUT);
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), burst)
        .await
        .expect("refused reads do not wait");
    assert_eq!(lstats_under(&backend, ROOT), 2);
    assert_eq!(backend.calls_of("stat"), 0);
    assert_eq!(backend.calls_of("list"), 0);
    assert_eq!(backend.calls_of("canonicalize"), 0);
    assert_eq!(backend.calls_of("read_link"), 0);

    // Another root and application data are untouched by it.
    assert_eq!(
        disk.fresh("/srv/fixture-other/x.mkv").await.exists(),
        Some(true)
    );
    assert_eq!(
        disk.fresh("/var/lib/invented-app/scryer.db").await.exists(),
        Some(true)
    );
    assert_eq!(lstats_under(&backend, OTHER_ROOT), 1);

    // The disk answers: the held reads return, the record clears, and reads
    // under the root go out again.
    drop(open);
    until_clear(&disk, ROOT).await;
    assert_eq!(
        disk.fresh("/srv/fixture-media/a.mkv").await.kind(),
        Some(EntryKind::File)
    );
    assert_eq!(
        disk.fresh("/srv/fixture-media/c.mkv").await.kind(),
        Some(EntryKind::File)
    );
    assert_eq!(
        lstats_under(&backend, ROOT),
        3,
        "only the new sibling probed"
    );
    assert!(disk.list("/srv/fixture-media").await.is_err());
    assert_eq!(backend.calls_of("list"), 1, "listings go out again");
    assert_eq!(
        disk.fresh("/srv/fixture-other/x.mkv").await.exists(),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_still_out_past_its_bound_is_refused_after_its_root_answers_again() {
    let clock = ManualClock::at(at(0));
    let (disk, backend, open) = two_roots_one_held(&clock, "/srv/fixture-media/a.mkv");
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    backend.stat_is("/srv/fixture-media/b.mkv", Ok(file(5)));

    assert_eq!(
        disk.fresh("/srv/fixture-media/a.mkv").await,
        PathReading::Unreachable { observed_at: at(0) }
    );
    // The trial answers, so the root reads again; the held path does not.
    assert_eq!(
        disk.fresh("/srv/fixture-media/b.mkv").await.kind(),
        Some(EntryKind::File)
    );
    clock.set(at(5));
    disk.set_probe_timeout_for_test(std::time::Duration::from_secs(3600));
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        for _ in 0..8 {
            assert_eq!(
                disk.fresh("/srv/fixture-media/a.mkv").await,
                PathReading::Unreachable { observed_at: at(5) }
            );
        }
    })
    .await
    .expect("the held path is refused without waiting");
    assert_eq!(
        backend.paths_of("lstat"),
        vec![p("/srv/fixture-media/a.mkv"), p("/srv/fixture-media/b.mkv")]
    );

    // Its late answer replaces the unreachable reading and clears the path.
    drop(open);
    until_clear(&disk, ROOT).await;
    assert_eq!(
        disk.fresh("/srv/fixture-media/a.mkv").await.kind(),
        Some(EntryKind::File)
    );
}

#[test]
fn blocking_reads_are_never_refused() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    let learned = read(&disk.inner.volumes)
        .learned(&p("/srv/fixture-media/a.mkv"))
        .unwrap();
    let ticket = volume::Ticket::new(
        learned.clone(),
        key_of(&p("/srv/fixture-media/a.mkv")),
        false,
    );
    ticket.timed_out();
    assert!(learned.is_stuck(&key_of(&p("/srv/fixture-media/a.mkv"))));
    assert_eq!(
        disk.fresh_blocking(&p("/srv/fixture-media/a.mkv")).exists(),
        Some(true)
    );
    assert_eq!(backend.calls_of("lstat"), 1);
    ticket.answered();
    assert!(learned.is_clear());
}

/// The staging path the last atomic write used.
fn staged(backend: &Scripted) -> PathBuf {
    backend
        .paths_of("write_new")
        .pop()
        .expect("a staging file was written")
}

#[test]
fn a_failed_atomic_write_removes_only_the_staging_file_it_created() {
    let destination = "/srv/fixture-media/Show/show.nfo";
    // (scripted failure, mode, the reason the caller sees)
    let cases: [(&'static str, io::ErrorKind, Replace, &str); 4] = [
        (
            "write_body",
            io::ErrorKind::StorageFull,
            Replace::Allowed,
            reason::DISK_NO_SPACE,
        ),
        (
            "rename",
            io::ErrorKind::PermissionDenied,
            Replace::Allowed,
            reason::DISK_PERMISSION_DENIED,
        ),
        (
            "rename_no_replace",
            io::ErrorKind::AlreadyExists,
            Replace::Never,
            reason::DISK_DESTINATION_EXISTS,
        ),
        (
            "rename_claimed",
            io::ErrorKind::AlreadyExists,
            Replace::Never,
            reason::DISK_DESTINATION_EXISTS,
        ),
    ];
    for (operation, kind, replace, expected) in cases {
        for cleanup_fails in [false, true] {
            let clock = ManualClock::at(at(0));
            let (disk, backend) = owner(&clock);
            backend.always_fails(operation, kind);
            if operation == "rename_claimed" {
                backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
            }
            if cleanup_fails {
                backend.always_fails("remove_file", io::ErrorKind::PermissionDenied);
            }
            let error = disk
                .write_atomic_blocking(&p(destination), b"x", replace, Access::Default)
                .unwrap_err();
            assert_eq!(error.reason(), expected, "{operation} {cleanup_fails}");
            assert_eq!(error.verb(), DiskVerb::Write);
            assert_eq!(error.path(), Path::new(destination));
            // One removal, of the staging file, never the destination, a
            // parent or a sibling; no other removal of any kind.
            assert_eq!(
                backend.paths_of("remove_file"),
                vec![staged(&backend)],
                "{operation}"
            );
            assert_eq!(backend.calls_of("remove_dir"), 0);
            assert_eq!(backend.calls_of("remove_tree"), 0);
        }
    }
}

#[test]
fn a_staging_file_this_write_did_not_create_is_left_alone() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.always_fails("write_new", io::ErrorKind::AlreadyExists);
    let error = disk
        .write_atomic_blocking(
            &p("/srv/fixture-media/Show/show.nfo"),
            b"x",
            Replace::Allowed,
            Access::Default,
        )
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
    assert_eq!(backend.calls_of("remove_file"), 0);
    assert_eq!(backend.calls_of("rename"), 0);
}

#[test]
fn a_failed_atomic_write_evicts_the_staging_path_and_the_destination() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(
        &disk,
        &backend,
        &[
            "/srv/fixture-media/Show/show.nfo",
            "/srv/fixture-media/Other",
        ],
    );
    backend.always_fails("rename", io::ErrorKind::PermissionDenied);
    disk.write_atomic_blocking(
        &p("/srv/fixture-media/Show/show.nfo"),
        b"x",
        Replace::Allowed,
        Access::Default,
    )
    .unwrap_err();
    let staged = staged(&backend);
    evicted(
        &disk,
        &[
            "/srv/fixture-media/Show/show.nfo",
            "/srv/fixture-media/Show",
            staged.to_str().unwrap(),
        ],
    );
    kept(&disk, &["/srv/fixture-media/Other"]);
}

#[test]
fn a_whole_read_answers_the_contents_and_a_missing_file_is_an_answer() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.holds("/srv/fixture-media/Show/show.nfo", b"<tvshow/>");
    assert_eq!(
        disk.read_blocking(&p("/srv/fixture-media/Show/show.nfo"))
            .unwrap(),
        b"<tvshow/>"
    );
    assert_eq!(
        disk.read_to_string_blocking(&p("/srv/fixture-media/Show/show.nfo"))
            .unwrap(),
        "<tvshow/>"
    );
    let missing = disk
        .read_blocking(&p("/srv/fixture-media/Show/absent.nfo"))
        .unwrap_err();
    assert_eq!(missing.reason(), reason::DISK_PATH_MISSING);
    assert_eq!(missing.verb(), DiskVerb::Read);
    assert_eq!(
        disk.read_blocking(&p("/srv/fixture-media/Show/absent.nfo"))
            .if_present()
            .unwrap(),
        None
    );
    backend.fails(
        "read",
        "/srv/fixture-media/Show/locked.nfo",
        io::ErrorKind::PermissionDenied,
    );
    let denied = disk
        .read_blocking(&p("/srv/fixture-media/Show/locked.nfo"))
        .if_present()
        .unwrap_err();
    assert_eq!(denied.reason(), reason::DISK_PERMISSION_DENIED);
    assert_eq!(backend.calls_of("read"), 5);
}

#[test]
fn text_that_is_not_utf8_is_a_failure_not_a_lossy_string() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.holds("/srv/fixture-media/Show/show.nfo", &[0xff, 0xfe, 0x00]);
    let error = disk
        .read_to_string_blocking(&p("/srv/fixture-media/Show/show.nfo"))
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_OPERATION_FAILED);
    assert_eq!(error.verb(), DiskVerb::Read);
}

#[test]
fn reads_and_read_opens_leave_the_cache_alone() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(
        &disk,
        &backend,
        &[
            "/srv/fixture-media/Show/show.nfo",
            "/srv/fixture-media/Show",
        ],
    );
    backend.holds("/srv/fixture-media/Show/show.nfo", b"x");
    disk.read_blocking(&p("/srv/fixture-media/Show/show.nfo"))
        .unwrap();
    disk.open_blocking(&p("/srv/fixture-media/Show/show.nfo"), Open::read())
        .unwrap();
    kept(
        &disk,
        &[
            "/srv/fixture-media/Show/show.nfo",
            "/srv/fixture-media/Show",
        ],
    );
}

#[test]
fn every_write_open_evicts_the_file_and_its_parent_even_when_it_fails() {
    for open in [
        Open::write_existing(),
        Open::create_new(),
        Open::create(),
        Open::create_or_keep(),
        Open::append(),
    ] {
        for fails in [false, true] {
            let clock = ManualClock::at(at(0));
            let (disk, backend) = owner(&clock);
            seed(
                &disk,
                &backend,
                &[
                    "/srv/fixture-media/Show/show.nfo",
                    "/srv/fixture-media/Show",
                    "/srv/fixture-media/Other",
                ],
            );
            if fails {
                backend.always_fails("open", io::ErrorKind::PermissionDenied);
            }
            let result = disk.open_blocking(&p("/srv/fixture-media/Show/show.nfo"), open);
            if fails {
                let error = result.unwrap_err();
                assert_eq!(error.reason(), reason::DISK_PERMISSION_DENIED);
                assert_eq!(error.verb(), DiskVerb::Write);
            } else {
                result.unwrap();
            }
            evicted(
                &disk,
                &[
                    "/srv/fixture-media/Show/show.nfo",
                    "/srv/fixture-media/Show",
                ],
            );
            kept(&disk, &["/srv/fixture-media/Other"]);
            assert_eq!(*backend.opened.lock().unwrap(), vec![open]);
        }
    }
}

#[test]
fn finishing_a_handle_syncs_it_and_evicts_again() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    let path = p("/srv/fixture-media/Show/show.nfo");
    let file = disk.open_blocking(&path, Open::create()).unwrap();
    // A reading taken while the file was being written.
    seed(&disk, &backend, &["/srv/fixture-media/Show/show.nfo"]);
    assert_eq!(
        disk.finish_blocking(&path, file).unwrap(),
        SyncOutcome::Durable
    );
    evicted(&disk, &["/srv/fixture-media/Show/show.nfo"]);

    let file = disk.open_blocking(&path, Open::create()).unwrap();
    seed(&disk, &backend, &["/srv/fixture-media/Show/show.nfo"]);
    backend.always_fails("sync_handle", io::ErrorKind::StorageFull);
    let error = disk.finish_blocking(&path, file).unwrap_err();
    assert_eq!(error.reason(), reason::DISK_NO_SPACE);
    assert_eq!(error.verb(), DiskVerb::Sync);
    evicted(&disk, &["/srv/fixture-media/Show/show.nfo"]);
    assert_eq!(backend.calls_of("sync_handle"), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_opens_finish_through_a_tokio_handle() {
    use tokio::io::AsyncWriteExt;
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    let mut file = disk
        .open("/srv/fixture-media/Show/show.nfo", Open::append())
        .await
        .unwrap();
    file.write_all(b"appended").await.unwrap();
    assert_eq!(
        disk.finish("/srv/fixture-media/Show/show.nfo", file)
            .await
            .unwrap(),
        SyncOutcome::Durable
    );
    assert_eq!(backend.calls_of("open"), 1);
    assert_eq!(backend.calls_of("sync_handle"), 1);
}

#[test]
fn a_plain_write_evicts_and_classifies_its_failures() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(
        &disk,
        &backend,
        &[
            "/srv/fixture-media/Show/show.nfo",
            "/srv/fixture-media/Show",
            "/srv/fixture-media/Other",
        ],
    );
    disk.write_blocking(
        &p("/srv/fixture-media/Show/show.nfo"),
        b"x",
        Replace::Allowed,
        Access::OwnerOnly,
    )
    .unwrap();
    evicted(
        &disk,
        &[
            "/srv/fixture-media/Show/show.nfo",
            "/srv/fixture-media/Show",
        ],
    );
    kept(&disk, &["/srv/fixture-media/Other"]);
    assert_eq!(backend.accesses(), vec![("write", Access::OwnerOnly)]);
    // A plain write creates no parents and syncs nothing.
    assert_eq!(backend.calls_of("create_dir_all"), 0);
    assert_eq!(backend.calls_of("sync_handle"), 0);

    for (kind, expected) in [
        (
            io::ErrorKind::AlreadyExists,
            reason::DISK_DESTINATION_EXISTS,
        ),
        (io::ErrorKind::StorageFull, reason::DISK_NO_SPACE),
        (io::ErrorKind::QuotaExceeded, reason::DISK_NO_SPACE),
        (io::ErrorKind::NotFound, reason::DISK_PATH_MISSING),
    ] {
        backend.always_fails("write", kind);
        seed(&disk, &backend, &["/srv/fixture-media/Show/show.nfo"]);
        let error = disk
            .write_blocking(
                &p("/srv/fixture-media/Show/show.nfo"),
                b"x",
                Replace::Never,
                Access::Default,
            )
            .unwrap_err();
        assert_eq!(error.reason(), expected, "{kind:?}");
        assert_eq!(error.verb(), DiskVerb::Write);
        evicted(&disk, &["/srv/fixture-media/Show/show.nfo"]);
    }
    // No write here removes anything.
    assert_eq!(backend.calls_of("remove_file"), 0);
}

#[test]
fn an_atomic_write_passes_its_access_to_the_new_file_only() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    disk.write_atomic_blocking(
        &p("/srv/fixture-media/Show/secret.json"),
        b"x",
        Replace::Allowed,
        Access::OwnerOnly,
    )
    .unwrap();
    assert_eq!(backend.accesses(), vec![("write_new", Access::OwnerOnly)]);
}

/// The staging path the last staged write opened.
fn opened_path(backend: &Scripted) -> PathBuf {
    backend.paths_of("open").pop().expect("a file was opened")
}

#[test]
fn a_committed_stage_lands_on_its_destination_and_removes_nothing() {
    for replace in [Replace::Allowed, Replace::Never] {
        let clock = ManualClock::at(at(0));
        let (disk, backend) = owner(&clock);
        let dest = p("/srv/fixture-media/Show/show.nfo");
        let mut staged = disk
            .stage_blocking(&dest, Parents::Create, Access::OwnerOnly)
            .unwrap();
        assert_eq!(
            backend.paths_of("create_dir_all"),
            vec![p("/srv/fixture-media/Show")]
        );
        let staging = opened_path(&backend);
        assert_eq!(staged.path(), staging);
        assert_eq!(staged.destination(), dest);
        assert_eq!(staging.parent(), dest.parent());
        let name = staging.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with(".scryer-") && name.ends_with(".partial"),
            "{name}"
        );
        assert_eq!(
            *backend.opened.lock().unwrap(),
            vec![Open::create_new().with_access(Access::OwnerOnly)]
        );
        io::Write::write_all(&mut staged, b"<tvshow/>").unwrap();
        seed(
            &disk,
            &backend,
            &[
                "/srv/fixture-media/Show/show.nfo",
                staging.to_str().unwrap(),
            ],
        );
        assert_eq!(
            staged.commit_blocking(replace).unwrap(),
            SyncOutcome::Durable
        );
        evicted(
            &disk,
            &[
                "/srv/fixture-media/Show/show.nfo",
                staging.to_str().unwrap(),
            ],
        );
        let renamed = match replace {
            Replace::Allowed => backend.paths_of("rename"),
            Replace::Never => backend.paths_of("rename_no_replace"),
        };
        assert_eq!(renamed, vec![staging]);
        assert_eq!(backend.calls_of("sync_handle"), 1);
        assert_eq!(backend.calls_of("remove_file"), 0);
    }
}

#[test]
fn a_stage_that_must_find_its_parent_creates_none() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.always_fails("open", io::ErrorKind::NotFound);
    let error = disk
        .stage_blocking(
            &p("/srv/fixture-media/Gone/show.nfo"),
            Parents::MustExist,
            Access::Default,
        )
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_PATH_MISSING);
    assert_eq!(error.path(), Path::new("/srv/fixture-media/Gone/show.nfo"));
    assert_eq!(backend.calls_of("create_dir_all"), 0);
    // Nothing was created, so nothing is removed.
    assert_eq!(backend.calls_of("remove_file"), 0);
}

#[test]
fn a_dropped_stage_removes_only_its_own_staging_file() {
    for cleanup_fails in [false, true] {
        let clock = ManualClock::at(at(0));
        let (disk, backend) = owner(&clock);
        if cleanup_fails {
            backend.always_fails("remove_file", io::ErrorKind::PermissionDenied);
        }
        let staged = disk
            .stage_blocking(
                &p("/srv/fixture-media/Show/show.nfo"),
                Parents::MustExist,
                Access::Default,
            )
            .unwrap();
        let staging = opened_path(&backend);
        seed(&disk, &backend, &[staging.to_str().unwrap()]);
        drop(staged);
        assert_eq!(backend.paths_of("remove_file"), vec![staging.clone()]);
        assert_eq!(backend.calls_of("remove_dir"), 0);
        assert_eq!(backend.calls_of("remove_tree"), 0);
        assert_eq!(backend.calls_of("rename"), 0);
        evicted(&disk, &[staging.to_str().unwrap()]);
    }
}

#[test]
fn a_failed_commit_keeps_the_destination_and_removes_the_staging_file() {
    let cases: [(&'static str, io::ErrorKind, Replace, &str); 3] = [
        (
            "sync_handle",
            io::ErrorKind::StorageFull,
            Replace::Allowed,
            reason::DISK_NO_SPACE,
        ),
        (
            "rename",
            io::ErrorKind::PermissionDenied,
            Replace::Allowed,
            reason::DISK_PERMISSION_DENIED,
        ),
        (
            "rename_no_replace",
            io::ErrorKind::AlreadyExists,
            Replace::Never,
            reason::DISK_DESTINATION_EXISTS,
        ),
    ];
    for (operation, kind, replace, expected) in cases {
        let clock = ManualClock::at(at(0));
        let (disk, backend) = owner(&clock);
        backend.always_fails(operation, kind);
        let staged = disk
            .stage_blocking(
                &p("/srv/fixture-media/Show/show.nfo"),
                Parents::MustExist,
                Access::Default,
            )
            .unwrap();
        let staging = opened_path(&backend);
        let error = staged.commit_blocking(replace).unwrap_err();
        assert_eq!(error.reason(), expected, "{operation}");
        assert_eq!(error.verb(), DiskVerb::Write);
        assert_eq!(error.path(), Path::new("/srv/fixture-media/Show/show.nfo"));
        assert_eq!(
            backend.paths_of("remove_file"),
            vec![staging],
            "{operation}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_async_stage_commits_off_the_runtime() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    let mut staged = disk
        .stage(
            "/srv/fixture-media/Show/show.nfo",
            Parents::Create,
            Access::Default,
        )
        .await
        .unwrap();
    io::Write::write_all(staged.file(), b"x").unwrap();
    staged.commit(Replace::Allowed).await.unwrap();
    assert_eq!(backend.calls_of("rename"), 1);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

#[test]
fn syncing_a_directory_is_counted_and_classified() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    assert_eq!(
        disk.sync_dir_blocking(&p("/srv/fixture-media/Show"))
            .unwrap(),
        SyncOutcome::Durable
    );
    assert_eq!(disk.calls("sync_dir"), 1);
    backend.always_fails("sync_dir", io::ErrorKind::NotFound);
    let error = disk
        .sync_dir_blocking(&p("/srv/fixture-media/Gone"))
        .unwrap_err();
    assert_eq!(error.reason(), reason::DISK_PATH_MISSING);
    assert_eq!(error.verb(), DiskVerb::Sync);
}

#[test]
fn a_scratch_dir_is_owner_only_and_removes_itself_when_dropped() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    let scratch = disk
        .scratch_dir_blocking(".scryer-invented-scratch-")
        .unwrap();
    let path = scratch.path().to_path_buf();
    assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
    assert!(
        path.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(".scryer-invented-scratch-")
    );
    assert_eq!(backend.accesses(), vec![("create_dir", Access::OwnerOnly)]);
    assert_eq!(backend.calls_of("remove_tree"), 0);
    drop(scratch);
    assert_eq!(backend.paths_of("remove_tree"), vec![path]);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

#[test]
fn a_scratch_dir_retries_a_taken_name_and_gives_up_on_anything_else() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.always_fails("create_dir", io::ErrorKind::AlreadyExists);
    let error = disk.scratch_dir_blocking("invented-").unwrap_err();
    assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
    assert_eq!(backend.calls_of("create_dir"), SCRATCH_NAME_TRIES);

    let (disk, backend) = owner(&clock);
    backend.always_fails("create_dir", io::ErrorKind::PermissionDenied);
    let error = disk.scratch_dir_blocking("invented-").unwrap_err();
    assert_eq!(error.reason(), reason::DISK_PERMISSION_DENIED);
    assert_eq!(error.verb(), DiskVerb::CreateDir);
    assert_eq!(backend.calls_of("create_dir"), 1);
    // A directory that was never made is never removed.
    assert_eq!(backend.calls_of("remove_tree"), 0);
}

#[test]
fn permission_and_time_changes_evict_and_classify() {
    type Change = fn(&Disk, &Path) -> Result<(), DiskError>;
    let changes: [(&'static str, Change); 2] = [
        ("restrict_to_owner", |disk, path| {
            disk.restrict_to_owner_blocking(path, OwnerOnly::File)
        }),
        ("set_modified", |disk, path| {
            disk.set_modified_blocking(path, SystemTime::UNIX_EPOCH)
        }),
    ];
    for (operation, change) in changes {
        let clock = ManualClock::at(at(0));
        let (disk, backend) = owner(&clock);
        seed(
            &disk,
            &backend,
            &[
                "/srv/fixture-media/Show/show.nfo",
                "/srv/fixture-media/Show",
                "/srv/fixture-media/Other",
            ],
        );
        change(&disk, &p("/srv/fixture-media/Show/show.nfo")).unwrap();
        assert_eq!(disk.calls(operation), 1);
        evicted(
            &disk,
            &[
                "/srv/fixture-media/Show/show.nfo",
                "/srv/fixture-media/Show",
            ],
        );
        kept(&disk, &["/srv/fixture-media/Other"]);

        backend.always_fails(operation, io::ErrorKind::PermissionDenied);
        seed(&disk, &backend, &["/srv/fixture-media/Show/show.nfo"]);
        let error = change(&disk, &p("/srv/fixture-media/Show/show.nfo")).unwrap_err();
        assert_eq!(
            error.reason(),
            reason::DISK_PERMISSION_DENIED,
            "{operation}"
        );
        assert_eq!(error.verb(), DiskVerb::Change);
        evicted(&disk, &["/srv/fixture-media/Show/show.nfo"]);
    }
}

#[test]
fn a_hard_link_evicts_its_source_too() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    seed(
        &disk,
        &backend,
        &[
            "/srv/fixture-media/a.mkv",
            "/srv/fixture-media/Show/b.mkv",
            "/srv/fixture-media/Other",
        ],
    );
    disk.link_blocking(
        &p("/srv/fixture-media/a.mkv"),
        &p("/srv/fixture-media/Show/b.mkv"),
        Link::Hard,
    )
    .unwrap();
    evicted(
        &disk,
        &["/srv/fixture-media/a.mkv", "/srv/fixture-media/Show/b.mkv"],
    );
    kept(&disk, &["/srv/fixture-media/Other"]);
}

#[test]
fn a_reading_carries_the_link_count() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is(
        "/srv/fixture-media/a.mkv",
        Ok(RawStat {
            links: Some(3),
            ..file(5)
        }),
    );
    assert_eq!(
        disk.fresh_blocking(&p("/srv/fixture-media/a.mkv"))
            .link_count(),
        Some(3)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_under_a_stuck_root_are_refused_but_writes_go_out() {
    let clock = ManualClock::at(at(0));
    let (disk, backend, open) = two_roots_one_held(&clock, ROOT);
    backend.stat_is("/srv/fixture-media/a.mkv", Ok(file(5)));
    backend.stat_is("/srv/fixture-media/b.mkv", Ok(file(5)));
    backend.holds("/srv/fixture-media/c.nfo", b"x");
    // The first probe outlives its bound and the trial after it is held too.
    disk.fresh("/srv/fixture-media/a.mkv").await;
    disk.fresh("/srv/fixture-media/b.mkv").await;

    disk.set_probe_timeout_for_test(std::time::Duration::from_secs(3600));
    for error in [
        disk.read("/srv/fixture-media/c.nfo").await.unwrap_err(),
        disk.read_to_string("/srv/fixture-media/c.nfo")
            .await
            .unwrap_err(),
        disk.open("/srv/fixture-media/c.nfo", Open::read())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.reason(), reason::DISK_TIMED_OUT);
        assert_eq!(error.verb(), DiskVerb::Read);
    }
    assert_eq!(backend.calls_of("read"), 0);
    assert_eq!(backend.calls_of("open"), 0);
    assert_eq!(backend.calls_of("read_from"), 0);

    // Writing is never refused or timed out.
    let file = disk
        .open("/srv/fixture-media/c.nfo", Open::create())
        .await
        .unwrap();
    disk.finish("/srv/fixture-media/c.nfo", file).await.unwrap();
    disk.write(
        "/srv/fixture-media/d.nfo",
        b"x".to_vec(),
        Replace::Allowed,
        Access::Default,
    )
    .await
    .unwrap();
    assert_eq!(backend.calls_of("open"), 1);
    assert_eq!(backend.calls_of("write"), 1);

    drop(open);
    until_clear(&disk, ROOT).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn under_a_root_only_reaching_the_file_is_bounded_not_reading_it() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    // A bound no correct open reaches on any machine: the open is the bounded
    // step, and the contents read after it must not be.
    disk.set_probe_timeout_for_test(std::time::Duration::from_secs(3600));
    backend.holds("/srv/fixture-media/Show/large.nfo", b"<tvshow/>");
    backend.stat_is("/srv/fixture-media/Show/other.nfo", Ok(file(5)));
    let (release, gate) = mpsc::channel();
    let (entered_tx, entered) = mpsc::channel();
    *backend.contents_gate.lock().unwrap() = Some(gate);
    *backend.contents_entered.lock().unwrap() = Some(entered_tx);

    let reading = {
        let disk = disk.clone();
        tokio::spawn(async move {
            disk.read_to_string("/srv/fixture-media/Show/large.nfo")
                .await
        })
    };
    // The contents read is under way and held until the test lets it go.
    tokio::task::spawn_blocking(move || entered.recv().expect("contents read entered"))
        .await
        .unwrap();
    let learned = read(&disk.inner.volumes)
        .learned(&p("/srv/fixture-media/Show/large.nfo"))
        .unwrap();
    assert!(
        learned.is_clear(),
        "a slow contents read marks nothing stuck"
    );
    // Unrelated reads under the root still go out while it is held.
    assert_eq!(
        disk.fresh("/srv/fixture-media/Show/other.nfo")
            .await
            .exists(),
        Some(true)
    );
    release.send(()).unwrap();
    assert_eq!(reading.await.unwrap().unwrap(), "<tvshow/>");
    assert!(learned.is_clear());
    assert_eq!(backend.calls_of("open"), 1);
    assert_eq!(backend.calls_of("read_from"), 1);
    assert_eq!(backend.calls_of("read"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn under_a_root_a_missing_or_undecodable_file_fails_as_the_blocking_read_does() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    let missing = disk
        .read("/srv/fixture-media/Show/absent.nfo")
        .await
        .unwrap_err();
    assert_eq!(missing.reason(), reason::DISK_PATH_MISSING);
    assert_eq!(missing.verb(), DiskVerb::Read);
    assert_eq!(backend.calls_of("read_from"), 0);
    backend.holds("/srv/fixture-media/Show/bad.nfo", &[0xff, 0xfe]);
    let bad = disk
        .read_to_string("/srv/fixture-media/Show/bad.nfo")
        .await
        .unwrap_err();
    assert_eq!(bad.reason(), reason::DISK_OPERATION_FAILED);
    backend.always_fails("read_from", io::ErrorKind::PermissionDenied);
    backend.holds("/srv/fixture-media/Show/locked.nfo", b"x");
    let denied = disk
        .read("/srv/fixture-media/Show/locked.nfo")
        .await
        .unwrap_err();
    assert_eq!(denied.reason(), reason::DISK_PERMISSION_DENIED);
    assert_eq!(
        denied.path(),
        Path::new("/srv/fixture-media/Show/locked.nfo")
    );
}

#[tokio::test]
async fn application_data_is_read_whole_in_one_call() {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.holds("/var/lib/invented-app/settings.json", b"{}");
    assert_eq!(
        disk.read_to_string("/var/lib/invented-app/settings.json")
            .await
            .unwrap(),
        "{}"
    );
    assert_eq!(backend.calls_of("read"), 1);
    assert_eq!(backend.calls_of("open"), 0);
    assert_eq!(backend.calls_of("read_from"), 0);
}

#[test]
fn if_present_turns_only_a_missing_path_into_none() {
    let missing: Result<(), DiskError> = Err(DiskError::new(
        DiskVerb::Read,
        Path::new("/srv/fixture-media/a"),
        io::ErrorKind::NotFound.into(),
    ));
    assert_eq!(missing.if_present().unwrap(), None);
    let denied: Result<(), DiskError> = Err(DiskError::new(
        DiskVerb::Read,
        Path::new("/srv/fixture-media/a"),
        io::ErrorKind::PermissionDenied.into(),
    ));
    assert!(denied.if_present().is_err());
    assert_eq!(Ok::<u8, DiskError>(7).if_present().unwrap(), Some(7));
}

mod system {
    //! The owner over the real filesystem, on synthetic files in a temporary
    //! directory.
    use super::*;

    fn real(root: &Path) -> Disk {
        let disk = Disk::with_seams(Arc::new(SystemBackend), Arc::new(SystemClock));
        disk.set_volumes([(root.to_path_buf(), VolumeClass::LibraryRoot)]);
        disk
    }

    #[test]
    fn write_read_move_and_remove_synthetic_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("library");
        let disk = real(&root);
        let nfo = root.join("Invented Show (2031)").join("tvshow.nfo");

        disk.write_atomic_blocking(&nfo, b"first", Replace::Never, Access::Default)
            .unwrap();
        assert_eq!(std::fs::read(&nfo).unwrap(), b"first");
        let refused = disk
            .write_atomic_blocking(&nfo, b"second", Replace::Never, Access::Default)
            .unwrap_err();
        assert_eq!(refused.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&nfo).unwrap(), b"first");
        disk.write_atomic_blocking(&nfo, b"third", Replace::Allowed, Access::Default)
            .unwrap();
        assert_eq!(std::fs::read(&nfo).unwrap(), b"third");
        assert!(matches!(
            disk.fresh_blocking(&nfo),
            PathReading::Known {
                kind: EntryKind::File,
                len: Some(5),
                ..
            }
        ));

        let moved = root.join("Invented Show (2031)").join("show.nfo");
        disk.move_exclusive_blocking(&nfo, &moved).unwrap();
        assert_eq!(disk.fresh_blocking(&nfo).exists(), Some(false));
        assert_eq!(disk.fresh_blocking(&moved).exists(), Some(true));

        let show = root.join("Invented Show (2031)");
        assert_eq!(
            disk.remove_blocking(&show, Remove::EmptyDir)
                .unwrap_err()
                .reason(),
            reason::DISK_DIRECTORY_NOT_EMPTY
        );
        let listed = disk.list_blocking(&show).unwrap();
        // The refused write removed the staging file it had created.
        assert_eq!(
            listed
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![moved.clone()]
        );
        assert_eq!(
            disk.remove_blocking(&show, Remove::Tree).unwrap(),
            Removal::Removed
        );
        assert_eq!(disk.fresh_blocking(&show).exists(), Some(false));
        assert_eq!(
            disk.remove_blocking(&show, Remove::Tree).unwrap(),
            Removal::AlreadyAbsent
        );
        assert!(root.exists(), "the root itself is untouched");
    }

    #[test]
    fn hard_links_share_an_identity() {
        let temp = tempfile::tempdir().expect("temp dir");
        let disk = real(temp.path());
        let source = temp.path().join("invented-a.mkv");
        let dest = temp.path().join("invented-b.mkv");
        std::fs::write(&source, b"payload").unwrap();
        if disk.link_blocking(&source, &dest, Link::Hard).unwrap() == LinkOutcome::Linked {
            let a = disk.fresh_blocking(&source);
            let b = disk.fresh_blocking(&dest);
            if let (
                PathReading::Known {
                    identity: Some(a), ..
                },
                PathReading::Known {
                    identity: Some(b), ..
                },
            ) = (a, b)
            {
                assert_eq!(a, b);
            }
        }
    }

    /// Every name in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_refused_exclusive_write_keeps_the_destination_and_removes_its_staging_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("library");
        let show = root.join("Invented Show (2031)");
        std::fs::create_dir_all(&show).unwrap();
        let nfo = show.join("tvshow.nfo");
        std::fs::write(&nfo, b"keep me").unwrap();
        std::fs::write(show.join("invented-sibling.txt"), b"sibling").unwrap();
        std::fs::write(show.join(".scryer-invented.partial"), b"not ours").unwrap();
        let disk = real(&root);

        let refused = disk
            .write_atomic_blocking(&nfo, b"new bytes", Replace::Never, Access::Default)
            .unwrap_err();
        assert_eq!(refused.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&nfo).unwrap(), b"keep me");
        assert_eq!(
            names(&show),
            vec![
                ".scryer-invented.partial",
                "invented-sibling.txt",
                "tvshow.nfo"
            ]
        );
        assert_eq!(
            std::fs::read(show.join(".scryer-invented.partial")).unwrap(),
            b"not ours"
        );
        assert_eq!(
            std::fs::read(show.join("invented-sibling.txt")).unwrap(),
            b"sibling"
        );
    }

    #[test]
    fn a_replacing_write_that_cannot_rename_removes_its_staging_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("library");
        let show = root.join("Invented Show (2031)");
        // A non-empty directory where the file should go: no rename of a
        // file can replace it.
        let blocker = show.join("tvshow.nfo");
        std::fs::create_dir_all(&blocker).unwrap();
        std::fs::write(blocker.join("invented-inner.txt"), b"inner").unwrap();
        std::fs::write(show.join("invented-sibling.txt"), b"sibling").unwrap();
        let disk = real(&root);

        disk.write_atomic_blocking(&blocker, b"new bytes", Replace::Allowed, Access::Default)
            .unwrap_err();
        assert_eq!(names(&show), vec!["invented-sibling.txt", "tvshow.nfo"]);
        assert_eq!(names(&blocker), vec!["invented-inner.txt"]);
        assert_eq!(
            std::fs::read(blocker.join("invented-inner.txt")).unwrap(),
            b"inner"
        );
    }

    #[test]
    fn open_modes_keep_create_or_refuse_as_named() {
        use std::io::{Read, Write};
        let temp = tempfile::tempdir().expect("temp dir");
        let disk = real(temp.path());
        let path = temp.path().join("invented.log");

        assert_eq!(
            disk.open_blocking(&path, Open::write_existing())
                .unwrap_err()
                .reason(),
            reason::DISK_PATH_MISSING
        );
        let mut file = disk.open_blocking(&path, Open::create_new()).unwrap();
        file.write_all(b"one").unwrap();
        disk.finish_blocking(&path, file).unwrap();
        assert_eq!(
            disk.open_blocking(&path, Open::create_new())
                .unwrap_err()
                .reason(),
            reason::DISK_DESTINATION_EXISTS
        );
        let mut file = disk.open_blocking(&path, Open::append()).unwrap();
        file.write_all(b"-two").unwrap();
        disk.finish_blocking(&path, file).unwrap();
        // Keeps what is there.
        let mut kept = String::new();
        disk.open_blocking(&path, Open::create_or_keep())
            .unwrap()
            .read_to_string(&mut kept)
            .unwrap();
        assert_eq!(kept, "one-two");
        let mut file = disk.open_blocking(&path, Open::write_existing()).unwrap();
        file.write_all(b"ONE").unwrap();
        disk.finish_blocking(&path, file).unwrap();
        assert_eq!(disk.read_to_string_blocking(&path).unwrap(), "ONE-two");
        let mut file = disk.open_blocking(&path, Open::create()).unwrap();
        file.write_all(b"3").unwrap();
        disk.finish_blocking(&path, file).unwrap();
        assert_eq!(disk.read_blocking(&path).unwrap(), b"3");
        assert!(matches!(
            disk.fresh_blocking(&path),
            PathReading::Known { len: Some(1), .. }
        ));

        let plain = temp.path().join("invented-plain.txt");
        disk.write_blocking(&plain, b"a", Replace::Never, Access::Default)
            .unwrap();
        assert_eq!(
            disk.write_blocking(&plain, b"b", Replace::Never, Access::Default)
                .unwrap_err()
                .reason(),
            reason::DISK_DESTINATION_EXISTS
        );
        disk.write_blocking(&plain, b"cc", Replace::Allowed, Access::Default)
            .unwrap();
        assert_eq!(disk.read_blocking(&plain).unwrap(), b"cc");
    }

    #[test]
    fn a_writing_handle_reads_back_only_when_asked() {
        use std::io::{Read, Seek, Write};
        let temp = tempfile::tempdir().expect("temp dir");
        let disk = real(temp.path());
        let read_back = |file: &mut std::fs::File| {
            let mut contents = Vec::new();
            file.rewind()
                .and_then(|_| file.read_to_end(&mut contents))
                .map(|_| contents)
        };

        let plain = temp.path().join("invented-plain.probe");
        let mut file = disk.open_blocking(&plain, Open::create_new()).unwrap();
        file.write_all(b"written").unwrap();
        assert!(read_back(&mut file).is_err());

        let checked = temp.path().join("invented-checked.probe");
        let mut file = disk
            .open_blocking(&checked, Open::create_new().with_read_back())
            .unwrap();
        file.write_all(b"written").unwrap();
        assert_eq!(read_back(&mut file).unwrap(), b"written");
        drop(file);
        // Still exclusive: an existing file is refused and kept.
        assert_eq!(
            disk.open_blocking(&checked, Open::create_new().with_read_back())
                .unwrap_err()
                .reason(),
            reason::DISK_DESTINATION_EXISTS
        );
        assert_eq!(disk.read_blocking(&checked).unwrap(), b"written");
    }

    #[test]
    fn a_handle_that_does_not_follow_links_refuses_them_and_non_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let disk = real(temp.path());
        let target = temp.path().join("invented-target.log");
        let link = temp.path().join("invented-link.log");
        std::fs::write(&target, b"target").unwrap();
        if disk.link_blocking(&target, &link, Link::Symbolic).unwrap() == LinkOutcome::Linked {
            for open in [Open::read(), Open::append(), Open::create_or_keep()] {
                let error = disk
                    .open_blocking(&link, open.without_following_links())
                    .unwrap_err();
                assert_eq!(error.reason(), reason::DISK_LINK_NOT_FOLLOWED, "{open:?}");
            }
            // Followed, the link reaches its target.
            assert_eq!(disk.read_blocking(&link).unwrap(), b"target");
        }
        let error = disk
            .open_blocking(temp.path(), Open::read().without_following_links())
            .unwrap_err();
        assert_eq!(error.reason(), reason::DISK_WRONG_KIND);
        assert_eq!(std::fs::read(&target).unwrap(), b"target");
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_access_applies_at_creation_and_on_request() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let temp = tempfile::tempdir().expect("temp dir");
        let disk = real(temp.path());

        let opened = temp.path().join("invented-secret.key");
        let file = disk
            .open_blocking(&opened, Open::create_new().with_access(Access::OwnerOnly))
            .unwrap();
        disk.finish_blocking(&opened, file).unwrap();
        assert_eq!(mode(&opened), 0o600);

        let written = temp.path().join("invented-written.key");
        disk.write_blocking(&written, b"k", Replace::Never, Access::OwnerOnly)
            .unwrap();
        assert_eq!(mode(&written), 0o600);

        let atomic = temp.path().join("invented-atomic.key");
        disk.write_atomic_blocking(&atomic, b"k", Replace::Never, Access::OwnerOnly)
            .unwrap();
        assert_eq!(mode(&atomic), 0o600);

        let staged_dest = temp.path().join("invented-staged.key");
        let staged = disk
            .stage_blocking(&staged_dest, Parents::MustExist, Access::OwnerOnly)
            .unwrap();
        staged.commit_blocking(Replace::Never).unwrap();
        assert_eq!(mode(&staged_dest), 0o600);

        let loose = temp.path().join("invented-loose.key");
        std::fs::write(&loose, b"k").unwrap();
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
        disk.restrict_to_owner_blocking(&loose, OwnerOnly::File)
            .unwrap();
        assert_eq!(mode(&loose), 0o600);
        let loose_dir = temp.path().join("invented-loose-dir");
        std::fs::create_dir(&loose_dir).unwrap();
        disk.restrict_to_owner_blocking(&loose_dir, OwnerOnly::Directory)
            .unwrap();
        assert_eq!(mode(&loose_dir), 0o700);
        assert_eq!(
            disk.restrict_to_owner_blocking(&temp.path().join("absent"), OwnerOnly::File)
                .unwrap_err()
                .reason(),
            reason::DISK_PATH_MISSING
        );
    }

    #[test]
    fn a_dropped_stage_and_a_scratch_dir_remove_only_what_they_made() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("library");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("invented-sibling.txt"), b"sibling").unwrap();
        let disk = real(&root);
        let dest = root.join("invented.nfo");

        let mut staged = disk
            .stage_blocking(&dest, Parents::MustExist, Access::Default)
            .unwrap();
        std::io::Write::write_all(&mut staged, b"draft").unwrap();
        assert_eq!(std::fs::read(staged.path()).unwrap(), b"draft");
        drop(staged);
        assert_eq!(names(&root), vec!["invented-sibling.txt"]);

        let mut staged = disk
            .stage_blocking(&dest, Parents::MustExist, Access::Default)
            .unwrap();
        std::io::Write::write_all(&mut staged, b"final").unwrap();
        staged.commit_blocking(Replace::Never).unwrap();
        assert_eq!(names(&root), vec!["invented-sibling.txt", "invented.nfo"]);
        assert_eq!(std::fs::read(&dest).unwrap(), b"final");

        let scratch = disk.scratch_dir_blocking(".scryer-invented-test-").unwrap();
        let path = scratch.path().to_path_buf();
        assert!(path.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        std::fs::write(path.join("invented-inner.bin"), b"inner").unwrap();
        drop(scratch);
        assert!(!path.exists());
        assert_eq!(names(&root), vec!["invented-sibling.txt", "invented.nfo"]);
    }

    #[test]
    fn modified_time_link_count_and_directory_sync_on_real_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let disk = real(temp.path());
        let path = temp.path().join("invented.nzb");
        std::fs::write(&path, b"nzb").unwrap();
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        disk.set_modified_blocking(&path, at).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), at);

        assert!(disk.sync_dir_blocking(temp.path()).is_ok());

        let second = temp.path().join("invented-second.nzb");
        if disk.link_blocking(&path, &second, Link::Hard).unwrap() == LinkOutcome::Linked {
            #[cfg(unix)]
            assert_eq!(disk.fresh_blocking(&path).link_count(), Some(2));
        }
    }
}
