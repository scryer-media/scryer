use super::*;
use scryer_application::{
    AppResult, IndexerClient, IndexerConfigRepository, IndexerPluginProvider,
    IndexerSearchCompletion, IndexerSearchResponse, RawTextSearchRequest,
};
use scryer_domain::{IndexerConfig, IndexerProviderCapabilities};
use scryer_infrastructure_acquisition::indexers::{
    config_store::IndexerConfigStore, search_client::MultiIndexerSearchClient,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

type ObservedRead = (String, RawTextSearchRequest, oneshot::Sender<()>);

#[derive(Default)]
struct ReadJournal {
    active: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
    total: std::sync::atomic::AtomicUsize,
}
struct ActiveRead(Arc<ReadJournal>);
impl Drop for ActiveRead {
    fn drop(&mut self) {
        self.0
            .active
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
struct ControlledProvider(mpsc::Sender<ObservedRead>, Arc<ReadJournal>);
struct ControlledSource(String, mpsc::Sender<ObservedRead>, Arc<ReadJournal>);

#[async_trait::async_trait]
impl IndexerClient for ControlledSource {
    async fn search_raw_text(
        &self,
        request: RawTextSearchRequest,
        cancel: CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        use std::sync::atomic::Ordering::SeqCst;
        let active = self.2.active.fetch_add(1, SeqCst) + 1;
        self.2.peak.fetch_max(active, SeqCst);
        self.2.total.fetch_add(1, SeqCst);
        let _active = ActiveRead(self.2.clone());
        let (release, wait) = oneshot::channel();
        self.1
            .send((self.0.clone(), request, release))
            .await
            .unwrap();
        tokio::select! {
            // Permit the aggregate's 75-second deadline while retaining the
            // existing indexer operation's 120-second failure bound.
            result = tokio::time::timeout(Duration::from_secs(120), wait) => result.unwrap().unwrap(),
            _ = cancel.cancelled() => panic!("HTTP disconnect cancelled a generation-owned source"),
        }
        Ok(IndexerSearchResponse {
            results: Vec::new(),
            completion: IndexerSearchCompletion::Complete,
            next_cursor: None,
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
            indexer_outcomes: Vec::new(),
        })
    }
}

impl IndexerPluginProvider for ControlledProvider {
    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        Some(Arc::new(ControlledSource(
            config.id.clone(),
            self.0.clone(),
            self.1.clone(),
        )))
    }
    fn available_provider_types(&self) -> Vec<String> {
        vec!["newznab".into()]
    }
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        Vec::new()
    }
    fn capabilities_for_provider(&self, _: &str) -> IndexerProviderCapabilities {
        serde_json::from_value(serde_json::json!({
            "raw_nab_search": true, "query_param": "q", "search": true,
            "protocols": ["usenet"], "limits": { "paged_search": true, "page_size": 100 }
        }))
        .unwrap()
    }
}

async fn next_read(reads: &mut mpsc::Receiver<ObservedRead>) -> ObservedRead {
    tokio::time::timeout(Duration::from_secs(60), reads.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn controlled_fixture() -> (
    TestContext,
    Router,
    String,
    mpsc::Receiver<ObservedRead>,
    Arc<ReadJournal>,
) {
    controlled_fixture_with_cooldown(false).await
}

async fn controlled_fixture_with_cooldown(
    backed_off: bool,
) -> (
    TestContext,
    Router,
    String,
    mpsc::Receiver<ObservedRead>,
    Arc<ReadJournal>,
) {
    let (context, _, key, clients) = fixture().await;
    let configs = Arc::new(IndexerConfigStore::new(
        context.db.datastore(),
        context.db.encryption_key_state(),
    ));
    let mut client = clients.get_by_id("http-client").await.unwrap().unwrap();
    let now = chrono::Utc::now();
    for index in 0..25 {
        let id = format!("controlled-{index}");
        configs.create(serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "provider_type": "newznab", "base_url": "https://fixture.invalid",
            "is_enabled": true, "enable_interactive_search": true, "enable_auto_search": false,
            "created_at": now, "updated_at": now,
            "disabled_until": backed_off.then(|| now + chrono::Duration::hours(1)),
            "caps_snapshot_json": serde_json::json!({
                "parser_version": scryer_domain::INDEXER_CAPS_PARSER_VERSION,
                "cache_fetched_at": now.to_rfc3339(), "searching_present": true,
                "limits_default": 100, "limits_max": 100,
                "search": { "available": true, "supported_params": ["q"] }
            }).to_string()
        })).unwrap()).await.unwrap();
        client.configuration.grants.push(IndexerClientGrant {
            indexer_id: id,
            categories: None,
        });
    }
    client.revision += 1;
    assert!(clients.save(&client, Some(1)).await.unwrap());
    let (sender, reads) = mpsc::channel(25);
    let journal = Arc::new(ReadJournal::default());
    let provider = Arc::new(ControlledProvider(sender, journal.clone()));
    let multi = Arc::new(MultiIndexerSearchClient::new(
        configs,
        context.indexer_stats.clone(),
        provider.clone(),
    ));
    let app = context
        .app
        .clone()
        .with_indexer_client_for_test(multi)
        .with_test_overrides(|builder| {
            builder
                .with_indexer_client_repository(clients)
                .with_plugin_provider(provider)
        });
    let router = nab_api::router(nab_api::NabState {
        app,
        cipher: Arc::new(ArtifactReferenceCipher::new(&EncryptionKey::from_bytes(
            [42; 32],
        ))),
        base_path: String::new(),
        tls: false,
    });
    (context, router, key, reads, journal)
}

#[tokio::test]
async fn twenty_five_sources_share_existing_admission_and_survive_http_disconnect() {
    let (_context, router, key, mut reads, journal) = controlled_fixture().await;
    let first = tokio::spawn({
        let router = router.clone();
        let key = key.clone();
        async move {
            call(
                &router,
                "/newznab/api?t=search&q=admission",
                Some(("x-api-key", &key)),
            )
            .await
        }
    });
    let mut held = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for _ in 0..24 {
        let (id, request, release) = next_read(&mut reads).await;
        assert!(ids.insert(id));
        assert_eq!(
            request.limit, None,
            "the provider keeps its native page size; the aggregate must not allocate a share"
        );
        assert!(request.page.is_some());
        held.push(release);
    }
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    held.pop()
        .unwrap()
        .send(())
        .expect("disconnect keeps an admitted read alive");
    let (id, request, release) = next_read(&mut reads).await;
    assert!(ids.insert(id));
    assert_eq!(ids.len(), 25);
    assert_eq!(request.limit, None);
    held.push(release);
    let continuation = tokio::spawn(async move {
        call(
            &router,
            "/newznab/api?t=search&q=admission&offset=1",
            Some(("x-api-key", &key)),
        )
        .await
    });
    for release in held {
        release
            .send(())
            .expect("continuation owns all remaining reads");
    }
    let (status, body) = tokio::time::timeout(Duration::from_secs(60), continuation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("<rss"));
    assert_eq!(journal.total.load(std::sync::atomic::Ordering::SeqCst), 25);
    assert_eq!(journal.peak.load(std::sync::atomic::Ordering::SeqCst), 24);
}

#[tokio::test]
async fn admitted_and_queued_reads_survive_deadline_then_expired_http_walk_rebuilds() {
    let (context, router, key, mut reads, journal) = controlled_fixture().await;
    let request = Request::builder()
        .uri("/newznab/api?t=search&q=lifecycle")
        .header("host", "scryer.example")
        .header("x-api-key", &key)
        .body(Body::empty())
        .unwrap();
    let initial = tokio::spawn({
        let router = router.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(180), router.oneshot(request))
                .await
                .unwrap()
                .unwrap()
        }
    });
    let mut held = Vec::new();
    let mut first_ids = std::collections::HashSet::new();
    for _ in 0..24 {
        let (id, request, release) = next_read(&mut reads).await;
        assert!(first_ids.insert(id));
        assert_eq!(request.limit, None);
        held.push(release);
    }
    // Advance only after all admitted reads have reached their explicit gate.
    // Resume before awaiting database/serializer work, avoiding auto-advance
    // while an external worker is still servicing this HTTP response.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(75)).await;
    tokio::time::resume();
    let response = tokio::time::timeout(Duration::from_secs(60), initial)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio::time::timeout(
        Duration::from_secs(60),
        to_bytes(response.into_body(), 1024 * 1024),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("<rss"));
    let clients = IndexerClientStore::new(context.db.datastore());
    assert_eq!(
        clients
            .get_by_id("http-client")
            .await
            .unwrap()
            .unwrap()
            .last_outcome_code
            .as_deref(),
        Some(usage::SEARCH_PARTIAL)
    );
    assert!(held.iter().all(|release| !release.is_closed()));

    held.pop().unwrap().send(()).unwrap();
    let (id, _, release) = next_read(&mut reads).await;
    assert!(first_ids.insert(id));
    assert_eq!(first_ids.len(), 25);
    held.push(release);
    for release in held {
        release.send(()).unwrap();
    }
    let uri = "/newznab/api?t=search&q=lifecycle&offset=1";
    let (status, _) = call(&router, uri, Some(("x-api-key", &key))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(journal.total.load(std::sync::atomic::Ordering::SeqCst), 25);
    assert_eq!(journal.peak.load(std::sync::atomic::Ordering::SeqCst), 24);

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(300)).await;
    tokio::time::resume();
    let rebuild = tokio::spawn({
        let router = router.clone();
        let key = key.clone();
        async move { call(&router, uri, Some(("x-api-key", &key))).await }
    });
    let mut rebuilt_ids = std::collections::HashSet::new();
    for _ in 0..25 {
        let (id, request, release) = next_read(&mut reads).await;
        assert!(rebuilt_ids.insert(id));
        assert_eq!(request.limit, None);
        release.send(()).unwrap();
    }
    let (status, _) = tokio::time::timeout(Duration::from_secs(60), rebuild)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rebuilt_ids, first_ids);
    assert_eq!(journal.total.load(std::sync::atomic::Ordering::SeqCst), 50);
    assert_eq!(journal.peak.load(std::sync::atomic::Ordering::SeqCst), 24);
}

#[tokio::test]
async fn all_backed_off_sources_return_http_429_without_dispatch() {
    let (_context, router, key, _reads, journal) = controlled_fixture_with_cooldown(true).await;
    let response = tokio::time::timeout(
        Duration::from_secs(60),
        router.oneshot(
            Request::builder()
                .uri("/newznab/api?t=search&q=cooldown")
                .header("host", "scryer.example")
                .header("x-api-key", key)
                .body(Body::empty())
                .unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        response.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    let body = tokio::time::timeout(
        Duration::from_secs(60),
        to_bytes(response.into_body(), 4096),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("code=\"500\""));
    assert_eq!(journal.total.load(std::sync::atomic::Ordering::SeqCst), 0);
}
