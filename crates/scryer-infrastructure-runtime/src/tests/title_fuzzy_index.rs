//! The Tantivy index itself: freshness, invalidation and blocking recovery.
//!
//! Every other search test goes through a store. These go at the index
//! directly: reads require a complete projection, wait for rebuilds, and return
//! an error when the index cannot serve them.

use super::*;
use scryer_infrastructure_library::media::titles::fuzzy_source::DatastoreTitleTermSource;
use scryer_infrastructure_library_search::TitleFuzzyIndex;
use scryer_infrastructure_library_search::fuzzy::{ProjectionStamp, ResolverFuzzyQuery};

async fn anime_title(catalog: &TitleStore, id: &str, name: &str) -> Title {
    let mut title = make_test_title(id, None);
    title.name = name.to_string();
    title.facet = MediaFacet::Anime;
    title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    TitleRepository::create(catalog, title.clone())
        .await
        .expect("title should insert");
    title
}

async fn open_index(services: &SqliteServices, dir: &std::path::Path) -> Arc<TitleFuzzyIndex> {
    TitleFuzzyIndex::open(
        dir,
        Arc::new(DatastoreTitleTermSource::new(services.datastore())),
    )
    .await
    .expect("index must open ready")
}

/// The resolver's own query shape, spelled the way the port spells it.
struct ObservedName {
    match_term: String,
    script: String,
    numbers_key: String,
}

fn observed(name: &str) -> ObservedName {
    ObservedName {
        match_term: scryer_domain::title_spelling::title_match_form(name, name, None).0,
        script: scryer_domain::title_spelling::title_script(name)
            .as_str()
            .to_string(),
        numbers_key: scryer_domain::title_spelling::title_numbers_key(name),
    }
}

fn resolver_query<'a>(name: &'a ObservedName, distance: u8) -> ResolverFuzzyQuery<'a> {
    ResolverFuzzyQuery {
        match_term: &name.match_term,
        script: &name.script,
        numbers_key: &name.numbers_key,
        facet: Some("anime"),
        distance,
        limit: 64,
    }
}

#[tokio::test]
async fn a_queued_title_is_visible_to_the_next_fuzzy_read() {
    let (services, db) = temp_services("scryer_fuzzy_index_queue_drain").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);
    let index = open_index(&services, dir.path()).await;
    index
        .rebuild(index_stamp(&services).await)
        .await
        .expect("the initial rebuild must succeed");

    // Created after the rebuild, so only the queue can carry it.
    let aokumo = anime_title(&catalog, "fuzzy-aokumo", "Aokumo").await;
    index.sync().await.expect("sync must succeed");

    let candidates = index
        .resolver_candidates(resolver_query(&observed("Akumo"), 1))
        .await
        .expect("lookup must succeed");
    assert!(
        !candidates.is_empty(),
        "a one-edit misspelling must reach the title queued a moment ago"
    );
    let hydrated = title_ids_for_terms(&services, &candidates).await;
    assert!(hydrated.contains(&aokumo.id.to_string()), "{hydrated:?}");

    let _ = std::fs::remove_file(db);
}

/// Gram noise cannot crowd out a name the automaton can see, and a lane that
/// fills its limit says so instead of answering short.
///
/// Both halves use one fixture, because they are one failure: a bucket whose
/// gram-sharing rows outnumber the limit. Asked at the distance the anchor can
/// actually consume, the rival is found — it is the automaton lane's hit, and
/// that lane is searched on its own. Asked at the blanket distance the
/// resolver used to send, the gram floor collapses to a single shared trigram,
/// the lane fills up, and the answer is incomplete.
#[tokio::test]
async fn gram_noise_cannot_hide_a_rival_and_a_full_lane_is_an_error() {
    let (services, db) = temp_services("scryer_fuzzy_index_gram_noise").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);

    // Every one of these shares the query's trigrams and is many edits away
    // from it. There are twice as many of them as the limit below.
    for (ordinal, suffix) in [
        "Drift", "Crest", "Pylon", "Harbour", "Quarry", "Beacon", "Mantle", "Thicket",
    ]
    .into_iter()
    .enumerate()
    {
        anime_title(
            &catalog,
            &format!("gram-noise-{ordinal}"),
            &format!("Zolarium {suffix}"),
        )
        .await;
    }
    // One edit from the query and sharing none of its trigrams: an edit in the
    // middle of a five-character name destroys every trigram it has, so only
    // the automaton lane can find this.
    let rival = anime_title(&catalog, "gram-noise-rival", "Zomar").await;
    let index = open_index(&services, dir.path()).await;

    let name = observed("Zolar");
    let mut narrow = resolver_query(&name, 2);
    narrow.limit = 4;
    let hits = index
        .resolver_candidates(narrow)
        .await
        .expect("the bucket must answer");
    let hydrated = title_ids_for_terms(&services, &hits).await;
    assert!(
        hydrated.contains(&rival.id.to_string()),
        "a one-edit rival must survive a bucket full of gram noise: {hydrated:?}"
    );

    let mut saturating = resolver_query(&name, 4);
    saturating.limit = 4;
    let error = index
        .resolver_candidates(saturating)
        .await
        .expect_err("a lane that fills its limit must not answer short");
    let message = error.to_string();
    assert!(
        message.contains("incomplete") && message.contains("4"),
        "the error must name the bucket and the limit: {message}"
    );

    let _ = std::fs::remove_file(db);
}

/// The two lanes do not compete for one ranked result set.
///
/// Neither lane fills the limit here, but together they match more rows than
/// it: ranked as one query, the gram rows score on BM25 and the automaton's
/// score a constant, so the exact hits are the ones that fall off the end.
/// Searched separately and unioned, both survive.
#[tokio::test]
async fn gram_hits_and_automaton_hits_do_not_share_one_ranking() {
    let (services, db) = temp_services("scryer_fuzzy_index_lane_union").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);

    for (ordinal, suffix) in ["Drift", "Crest", "Pylon"].into_iter().enumerate() {
        anime_title(
            &catalog,
            &format!("lane-union-noise-{ordinal}"),
            &format!("Zolarium {suffix}"),
        )
        .await;
    }
    // Two edits away and sharing no trigram, so it is an automaton hit only.
    // Created first, so it holds the lower document address and wins a tie.
    anime_title(&catalog, "lane-union-decoy", "Zemar").await;
    let rival = anime_title(&catalog, "lane-union-rival", "Zomar").await;
    let index = open_index(&services, dir.path()).await;

    let name = observed("Zolar");
    // Distance three: past the automaton's ceiling, so the gram lane runs too.
    let mut query = resolver_query(&name, 3);
    query.limit = 4;
    let hits = index
        .resolver_candidates(query)
        .await
        .expect("neither lane fills the limit");
    let hydrated = title_ids_for_terms(&services, &hits).await;
    assert!(
        hydrated.contains(&rival.id.to_string()),
        "an exact hit must not be ranked away by gram hits: {hydrated:?}"
    );

    let _ = std::fs::remove_file(db);
}

/// A wide script gets the automaton too. Two three-character names one
/// character apart share no bigram, so the gram lane alone cannot see the
/// rival — and the automaton counts characters, not bytes, so it can.
#[tokio::test]
async fn a_wide_script_rival_one_character_away_is_found() {
    let (services, db) = temp_services("scryer_fuzzy_index_wide_script").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);
    let rival = anime_title(&catalog, "wide-script-rival", "蒼硯録").await;
    let index = open_index(&services, dir.path()).await;

    let name = observed("蒼曜録");
    let hits = index
        .resolver_candidates(resolver_query(&name, 1))
        .await
        .expect("the bucket must answer");
    let hydrated = title_ids_for_terms(&services, &hits).await;
    assert!(
        hydrated.contains(&rival.id.to_string()),
        "a one-character rival in a wide script must be visible: {hydrated:?}"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_deleted_title_leaves_the_index_on_the_next_sync() {
    let (services, db) = temp_services("scryer_fuzzy_index_delete").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);
    let index = open_index(&services, dir.path()).await;

    let aokumo = anime_title(&catalog, "fuzzy-aokumo-delete", "Aokumo").await;
    index
        .rebuild(index_stamp(&services).await)
        .await
        .expect("rebuild must succeed");
    assert!(
        !index
            .resolver_candidates(resolver_query(&observed("Akumo"), 1))
            .await
            .expect("lookup must succeed")
            .is_empty()
    );

    TitleRepository::delete(&catalog, aokumo.id.as_str())
        .await
        .expect("delete should succeed");
    index.sync().await.expect("sync must succeed");
    assert!(
        index
            .resolver_candidates(resolver_query(&observed("Akumo"), 1))
            .await
            .expect("lookup must succeed")
            .is_empty(),
        "a deleted title must not keep answering from the index"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_stamp_from_another_projection_marks_the_directory_for_rebuild() {
    let (services, db) = temp_services("scryer_fuzzy_index_stamp").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);
    anime_title(&catalog, "fuzzy-aokumo-stamp", "Aokumo").await;

    let index = open_index(&services, dir.path()).await;
    let current = index_stamp(&services).await;
    // A stamp from a different projection generation. The contents may be
    // perfectly good, but nothing on disk can prove they were built from the
    // projection as it now stands, so the directory must not be trusted.
    index
        .rebuild(ProjectionStamp {
            collation_version: "not-the-current-one".to_string(),
            projection_generation: -1,
        })
        .await
        .expect("rebuild must succeed");
    assert!(
        !index.stamp_matches(&current),
        "a foreign stamp must send the next open into a rebuild"
    );

    drop(index);
    let index = open_index(&services, dir.path()).await;
    assert!(index.stamp_matches(&current));
    assert!(
        !index
            .resolver_candidates(resolver_query(&observed("Akumo"), 1))
            .await
            .expect("lookup must succeed")
            .is_empty(),
        "the rebuilt index must answer again"
    );

    sqlx::query("UPDATE title_search_meta SET projection_generation = projection_generation + 1")
        .execute(services.pool())
        .await
        .unwrap();
    let restored = index_stamp(&services).await;
    assert!(!index.stamp_matches(&restored));
    assert!(
        !index
            .resolver_candidates(resolver_query(&observed("Akumo"), 1))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        index.stamp_matches(&restored),
        "a live projection change must rebuild before the read returns"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn an_unusable_directory_is_an_error() {
    let (services, db) = temp_services("scryer_fuzzy_index_unusable").await;
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("blocked");
    // A file where the index directory should be: opening it cannot succeed.
    std::fs::write(&blocked, b"not a directory").unwrap();

    assert!(
        TitleFuzzyIndex::open(
            &blocked,
            Arc::new(DatastoreTitleTermSource::new(services.datastore())),
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&blocked).unwrap(), b"not a directory");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn corrupt_missing_and_incompatible_indexes_rebuild_before_open_returns() {
    for damage in ["malformed", "missing", "schema", "segment"] {
        let dir = tempfile::tempdir().unwrap();
        let services = SqliteServices::new(dir.path().join("fixture.db").to_string_lossy())
            .await
            .unwrap();
        let catalog = title_store(&services);
        anime_title(&catalog, "recovery-title", "Aokumo").await;
        let index = open_index(&services, dir.path()).await;
        drop(index);
        let index_dir = dir.path().join("title-fuzzy-index");
        std::fs::write(index_dir.join("unrelated.bin"), b"preserve inside").unwrap();
        std::fs::write(dir.path().join("unrelated.bin"), b"preserve outside").unwrap();
        let mut damaged_file = "meta.json".to_string();
        if damage == "missing" {
            std::fs::rename(
                index_dir.join("meta.json"),
                index_dir.join("saved-meta.json"),
            )
            .unwrap();
        } else if damage == "schema" {
            let mut meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(index_dir.join("meta.json")).unwrap())
                    .unwrap();
            meta["schema"] = serde_json::json!([]);
            std::fs::write(
                index_dir.join("meta.json"),
                serde_json::to_vec(&meta).unwrap(),
            )
            .unwrap();
        } else if damage == "segment" {
            damaged_file = std::fs::read_dir(&index_dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .find(|name| name.ends_with(".term"))
                .expect("term segment");
            std::fs::write(index_dir.join(&damaged_file), b"corrupt fixture").unwrap();
        } else {
            std::fs::write(index_dir.join("meta.json"), b"corrupt fixture").unwrap();
        }
        let index = open_index(&services, dir.path()).await;
        let hits = index
            .resolver_candidates(resolver_query(&observed("Akumo"), 1))
            .await
            .unwrap();
        assert!(!hits.is_empty());
        let preserved = dir
            .path()
            .join("title-fuzzy-index.recovery-0/title-fuzzy-index");
        assert_eq!(
            std::fs::read(preserved.join("unrelated.bin")).unwrap(),
            b"preserve inside"
        );
        assert_eq!(
            std::fs::read(dir.path().join("unrelated.bin")).unwrap(),
            b"preserve outside"
        );
        if damage == "missing" {
            assert!(preserved.join("saved-meta.json").is_file());
        } else if damage == "schema" {
            let meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(preserved.join("meta.json")).unwrap())
                    .unwrap();
            assert_eq!(meta["schema"], serde_json::json!([]));
        } else {
            assert_eq!(
                std::fs::read(preserved.join(damaged_file)).unwrap(),
                b"corrupt fixture"
            );
        }
    }
}

#[tokio::test]
async fn an_owned_index_cannot_be_moved_by_another_open() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("fixture.db").to_string_lossy())
        .await
        .unwrap();
    let index = open_index(&services, dir.path()).await;
    let meta_path = dir.path().join("title-fuzzy-index/meta.json");
    let before = std::fs::read(&meta_path).unwrap();
    assert!(
        TitleFuzzyIndex::open(
            dir.path(),
            Arc::new(DatastoreTitleTermSource::new(services.datastore()))
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(meta_path).unwrap(), before);
    assert!(!dir.path().join("title-fuzzy-index.recovery-0").exists());
    drop(index);
    open_index(&services, dir.path()).await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_index_is_rejected_without_touching_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    std::fs::write(target.path().join("unrelated.bin"), b"preserve target").unwrap();
    std::os::unix::fs::symlink(target.path(), dir.path().join("title-fuzzy-index")).unwrap();
    let services = SqliteServices::new(dir.path().join("fixture.db").to_string_lossy())
        .await
        .unwrap();
    assert!(
        TitleFuzzyIndex::open(
            dir.path(),
            Arc::new(DatastoreTitleTermSource::new(services.datastore()))
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read(target.path().join("unrelated.bin")).unwrap(),
        b"preserve target"
    );
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 1);
}

struct GatedTermSource {
    inner: DatastoreTitleTermSource,
    pause: std::sync::atomic::AtomicBool,
    fail: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl scryer_infrastructure_library_search::fuzzy::TitleTermSource for GatedTermSource {
    async fn page_terms(
        &self,
        after: i64,
        limit: i64,
    ) -> AppResult<Vec<scryer_infrastructure_library_search::fuzzy::IndexedTerm>> {
        if self.pause.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.acquire().await.unwrap().forget();
        }
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::Repository(
                "fixture projection unavailable".into(),
            ));
        }
        self.inner.page_terms(after, limit).await
    }
    async fn terms_for_titles(
        &self,
        ids: &[String],
    ) -> AppResult<Vec<scryer_infrastructure_library_search::fuzzy::IndexedTerm>> {
        self.inner.terms_for_titles(ids).await
    }
    async fn queued_titles(
        &self,
        limit: i64,
    ) -> AppResult<Vec<scryer_infrastructure_library_search::fuzzy::QueuedTitle>> {
        self.inner.queued_titles(limit).await
    }
    async fn clear_queued(&self, seqs: &[i64]) -> AppResult<()> {
        self.inner.clear_queued(seqs).await
    }
    async fn projection_stamp(&self) -> AppResult<ProjectionStamp> {
        self.inner.projection_stamp().await
    }
}

#[tokio::test]
async fn reads_wait_for_rebuild_and_failed_rebuilds_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("fixture.db").to_string_lossy())
        .await
        .unwrap();
    anime_title(&title_store(&services), "blocked-title", "Aokumo").await;
    let source = Arc::new(GatedTermSource {
        inner: DatastoreTitleTermSource::new(services.datastore()),
        pause: false.into(),
        fail: false.into(),
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Semaphore::new(0),
    });
    let index = TitleFuzzyIndex::open(dir.path(), source.clone())
        .await
        .unwrap();
    source
        .pause
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let stamp = index_stamp(&services).await;
    let rebuilding = {
        let index = index.clone();
        tokio::spawn(async move { index.rebuild(stamp).await })
    };
    let bound = std::time::Duration::from_secs(30);
    tokio::time::timeout(bound, source.entered.notified())
        .await
        .unwrap();
    let name = observed("Akumo");
    let mut lookup = std::pin::pin!(index.resolver_candidates(resolver_query(&name, 1)));
    tokio::select! {
        biased;
        result = &mut lookup => panic!("lookup escaped an incomplete rebuild: {result:?}"),
        () = std::future::ready(()) => {}
    }
    source.resume.add_permits(1);
    tokio::time::timeout(bound, rebuilding)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        !tokio::time::timeout(bound, lookup)
            .await
            .unwrap()
            .unwrap()
            .is_empty()
    );
    source.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(index.rebuild(index_stamp(&services).await).await.is_err());
    assert!(
        index
            .resolver_candidates(resolver_query(&name, 1))
            .await
            .is_err()
    );
    source
        .fail
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(
        !index
            .resolver_candidates(resolver_query(&name, 1))
            .await
            .unwrap()
            .is_empty()
    );
}

/// A caller that goes away mid-write cannot break the next reader.
///
/// Dropping the awaiting future used to release the in-process writer lock
/// while the detached blocking task still held tantivy's own lock file, so the
/// next reader's writer could not be created at all and a match, import or
/// search failed on it. The guard now lives inside the blocking task, so the
/// next reader waits instead of failing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_write_does_not_break_the_next_read() {
    let (services, db) = temp_services("scryer_fuzzy_index_cancelled_write").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);
    // Enough rows that the rebuild's blocking write is real work rather than
    // an instant no-op, so the drop below lands while it is in flight.
    for ordinal in 0..50u8 {
        anime_title(
            &catalog,
            &format!("cancelled-write-{ordinal}"),
            &format!(
                "Zephran {}{}",
                (b'a' + ordinal / 26) as char,
                (b'a' + ordinal % 26) as char
            ),
        )
        .await;
    }
    let subject = anime_title(&catalog, "cancelled-write-subject", "Aokumo").await;

    let source = Arc::new(GatedTermSource {
        inner: DatastoreTitleTermSource::new(services.datastore()),
        pause: false.into(),
        fail: false.into(),
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Semaphore::new(0),
    });
    let index = TitleFuzzyIndex::open(dir.path(), source.clone())
        .await
        .unwrap();
    source
        .pause
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let stamp = index_stamp(&services).await;
    let bound = std::time::Duration::from_secs(30);

    {
        let mut rebuilding = std::pin::pin!(index.rebuild(stamp));
        let entered = source.entered.notified();
        let deadline = tokio::time::sleep(bound);
        tokio::pin!(entered, deadline);
        // Polls the rebuild until it parks on the gated projection read, which
        // is past the blocking delete step.
        tokio::select! {
            biased;
            result = &mut rebuilding => panic!("the rebuild must still be gated: {result:?}"),
            () = &mut entered => {}
            () = &mut deadline => panic!("the rebuild never reached the projection read"),
        }
        source.resume.add_permits(1);
        // Now drive it into the blocking write and abandon it there.
        for _ in 0..8 {
            tokio::select! {
                biased;
                result = &mut rebuilding => panic!("the rebuild must not have finished: {result:?}"),
                () = tokio::task::yield_now() => {}
            }
        }
    }

    let name = observed("Akumo");
    let hits = tokio::time::timeout(bound, index.resolver_candidates(resolver_query(&name, 1)))
        .await
        .expect("a cancelled write must not stall the next read forever")
        .expect("a cancelled write must not make the next read fail");
    let hydrated = title_ids_for_terms(&services, &hits).await;
    assert!(
        hydrated.contains(&subject.id.to_string()),
        "the read after a cancelled write must serve a complete index: {hydrated:?}"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_search_keeps_one_snapshot_without_blocking_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("fixture.db").to_string_lossy())
        .await
        .unwrap();
    let catalog = title_store(&services);
    anime_title(&catalog, "original-title", "Aokumo Lantern").await;
    let index = open_index(&services, dir.path()).await;
    let stamp = index_stamp(&services).await;
    let bound = std::time::Duration::from_secs(30);
    let (start, started) = tokio::sync::oneshot::channel();
    let (finished, finish) = std::sync::mpsc::channel();
    let rebuilding = {
        let index = index.clone();
        tokio::spawn(async move {
            tokio::time::timeout(bound, started).await.unwrap().unwrap();
            anime_title(&catalog, "new-title", "Aokumo Lantern").await;
            tokio::time::timeout(bound, index.rebuild(stamp))
                .await
                .unwrap()
                .unwrap();
            finished.send(()).unwrap();
        })
    };
    let start = std::cell::RefCell::new(Some(start));
    let tokens = vec!["aokumo".to_string(), "lantern".to_string()];
    let hits = tokio::time::timeout(
        bound,
        index.ui_candidates(
            &tokens,
            &["anime"],
            |_| {
                // The snapshot is pinned before the first token is evaluated.
                if let Some(start) = start.borrow_mut().take() {
                    start.send(()).unwrap();
                    finish
                        .recv_timeout(bound)
                        .expect("rebuild must not wait for the search");
                }
                0
            },
            64,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(bound, rebuilding)
        .await
        .unwrap()
        .unwrap();
    for token in &tokens {
        assert!(hits.iter().any(|hit| hit.token_key == *token));
    }
    assert!(hits.iter().all(|hit| hit.title_id == "original-title"));
    let current = index
        .ui_candidates(&tokens, &["anime"], |_| 0, 64)
        .await
        .unwrap();
    assert!(current.iter().any(|hit| hit.title_id == "new-title"));
}

/// The UI typo lane counts edits in characters, not UTF-8 bytes: one
/// substituted Han character (three bytes) is one edit, so a wide-script token
/// is not reduced to an exact-only lookup.
#[tokio::test]
async fn ui_typo_lane_finds_a_han_token_one_character_away() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("fixture.db").to_string_lossy())
        .await
        .unwrap();
    let catalog = title_store(&services);
    anime_title(&catalog, "han-title", "青雲灯籠").await;
    let index = open_index(&services, dir.path()).await;

    let tokens = vec!["青雨灯籠".to_string()];
    let hits = index
        .ui_candidates(
            &tokens,
            &["anime"],
            scryer_infrastructure_library_search::fuzzy_typo_distance,
            64,
        )
        .await
        .expect("ui lookup must succeed");
    assert!(
        hits.iter().any(|hit| hit.title_id == "han-title"),
        "one substituted Han character must stay within the typo distance: {hits:?}"
    );
}

async fn index_stamp(services: &SqliteServices) -> ProjectionStamp {
    use scryer_infrastructure_library_search::fuzzy::TitleTermSource;
    DatastoreTitleTermSource::new(services.datastore())
        .projection_stamp()
        .await
        .expect("the projection stamp should load")
}

async fn title_ids_for_terms(services: &SqliteServices, term_ids: &[i64]) -> Vec<String> {
    if term_ids.is_empty() {
        return Vec::new();
    }
    let placeholders = term_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
        "SELECT DISTINCT title_id FROM title_search_terms WHERE term_id IN ({placeholders})"
    )));
    for term_id in term_ids {
        query = query.bind(term_id);
    }
    query.fetch_all(services.pool()).await.unwrap()
}

/// What one release costs against a library-sized catalog.
///
/// Ignored by default: it builds 12,000 titles and runs 1,000 lookups, which
/// is minutes of work and a real file on disk, not something a normal test
/// run should pay for. Run it with
/// `cargo test -p scryer-infrastructure-runtime --release fuzzy_lookup_timing
/// -- --ignored --nocapture` when the lane's cost is the question.
///
/// It reports the median and the p99 rather than a mean, because the mean of
/// a lookup distribution with a rare pathological bucket says nothing useful
/// about either the normal case or the bad one.
#[tokio::test]
#[ignore = "timing measurement over a 12,000-title catalog"]
async fn fuzzy_lookup_timing_over_a_library_sized_catalog() {
    const TITLES: usize = 12_000;
    const RELEASES: usize = 1_000;

    let (services, db) = temp_services("scryer_fuzzy_timing").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);

    // A catalog with the shapes that actually collide: shared prefixes,
    // shared numbers, and a native-script tail that shares no Latin letters
    // with any of it.
    for index in 0..TITLES {
        let name = match index % 4 {
            0 => format!("Harbor Lights {}", index / 4),
            1 => format!("Harbour Lights Season {}", index / 4),
            2 => format!("Aokumo no Kiroku {}", index / 4),
            _ => format!("蒼雲の記録 {}", index / 4),
        };
        let mut title = make_test_title(&format!("timing-title-{index}"), None);
        title.name = name;
        title.facet = MediaFacet::Anime;
        title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
        TitleRepository::create(&catalog, title)
            .await
            .expect("title should insert");
    }

    let index = open_index(&services, dir.path()).await;
    let rebuild_start = std::time::Instant::now();
    index
        .rebuild(index_stamp(&services).await)
        .await
        .expect("rebuild must succeed");
    let rebuild = rebuild_start.elapsed();
    let catalog = catalog.with_fuzzy_index(index);

    let mut elapsed = Vec::with_capacity(RELEASES);
    for release in 0..RELEASES {
        // One edit away from a name that exists, which is the case the lane
        // is for: an exact hit would never reach it.
        let release_name = match release % 4 {
            0 => format!("Harbor Ligths {release}"),
            1 => format!("Harbour Lihgts Season {release}"),
            2 => format!("Aokumo no Kirouk {release}"),
            _ => format!("蒼雲の記緑 {release}"),
        };
        let name = observed(&release_name);
        let start = std::time::Instant::now();
        let candidates = TitleRepository::find_title_name_candidates(
            &catalog,
            scryer_application::TitleNameBucketQuery {
                facet: Some("anime"),
                script: &name.script,
                numbers_key: &name.numbers_key,
                typo_distance: Some(4),
                match_term: &name.match_term,
                romanization_key: None,
                collation_keys: &[],
                limit: 256,
            },
        )
        .await
        .expect("candidate lookup must succeed");
        elapsed.push(start.elapsed());
        std::hint::black_box(candidates);
    }

    elapsed.sort();
    let median = elapsed[elapsed.len() / 2];
    let p99 = elapsed[(elapsed.len() * 99) / 100];
    println!(
        "fuzzy lookup over {TITLES} titles: rebuild {rebuild:?}, \
         median {median:?}, p99 {p99:?} over {RELEASES} releases"
    );

    let _ = std::fs::remove_file(db);
}

/// Counts the freshness reads a fuzzy lookup pays, passing everything through.
struct CountingTermSource {
    inner: DatastoreTitleTermSource,
    stamps: std::sync::atomic::AtomicUsize,
    queue_polls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl scryer_infrastructure_library_search::fuzzy::TitleTermSource for CountingTermSource {
    async fn page_terms(
        &self,
        after: i64,
        limit: i64,
    ) -> AppResult<Vec<scryer_infrastructure_library_search::fuzzy::IndexedTerm>> {
        self.inner.page_terms(after, limit).await
    }
    async fn terms_for_titles(
        &self,
        ids: &[String],
    ) -> AppResult<Vec<scryer_infrastructure_library_search::fuzzy::IndexedTerm>> {
        self.inner.terms_for_titles(ids).await
    }
    async fn queued_titles(
        &self,
        limit: i64,
    ) -> AppResult<Vec<scryer_infrastructure_library_search::fuzzy::QueuedTitle>> {
        self.queue_polls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.queued_titles(limit).await
    }
    async fn clear_queued(&self, seqs: &[i64]) -> AppResult<()> {
        self.inner.clear_queued(seqs).await
    }
    async fn projection_stamp(&self) -> AppResult<ProjectionStamp> {
        self.stamps
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.projection_stamp().await
    }
}

/// A batch of buckets pays one freshness check, not one per bucket, and
/// answers each bucket exactly as a lone lookup would.
///
/// One release's anchors used to pay the stamp reads and queue poll per anchor
/// per facet, which was most of the index's statement count on an idle walk.
#[tokio::test]
async fn a_batched_lookup_checks_freshness_once_and_answers_like_single_lookups() {
    let (services, _db) = temp_services("scryer_fuzzy_index_batch").await;
    let dir = tempfile::tempdir().unwrap();
    let catalog = title_store(&services);
    anime_title(&catalog, "batch-aokumo", "Aokumo").await;
    anime_title(&catalog, "batch-velmora", "Velmora").await;
    anime_title(&catalog, "batch-quistral", "Quistral").await;
    let source = Arc::new(CountingTermSource {
        inner: DatastoreTitleTermSource::new(services.datastore()),
        stamps: 0.into(),
        queue_polls: 0.into(),
    });
    let index = TitleFuzzyIndex::open(dir.path(), source.clone())
        .await
        .expect("index must open ready");
    index
        .rebuild(index_stamp(&services).await)
        .await
        .expect("the initial rebuild must succeed");

    let names = ["Akumo", "Velmore", "Quistrel", "Zzyzxq"].map(observed);
    let queries = names
        .iter()
        .map(|name| resolver_query(name, 1))
        .collect::<Vec<_>>();

    let reads = |source: &CountingTermSource| {
        (
            source.stamps.load(std::sync::atomic::Ordering::SeqCst),
            source.queue_polls.load(std::sync::atomic::Ordering::SeqCst),
        )
    };
    let before = reads(&source);
    let batched = index
        .resolver_candidates_batch(&queries)
        .await
        .expect("the batch must answer");
    let after_batch = reads(&source);
    let one_check = (after_batch.0 - before.0, after_batch.1 - before.1);

    let mut singles = Vec::new();
    for query in &queries {
        singles.push(
            index
                .resolver_candidates(query.clone())
                .await
                .expect("a lone lookup must answer"),
        );
    }
    let after_singles = reads(&source);

    assert_eq!(batched, singles);
    assert!(
        batched[..3].iter().all(|hits| !hits.is_empty()),
        "each near spelling must find its title: {batched:?}"
    );
    assert!(batched[3].is_empty(), "{batched:?}");
    assert!(one_check.0 >= 1, "a batch must still check freshness");
    assert_eq!(
        (
            after_singles.0 - after_batch.0,
            after_singles.1 - after_batch.1
        ),
        (one_check.0 * queries.len(), one_check.1 * queries.len()),
        "lone lookups pay one check each; the batch paid one in total"
    );
}
