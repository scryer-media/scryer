#![recursion_limit = "256"]

#[path = "integration_nab_requests/admission.rs"]
mod admission;
mod common;
#[allow(
    dead_code,
    reason = "this integration test imports only part of the application module"
)]
#[path = "../src/nab_api/mod.rs"]
mod nab_api;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use common::TestContext;
use scryer_application::{indexer_clients::*, testing::AppUseCaseTestExt};
use scryer_infrastructure_acquisition::indexers::client_store::IndexerClientStore;
use scryer_infrastructure_crypto::{EncryptionKey, artifact_reference::ArtifactReferenceCipher};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

async fn fixture() -> (TestContext, Router, String, Arc<IndexerClientStore>) {
    let context = TestContext::new().await;
    let store = Arc::new(IndexerClientStore::new(context.db.datastore()));
    let key = issue_indexer_client_key("test-salt").unwrap();
    let now = chrono::Utc::now();
    assert!(
        store
            .save(
                &IndexerClientRecord {
                    id: "http-client".into(),
                    configuration: IndexerClientConfiguration {
                        name: "Consumer fixture".into(),
                        enabled: true,
                        protocols: vec![
                            IndexerClientProtocol::Newznab,
                            IndexerClientProtocol::Torznab
                        ],
                        grants: vec![],
                    },
                    lookup_id: Some(key.lookup_id),
                    secret_hash: Some(key.secret_hash),
                    key_generation: 1,
                    revision: 1,
                    last_used_at: None,
                    last_outcome_code: None,
                    created_at: now,
                    updated_at: now,
                },
                None
            )
            .await
            .unwrap()
    );
    let app = context
        .app
        .with_test_overrides(|builder| builder.with_indexer_client_repository(store.clone()));
    let router = nab_api::router(nab_api::NabState {
        app,
        cipher: Arc::new(ArtifactReferenceCipher::new(&EncryptionKey::from_bytes(
            [42; 32],
        ))),
        base_path: String::new(),
        tls: false,
    });
    (context, router, key.raw_key, store)
}

async fn call(router: &Router, uri: &str, key: Option<(&str, &str)>) -> (StatusCode, String) {
    let mut request = Request::builder().uri(uri).header("host", "scryer.example");
    if let Some((name, value)) = key {
        request = request.header(name, value);
    }
    let response = tokio::time::timeout(
        Duration::from_secs(60),
        router.clone().oneshot(request.body(Body::empty()).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let status = response.status();
    let bytes = tokio::time::timeout(
        Duration::from_secs(60),
        to_bytes(response.into_body(), 1024 * 1024),
    )
    .await
    .unwrap()
    .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn shared_routes_require_a_client_key_for_caps_and_search() {
    let (_context, router, key, _) = fixture().await;
    for path in ["/newznab/api", "/torznab/api"] {
        for operation in ["caps", "search"] {
            let (status, body) = call(&router, &format!("{path}?t={operation}&o=json"), None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(body.contains("100"));
        }
        let (status, xml) = call(&router, &format!("{path}?t=caps&apikey={key}"), None).await;
        assert_eq!(status, StatusCode::OK, "{xml}");
        assert!(xml.contains("<caps>") && xml.contains("default=\"100\""));
        let (status, json) = call(
            &router,
            &format!("{path}?t=caps&o=json"),
            Some(("authorization", &format!("Bearer {key}"))),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        serde_json::from_str::<serde_json::Value>(&json).unwrap();
        let (status, xml) = call(
            &router,
            &format!("{path}?t=search&q=fixture"),
            Some(("x-api-key", &key)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{xml}");
        assert!(xml.contains("<rss") && !xml.contains("<item>"));
        assert!(!xml.contains(&key));
    }
}

#[tokio::test]
async fn authenticated_outcomes_are_recorded_without_stamping_invalid_keys() {
    let (_context, router, key, store) = fixture().await;
    assert!(
        store
            .get_by_id("http-client")
            .await
            .unwrap()
            .unwrap()
            .last_used_at
            .is_none()
    );

    let (status, _) = call(
        &router,
        "/newznab/api?t=caps",
        Some(("x-api-key", "invalid")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        store
            .get_by_id("http-client")
            .await
            .unwrap()
            .unwrap()
            .last_used_at
            .is_none()
    );

    for (operation, outcome) in [("caps", usage::CAPS_READY), ("search", usage::SEARCH_READY)] {
        let (status, _) = call(
            &router,
            &format!("/newznab/api?t={operation}"),
            Some(("x-api-key", &key)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let record = store.get_by_id("http-client").await.unwrap().unwrap();
        assert!(record.last_used_at.is_some());
        assert_eq!(record.last_outcome_code.as_deref(), Some(outcome));
    }

    let mut record = store.get_by_id("http-client").await.unwrap().unwrap();
    record.configuration.protocols = vec![IndexerClientProtocol::Newznab];
    record.revision += 1;
    assert!(store.save(&record, Some(1)).await.unwrap());
    let (status, _) = call(&router, "/torznab/api?t=caps", Some(("x-api-key", &key))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let refused = store.get_by_id("http-client").await.unwrap().unwrap();
    assert_eq!(refused.last_outcome_code.as_deref(), Some(usage::REFUSED));

    let (status, _) = call(
        &router,
        "/newznab/api?t=caps",
        Some(("x-api-key", "invalid")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let unchanged = store.get_by_id("http-client").await.unwrap().unwrap();
    assert_eq!(unchanged.last_used_at, refused.last_used_at);
    assert_eq!(unchanged.last_outcome_code, refused.last_outcome_code);
}

#[tokio::test]
async fn keys_do_not_cross_protocol_grants_and_revocation_applies_to_both_routes() {
    let (context, router, key, store) = fixture().await;
    let actor = context.app.find_or_create_default_user().await.unwrap();
    let personal = context
        .app
        .create_api_key(
            &actor,
            scryer_application::CreateApiKey {
                label: "Personal fixture".into(),
                expiry: scryer_application::ApiKeyExpiryPreset::Days30,
            },
        )
        .await
        .unwrap();
    assert!(personal.raw_key.starts_with("ska_"));
    assert!(
        context
            .app
            .authenticate_api_key(&personal.raw_key)
            .await
            .is_ok()
    );
    let (status, _) = call(
        &router,
        "/newznab/api?t=caps",
        Some(("x-api-key", &personal.raw_key)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let mut record = store.get_by_id("http-client").await.unwrap().unwrap();
    record.configuration.protocols = vec![IndexerClientProtocol::Newznab];
    record.revision += 1;
    assert!(store.save(&record, Some(1)).await.unwrap());
    let (status, _) = call(&router, "/torznab/api?t=caps", Some(("x-api-key", &key))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = call(
        &router,
        "/newznab/api?t=get&id=https%3A%2F%2Fupstream.example%2Fsecret",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("upstream.example") && !body.contains("secret"));
    record.secret_hash = None;
    record.lookup_id = None;
    record.revision += 1;
    assert!(store.save(&record, Some(2)).await.unwrap());
    let (status, _) = call(&router, "/newznab/api?t=caps", Some(("x-api-key", &key))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn retrieval_survives_idle_expiry_without_an_interactive_actor_or_cached_search() {
    use scryer_application::{
        AppResult, DownloadClient, DownloadClientAddRequest, DownloadGrabResult,
        IndexerArtifactResolutionRequest, IndexerConfigRepository, ResolvedDownloadArtifact,
    };
    use scryer_infrastructure_acquisition::indexers::config_store::IndexerConfigStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct ArtifactOnly(AtomicUsize);
    #[async_trait::async_trait]
    impl DownloadClient for ArtifactOnly {
        async fn submit_download(
            &self,
            _: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            panic!("a client artifact request must never enqueue a download");
        }
        async fn fetch_release_artifact(
            &self,
            request: &IndexerArtifactResolutionRequest,
        ) -> AppResult<ResolvedDownloadArtifact> {
            assert_eq!(request.indexer_id.as_deref(), Some("artifact-source"));
            assert_eq!(request.source_url, "https://fixture.invalid/artifact");
            assert!(request.title_id.is_none() && request.search_facet.is_none());
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ResolvedDownloadArtifact::Nzb {
                bytes: b"<nzb/>".to_vec(),
                password_candidates: Default::default(),
                file_name: None,
                content_type: None,
            })
        }
    }
    let (context, _, _, clients) = fixture().await;
    let now = chrono::Utc::now();
    let configs =
        IndexerConfigStore::new(context.db.datastore(), context.db.encryption_key_state());
    configs
        .create(
            serde_json::from_value(serde_json::json!({
                "id": "artifact-source", "name": "Artifact fixture", "provider_type": "newznab",
                "base_url": "https://fixture.invalid", "is_enabled": true,
                "enable_interactive_search": true, "enable_auto_search": false,
                "created_at": now, "updated_at": now
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let mut client = clients.get_by_id("http-client").await.unwrap().unwrap();
    client.configuration.grants.push(IndexerClientGrant {
        indexer_id: "artifact-source".into(),
        categories: None,
    });
    client.revision += 1;
    assert!(clients.save(&client, Some(1)).await.unwrap());
    let reference: IndexerClientArtifactReference = serde_json::from_value(serde_json::json!({
        "client_id": client.id, "key_generation": client.key_generation,
        "protocol": IndexerClientProtocol::Newznab, "indexer_id": "artifact-source",
        "result_identity": "release", "source_url": "https://fixture.invalid/artifact",
        "source_kind": scryer_application::DownloadSourceKind::NzbUrl,
        "categories": [], "info_hash_hint": null
    }))
    .unwrap();
    let cipher = Arc::new(ArtifactReferenceCipher::new(&EncryptionKey::from_bytes(
        [42; 32],
    )));
    let envelope = cipher
        .seal(&serde_json::to_vec(&reference).unwrap())
        .unwrap();
    let download = Arc::new(ArtifactOnly(AtomicUsize::new(0)));
    let app = context.app.clone().with_test_overrides(|builder| {
        builder
            .with_indexer_client_repository(clients)
            .with_download_client(download.clone())
    });
    // There is deliberately no actor or interactive job. The browser-download
    // workflow requires both, so a successful fetch proves this stays on the
    // client's narrow artifact path even after the raw cache's idle lifetime.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    tokio::time::resume();
    let router = nab_api::router(nab_api::NabState {
        app,
        cipher,
        base_path: String::new(),
        tls: false,
    });
    let (status, body) = call(&router, &format!("/newznab/api?t=get&id={envelope}"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "<nzb/>");
    assert_eq!(download.0.load(Ordering::SeqCst), 1);
}
