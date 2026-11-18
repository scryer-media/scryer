//! Full hashes: what answers an ask, what a read records, and what it never
//! records, on synthetic files and a fake table.

use std::sync::atomic::AtomicUsize;

use super::*;
use crate::backend::Extra;

/// A table the tests fill and inspect. A recorded hash is what the row then
/// holds.
#[derive(Default)]
struct FakeStore {
    rows: Mutex<HashMap<HashRow, StoredHash>>,
    asked: AtomicUsize,
    recorded: Mutex<Vec<(HashRow, KnownHash)>>,
}

impl FakeStore {
    fn holds(&self, row: &HashRow, stored: StoredHash) {
        self.rows.lock().unwrap().insert(row.clone(), stored);
    }
    fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn recorded(&self) -> Vec<(HashRow, KnownHash)> {
        self.recorded.lock().unwrap().clone()
    }
}

impl HashStore for FakeStore {
    fn stored<'a>(&'a self, row: &'a HashRow) -> StoreFuture<'a, Option<StoredHash>> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let stored = self.rows.lock().unwrap().get(row).cloned();
        Box::pin(async move { stored })
    }
    fn record<'a>(&'a self, row: &'a HashRow, hash: &'a KnownHash) -> StoreFuture<'a, ()> {
        self.holds(
            row,
            StoredHash {
                blake3: hash.blake3.clone(),
                crc: hash.crc,
                mark: Some(hash.mark.clone()),
            },
        );
        self.recorded
            .lock()
            .unwrap()
            .push((row.clone(), hash.clone()));
        Box::pin(async {})
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    disk: Disk,
    store: Arc<FakeStore>,
    path: PathBuf,
}

const BYTES: &[u8] = b"synthetic fixture media bytes";

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temp dir");
    let disk = Disk::with_seams(Arc::new(SystemBackend), Arc::new(SystemClock));
    disk.set_volumes([(temp.path().to_path_buf(), VolumeClass::LibraryRoot)]);
    disk.count_whole_reads();
    let store = Arc::new(FakeStore::default());
    disk.install_hash_store(store.clone());
    let path = temp.path().join("Fixture Show - 01.mkv");
    std::fs::write(&path, BYTES).unwrap();
    Fixture {
        _temp: temp,
        disk,
        store,
        path,
    }
}

impl Fixture {
    fn mark(&self) -> Mark {
        self.disk.fingerprint_followed_blocking(&self.path).unwrap()
    }
    fn reads(&self) -> usize {
        self.disk.whole_reads_of(&self.path)
    }
    fn row() -> HashRow {
        HashRow::Id("fixture-row".into())
    }
    fn of_row(&self) -> HashAsk {
        HashAsk::new(&self.path).of_row(Self::row())
    }
    fn rowless(&self) -> HashAsk {
        HashAsk::new(&self.path)
    }
    /// Gives the file `seconds` as its modification time.
    fn set_modified(&self, seconds: u64) {
        std::fs::File::options()
            .write(true)
            .open(&self.path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
            .unwrap();
    }
    fn stored(&self, blake3: &str, mark: Option<Mark>) -> StoredHash {
        StoredHash {
            blake3: blake3.into(),
            crc: None,
            mark,
        }
    }
}

fn expected_blake3() -> String {
    blake3::hash(BYTES).to_hex().to_string()
}

/// A fresh ask reads the file whatever is cached or stored of it, so a
/// rewrite that kept the length and the modification time is seen; and it
/// records what it read as any read does.
#[tokio::test]
async fn a_fresh_ask_reads_past_a_cached_and_a_stored_hash_that_still_fit() {
    let fixture = fixture();
    fixture.set_modified(1_700_000_000);
    let cached = fixture.disk.full_hash(fixture.rowless()).await;
    assert!(matches!(cached, FullHashAnswer::Read(_)), "{cached:?}");
    fixture.store.holds(
        &Fixture::row(),
        fixture.stored(&expected_blake3(), Some(fixture.mark())),
    );
    let rewritten = b"synthetic fixture media BYTES";
    assert_eq!(rewritten.len(), BYTES.len());
    std::fs::write(&fixture.path, rewritten).unwrap();
    fixture.set_modified(1_700_000_000);
    let new_blake3 = blake3::hash(rewritten).to_hex().to_string();

    for ask in [fixture.rowless(), fixture.of_row()] {
        let held = fixture.disk.full_hash(ask.clone()).await;
        assert_eq!(
            held.known().map(|known| known.blake3.clone()),
            Some(expected_blake3()),
            "without fresh, what still fits answers"
        );
        let answer = fixture.disk.full_hash(ask.fresh()).await;
        let FullHashAnswer::Read(read) = answer else {
            panic!("a fresh ask reads: {answer:?}");
        };
        assert_eq!(read.blake3, new_blake3);
    }
    assert_eq!(fixture.reads(), 3, "one read before, one per fresh ask");
    assert_eq!(
        fixture
            .disk
            .check(
                CheckSide::new(&fixture.path).fresh(),
                CheckSide::new(&fixture.path).of_row(Fixture::row()),
                Depth::Full,
            )
            .await,
        Check::Same,
        "the fresh read was recorded on the row"
    );
}

#[tokio::test]
async fn a_file_with_no_row_is_answered_from_the_cache_while_it_fits() {
    let fixture = fixture();

    let first = fixture.disk.full_hash(fixture.rowless()).await;
    let FullHashAnswer::Read(read) = first else {
        panic!("the first ask reads: {first:?}");
    };
    assert_eq!(read.blake3, expected_blake3());
    assert_eq!(read.size_bytes, BYTES.len() as u64);
    assert_eq!(fixture.reads(), 1);

    let second = fixture.disk.full_hash(fixture.rowless()).await;
    assert!(
        matches!(second, FullHashAnswer::Cached(ref known) if known.blake3 == expected_blake3())
    );
    assert_eq!(fixture.reads(), 1, "the second ask read nothing");
    assert_eq!(
        fixture.store.asked(),
        0,
        "a file with no row never asks the table"
    );
    assert!(fixture.store.recorded().is_empty());

    fixture.set_modified(1_000_000);
    assert!(matches!(
        fixture.disk.full_hash(fixture.rowless()).await,
        FullHashAnswer::Read(_)
    ));
    assert_eq!(
        fixture.reads(),
        2,
        "another modification time is read again"
    );
}

#[tokio::test]
async fn a_stored_hash_answers_while_the_file_fits_its_mark_and_is_read_again_once_it_does_not() {
    let fixture = fixture();
    fixture.store.holds(
        &Fixture::row(),
        fixture.stored("stored-hash", Some(fixture.mark())),
    );

    let answer = fixture.disk.full_hash(fixture.of_row()).await;
    assert!(matches!(answer, FullHashAnswer::Stored(ref known) if known.blake3 == "stored-hash"));
    assert_eq!(fixture.reads(), 0);
    assert!(
        fixture.store.recorded().is_empty(),
        "nothing read, nothing written"
    );

    fixture.set_modified(1_000_000);
    let answer = fixture.disk.full_hash(fixture.of_row()).await;
    assert!(matches!(answer, FullHashAnswer::Read(ref known) if known.blake3 == expected_blake3()));
    assert_eq!(fixture.reads(), 1);
    let recorded = fixture.store.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, Fixture::row());
    assert_eq!(recorded[0].1.blake3, expected_blake3());
    assert_eq!(recorded[0].1.mark.modified(), fixture.mark().modified());
    assert_eq!(
        fixture.disk.inner.cache.hashes.len(),
        0,
        "a hash with a row is kept on the row only"
    );
}

#[tokio::test]
async fn a_hash_stored_without_a_mark_is_read_once_and_recorded_with_one() {
    let fixture = fixture();
    fixture
        .store
        .holds(&Fixture::row(), fixture.stored("unmarked-hash", None));

    let answer = fixture.disk.full_hash(fixture.of_row()).await;
    assert!(matches!(answer, FullHashAnswer::Read(ref known) if known.blake3 == expected_blake3()));
    assert_eq!(fixture.reads(), 1);
    assert_eq!(fixture.store.recorded().len(), 1);

    let answer = fixture.disk.full_hash(fixture.of_row()).await;
    assert!(
        matches!(answer, FullHashAnswer::Stored(ref known) if known.blake3 == expected_blake3())
    );
    assert_eq!(fixture.reads(), 1, "the recorded mark answers the next ask");
}

#[tokio::test]
async fn asks_of_one_path_at_once_read_it_once() {
    let fixture = fixture();
    let (left, right) = tokio::join!(
        fixture.disk.full_hash(fixture.rowless()),
        fixture.disk.full_hash(fixture.rowless()),
    );
    let answers = [left, right];
    assert_eq!(
        answers
            .iter()
            .filter(|answer| matches!(answer, FullHashAnswer::Read(_)))
            .count(),
        1
    );
    assert!(
        answers
            .iter()
            .any(|answer| matches!(answer, FullHashAnswer::Cached(_)))
    );
    assert_eq!(fixture.reads(), 1);
    assert_eq!(
        fixture.disk.inner.hash_turns.paths_out(),
        0,
        "a path's turn is dropped with its last ask"
    );
}

#[tokio::test]
async fn a_file_that_changes_while_it_is_read_records_nothing() {
    for with_row in [false, true] {
        let fixture = fixture();
        let path = fixture.path.clone();
        let touched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let once = touched.clone();
        let progress = Progress::to(move |_, _| {
            if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                std::fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(3_000_000))
                    .unwrap();
            }
        });
        let ask = match with_row {
            true => fixture.of_row(),
            false => fixture.rowless(),
        }
        .progress(progress);

        let answer = fixture.disk.full_hash(ask).await;
        assert!(touched.load(std::sync::atomic::Ordering::SeqCst));
        assert!(matches!(answer, FullHashAnswer::Changed), "{answer:?}");
        assert!(fixture.store.recorded().is_empty());
        assert_eq!(fixture.disk.inner.cache.hashes.len(), 0);
    }
}

#[tokio::test]
async fn a_cancelled_ask_reads_nothing_and_records_nothing() {
    let fixture = fixture();
    let ask = fixture.of_row().cancel(Cancel::when(|| true));

    assert!(matches!(
        fixture.disk.full_hash(ask).await,
        FullHashAnswer::Cancelled
    ));
    assert!(fixture.store.recorded().is_empty());
    assert_eq!(fixture.disk.inner.cache.hashes.len(), 0);
}

#[tokio::test]
async fn a_change_through_the_owner_forgets_the_cached_hash() {
    let fixture = fixture();
    assert!(matches!(
        fixture.disk.full_hash(fixture.rowless()).await,
        FullHashAnswer::Read(_)
    ));
    assert_eq!(fixture.disk.inner.cache.hashes.len(), 1);

    fixture
        .disk
        .remove_blocking(&fixture.path, Remove::File)
        .unwrap();
    assert_eq!(fixture.disk.inner.cache.hashes.len(), 0);
    std::fs::write(&fixture.path, BYTES).unwrap();

    assert!(matches!(
        fixture.disk.full_hash(fixture.rowless()).await,
        FullHashAnswer::Read(_)
    ));
    assert_eq!(fixture.reads(), 2);
}

#[test]
fn a_read_that_began_before_an_eviction_is_not_kept() {
    let cache = crate::cache::StatCache::default();
    let path = Path::new("/srv/fixture-media/a.mkv");
    let known = KnownHash {
        size_bytes: 1,
        blake3: "read-before".into(),
        crc: None,
        mark: stat(1, 5, 1, 7, 100),
    };
    let epoch = cache.epoch();
    cache.evict(path);
    cache.record_hash(path, known.clone(), epoch);
    assert_eq!(cache.hashes.len(), 0);

    cache.record_hash(path, known, cache.epoch());
    assert_eq!(cache.hashes.len(), 1);
}

#[test]
fn the_cache_holds_a_bounded_number_of_hashes_and_drops_the_oldest_first() {
    let cache = crate::hashed::HashCache::default();
    let known = |index: usize| KnownHash {
        size_bytes: 1,
        blake3: index.to_string(),
        crc: None,
        mark: stat(1, 5, 1, index as u64 + 1, 100),
    };
    let path = |index: usize| PathBuf::from(format!("/srv/fixture-media/{index}.mkv"));
    for index in 0..crate::hashed::HASH_CACHE_MAX {
        cache.record_if(&path(index), known(index), || true);
    }
    // The first path recorded again is now the newest, not the oldest.
    cache.record_if(&path(0), known(0), || true);
    let past = crate::hashed::HASH_CACHE_MAX;
    cache.record_if(&path(past), known(past), || true);
    assert!(cache.len() <= crate::hashed::HASH_CACHE_MAX * 3 / 4 + 1);
    assert!(cache.len() >= crate::hashed::HASH_CACHE_MAX * 3 / 4);
    assert!(
        cache.kept(&path(0)).is_some(),
        "a path recorded again outlives older ones"
    );
    assert!(cache.kept(&path(1)).is_none());
}

fn stat(len: u64, modified: u64, device: u64, inode: u64, changed: i64) -> Mark {
    Mark::of_stat(&RawStat {
        kind: EntryKind::File,
        len,
        modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified)),
        identity: None,
        links: Some(1),
        extra: Extra {
            device: Some(device),
            inode: Some(inode),
            changed: Some((changed, 0)),
            ..Default::default()
        },
    })
}

#[test]
fn a_hash_fits_a_file_by_length_and_modification_time_alone() {
    let fits = crate::hashed::fits_for_test;
    let recorded = stat(10, 5, 1, 7, 100);

    assert!(fits(&recorded, &stat(10, 5, 2, 7, 100)), "another volume");
    assert!(
        fits(&recorded, &stat(10, 5, 1, 7, 900)),
        "another change time"
    );
    assert!(
        fits(&recorded, &stat(10, 5, 1, 8, 100)),
        "another file number"
    );
    assert!(!fits(&recorded, &stat(11, 5, 1, 7, 100)), "another length");
    assert!(!fits(&recorded, &stat(10, 6, 1, 7, 100)), "another time");

    let no_time = Mark::of_stat(&RawStat {
        kind: EntryKind::File,
        len: 10,
        modified: None,
        identity: None,
        links: Some(1),
        extra: Extra::default(),
    });
    assert!(
        !fits(&no_time, &no_time),
        "no modification time fits nothing"
    );
    let folder = Mark::of_stat(&RawStat {
        kind: EntryKind::Directory,
        len: 10,
        modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(5)),
        identity: None,
        links: Some(1),
        extra: Extra::default(),
    });
    assert!(!fits(&recorded, &folder), "a folder is no file");
    // A stored mark reads back to the same judgement.
    let stored = Mark::from_stored(&recorded.to_stored()).unwrap();
    assert!(fits(&stored, &stat(10, 5, 3, 9, 4)));
}

// ── Content checks ───────────────────────────────────────────────────────────

impl Fixture {
    /// A second file beside the fixture's, holding `bytes`.
    fn beside(&self, bytes: &[u8]) -> PathBuf {
        let other = self.path.with_file_name("Fixture Show - 01 copy.mkv");
        std::fs::write(&other, bytes).unwrap();
        other
    }
}

#[tokio::test]
async fn a_quick_check_samples_both_files_live_and_reads_neither_whole() {
    let fixture = fixture();
    let same = fixture.beside(BYTES);
    assert_eq!(
        fixture
            .disk
            .check(fixture.path.as_path(), same.as_path(), Depth::Quick)
            .await,
        Check::Same
    );
    let mut differing = BYTES.to_vec();
    differing[0] ^= 1;
    let other = fixture.beside(&differing);
    assert_eq!(
        fixture
            .disk
            .check(fixture.path.as_path(), other.as_path(), Depth::Quick)
            .await,
        Check::Differs
    );
    assert_eq!(fixture.reads(), 0);
    assert_eq!(fixture.disk.whole_reads_of(&other), 0);
    assert!(
        fixture.store.recorded().is_empty(),
        "a quick check stores nothing"
    );
    let missing = fixture.path.with_file_name("Fixture Show - 02.mkv");
    assert!(matches!(
        fixture
            .disk
            .check(fixture.path.as_path(), missing.as_path(), Depth::Quick)
            .await,
        Check::Unread(_)
    ));
}

#[tokio::test]
async fn a_full_check_compares_full_hashes_and_answers_a_row_from_its_fitting_record() {
    let fixture = fixture();
    let copy = fixture.beside(BYTES);
    let catalog = CheckSide::new(&fixture.path).of_row(Fixture::row());

    assert_eq!(
        fixture
            .disk
            .check(catalog.clone(), copy.as_path(), Depth::Full)
            .await,
        Check::Same
    );
    assert_eq!(fixture.reads(), 1);
    assert_eq!(fixture.disk.whole_reads_of(&copy), 1);
    assert_eq!(
        fixture.store.recorded().len(),
        1,
        "the row's read is recorded"
    );

    // The row's record fits the file, so the catalog file is not read again;
    // a rewrite of the copy that keeps its length is read and told apart.
    let mut differing = BYTES.to_vec();
    differing[3] ^= 1;
    std::fs::write(&copy, &differing).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&copy)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000))
        .unwrap();
    assert_eq!(
        fixture
            .disk
            .check(catalog, copy.as_path(), Depth::Full)
            .await,
        Check::Differs
    );
    assert_eq!(fixture.reads(), 1, "the fitting record answered");
    assert_eq!(fixture.disk.whole_reads_of(&copy), 2);
}

#[tokio::test]
async fn one_catalog_file_is_held_to_its_recorded_hash_and_with_none_the_read_is_recorded() {
    let fixture = fixture();
    let catalog = || CheckSide::new(&fixture.path).of_row(Fixture::row());

    assert_eq!(
        fixture.disk.check_against(catalog(), None).await,
        Check::Same,
        "nothing recorded contradicts the file"
    );
    assert_eq!(fixture.reads(), 1);
    let recorded = fixture.store.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1.blake3, expected_blake3());

    assert_eq!(
        fixture
            .disk
            .check_against(catalog(), Some(&expected_blake3()))
            .await,
        Check::Same
    );
    assert_eq!(
        fixture
            .disk
            .check_against(catalog(), Some("another-files-hash"))
            .await,
        Check::Differs
    );
    assert_eq!(
        fixture.reads(),
        1,
        "the record fits, so nothing is read again"
    );
    let missing = fixture.path.with_file_name("Fixture Show - 02.mkv");
    assert!(matches!(
        fixture
            .disk
            .check_against(missing.as_path(), Some(&expected_blake3()))
            .await,
        Check::Unread(_)
    ));
}

#[test]
fn a_blocking_check_tells_apart_a_rewrite_that_keeps_the_length() {
    let fixture = fixture();
    let copy = fixture.beside(BYTES);
    for depth in [Depth::Quick, Depth::Full] {
        assert_eq!(
            fixture.disk.check_blocking(&fixture.path, &copy, depth),
            Check::Same
        );
    }
    let mut differing = BYTES.to_vec();
    differing[BYTES.len() - 1] ^= 1;
    std::fs::write(&copy, &differing).unwrap();
    for depth in [Depth::Quick, Depth::Full] {
        assert_eq!(
            fixture.disk.check_blocking(&fixture.path, &copy, depth),
            Check::Differs
        );
    }
}
