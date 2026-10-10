//! A Newznab indexer whose answer for one title is larger than the plugin's
//! page ceiling. The background lane must not ask it the same capped question
//! on every pass: the strategy is contained, waits out a backoff that survives
//! a restart, and is never reported complete. Operator searches still ask, and
//! do not reset the background backoff.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{disable_platform_keystore_for_tests, initialize_wasm_runtime_for_tests};
use scryer_application::{
    IndexerClient, IndexerConfigRepository, IndexerErrorOperation, IndexerPluginProvider,
    IndexerSearchCompletion, IndexerSearchIncompleteReason, IndexerSearchLearningContext,
    IndexerSearchOutcome, IndexerSearchResponse, ReleaseSearchSubjectKind, SearchMode,
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

/// More hits than the plugin's page ceiling will read in one invocation.
const TOTAL_HITS: usize = 3_200;
const INDEXER_ID: &str = "synthetic-ceiling-indexer";
const COVERAGE_SCOPE: &str = "title:synthetic-ceiling-title";

/// Serves a synthetic Newznab feed of [`TOTAL_HITS`] items, paged by the
/// request's `offset` and `limit`, and counts every search page it serves.
struct CeilingFeed {
    search_pages: Arc<AtomicUsize>,
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
        self.search_pages.fetch_add(1, Ordering::SeqCst);
        let offset = params
            .get("offset")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let limit = params
            .get("limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(100);
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

fn assert_contained(outcome: IndexerSearchOutcome, what: &str) {
    match outcome {
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::PageCeilingReached),
            retry_after: Some(retry_after),
            ..
        } => assert!(
            retry_after > std::time::Duration::from_secs(50 * 60)
                && retry_after <= std::time::Duration::from_secs(3_600),
            "{what}: a first containment waits about an hour, got {retry_after:?}"
        ),
        other => panic!("{what}: expected a contained partial outcome, got {other:?}"),
    }
    assert!(
        !outcome.coverage_eligible(),
        "{what}: a contained indexer is never coverage"
    );
}

#[tokio::test]
async fn a_capped_strategy_is_contained_across_passes_and_restarts() {
    disable_platform_keystore_for_tests();
    initialize_wasm_runtime_for_tests();

    let server = MockServer::start().await;
    let search_pages = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .respond_with(CeilingFeed {
            search_pages: search_pages.clone(),
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

    // Pass one asks the indexer and reads up to the plugin's page ceiling.
    let first = search(
        &client,
        "Synthetic Ceiling Title",
        context("synthetic-ceiling-title", "background-1", true),
    )
    .await;
    let first_pages = search_pages.load(Ordering::SeqCst);
    assert!(first_pages > 1, "the first pass pages through the feed");
    assert!(
        !first.results.is_empty() && first.results.len() < TOTAL_HITS,
        "the ceiling stops the plugin short of the whole feed: {} results",
        first.results.len()
    );
    assert!(
        !first
            .results
            .iter()
            .any(|result| result.title.contains(&format!("Part{}-", TOTAL_HITS - 1))),
        "the release past the ceiling is not reachable in one invocation"
    );
    assert_contained(indexer_outcome(&first), "first pass");
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
        holds.contains_key(INDEXER_ID),
        "the acquisition gate sees the contained indexer"
    );

    // Pass two, five minutes later in production terms, does not ask again.
    let second = search(
        &client,
        "Synthetic Ceiling Title",
        context("synthetic-ceiling-title", "background-2", true),
    )
    .await;
    assert_eq!(
        search_pages.load(Ordering::SeqCst),
        first_pages,
        "a contained strategy is not re-asked before it is due"
    );
    assert_contained(indexer_outcome(&second), "second pass");
    assert!(
        !second.results.is_empty(),
        "the held strategy still replays what it found"
    );

    // A restart (a new application instance on the same database) keeps it.
    drop(client);
    drop(db);
    let (client, _db) = open_client(&db_path, &encryption_key).await;
    let restarted = search(
        &client,
        "Synthetic Ceiling Title",
        context("synthetic-ceiling-title", "background-3", true),
    )
    .await;
    assert_eq!(
        search_pages.load(Ordering::SeqCst),
        first_pages,
        "the backoff survives a restart"
    );
    assert_contained(indexer_outcome(&restarted), "after restart");

    // Another title is not held by this one's containment.
    let before_other = search_pages.load(Ordering::SeqCst);
    search(
        &client,
        "Synthetic Other Title",
        context("synthetic-other-title", "background-4", true),
    )
    .await;
    assert!(
        search_pages.load(Ordering::SeqCst) > before_other,
        "a second title is searched in the same window"
    );

    // An operator search asks live, and does not reset the background backoff.
    let before_operator = search_pages.load(Ordering::SeqCst);
    let operator = search(
        &client,
        "Synthetic Ceiling Title",
        context("synthetic-ceiling-title", "operator-1", false),
    )
    .await;
    assert!(
        search_pages.load(Ordering::SeqCst) > before_operator,
        "an operator search asks the indexer"
    );
    assert!(
        !indexer_outcome(&operator).coverage_eligible(),
        "an operator search past the ceiling is still reported partial"
    );
    let after_operator = search_pages.load(Ordering::SeqCst);
    let background_again = search(
        &client,
        "Synthetic Ceiling Title",
        context("synthetic-ceiling-title", "background-5", true),
    )
    .await;
    assert_eq!(
        search_pages.load(Ordering::SeqCst),
        after_operator,
        "the operator's search did not reset the background backoff"
    );
    assert_contained(indexer_outcome(&background_again), "after operator search");
}
