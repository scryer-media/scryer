//! Full hashes: what answers an ask, what a read records, and what it never
//! records, on synthetic files and a fake table.

use std::sync::atomic::AtomicUsize;

use super::*;
use crate::backend::Extra;

/// A table the tests fill and inspect.
#[derive(Default)]
struct FakeStore {
    rows: Mutex<HashMap<HashRow, KnownHash>>,
    asked: AtomicUsize,
    recorded: Mutex<Vec<(HashRow, KnownHash)>>,
}

impl FakeStore {
    fn holds(&self, row: &HashRow, known: KnownHash) {
        self.rows.lock().unwrap().insert(row.clone(), known);
    }
    fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn recorded(&self) -> Vec<(HashRow, KnownHash)> {
        self.recorded.lock().unwrap().clone()
    }
}

impl HashStore for FakeStore {
    fn stored<'a>(&'a self, row: &'a HashRow) -> StoreFuture<'a, Option<KnownHash>> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let stored = self.rows.lock().unwrap().get(row).cloned();
        Box::pin(async move { stored })
    }
    fn record<'a>(&'a self, row: &'a HashRow, hash: &'a KnownHash) -> StoreFuture<'a, ()> {
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
    fn reuse(&self) -> HashAsk {
        HashAsk::new(&self.path, Some(self.mark()), Fresh::MayReuse).of_row(Self::row())
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
}

fn expected_blake3() -> String {
    blake3::hash(BYTES).to_hex().to_string()
}

#[tokio::test]
async fn a_second_ask_is_answered_from_the_cache_without_a_read() {
    let fixture = fixture();

    let first = fixture.disk.full_hash(fixture.reuse()).await;
    let FullHashAnswer::Read(read) = first else {
        panic!("the first ask reads: {first:?}");
    };
    assert_eq!(read.blake3, expected_blake3());
    assert_eq!(read.size_bytes, BYTES.len() as u64);
    assert_eq!(fixture.reads(), 1);

    let second = fixture.disk.full_hash(fixture.reuse()).await;
    assert!(
        matches!(second, FullHashAnswer::Cached(ref known) if known.blake3 == expected_blake3())
    );
    assert_eq!(fixture.reads(), 1, "the second ask read nothing");
    assert_eq!(
        fixture.store.asked(),
        1,
        "the cache answered before the table"
    );
    assert_eq!(
        fixture.store.recorded().len(),
        1,
        "only the read was recorded"
    );
}

#[tokio::test]
async fn a_stored_hash_answers_while_the_mark_holds_and_is_read_again_once_it_does_not() {
    let fixture = fixture();
    let stored = KnownHash {
        size_bytes: BYTES.len() as u64,
        blake3: "stored-hash".into(),
        crc: None,
        mark: fixture.mark(),
    };
    fixture.store.holds(&Fixture::row(), stored);

    let answer = fixture.disk.full_hash(fixture.reuse()).await;
    assert!(matches!(answer, FullHashAnswer::Stored(ref known) if known.blake3 == "stored-hash"));
    assert_eq!(fixture.reads(), 0);
    assert!(
        fixture.store.recorded().is_empty(),
        "nothing read, nothing written"
    );

    fixture.set_modified(1_000_000);
    let answer = fixture.disk.full_hash(fixture.reuse()).await;
    assert!(matches!(answer, FullHashAnswer::Read(ref known) if known.blake3 == expected_blake3()));
    assert_eq!(fixture.reads(), 1);
    let recorded = fixture.store.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, Fixture::row());
    assert_eq!(recorded[0].1.blake3, expected_blake3());
}

#[tokio::test]
async fn a_hash_stored_without_a_mark_is_judged_by_length_and_modification_time() {
    let fixture = fixture();
    fixture.set_modified(2_000_000);
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000);
    let legacy = |len| KnownHash {
        size_bytes: len,
        blake3: "legacy-hash".into(),
        crc: None,
        mark: Mark::of_length_and_modified(len, modified),
    };

    fixture
        .store
        .holds(&Fixture::row(), legacy(BYTES.len() as u64));
    let answer = fixture.disk.full_hash(fixture.reuse()).await;
    assert!(matches!(answer, FullHashAnswer::Stored(ref known) if known.blake3 == "legacy-hash"));
    assert_eq!(fixture.reads(), 0);

    // Another length, same time: not this file's hash.
    let fixture = self::fixture();
    fixture.set_modified(2_000_000);
    fixture
        .store
        .holds(&Fixture::row(), legacy(BYTES.len() as u64 + 1));
    let answer = fixture.disk.full_hash(fixture.reuse()).await;
    assert!(matches!(answer, FullHashAnswer::Read(_)));
    assert_eq!(fixture.reads(), 1);
}

#[tokio::test]
async fn a_must_read_ask_always_reads_and_records() {
    let fixture = fixture();
    fixture.store.holds(
        &Fixture::row(),
        KnownHash {
            size_bytes: BYTES.len() as u64,
            blake3: "stored-hash".into(),
            crc: None,
            mark: fixture.mark(),
        },
    );
    let must = || {
        HashAsk::new(&fixture.path, Some(fixture.mark()), Fresh::MustRead).of_row(Fixture::row())
    };

    assert!(matches!(
        fixture.disk.full_hash(must()).await,
        FullHashAnswer::Read(_)
    ));
    assert!(matches!(
        fixture.disk.full_hash(must()).await,
        FullHashAnswer::Read(_)
    ));
    assert_eq!(
        fixture.reads(),
        2,
        "never answered from the cache or the table"
    );
    assert_eq!(fixture.store.asked(), 0);
    assert_eq!(fixture.store.recorded().len(), 2);
}

#[tokio::test]
async fn an_ask_without_a_mark_reads() {
    let fixture = fixture();
    let unmarked = || HashAsk::new(&fixture.path, None, Fresh::MayReuse).of_row(Fixture::row());

    assert!(matches!(
        fixture.disk.full_hash(unmarked()).await,
        FullHashAnswer::Read(_)
    ));
    assert!(matches!(
        fixture.disk.full_hash(unmarked()).await,
        FullHashAnswer::Read(_)
    ));
    assert_eq!(fixture.reads(), 2);
    assert_eq!(fixture.store.asked(), 0);
}

#[tokio::test]
async fn a_file_that_changes_while_it_is_read_records_nothing() {
    for marked in [false, true] {
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
        let mark = marked.then(|| fixture.mark());
        let ask = HashAsk::new(&fixture.path, mark, Fresh::MustRead)
            .of_row(Fixture::row())
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
    let ask = HashAsk::new(&fixture.path, None, Fresh::MustRead)
        .of_row(Fixture::row())
        .cancel(Cancel::when(|| true));

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
        fixture.disk.full_hash(fixture.reuse()).await,
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
        fixture.disk.full_hash(fixture.reuse()).await,
        FullHashAnswer::Read(_)
    ));
    assert_eq!(fixture.reads(), 2);
}

#[tokio::test]
async fn the_blocking_ask_answers_from_the_cache_and_never_touches_the_table() {
    let fixture = fixture();
    let ask = fixture.reuse();

    assert!(matches!(
        fixture.disk.full_hash_blocking(&ask),
        FullHashAnswer::Read(_)
    ));
    assert!(matches!(
        fixture.disk.full_hash_blocking(&ask),
        FullHashAnswer::Cached(_)
    ));
    assert_eq!(fixture.reads(), 1);
    assert_eq!(fixture.store.asked(), 0);
    assert!(fixture.store.recorded().is_empty());
}

#[test]
fn the_cache_holds_a_bounded_number_of_hashes() {
    let cache = crate::hashed::HashCache::default();
    let mark = Mark::of_length_and_modified(1, std::time::UNIX_EPOCH);
    for index in 0..=crate::hashed::HASH_CACHE_MAX {
        cache.record(
            Path::new(&format!("/srv/fixture-media/{index}.mkv")),
            KnownHash {
                size_bytes: 1,
                blake3: index.to_string(),
                crc: None,
                mark: mark.clone(),
            },
        );
    }
    assert!(cache.len() <= crate::hashed::HASH_CACHE_MAX * 3 / 4 + 1);
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
fn a_hash_describes_a_file_by_length_time_and_number_never_by_volume_or_change_time() {
    let recorded = stat(10, 5, 1, 7, 100);

    assert!(
        stat(10, 5, 2, 7, 100).content_unchanged_since(&recorded),
        "a remount"
    );
    assert!(
        stat(10, 5, 1, 7, 900).content_unchanged_since(&recorded),
        "a rename"
    );
    assert!(
        !stat(10, 5, 1, 8, 100).content_unchanged_since(&recorded),
        "another file"
    );
    assert!(
        !stat(11, 5, 1, 7, 100).content_unchanged_since(&recorded),
        "another length"
    );
    assert!(
        !stat(10, 6, 1, 7, 100).content_unchanged_since(&recorded),
        "another time"
    );

    // A zero number tells nothing, so length and time decide.
    assert!(stat(10, 5, 1, 0, 100).content_unchanged_since(&stat(10, 5, 1, 0, 1)));
    let legacy = Mark::of_length_and_modified(
        10,
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(5),
    );
    assert!(stat(10, 5, 1, 7, 100).content_unchanged_since(&legacy));
    assert!(!stat(10, 6, 1, 7, 100).content_unchanged_since(&legacy));
    // A stored mark reads back to the same judgement.
    let stored = Mark::from_stored(&recorded.to_stored()).unwrap();
    assert!(stat(10, 5, 3, 7, 4).content_unchanged_since(&stored));
}
