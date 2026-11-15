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
    }
}

fn dir() -> RawStat {
    RawStat {
        kind: EntryKind::Directory,
        len: 0,
        modified: None,
        identity: None,
    }
}

/// Answers stat calls from a script (a path with no script is not found),
/// lists from a script, and lets every mutation succeed unless a failure is
/// scripted for that operation and path, or for that operation on any path.
/// `write_body` scripts a write that fails after its file was created.
/// Records every call. A gate, when set, holds every stat call (or only those
/// under `gate_under`) until the test opens it.
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
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        self.record("create_dir", path)
    }
    fn write_new(&self, path: &Path, _bytes: &[u8]) -> Result<SyncOutcome, WriteFailure> {
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
        disk.write_atomic_blocking(&p("/srv/fixture-media/Show/show.nfo"), b"x", Replace::Never)
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
            reason::DISK_OPERATION_FAILED,
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
                .write_atomic_blocking(&p(destination), b"x", replace)
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

        disk.write_atomic_blocking(&nfo, b"first", Replace::Never)
            .unwrap();
        assert_eq!(std::fs::read(&nfo).unwrap(), b"first");
        let refused = disk
            .write_atomic_blocking(&nfo, b"second", Replace::Never)
            .unwrap_err();
        assert_eq!(refused.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&nfo).unwrap(), b"first");
        disk.write_atomic_blocking(&nfo, b"third", Replace::Allowed)
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
            .write_atomic_blocking(&nfo, b"new bytes", Replace::Never)
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

        disk.write_atomic_blocking(&blocker, b"new bytes", Replace::Allowed)
            .unwrap_err();
        assert_eq!(names(&show), vec!["invented-sibling.txt", "tvshow.nfo"]);
        assert_eq!(names(&blocker), vec!["invented-inner.txt"]);
        assert_eq!(
            std::fs::read(blocker.join("invented-inner.txt")).unwrap(),
            b"inner"
        );
    }
}
