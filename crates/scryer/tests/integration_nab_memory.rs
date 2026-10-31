//! Measures encoder workspace independently of already-owned result payloads.

#[allow(dead_code)]
#[path = "../src/nab_api/feed.rs"]
mod feed;
#[allow(dead_code)]
#[path = "../src/nab_api/request.rs"]
mod request;
#[allow(dead_code)]
#[path = "../src/nab_api/stream.rs"]
mod stream;

use scryer_application::{IndexerResponseAttributes, IndexerSearchResult, indexer_clients::*};
use scryer_infrastructure_acquisition::indexers::nab_feed::*;
use scryer_infrastructure_crypto::{EncryptionKey, artifact_reference::ArtifactReferenceCipher};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    io,
};

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

// SAFETY: Forward every allocation operation unchanged to System; thread-local
// counters allocate nothing and do not modify pointers or allocation layouts.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Forward the caller's valid allocation layout.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            record(layout.size() as isize);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        record(-(layout.size() as isize));
        // SAFETY: Forward the original pointer and allocation layout.
        unsafe { System.dealloc(p, layout) };
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: Forward the original pointer, layout and requested size.
        let next = unsafe { System.realloc(p, layout, size) };
        if !next.is_null() {
            record(size as isize - layout.size() as isize);
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

fn result() -> IndexerSearchResult {
    IndexerSearchResult {
        indexer_id: Some("source".into()),
        source: "source".into(),
        title: "Fixture".into(),
        link: None,
        download_url: Some("https://source.example/get?apikey=upstream".into()),
        source_kind: None,
        size_bytes: Some(123),
        published_at: Some("2026-10-01T00:00:00Z".into()),
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: None,
        password_hint: None,
        parsed_release_metadata: None,
        quality_profile_decision: None,
        extra: Default::default(),
        response_attributes: IndexerResponseAttributes::default(),
        guid: Some("release".into()),
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
    }
}

fn client() -> AuthenticatedIndexerClient {
    let now = chrono::Utc::now();
    AuthenticatedIndexerClient {
        client: IndexerClientRecord {
            id: "client".into(),
            configuration: IndexerClientConfiguration {
                name: "Fixture".into(),
                enabled: true,
                protocols: vec![IndexerClientProtocol::Newznab],
                grants: vec![IndexerClientGrant {
                    indexer_id: "source".into(),
                    categories: None,
                }],
            },
            lookup_id: Some("lookup".into()),
            secret_hash: Some("hash".into()),
            key_generation: 1,
            revision: 1,
            last_used_at: None,
            last_outcome_code: None,
            created_at: now,
            updated_at: now,
        },
    }
}

#[test]
fn encoder_workspace_for_large_attributes_and_references() {
    let client = client();
    let cipher = ArtifactReferenceCipher::new(&EncryptionKey::from_bytes([42; 32]));
    let endpoint = "https://scryer.example/newznab/api";
    let metadata = NabFeedPage {
        offset: 0,
        total: None,
        generated_at: chrono::DateTime::UNIX_EPOCH,
    };
    for case in [
        "plain",
        "escaped",
        "many",
        "names",
        "url",
        "reference",
        "reference_without_guid",
    ] {
        let mut result = result();
        if matches!(case, "reference" | "reference_without_guid") {
            result.download_url = Some(format!(
                "https://source.example/get?token={}",
                "x".repeat(1024 * 1024)
            ));
            if case == "reference_without_guid" {
                result.guid = None;
            }
        } else {
            let count = if case == "many" { 8192 } else { 1 };
            for index in 0..count {
                let (name, value) = match case {
                    "escaped" => ("custom".into(), "\"".repeat(1024 * 1024)),
                    "many" => (format!("custom-{index}"), "x".repeat(128)),
                    "names" => ("x".repeat(1024 * 1024), "x".into()),
                    "url" => (
                        "custom".into(),
                        format!("https://source.example/?custom={}", "%20".repeat(350000)),
                    ),
                    _ => ("custom".into(), "x".repeat(1024 * 1024)),
                };
                result.response_attributes.original.push(
                    serde_json::from_value(serde_json::json!({
                        "origin": "attribute", "namespace": "newznab", "name": name, "value": value,
                    }))
                    .unwrap(),
                );
            }
        }
        // Compiled redaction expressions and search caches outlive an item.
        // Initialize those dependencies before measuring encoder-owned workspace.
        for attribute in &result.response_attributes.original {
            drop(scryer_application::url_redaction::redact_url_credentials(
                &attribute.value,
            ));
        }
        if let Some(size) = result.size_bytes {
            drop(scryer_application::url_redaction::redact_url_credentials(
                &size.to_string(),
            ));
        }
        for xml in [true, false] {
            let estimate = feed::scratch_bytes(
                &result,
                &client,
                endpoint,
                if xml {
                    request::Format::Xml
                } else {
                    request::Format::Json
                },
            );
            let baseline = LIVE.get();
            HIGH.set(baseline);
            let items = std::iter::once_with(|| {
                feed::item(
                    &result,
                    &client,
                    IndexerClientProtocol::Newznab,
                    endpoint,
                    &cipher,
                )
            });
            if xml {
                try_write_xml_feed(io::sink(), NabProtocol::Newznab, &metadata, items).unwrap();
            } else {
                try_write_json_feed(io::sink(), NabProtocol::Newznab, &metadata, items).unwrap();
            }
            let peak = HIGH.get() - baseline;
            assert_eq!(LIVE.get(), baseline, "encoder must release its workspace");
            assert!(
                estimate >= peak as usize,
                "case={case} xml={xml} estimate={estimate} peak={peak}"
            );
            assert!(
                estimate + 4 * 1024 * 1024 < 64 * 1024 * 1024,
                "the MiB-sized fixture and its streaming workspace must fit the shared pool"
            );
            eprintln!(
                "case={case} xml={xml} encoder_peak_bytes={peak} reserved_scratch={estimate}"
            );
        }
    }
}
