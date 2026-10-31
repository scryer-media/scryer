//! A Newznab indexer whose answer for one title is larger than one background
//! pass reads. The built-in Newznab plugin pages through the provider one page
//! per call, so the background lane must not ask it the same question from the
//! top on every pass: each pass reads a bounded number of pages and the next
//! one resumes from the cursor where it stopped, across a restart, until the
//! provider has nothing further. A pass that stops with pages left is never
//! reported complete and is not held as contained. Operator searches still
//! ask, and do not reset the background read.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use common::{disable_platform_keystore_for_tests, initialize_wasm_runtime_for_tests};
use scryer_application::{
    IndexerClient, IndexerConfigRepository, IndexerErrorOperation, IndexerPluginProvider,
    IndexerSearchCompletion, IndexerSearchLearningContext, IndexerSearchOutcome,
    IndexerSearchResponse, ReleaseSearchSubjectKind, SearchMode,
};
use scryer_domain::IndexerConfig;
use scryer_infrastructure_acquisition::indexers::{
    config_store::IndexerConfigStore, search_client::MultiIndexerSearchClient,
    search_learning::IndexerSearchLearningStore, stats::InMemoryIndexerStatsTracker,
};
use scryer_infrastructure_crypto::EncryptionKey;
use scryer_infrastructure_datastore::SqliteServices;
use tokio_util::sync::CancellationToken;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers::method};

/// More hits than one background pass reads.
const TOTAL_HITS: usize = 3_200;
/// The page size the plugin asks this fixture for.
const PAGE_SIZE: usize = 100;
const INDEXER_ID: &str = "synthetic-ceiling-indexer";
const COVERAGE_SCOPE: &str = "title:synthetic-ceiling-title";

/// One search page the fixture served: the query it answered and the offset
/// it started at.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ServedPage {
    query: String,
    offset: usize,
}

/// Serves a synthetic Newznab feed of [`TOTAL_HITS`] items, paged by the
/// request's `offset` and `limit`, and records every search page it serves.
struct CeilingFeed {
    served: Arc<Mutex<Vec<ServedPage>>>,
}

impl Respond for CeilingFeed {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let params = request
            .url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<HashMap<_, _>>();
        if params.get("t").map(String::as_str) == Some("caps") {
            return ResponseTemplate::new(200).set_body_string(r#"{"channel":{"item":[]}}"#);
        }
        let offset = params
            .get("offset")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let limit = params
            .get("limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(PAGE_SIZE);
        self.served.lock().expect("served pages").push(ServedPage {
            query: params.get("q").cloned().unwrap_or_default(),
            offset,
        });
        let items = (offset..TOTAL_HITS.min(offset + limit))
            .map(|position| {
                let guid = format!("synthetic-guid-{position}");
                serde_json::json!({
                    "title": format!("Synthetic.Ceiling.Title.2024.1080p.WEB-DL.Part{position}-SYNGRP"),
                    "link": format!("https://indexer.invalid/details/{guid}"),
                    "pubDate": "Wed, 15 Jan 2025 12:00:00 +0000",
                    "enclosure": {"@attributes": {
                        "url": format!("https://indexer.invalid/api?t=get&id={guid}"),
                        "length": "4294967296",
                        "type": "application/x-nzb"
                    }},
                    "attr": [
                        {"@attributes": {"name": "size", "value": "4294967296"}},
                        {"@attributes": {"name": "guid", "value": guid}}
                    ]
                })
            })
            .collect::<Vec<_>>();
        ResponseTemplate::new(200).set_body_json(serde_json::json!({"channel": {"item": items}}))
    }
}

fn indexer_config(server: &MockServer) -> IndexerConfig {
    let now = chrono::Utc::now();
    let base_url = format!("{}/api", server.uri());
    IndexerConfig {
        last_error_code: None,
        id: INDEXER_ID.into(),
        name: "Synthetic Ceiling Indexer".into(),
        provider_type: "newznab".into(),
        base_url: base_url.clone(),
        api_key_encrypted: Some("synthetic-api-key".into()),
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        rate_limit_seconds: Some(0),
        rate_limit_burst: None,
        max_queries_per_minute: None,
        disabled_until: None,
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        config_json: Some(
            // No pacing between pages: the fixture asserts request counts, not timing.
            serde_json::json!({
                "base_url": base_url,
                "api_key": "synthetic-api-key",
                "request_interval_ms": "1",
            })
            .to_string(),
        ),
        created_at: now,
        updated_at: now,
    }
}

/// One application instance: a fresh connection to the same database file,
/// a fresh plugin runtime and a fresh search client.
async fn open_client(
    db_path: &str,
    key: &EncryptionKey,
) -> (MultiIndexerSearchClient, SqliteServices) {
    let db = SqliteServices::new(db_path).await.expect("open sqlite");
    db.set_encryption_key(key.clone())
        .await
        .expect("set encryption key");
    let datastore = db.datastore();
    let key = db.encryption_key_state();
    let configs = Arc::new(IndexerConfigStore::new(datastore.clone(), key.clone()));
    let plugin_provider: Arc<dyn IndexerPluginProvider> =
        Arc::new(scryer_plugins::DynamicPluginProvider::new(
            scryer_plugins::build_indexer_plugin_provider(&[], &[]),
        ));
    let client = MultiIndexerSearchClient::new(
        configs,
        Arc::new(InMemoryIndexerStatsTracker::new(None)),
        plugin_provider,
    )
    .with_search_learning_repository(Arc::new(IndexerSearchLearningStore::new(datastore, key)));
    (client, db)
}

fn context(title_id: &str, session: &str, background: bool) -> IndexerSearchLearningContext {
    IndexerSearchLearningContext {
        title_id: title_id.into(),
        facet: "movie".into(),
        subject_kind: ReleaseSearchSubjectKind::Title,
        search_session_id: session.into(),
        background_value: background.then_some(0.5),
        candidate_reuse_allowed: background,
    }
}

async fn search(
    client: &MultiIndexerSearchClient,
    query: &str,
    learning_context: IndexerSearchLearningContext,
) -> IndexerSearchResponse {
    <MultiIndexerSearchClient as IndexerClient>::search(
        client,
        query.to_string(),
        HashMap::new(),
        None,
        Some("movie".to_string()),
        None,
        None,
        None,
        SearchMode::Auto,
        IndexerErrorOperation::AutomaticSearch,
        None,
        None,
        None,
        None,
        vec![],
        Some(learning_context),
        CancellationToken::new(),
    )
    .await
    .expect("search should succeed")
}

fn indexer_outcome(response: &IndexerSearchResponse) -> IndexerSearchOutcome {
    response
        .indexer_outcomes
        .iter()
        .find(|outcome| outcome.indexer_id == INDEXER_ID)
        .map(|outcome| outcome.outcome)
        .unwrap_or_else(|| match response.completion {
            IndexerSearchCompletion::Complete => IndexerSearchOutcome::Complete {
                empty: response.results.is_empty(),
            },
            IndexerSearchCompletion::Partial {
                reason,
                retry_after,
            } => IndexerSearchOutcome::Partial {
                empty: response.results.is_empty(),
                reason,
                retry_after,
            },
        })
}

/// The pages served since `mark` for `query`, in the order they were asked.
fn offsets_since(served: &Mutex<Vec<ServedPage>>, mark: usize, query: &str) -> Vec<usize> {
    served.lock().expect("served pages")[mark..]
        .iter()
        .filter(|page| page.query == query)
        .map(|page| page.offset)
        .collect()
}

fn served_count(served: &Mutex<Vec<ServedPage>>) -> usize {
    served.lock().expect("served pages").len()
}

/// A background pass that stopped with pages left: not coverage, and still
/// moving rather than contained.
fn assert_resumable(outcome: IndexerSearchOutcome, what: &str) {
    assert!(
        matches!(outcome, IndexerSearchOutcome::Partial { .. }),
        "{what}: a pass that stopped with pages left is partial, got {outcome:?}"
    );
    assert!(
        !outcome.coverage_eligible(),
        "{what}: an unfinished paged read is never coverage"
    );
}

/// Asserts that a pass read consecutive pages starting at `from`, and returns
/// the offset the next page would start at.
fn assert_reads_on_from(offsets: &[usize], from: usize, what: &str) -> usize {
    assert!(!offsets.is_empty(), "{what}: the pass asked the indexer");
    let expected = (0..offsets.len())
        .map(|page| from + page * PAGE_SIZE)
        .collect::<Vec<_>>();
    assert_eq!(
        offsets, expected,
        "{what}: the pass resumes where the last one stopped and reads each page once"
    );
    from + offsets.len() * PAGE_SIZE
}

const TITLE_QUERY: &str = "Synthetic Ceiling Title";
const OTHER_QUERY: &str = "Synthetic Other Title";

#[tokio::test]
async fn a_long_paged_read_resumes_across_passes_and_restarts() {
    disable_platform_keystore_for_tests();
    initialize_wasm_runtime_for_tests();

    let server = MockServer::start().await;
    let served = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("GET"))
        .respond_with(CeilingFeed {
            served: served.clone(),
        })
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("scryer.db");
    let db_path = db_path.to_str().expect("utf-8 path").to_string();

    let encryption_key = EncryptionKey::generate();
    let (client, db) = open_client(&db_path, &encryption_key).await;
    IndexerConfigStore::new(db.datastore(), db.encryption_key_state())
        .create(indexer_config(&server))
        .await
        .expect("create indexer");

    // Pass one reads a bounded run of pages from the top of the feed.
    let first = search(
        &client,
        TITLE_QUERY,
        context("synthetic-ceiling-title", "background-1", true),
    )
    .await;
    let first_offsets = offsets_since(&served, 0, TITLE_QUERY);
    assert!(
        first_offsets.len() > 1,
        "the first pass pages through the feed"
    );
    let mut next_offset = assert_reads_on_from(&first_offsets, 0, "first pass");
    assert!(
        next_offset < TOTAL_HITS,
        "one pass stops short of the whole feed"
    );
    assert!(
        !first.results.is_empty() && first.results.len() < TOTAL_HITS,
        "the first pass returns what it read: {} results",
        first.results.len()
    );
    assert!(
        !first
            .results
            .iter()
            .any(|result| result.title.contains(&format!("Part{}-", TOTAL_HITS - 1))),
        "the last release is not reachable in one pass"
    );
    assert_resumable(indexer_outcome(&first), "first pass");
    client
        .link_search_session_coverage_scope("background-1", COVERAGE_SCOPE)
        .await
        .expect("link session");
    let holds = client
        .contained_search_holds(
            COVERAGE_SCOPE,
            &[INDEXER_ID.to_string()],
            chrono::Utc::now(),
        )
        .await
        .expect("read holds");
    assert!(
        !holds.contains_key(INDEXER_ID),
        "a paged read with pages left is resumed, not held as contained"
    );

    // Pass two continues from the cursor instead of asking from the top, and
    // still replays what pass one found.
    let mark = served_count(&served);
    let second = search(
        &client,
        TITLE_QUERY,
        context("synthetic-ceiling-title", "background-2", true),
    )
    .await;
    next_offset = assert_reads_on_from(
        &offsets_since(&served, mark, TITLE_QUERY),
        next_offset,
        "second pass",
    );
    assert_resumable(indexer_outcome(&second), "second pass");
    assert!(
        second.results.len() > first.results.len(),
        "the resumed pass replays earlier pages alongside the new ones: {} results",
        second.results.len()
    );

    // A restart (a new application instance on the same database) keeps the
    // cursor.
    drop(client);
    drop(db);
    let (client, _db) = open_client(&db_path, &encryption_key).await;
    let mark = served_count(&served);
    let restarted = search(
        &client,
        TITLE_QUERY,
        context("synthetic-ceiling-title", "background-3", true),
    )
    .await;
    next_offset = assert_reads_on_from(
        &offsets_since(&served, mark, TITLE_QUERY),
        next_offset,
        "after restart",
    );
    assert_resumable(indexer_outcome(&restarted), "after restart");

    // Another title starts its own read from the top.
    let mark = served_count(&served);
    search(
        &client,
        OTHER_QUERY,
        context("synthetic-other-title", "background-4", true),
    )
    .await;
    assert_reads_on_from(
        &offsets_since(&served, mark, OTHER_QUERY),
        0,
        "another title",
    );

    // An operator search asks live, and does not move the background cursor.
    let mark = served_count(&served);
    let operator = search(
        &client,
        TITLE_QUERY,
        context("synthetic-ceiling-title", "operator-1", false),
    )
    .await;
    assert_eq!(
        offsets_since(&served, mark, TITLE_QUERY).first(),
        Some(&0),
        "an operator search asks the indexer from the top"
    );
    assert!(
        !operator.results.is_empty(),
        "the operator search returns what it read"
    );

    // Later background passes carry on from the cursor until the provider
    // has nothing further; each page is read exactly once by the background
    // lane and the last release is finally reached.
    let mut last_release_reached = false;
    let mut completed = false;
    let max_passes = TOTAL_HITS / PAGE_SIZE + 1;
    for pass in 5..5 + max_passes {
        let mark = served_count(&served);
        let response = search(
            &client,
            TITLE_QUERY,
            context(
                "synthetic-ceiling-title",
                &format!("background-{pass}"),
                true,
            ),
        )
        .await;
        next_offset = assert_reads_on_from(
            &offsets_since(&served, mark, TITLE_QUERY),
            next_offset,
            &format!("background pass {pass}"),
        );
        last_release_reached |= response
            .results
            .iter()
            .any(|result| result.title.contains(&format!("Part{}-", TOTAL_HITS - 1)));
        let outcome = indexer_outcome(&response);
        if outcome.coverage_eligible() {
            completed = true;
            break;
        }
        assert_resumable(outcome, &format!("background pass {pass}"));
        assert!(
            next_offset <= TOTAL_HITS,
            "background pass {pass}: the read stops once the feed runs out"
        );
    }
    assert!(
        completed,
        "the background read completes once the provider has nothing further"
    );
    assert!(
        next_offset > TOTAL_HITS - PAGE_SIZE,
        "the background read reached the end of the feed (next offset {next_offset})"
    );
    assert!(
        last_release_reached,
        "the release past the first pass is reached by a later one"
    );
}
