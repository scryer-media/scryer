//! Allocator evidence for the raw coordinator's retained JSON accounting.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

pub use scryer_application::{IndexerSearchResult, ScoringEntry, ScoringSource, SubmissionScope};

#[allow(dead_code)]
#[path = "../src/catalog/interactive_release_search/memory.rs"]
mod accounting;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static HIGH: Cell<isize> = const { Cell::new(0) };
}

struct MeasuredAllocator;

fn record(delta: isize) {
    let _ = LIVE.try_with(|live| {
        let next = live.get() + delta;
        live.set(next);
        let _ = HIGH.try_with(|high| high.set(high.get().max(next)));
    });
}

// SAFETY: Every allocation operation is forwarded unchanged to System. The
// thread-local counters allocate nothing and never alter pointers or layouts.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies the allocator contract's valid layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size() as isize);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(-(layout.size() as isize));
        // SAFETY: The pointer and original layout are forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The caller's pointer, layout and new size retain their contract.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            record(new_size as isize - layout.size() as isize);
        }
        replacement
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

fn verify_object_allocations(entries: usize, original_capacity: usize, nested: bool) {
    let baseline = LIVE.get();
    HIGH.set(baseline);
    let mut map = serde_json::Map::with_capacity(original_capacity);
    for index in 0..original_capacity.max(entries) {
        map.insert(
            index.to_string(),
            serde_json::json!({ "custom": ["value", "value"] }),
        );
    }
    map.retain(|key, _| key.parse::<usize>().unwrap() < entries);
    let mut value = serde_json::Value::Object(map);
    if nested {
        for _ in 0..8 {
            value = serde_json::json!({ "nested": value });
        }
    }
    accounting::compact_json_objects(&mut value);
    let retained = (LIVE.get() - baseline) as usize;
    let high_water = (HIGH.get() - baseline) as usize;
    let charged = accounting::json_heap(&value);
    assert!(
        charged >= retained,
        "charged={charged}, retained={retained}, peak={high_water}"
    );
    if original_capacity > entries * 100 + 100 {
        assert!(high_water > retained * 10);
    }
    drop(value);
    assert_eq!(
        LIVE.get(),
        baseline,
        "fixture must release all owned allocations"
    );
    eprintln!(
        "entries={entries} prior_capacity={original_capacity} nested={nested}: retained={retained} charged={charged} transient_peak={high_water}"
    );
}

#[test]
fn raw_json_small_and_nested_objects_fit_the_retained_charge() {
    for entries in [0, 1, 2, 11, 17, 128] {
        verify_object_allocations(entries, entries, false);
        verify_object_allocations(entries, entries, true);
    }
}

#[test]
fn raw_json_removed_entries_cannot_hide_spare_object_capacity() {
    for entries in [0, 1, 2] {
        verify_object_allocations(entries, 4096, false);
        verify_object_allocations(entries, 4096, true);
    }
}

#[test]
fn representative_source_pages_and_shared_readers_fit_owned_storage_accounting() {
    use std::{mem::size_of, sync::Arc};
    for (sources, attributes, value_bytes) in [(50, 8, 256), (25, 32, 1024)] {
        let pool = Arc::new(accounting::RawSearchBudget::default());
        let baseline = LIVE.get();
        HIGH.set(baseline);
        let pages: Vec<Arc<Vec<IndexerSearchResult>>> = (0..sources)
            .map(|source| {
                Arc::new(
                    (0..100)
                        .map(|rank| IndexerSearchResult {
                            indexer_id: Some(source.to_string()),
                            source: format!("Source {source}"),
                            title: format!("Fixture.{rank}.1080p.WEB-DL"),
                            link: None,
                            download_url: Some(format!("https://indexer.example/{source}/{rank}")),
                            source_kind: None,
                            size_bytes: Some(1024),
                            published_at: Some("2026-01-01T00:00:00Z".into()),
                            thumbs_up: None,
                            thumbs_down: None,
                            indexer_languages: None,
                            indexer_subtitles: None,
                            indexer_grabs: None,
                            password_hint: Some("fixture-password".into()),
                            parsed_release_metadata: None,
                            quality_profile_decision: None,
                            extra: Default::default(),
                            response_attributes: scryer_application::IndexerResponseAttributes {
                                original: (0..attributes)
                                    .map(|_| scryer_plugin_sdk::PluginReleaseAttribute {
                                        origin: scryer_plugin_sdk::PluginReleaseAttributeOrigin::Attribute,
                                        name: "repeated-custom".into(),
                                        value: "x".repeat(value_bytes),
                                        namespace: Some("newznab".into()),
                                        namespace_uri: Some(
                                            "http://www.newznab.com/DTD/2010/feeds/attributes/"
                                                .into(),
                                        ),
                                    })
                                    .collect(),
                                ..Default::default()
                            },
                            guid: Some(format!("{source}-{rank}")),
                            info_url: None,
                            provenance: None,
                            candidate_token: None,
                            queue_scope: None,
                            coverage_scope: None,
                            auto_eligible: None,
                            auto_decision_code: None,
                            auto_decision_summary: None,
                            release_listing_json: None,
                            http: None,
                        })
                        .collect(),
                )
            })
            .collect();
        let charge = pages.capacity() * size_of::<Arc<Vec<IndexerSearchResult>>>()
            + pages
                .iter()
                .map(|page| {
                    2 * size_of::<usize>()
                        + size_of::<Vec<IndexerSearchResult>>()
                        + page.capacity() * size_of::<IndexerSearchResult>()
                        + page.iter().map(accounting::result_heap).sum::<usize>()
                })
                .sum::<usize>();
        let retained = (LIVE.get() - baseline) as usize;
        let reservation = pool.reserve(charge);
        assert_eq!(
            reservation.is_some(),
            charge <= 64 * 1024 * 1024,
            "oversized source payloads must be refused instead of admitted without a charge"
        );
        drop(reservation);
        assert!(
            charge >= retained,
            "sources={sources} charged={charge} retained={retained}"
        );
        let readers: Vec<_> = (0..16).map(|_| pages.clone()).collect();
        let reader_bytes = (LIVE.get() - baseline) as usize - retained;
        let reader_charge = readers.capacity() * size_of::<Vec<Arc<Vec<IndexerSearchResult>>>>()
            + readers
                .iter()
                .map(|pages| pages.capacity() * size_of::<Arc<Vec<IndexerSearchResult>>>())
                .sum::<usize>();
        assert_eq!(
            reader_bytes, reader_charge,
            "readers must share the large page payloads"
        );
        drop(pages);
        assert!(
            LIVE.get() > baseline,
            "active readers retain their source pages"
        );
        drop(readers);
        assert_eq!(
            LIVE.get(),
            baseline,
            "all payload allocations release with their last reader"
        );
        eprintln!(
            "sources={sources} rows={} attrs={attributes} value_bytes={value_bytes} retained={retained} charged={charge} reader_bytes={reader_bytes} peak={}",
            sources * 100,
            HIGH.get() - baseline
        );
    }
}
