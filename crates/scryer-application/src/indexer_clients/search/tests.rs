use super::*;
use scryer_domain::IndexerCapsSnapshot;

fn resolved(snapshot: Option<&IndexerCapsSnapshot>) -> crate::ResolvedIndexerCaps {
    crate::ResolvedIndexerCaps {
        snapshot: snapshot.cloned(),
        refresh_error: None,
        source: if snapshot.is_some() {
            crate::IndexerCapsSource::Fresh
        } else {
            crate::IndexerCapsSource::ProviderDeclarations
        },
    }
}

fn request() -> IndexerClientSearchRequest {
    IndexerClientSearchRequest {
        protocol: IndexerClientProtocol::Newznab,
        operation: PluginNabOperation::Search,
        query: "software release".into(),
        parameters: BTreeMap::new(),
        categories: vec!["4000".into(), "6000".into(), "100123".into()],
        offset: 300,
        limit: 100,
    }
}

fn capabilities(exact_nab: bool) -> IndexerProviderCapabilities {
    IndexerProviderCapabilities {
        raw_nab_search: exact_nab,
        query_param: Some("q".into()),
        rss: true,
        protocols: vec![IndexerProtocolCapability::Usenet],
        ..Default::default()
    }
}

#[test]
fn native_json_adapter_uses_existing_raw_query_without_nab_or_aggregate_paging_fields() {
    let request = request();
    let source = source_request(&request, &capabilities(false), &resolved(None)).unwrap();
    assert_eq!(source.query, request.query);
    assert!(source.nab.is_none() && source.limit.is_none() && source.page.is_none());
    assert!(!source.recent_paging);
    for operation in [
        PluginNabOperation::Movie,
        PluginNabOperation::Tv,
        PluginNabOperation::Music,
        PluginNabOperation::Book,
    ] {
        let mut structured = request.clone();
        structured.operation = operation;
        assert!(source_request(&structured, &capabilities(false), &resolved(None)).is_none());
    }
    let mut recent = request.clone();
    recent.query.clear();
    assert!(source_request(&recent, &capabilities(false), &resolved(None)).is_none());
    let mut constrained = request;
    constrained.parameters.insert("imdbid".into(), "123".into());
    assert!(source_request(&constrained, &capabilities(false), &resolved(None)).is_none());
}

#[test]
fn exact_modes_preserve_complete_parameters_and_never_use_a_union_of_different_sources() {
    let mut request = request();
    request.operation = PluginNabOperation::Music;
    request.parameters = BTreeMap::from([
        ("artist".into(), "artist name".into()),
        ("album".into(), "album title".into()),
    ]);
    let node = scryer_domain::IndexerCapsSearchNode {
        available: true,
        supported_params: vec!["q".into(), "artist".into(), "album".into()],
        search_engine: None,
    };
    let mut caps = IndexerCapsSnapshot {
        searching_present: true,
        audio_search: Some(node),
        ..Default::default()
    };
    let source = source_request(&request, &capabilities(true), &resolved(Some(&caps))).unwrap();
    let nab = source.nab.unwrap();
    assert_eq!(nab.operation, PluginNabOperation::Music);
    assert_eq!(nab.parameters, request.parameters);
    assert!(source.limit.is_none());
    caps.audio_search
        .as_mut()
        .unwrap()
        .supported_params
        .retain(|param| param != "album");
    assert!(source_request(&request, &capabilities(true), &resolved(Some(&caps))).is_none());
    caps.audio_search.as_mut().unwrap().supported_params = vec!["q".into(), "album".into()];
    assert!(source_request(&request, &capabilities(true), &resolved(Some(&caps))).is_none());
}

#[test]
fn unknown_caps_attempt_exact_modes_but_explicit_missing_modes_remain_unavailable() {
    let mut request = request();
    let caps = capabilities(true);
    assert!(source_request(&request, &caps, &resolved(None)).is_some());
    request.query.clear();
    assert!(
        source_request(&request, &caps, &resolved(None))
            .unwrap()
            .recent_paging
    );
    let explicit = IndexerCapsSnapshot {
        searching_present: true,
        ..Default::default()
    };
    assert!(source_request(&request, &caps, &resolved(Some(&explicit))).is_none());
    request.operation = PluginNabOperation::Book;
    request
        .parameters
        .insert("author".into(), "Example Author".into());
    for snapshot in [None, Some(IndexerCapsSnapshot::default())] {
        let routed = source_request(&request, &caps, &resolved(snapshot.as_ref())).unwrap();
        let nab = routed.nab.unwrap();
        assert_eq!(nab.operation, PluginNabOperation::Book);
        assert_eq!(nab.parameters, request.parameters);
        assert!(!routed.recent_paging);
    }
}

#[test]
fn typed_searches_reach_the_existing_resolver_before_missing_or_stale_caps_can_refuse_them() {
    let mut request = request();
    request.operation = PluginNabOperation::Tv;
    request.query.clear();
    request.parameters.insert("tvdbid".into(), "12345".into());
    for source in [
        crate::IndexerCapsSource::Permissive,
        crate::IndexerCapsSource::Stale,
    ] {
        let caps = crate::ResolvedIndexerCaps {
            source,
            refresh_error: None,
            snapshot: (source == crate::IndexerCapsSource::Stale).then(|| IndexerCapsSnapshot {
                searching_present: true,
                ..Default::default()
            }),
        };
        let routed = source_request(&request, &capabilities(true), &caps).unwrap();
        let nab = routed.nab.unwrap();
        assert_eq!(nab.operation, PluginNabOperation::Tv);
        assert_eq!(nab.parameters, request.parameters);
        assert!(routed.query.is_empty());
    }
}

#[test]
fn category_intersection_never_broadens_empty_or_custom_grants() {
    let categories = |values: &[&str]| {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(intersect_categories(&[], None), Some(Vec::new()));
    assert_eq!(intersect_categories(&[], Some(&[])), None);
    assert_eq!(
        intersect_categories(&categories(&["4000"]), Some(&categories(&["4010"]))),
        Some(categories(&["4010"]))
    );
    assert_eq!(
        intersect_categories(&categories(&["4010"]), Some(&categories(&["4000"]))),
        Some(categories(&["4010"]))
    );
    assert_eq!(
        intersect_categories(&categories(&["6000"]), Some(&categories(&["4000"]))),
        None
    );
    assert_eq!(
        intersect_categories(&categories(&["100001"]), Some(&categories(&["100000"]))),
        None
    );
    assert_eq!(
        intersect_categories(&[], Some(&categories(&["100123", "6000"]))),
        Some(categories(&["100123", "6000"]))
    );
    assert_eq!(
        intersect_categories(&[], Some(&categories(&["source-string-category"]))),
        None
    );
    assert_eq!(
        normalize_categories(&categories(&["04000", "6000", "4000"])).unwrap(),
        categories(&["4000", "6000"])
    );
    assert_eq!(
        normalize_categories(&categories(&["software"]))
            .unwrap_err()
            .reason(),
        Some(reasons::CATEGORY_INVALID)
    );
}

struct ClientRecords(std::sync::Mutex<IndexerClientRecord>);

#[async_trait::async_trait]
impl IndexerClientRepository for ClientRecords {
    async fn list(&self) -> AppResult<Vec<IndexerClientRecord>> {
        Ok(vec![self.0.lock().unwrap().clone()])
    }
    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerClientRecord>> {
        let record = self.0.lock().unwrap();
        Ok((record.id == id).then(|| record.clone()))
    }
    async fn get_by_lookup_id(&self, _: &str) -> AppResult<Option<IndexerClientRecord>> {
        panic!("fixture begins after key authentication")
    }
    async fn save(&self, _: &IndexerClientRecord, _: Option<u64>) -> AppResult<bool> {
        panic!("search cannot edit client configuration")
    }
    async fn record_use(
        &self,
        _: &str,
        _: u64,
        _: chrono::DateTime<chrono::Utc>,
        _: &str,
    ) -> AppResult<()> {
        Ok(())
    }
}

struct NativeProvider;

impl crate::IndexerPluginProvider for NativeProvider {
    fn available_provider_types(&self) -> Vec<String> {
        vec!["json-fixture".into()]
    }
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        Vec::new()
    }
    fn client_for_provider(
        &self,
        _: &IndexerConfig,
    ) -> Option<std::sync::Arc<dyn crate::IndexerClient>> {
        panic!("the shared indexer client owns provider dispatch")
    }
    fn capabilities_for_provider(&self, _: &str) -> IndexerProviderCapabilities {
        capabilities(false)
    }
}

struct ControlledSource(
    tokio::sync::mpsc::UnboundedSender<(RawTextSearchRequest, tokio::sync::oneshot::Sender<()>)>,
);

#[async_trait::async_trait]
impl crate::IndexerClient for ControlledSource {
    async fn search_raw_text(
        &self,
        request: RawTextSearchRequest,
        _: tokio_util::sync::CancellationToken,
    ) -> AppResult<crate::IndexerSearchResponse> {
        let (send, receive) = tokio::sync::oneshot::channel();
        self.0.send((request, send)).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(120), receive)
            .await
            .unwrap()
            .unwrap();
        Ok(crate::IndexerSearchResponse {
            results: Vec::new(),
            completion: crate::IndexerSearchCompletion::Complete,
            next_cursor: None,
            indexer_outcomes: Vec::new(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[tokio::test(start_paused = true)]
async fn client_search_uses_shared_raw_dispatch_and_rechecks_grants_before_delivery() {
    use std::{sync::Arc, time::Duration};
    for grants_change_during_read in [false, true] {
        let (mut app, _) = crate::lib_tests::bootstrap();
        let now = app.runtime.environment.now();
        let config = IndexerConfig {
            id: "json-source".into(),
            name: "JSON source".into(),
            provider_type: "json-fixture".into(),
            base_url: "https://indexer.example".into(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
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
            last_health_status: None,
            last_error_message: None,
            last_error_code: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let before_refresh = search_identity(&config, &NativeProvider);
        let mut refreshed = config.clone();
        refreshed.caps_snapshot_json = Some(
            serde_json::json!({
                "searching_present": true,
                "tv_search": {"available": true, "supported_params": ["tvdbid"]}
            })
            .to_string(),
        );
        assert_eq!(before_refresh, search_identity(&refreshed, &NativeProvider));
        refreshed.base_url = "https://different-indexer.example".into();
        assert_ne!(before_refresh, search_identity(&refreshed, &NativeProvider));
        app.services
            .integrations
            .indexer_configs
            .create(config)
            .await
            .unwrap();
        let authenticated = AuthenticatedIndexerClient {
            client: IndexerClientRecord {
                id: "tool".into(),
                lookup_id: Some("lookup".into()),
                secret_hash: Some("verified-at-entry".into()),
                key_generation: 1,
                revision: 1,
                last_used_at: None,
                last_outcome_code: None,
                created_at: now,
                updated_at: now,
                configuration: IndexerClientConfiguration {
                    name: "Tool".into(),
                    enabled: true,
                    protocols: vec![IndexerClientProtocol::Newznab],
                    grants: vec![IndexerClientGrant {
                        indexer_id: "json-source".into(),
                        categories: None,
                    }],
                },
            },
        };
        let records = Arc::new(ClientRecords(std::sync::Mutex::new(
            authenticated.client.clone(),
        )));
        app.services.integrations.indexer_clients = crate::RuntimeFeature::Enabled(records.clone());
        app.services.integrations.plugin_provider =
            crate::RuntimeFeature::Enabled(Arc::new(NativeProvider));
        if !grants_change_during_read {
            for (grant, requested, expected) in [
                (
                    Some(vec!["4000".into()]),
                    vec!["4010".into()],
                    Some(vec!["4010".into()]),
                ),
                (
                    Some(vec!["4010".into()]),
                    vec!["4000".into()],
                    Some(vec!["4010".into()]),
                ),
                (None, Vec::new(), None),
                (None, vec!["100123".into()], Some(vec!["100123".into()])),
            ] {
                let mut principal = authenticated.clone();
                principal.client.configuration.grants[0].categories = grant;
                let mut query = request();
                query.categories = requested;
                let resolved = app
                    .resolve_indexer_client_search(&principal, &query)
                    .await
                    .unwrap();
                assert_eq!(resolved.sources.len(), 1);
                assert_eq!(resolved.sources[0].result_scope.categories, expected);
            }
            let mut principal = authenticated.clone();
            principal.client.configuration.grants[0].categories = Some(Vec::new());
            assert!(
                app.resolve_indexer_client_search(&principal, &request())
                    .await
                    .unwrap()
                    .sources
                    .is_empty()
            );
        }
        let (send, mut reads) = tokio::sync::mpsc::unbounded_channel();
        app.services.integrations.indexer_client = Arc::new(ControlledSource(send));
        let worker = app.clone();
        let principal = authenticated.clone();
        let mut search = request();
        search.offset = 0;
        let next_request = search.clone();
        let task =
            tokio::spawn(async move { worker.search_indexer_client(&principal, search).await });
        let (source_request, answer) = tokio::time::timeout(Duration::from_secs(60), reads.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            source_request.indexer_ids.unwrap(),
            std::collections::HashSet::from(["json-source".into()])
        );
        assert_eq!(source_request.categories, ["100123", "4000", "6000"]);
        assert!(source_request.nab.is_none() && source_request.limit.is_none());
        if grants_change_during_read {
            records.0.lock().unwrap().configuration.grants.clear();
        }
        answer.send(()).unwrap();
        let response = tokio::time::timeout(Duration::from_secs(90), task)
            .await
            .unwrap()
            .unwrap();
        if grants_change_during_read {
            assert_eq!(response.unwrap_err().reason(), Some(reasons::KEY_INVALID));
        } else {
            assert!(response.unwrap().exhausted);
            let mut next = next_request;
            next.offset = 100;
            assert!(
                app.search_indexer_client(&authenticated, next)
                    .await
                    .unwrap()
                    .exhausted
            );
            assert!(
                reads.try_recv().is_err(),
                "an offset read reuses the shared generation"
            );
        }
    }
}
