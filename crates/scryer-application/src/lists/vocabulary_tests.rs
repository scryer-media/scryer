use super::*;
use crate::lists::test_support::MemoryListStore;
use crate::{
    BulkMetadataResult, MetadataSearchItem, MetadataSearchQuery, MovieMetadata,
    MultiMetadataSearchResult, RichMetadataSearchItem,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};

struct Gateway {
    calls: StdMutex<Vec<Option<String>>>,
    reply: StdMutex<VocabularyReply>,
    fail: AtomicBool,
}
impl Gateway {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: StdMutex::new(vec![]),
            reply: StdMutex::new(reply()),
            fail: AtomicBool::new(false),
        })
    }
}
fn reply() -> VocabularyReply {
    VocabularyReply {
        version: "fixture-v1".into(),
        unchanged: false,
        entries: vec![VocabularyEntry {
            key: "canonical:genre:action".into(),
            category: "genre".into(),
            name: "Action".into(),
            aliases: vec!["action-adventure".into()],
        }],
    }
}
#[async_trait]
impl MetadataGateway for Gateway {
    async fn canonical_tag_vocabulary(&self, known: Option<&str>) -> AppResult<VocabularyReply> {
        self.calls.lock().unwrap().push(known.map(str::to_owned));
        if self.fail.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        Ok(self.reply.lock().unwrap().clone())
    }
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        unimplemented!("list resolver fixture only resolves titles")
    }
}

#[tokio::test]
async fn cold_concurrent_warm_and_restart_have_bounded_requests() {
    let runtime = Arc::new(VocabularyRuntime::default());
    let store = Arc::new(MemoryListStore::default());
    let gateway = Gateway::new();
    assert!(
        gateway.calls.lock().unwrap().is_empty(),
        "construction does no work"
    );
    let (a, b, c) = tokio::join!(
        runtime.get(store.clone(), gateway.clone(), false),
        runtime.get(store.clone(), gateway.clone(), false),
        runtime.get(store.clone(), gateway.clone(), false)
    );
    assert_eq!(a.unwrap().version, b.unwrap().version);
    c.unwrap();
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
    assert_eq!(store.vocabulary_reads.load(Ordering::SeqCst), 1);
    assert_eq!(*store.vocabulary_writes.lock().unwrap(), vec![false]);
    for _ in 0..10 {
        runtime
            .get(store.clone(), gateway.clone(), false)
            .await
            .unwrap();
    }
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
    assert_eq!(store.vocabulary_reads.load(Ordering::SeqCst), 1);
    assert_eq!(store.vocabulary_writes.lock().unwrap().len(), 1);
    let restart = Arc::new(VocabularyRuntime::default());
    let restored = restart
        .get(store.clone(), gateway.clone(), false)
        .await
        .unwrap();
    assert_eq!(
        restored.jitter_seconds,
        store
            .vocabulary
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .jitter_seconds
    );
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
    assert_eq!(store.vocabulary_reads.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn stale_demand_unchanged_and_failure_cooldown_preserve_snapshot() {
    let runtime = Arc::new(VocabularyRuntime::default());
    let store = Arc::new(MemoryListStore::default());
    let gateway = Gateway::new();
    let first = runtime
        .get(store.clone(), gateway.clone(), false)
        .await
        .unwrap();
    *gateway.reply.lock().unwrap() = VocabularyReply {
        version: first.version.clone(),
        unchanged: true,
        entries: vec![],
    };
    runtime
        .get(store.clone(), gateway.clone(), true)
        .await
        .unwrap();
    assert_eq!(*store.vocabulary_writes.lock().unwrap(), vec![false, true]);
    assert_eq!(
        gateway.calls.lock().unwrap()[1],
        Some(first.version.clone())
    );
    runtime.snapshot.write().await.as_mut().unwrap().checked_at =
        Utc::now() - chrono::Duration::hours(31);
    gateway.fail.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(3600)).await;
    assert_eq!(
        gateway.calls.lock().unwrap().len(),
        2,
        "idle time causes no refresh"
    );
    // Warm stale returns the retained snapshot, even with the gateway failing.
    assert_eq!(
        runtime
            .get(store.clone(), gateway.clone(), false)
            .await
            .unwrap()
            .version,
        first.version
    );
    {
        let _finished = tokio::time::timeout(Duration::from_secs(30), runtime.refresh.lock())
            .await
            .expect("refresh completed");
    }
    assert_eq!(gateway.calls.lock().unwrap().len(), 3);
    runtime
        .get(store.clone(), gateway.clone(), false)
        .await
        .unwrap();
    assert_eq!(gateway.calls.lock().unwrap().len(), 3, "retry suppressed");
    assert!(
        runtime
            .get(store.clone(), gateway.clone(), true)
            .await
            .is_err()
    );
    assert_eq!(
        gateway.calls.lock().unwrap().len(),
        4,
        "explicit retry bypasses cooldown"
    );
    tokio::time::advance(Duration::from_secs(901)).await;
    runtime
        .get(store.clone(), gateway.clone(), false)
        .await
        .unwrap();
    {
        let _finished = tokio::time::timeout(Duration::from_secs(30), runtime.refresh.lock())
            .await
            .expect("refresh completed");
    }
    assert_eq!(gateway.calls.lock().unwrap().len(), 5);
    assert_eq!(*store.vocabulary_writes.lock().unwrap(), vec![false, true]);
}

#[tokio::test]
async fn malformed_refresh_never_replaces_last_good_snapshot() {
    let runtime = Arc::new(VocabularyRuntime::default());
    let store = Arc::new(MemoryListStore::default());
    let gateway = Gateway::new();
    runtime
        .get(store.clone(), gateway.clone(), false)
        .await
        .unwrap();
    let mut bad = reply();
    bad.version = "v2".into();
    bad.entries.push(bad.entries[0].clone());
    *gateway.reply.lock().unwrap() = bad;
    assert!(
        runtime
            .get(store.clone(), gateway.clone(), true)
            .await
            .is_err()
    );
    assert_eq!(
        store.vocabulary.lock().unwrap().as_ref().unwrap().version,
        "fixture-v1"
    );
    assert_eq!(
        runtime.snapshot.read().await.as_ref().unwrap().version,
        "fixture-v1"
    );
    assert_eq!(store.vocabulary_writes.lock().unwrap().len(), 1);
}

#[test]
fn validation_rejects_duplicates_categories_and_oversize() {
    let mut bad = reply();
    bad.entries.push(bad.entries[0].clone());
    assert!(validate_vocabulary(&bad).is_err());
    let mut bad = reply();
    bad.entries[0].category = "person".into();
    assert!(validate_vocabulary(&bad).is_err());
    let mut bad = reply();
    bad.entries[0].aliases = vec!["x".repeat(257)];
    assert!(validate_vocabulary(&bad).is_err());
    let mut bad = reply();
    bad.entries = vec![bad.entries[0].clone(); 10_001];
    assert!(validate_vocabulary(&bad).is_err());
    let mut bad = reply();
    bad.entries = (0..100)
        .map(|i| VocabularyEntry {
            key: format!("canonical:genre:fixture-{i}"),
            category: "genre".into(),
            name: "Fixture".into(),
            aliases: vec!["x".repeat(256); 256],
        })
        .collect();
    assert!(
        validate_vocabulary(&bad).is_err(),
        "bounded total payload, even with individually valid fields"
    );
}

#[tokio::test]
async fn cold_failure_is_deduplicated_and_explicit_retry_recovers() {
    let runtime = Arc::new(VocabularyRuntime::default());
    let store = Arc::new(MemoryListStore::default());
    let gateway = Gateway::new();
    gateway.fail.store(true, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        runtime.get(store.clone(), gateway.clone(), false),
        runtime.get(store.clone(), gateway.clone(), false)
    );
    assert!(a.is_err() && b.is_err());
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
    assert!(
        runtime
            .get(store.clone(), gateway.clone(), false)
            .await
            .is_err()
    );
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
    gateway.fail.store(false, Ordering::SeqCst);
    runtime
        .get(store.clone(), gateway.clone(), true)
        .await
        .unwrap();
    assert_eq!(gateway.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn failed_atomic_write_retains_memory_and_persisted_snapshot() {
    let runtime = Arc::new(VocabularyRuntime::default());
    let store = Arc::new(MemoryListStore::default());
    let gateway = Gateway::new();
    let first = runtime
        .get(store.clone(), gateway.clone(), false)
        .await
        .unwrap();
    gateway.reply.lock().unwrap().version = "fixture-v2".into();
    store.fail_vocabulary_write.store(true, Ordering::SeqCst);
    assert!(
        runtime
            .get(store.clone(), gateway.clone(), true)
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .get(store.clone(), gateway.clone(), false)
            .await
            .unwrap()
            .version,
        first.version
    );
    assert_eq!(
        store.vocabulary.lock().unwrap().as_ref().unwrap().version,
        first.version
    );
    store.fail_vocabulary_write.store(false, Ordering::SeqCst);
    assert_eq!(
        runtime
            .get(store.clone(), gateway.clone(), true)
            .await
            .unwrap()
            .version,
        "fixture-v2"
    );
}

#[test]
fn alpha_conversion_uses_only_unambiguous_registered_aliases() {
    use scryer_domain::{ListFilter, MediaFacet};
    let filters = vec![
        ListFilter::RatingAtLeast {
            scale: "imdb".into(),
            value: 7.0,
        },
        ListFilter::ExcludeGenres {
            genres: vec!["action-adventure".into(), "unknown".into()],
        },
    ];
    let normalized = normalize_filters(
        &filters,
        &[MediaFacet::Movie, MediaFacet::Anime],
        &reply().entries,
    );
    assert_eq!(normalized.len(), 4);
    let ListFilter::ExcludeCanonicalTags {
        keys,
        unresolved_labels,
        ..
    } = &normalized[2]
    else {
        panic!()
    };
    assert_eq!(keys, &["canonical:genre:action"]);
    assert_eq!(unresolved_labels, &["unknown"]);
    assert_eq!(
        normalize_filters(
            &normalized,
            &[MediaFacet::Movie, MediaFacet::Anime],
            &reply().entries
        ),
        normalized
    );
    let mut entries = reply().entries;
    let mut duplicate_alias = entries[0].clone();
    duplicate_alias.key = "canonical:genre:other".into();
    entries.push(duplicate_alias);
    let ambiguous = normalize_filters(&filters, &[MediaFacet::Movie], &entries);
    let ListFilter::ExcludeCanonicalTags {
        keys,
        unresolved_labels,
        ..
    } = &ambiguous[1]
    else {
        panic!()
    };
    assert!(keys.is_empty());
    assert_eq!(unresolved_labels.len(), 2);
}
