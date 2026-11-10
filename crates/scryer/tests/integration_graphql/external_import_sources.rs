use super::*;
use scryer_application::source_import::{
    SourceImportInput, SourceImportSelection, SourceImportState,
};
use scryer_application::{
    IndexerClient, IndexerManagementClient, IndexerPluginProvider, IndexerValidationResult,
};
use scryer_domain::{
    ConfigFieldDef, IndexerConfig, IndexerManagementCapabilities, NewIndexerConfig,
};
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn source_import_retry_persists_the_correct_download_credential_field() {
    use scryer_application::{
        JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource,
    };
    use scryer_domain::NewDownloadClientConfig;
    use scryer_infrastructure_workflow::workflow::stores::WorkflowOperationStore;
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new().await;
        let actor = ctx.users.create(User::new_admin("source-client-correction")).await.expect("operator");
        let store = WorkflowOperationStore::new(ctx.db.datastore()).with_encryption_key(ctx.db.encryption_key_state());
        for (client_type, expected_field, preserved_field, preserved_value) in [("qbittorrent","password","api_key","previous-api-key"),("nzbget","password","api_key","previous-api-key"),("sabnzbd","api_key","password","previous-password"),("weaver","api_key","password","previous-password")] {
            let key = format!("credential-fixture:{client_type}");
            let config_id = scryer_domain::Id::new().0;
            // An independently invalid name keeps this fixture at the correction/persistence boundary, before client I/O.
            let selected = SourceImportSelection { key:key.clone(),name:"Credential fixture".into(),input:SourceImportInput::DownloadClient(NewDownloadClientConfig { name:String::new(),client_type:client_type.into(),config_json:json!({"api_key":"previous-api-key","password":"previous-password"}).to_string(),client_priority:0,is_enabled:true,proxy_config_id:None }) };
            let now = chrono::Utc::now();
            let run = JobRunRecord { id:scryer_domain::Id::new().0,job_key:JobKey::SourceImport,operation_type:"source_import".into(),status:JobRunStatus::Failed,trigger_source:JobTriggerSource::Manual,actor_user_id:Some(actor.id.clone()),progress_json:Some(json!({"items":[{"key":key,"name":"Credential fixture","config_id":config_id,"kind":"download_client","state":"failed","reason":"SOURCE_IMPORT_ITEM_FAILED"}]}).to_string()),summary_json:None,summary_text:None,error_text:None,started_at:now,completed_at:Some(now),created_at:now,updated_at:now };
            store.accept_private_job(&run,&format!("source-import:correction-{client_type}"),&json!([{"selection":selected,"config_id":config_id,"reuse_existing":false}]).to_string()).await.expect("failed receipt");
            ctx.app.retry_source_import(&actor,&run.id,vec![(key,"corrected-synthetic-secret".into())]).await.expect("credential correction accepted");
            ctx.app.wait_source_import(&actor,&run.id).await.expect("attempt settles without client I/O");
            let private: Value = serde_json::from_str(&store.private_job_input(&run.id,&actor.id).await.expect("private input").expect("owned input")).expect("private JSON");
            let config: Value = serde_json::from_str(private[0]["selection"]["input"]["DownloadClient"]["config_json"].as_str().expect("client config")).expect("config JSON");
            assert_eq!(config[expected_field],"corrected-synthetic-secret","{client_type} correction field");
            assert_eq!(config[preserved_field],preserved_value,"unrelated credential is preserved");
            assert!(!ctx.app.source_import_status(&actor,&run.id).await.expect("public status").0.progress_json.expect("progress").contains("corrected-synthetic-secret"));
        }
    }).await.expect("credential correction scenario failure bound");
}

#[tokio::test]
async fn source_import_plugin_install_is_single_flight_and_retry_preserves_successful_siblings() {
    use scryer_application::PluginInstallationRepository;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new().await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx.users.create(User::new_admin("source-install-operator")).await.expect("operator");
        let server = MockServer::start().await;
        // The real destination policy rejects loopback before fetching artifacts or signatures.
        Mock::given(method("GET")).and(path("/source-plugin.wasm.zst")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1,2,3,4])).expect(0).mount(&server).await;
        Mock::given(method("GET")).and(path("/source-plugin.bundle.json")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(0).mount(&server).await;
        let digest = format!("blake3:{}", "0".repeat(64));
        let catalog = json!({"schema_version":"scryer.plugin.catalog.v3","catalog_version":1,"rule_packs":[],"plugins":[{
            "id":"source-install-fixture","name":"Source install fixture","description":"Isolated source installation fixture","plugin_type":"usenet_indexer","provider_type":"source-install-fixture","publisher":"scryer","support_tier":"official","status":"active","docs_url":"https://example.test/source-plugin","source_repo":"https://github.com/scryer-media/source-install-fixture","required_signer":{"github_repository":"scryer-media/source-install-fixture"},
            "releases":[{"version":"1.0.0","sdk_constraint":scryer_plugins::sdk_constraint_or_legacy(scryer_plugins::SDK_VERSION, ""),"artifacts":[{"runtime":"wasm32-wasip2","required_features":[],"url":format!("{}/source-plugin.wasm.zst",server.uri()),"mirror_urls":[],"signature_url":format!("{}/source-plugin.bundle.json",server.uri()),"signature_mirror_urls":[],"digests":[digest],"wasm_digests":[digest],"bytes":4}]}]
        }]}).to_string();
        let now = chrono::Utc::now();
        ctx.customization.upsert_plugin_catalog_source(&scryer_domain::PluginCatalogSource { source_key:"__central_catalog".into(),source_kind:"central".into(),source_url:format!("{}/catalog.json",server.uri()),github_repo:Some("scryer-media/scryer-plugins".into()),support_tier:scryer_domain::PluginSupportTier::Official,catalog_json:Some(catalog),last_success_at:Some(now),last_error:None,updated_at:now }).await.expect("seed official catalog fixture");
        let mut selections: Vec<_> = (0..15).map(|index| {
            let mut item = selection(index, false);
            item.key = format!("source-install-fixture:{index}");
            let SourceImportInput::Indexer(input) = &mut item.input else { unreachable!() };
            input.provider_type = "source-install-fixture".into();
            item
        }).collect();
        let sibling = selection(50, false);
        selections.push(sibling.clone());
        let generation = ctx.plugin_install_orchestrator.operation_generation().await;
        let accepted = ctx.app.start_source_import(&actor,"source-install-dedup",selections).await.expect("accepted");
        let progress = ctx.app.wait_source_import(&actor,&accepted.id).await.expect("first attempt");
        assert_eq!(progress.items.iter().filter(|item|item.state == SourceImportState::Failed).count(),15);
        let successful = progress.items.iter().find(|item|item.key == sibling.key).expect("successful sibling");
        assert_eq!(successful.state,SourceImportState::Succeeded);
        let sibling_id = successful.config_id.clone();
        assert_eq!(ctx.plugin_install_orchestrator.operation_generation().await,generation+1,"one actual install attempt per missing provider");
        for item in progress.items.iter().filter(|item|item.key != sibling.key) {
            assert_eq!(item.reason.as_deref(),Some("PLUGIN_ARTIFACT_DESTINATION_FORBIDDEN"),"typed refusal for {}: {:?}",item.key,item.message);
        }
        assert!(server.received_requests().await.expect("install requests").is_empty());
        ctx.app.retry_source_import(&actor,&accepted.id,Vec::new()).await.expect("explicit retry");
        let retried = ctx.app.wait_source_import(&actor,&accepted.id).await.expect("retry settles");
        assert_eq!(ctx.plugin_install_orchestrator.operation_generation().await,generation+2,"explicit retry makes exactly one new provider install attempt");
        let successful = retried.items.iter().find(|item|item.key == sibling.key).expect("successful sibling retained");
        assert_eq!(successful.config_id,sibling_id);
        assert_eq!(successful.state,SourceImportState::Succeeded);
        assert_eq!(ctx.app.list_indexer_configs(&actor,Some("newznab".into())).await.expect("stored sibling").iter().filter(|config|config.name == sibling.name).count(),1);
        assert!(ctx.app.list_indexer_configs(&actor,Some("source-install-fixture".into())).await.expect("failed provider configs").is_empty());
        assert!(ctx.customization.get_plugin_installation("source-install-fixture").await.expect("installation lookup").is_none(),"rejected destination never creates an installation");
        assert!(server.received_requests().await.expect("retry install requests").is_empty());
        server.verify().await;
    }).await.expect("installation single-flight scenario failure bound");
}

#[tokio::test]
async fn source_import_shutdown_stops_queued_indexers_and_preserves_the_receipt() {
    timeout(Duration::from_secs(180), async {
        let (events, mut observed) = mpsc::unbounded_channel();
        let releases = (0..16).map(|_| Arc::new(Semaphore::new(0))).collect();
        let ctx = TestContext::new_with_indexer_provider(Arc::new(BarrierProvider {
            events,
            releases,
            base: Arc::new(scryer_plugins::build_indexer_plugin_provider(&[], &[])),
        }))
        .await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx
            .users
            .create(User::new_admin("source-shutdown-operator"))
            .await
            .expect("operator");
        let selections: Vec<_> = (0..16).map(|index| selection(index, true)).collect();
        let accepted = ctx
            .app
            .start_source_import(&actor, "source-shutdown", selections.clone())
            .await
            .expect("accepted");
        for _ in 0..15 {
            observed.recv().await.expect("fifteen active probes");
        }
        ctx.app.restore_coordinator().shutdown_workers.cancel();
        let progress = ctx
            .app
            .wait_source_import(&actor, &accepted.id)
            .await
            .expect("shutdown settles source job");
        assert!(
            progress
                .items
                .iter()
                .all(|item| item.state == SourceImportState::Interrupted)
        );
        assert!(
            observed.try_recv().is_err(),
            "shutdown never starts the queued sixteenth indexer"
        );
        assert_eq!(
            ctx.app
                .start_source_import(&actor, "source-shutdown", selections)
                .await
                .expect("accepted receipt remains idempotent during shutdown")
                .id,
            accepted.id
        );
        assert_eq!(
            ctx.app
                .start_source_import(&actor, "source-after-shutdown", vec![selection(20, false)])
                .await
                .expect_err("new scheduling refused")
                .validation_reason(),
            Some(scryer_application::source_import::refusal::INTERRUPTED)
        );
    })
    .await
    .expect("shutdown scenario failure bound");
}

#[tokio::test]
async fn source_import_missing_download_credential_is_accepted_and_fails_only_that_source() {
    use scryer_application::external_import::ArrDownloadClient;
    use scryer_application::{
        ExternalImportArrSourceKind, ExternalImportArrSourceWarmupResult,
        ExternalImportMonitorWarmupPhase, ExternalImportMonitorWarmupStatus,
    };
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new().await;
        let actor = ctx.app.find_or_create_default_user().await.expect("HTTP operator");
        let begin = ctx.app.begin_external_import_monitor_warmup(&actor, "source-credential-fixture").await.expect("source discovery");
        let id = begin.snapshot.session_id.clone();
        ctx.app.set_external_import_arr_source_warmup_result(&id, ExternalImportArrSourceWarmupResult {
            source_key: "credential-source".into(), kind: ExternalImportArrSourceKind::Radarr, base_url: "http://source-fixture.invalid".into(), version: None, root_folders: Vec::new(), title_root_paths: Vec::new(), naming_config: None, media_management_config: None, metadata_providers: Vec::new(), quality_profiles: Vec::new(), signal_warnings: Vec::new(), indexers: Vec::new(),
            download_clients: vec![ArrDownloadClient { id: 1, name: "Masked download client".into(), implementation: "Sabnzbd".into(), fields: std::collections::HashMap::from([("host".into(), json!("download-fixture.invalid")),("port".into(),json!(8080)),("apiKey".into(),json!("********"))]) }],
        }).await;
        let mut snapshot = begin.snapshot;
        snapshot.status = ExternalImportMonitorWarmupStatus::Completed; snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
        ctx.app.update_external_import_monitor_warmup_progress(&id, snapshot).await;
        let key = "sabnzbd:download-fixture.invalid:8080";
        let query = r#"mutation($input: ExecuteExternalImportInput!) { startExternalImportSources(requestKey:"missing-credential",input:$input) { runId } }"#;
        let input = json!({"sourceWarmupSessionIds":[id],"selectedDownloadClientDedupKeys":[key],"selectedIndexerDedupKeys":[],"downloadClientApiKeyOverrides":[],"downloadClientPasswordOverrides":[],"indexerApiKeyOverrides":[]});
        let body = gql(&ctx, query, json!({"input":input})).await;
        assert_no_errors(&body);
        let run_id = body["data"]["startExternalImportSources"]["runId"].as_str().expect("accepted run").to_owned();
        let progress = ctx.app.wait_source_import(&actor, &run_id).await.expect("run settles without client I/O");
        assert_eq!(progress.items.len(), 1);
        assert_eq!(progress.items[0].state, SourceImportState::Failed);
        assert_eq!(progress.items[0].reason.as_deref(), Some(scryer_application::source_import::refusal::CREDENTIAL_REQUIRED));
        assert!(ctx.app.list_download_client_configs(&actor, None).await.expect("clients").is_empty(), "a source without its credential is never saved");
        // Retrying without a correction fails the same way; the item stays retryable.
        ctx.app.retry_source_import(&actor, &run_id, Vec::new()).await.expect("retry accepted");
        let retried = ctx.app.wait_source_import(&actor, &run_id).await.expect("retry settles");
        assert_eq!(retried.items[0].reason.as_deref(), Some(scryer_application::source_import::refusal::CREDENTIAL_REQUIRED));
        ctx.app.skip_source_import_item(&actor, &run_id, key).await.expect("explicit skip");
        ctx.app.require_source_import_resolved(&actor, &run_id).await.expect("skipped source no longer blocks setup");
    }).await.expect("credential scenario failure bound");
}

#[tokio::test]
async fn source_import_batch_with_a_missing_credential_imports_the_other_sources() {
    use scryer_domain::NewDownloadClientConfig;
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new().await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx
            .users
            .create(User::new_admin("source-mixed-credential"))
            .await
            .expect("operator");
        let sibling = selection(0, false);
        let missing = SourceImportSelection {
            key: "nzbget:download-fixture.invalid:6789".into(),
            name: "Passwordless fixture".into(),
            input: SourceImportInput::DownloadClient(NewDownloadClientConfig {
                name: "Passwordless fixture".into(),
                client_type: "nzbget".into(),
                config_json: json!({"host":"download-fixture.invalid","port":"6789"}).to_string(),
                client_priority: 0,
                is_enabled: true,
                proxy_config_id: None,
            }),
        };
        let accepted = ctx
            .app
            .start_source_import(
                &actor,
                "source-mixed-credential",
                vec![sibling.clone(), missing.clone()],
            )
            .await
            .expect("batch accepted");
        let progress = ctx
            .app
            .wait_source_import(&actor, &accepted.id)
            .await
            .expect("batch settles");
        let state = |key: &str| {
            progress
                .items
                .iter()
                .find(|item| item.key == key)
                .expect("item receipt")
                .clone()
        };
        assert_eq!(state(&sibling.key).state, SourceImportState::Succeeded);
        let failed = state(&missing.key);
        assert_eq!(failed.state, SourceImportState::Failed);
        assert_eq!(
            failed.reason.as_deref(),
            Some(scryer_application::source_import::refusal::CREDENTIAL_REQUIRED)
        );
        assert_eq!(
            ctx.app
                .list_indexer_configs(&actor, Some("newznab".into()))
                .await
                .expect("configs")
                .iter()
                .filter(|config| config.name == sibling.name)
                .count(),
            1
        );
    })
    .await
    .expect("mixed credential scenario failure bound");
}

#[tokio::test]
async fn source_import_prowlarr_parent_without_its_key_fails_alone_and_retries_with_a_correction() {
    timeout(Duration::from_secs(180), async {
        // The parent's provider is installed, as in production, so the
        // credential gate is what fails the item rather than a plugin install.
        let ctx = TestContext::new_with_indexer_provider(Arc::new(ParentProvider {
            base: Arc::new(scryer_plugins::build_indexer_plugin_provider(&[], &[])),
        }))
        .await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx
            .users
            .create(User::new_admin("source-parent-credential"))
            .await
            .expect("operator");
        let sibling = selection(0, false);
        let parent_name = "Keyless parent fixture".to_owned();
        let parent = SourceImportSelection {
            key: "prowlarr:https://parent-fixture.example.test".into(),
            name: parent_name.clone(),
            input: SourceImportInput::Indexer(NewIndexerConfig {
                name: parent_name.clone(),
                provider_type: "prowlarr".into(),
                config_json: Some(
                    json!({"base_url":"https://parent-fixture.example.test"}).to_string(),
                ),
                rate_limit_seconds: None,
                rate_limit_burst: None,
                max_queries_per_minute: None,
                is_enabled: false,
                enable_interactive_search: true,
                enable_auto_search: true,
                proxy_config_id: None,
                download_client_id: None,
            }),
        };
        let accepted = ctx
            .app
            .start_source_import(
                &actor,
                "source-parent-credential",
                vec![sibling.clone(), parent.clone()],
            )
            .await
            .expect("batch accepted");
        let progress = ctx
            .app
            .wait_source_import(&actor, &accepted.id)
            .await
            .expect("batch settles");
        let item = |progress: &scryer_application::source_import::SourceImportProgress,
                    key: &str| {
            progress
                .items
                .iter()
                .find(|item| item.key == key)
                .expect("item receipt")
                .clone()
        };
        assert_eq!(
            item(&progress, &sibling.key).state,
            SourceImportState::Succeeded
        );
        let failed = item(&progress, &parent.key);
        assert_eq!(failed.state, SourceImportState::Failed);
        assert_eq!(
            failed.reason.as_deref(),
            Some(scryer_application::source_import::refusal::CREDENTIAL_REQUIRED)
        );
        assert!(
            ctx.app
                .list_indexer_configs(&actor, Some("prowlarr".into()))
                .await
                .expect("parent configs")
                .is_empty(),
            "a parent without its key is never saved"
        );
        // A correction supplies the key: the refusal clears and the item moves
        // past the credential gate on retry.
        ctx.app
            .retry_source_import(
                &actor,
                &accepted.id,
                vec![(parent.key.clone(), "corrected-synthetic-secret".into())],
            )
            .await
            .expect("correction accepted");
        let retried = ctx
            .app
            .wait_source_import(&actor, &accepted.id)
            .await
            .expect("retry settles");
        assert_eq!(
            item(&retried, &sibling.key).state,
            SourceImportState::Succeeded
        );
        assert_ne!(
            item(&retried, &parent.key).reason.as_deref(),
            Some(scryer_application::source_import::refusal::CREDENTIAL_REQUIRED),
            "the corrected key clears the credential refusal"
        );
    })
    .await
    .expect("parent credential scenario failure bound");
}

/// The base provider with a Prowlarr parent type available and no live client.
struct ParentProvider {
    base: Arc<dyn IndexerPluginProvider>,
}
impl IndexerPluginProvider for ParentProvider {
    fn client_for_provider(&self, _: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        None
    }
    fn available_provider_types(&self) -> Vec<String> {
        vec!["newznab".into(), "prowlarr".into()]
    }
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        Vec::new()
    }
    fn config_fields_for_provider(&self, provider: &str) -> Vec<ConfigFieldDef> {
        self.base.config_fields_for_provider(provider)
    }
    fn default_base_url_for_provider(&self, provider: &str) -> Option<String> {
        self.base.default_base_url_for_provider(provider)
    }
}

struct AutoDefaultProvider {
    base: Arc<dyn IndexerPluginProvider>,
}
impl IndexerPluginProvider for AutoDefaultProvider {
    fn client_for_provider(&self, _: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        None
    }
    fn available_provider_types(&self) -> Vec<String> {
        vec!["newznab".into()]
    }
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        Vec::new()
    }
    fn config_fields_for_provider(&self, provider: &str) -> Vec<ConfigFieldDef> {
        self.base
            .config_fields_for_provider(provider)
            .into_iter()
            .map(|mut field| {
                field.required = false;
                field
            })
            .collect()
    }
    fn default_base_url_for_provider(&self, _: &str) -> Option<String> {
        Some("https://default-indexer.example.test".into())
    }
}

#[tokio::test]
async fn source_import_reservation_survives_restart_before_plugin_default_creation() {
    use scryer_application::{
        JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource,
    };
    use scryer_infrastructure_workflow::workflow::stores::WorkflowOperationStore;
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new_with_indexer_provider(Arc::new(AutoDefaultProvider { base: Arc::new(scryer_plugins::build_indexer_plugin_provider(&[], &[])) })).await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx.users.create(User::new_admin("source-plugin-recovery")).await.expect("operator");
        let selected = selection(0, false);
        let config_id = scryer_domain::Id::new().0;
        let now = chrono::Utc::now();
        let run = JobRunRecord { id: scryer_domain::Id::new().0, job_key: JobKey::SourceImport, operation_type: "source_import".into(), status: JobRunStatus::Running, trigger_source: JobTriggerSource::Manual, actor_user_id: Some(actor.id.clone()),
            progress_json: Some(json!({"items":[{"key":selected.key,"name":selected.name,"config_id":config_id,"kind":"indexer","state":"installing","reason":null}]}).to_string()), summary_json: None, summary_text: None, error_text: None, started_at: now, completed_at: None, created_at: now, updated_at: now };
        let store = WorkflowOperationStore::new(ctx.db.datastore()).with_encryption_key(ctx.db.encryption_key_state());
        store.accept_private_job(&run, "source-import:plugin-restart", &json!([{"selection":selected,"config_id":config_id,"reuse_existing":false}]).to_string()).await.expect("accepted reservation before plugin installation completed");
        store.reconcile_interrupted_job_runs(&[]).await.expect("restart after provider became available");
        ctx.app.reconcile_indexer_configs().await.expect("startup provider reconciliation");
        assert!(ctx.app.list_indexer_configs(&actor, Some("newznab".into())).await.expect("configs").is_empty(), "a pending import owns its reserved configuration; startup must not create a second default");
        ctx.app.skip_source_import_item(&actor, &run.id, &selected.key).await.expect("explicitly skip interrupted source");
        ctx.app.reconcile_indexer_configs().await.expect("normal defaults after reservation is resolved");
        assert_eq!(ctx.app.list_indexer_configs(&actor, Some("newznab".into())).await.expect("configs").len(), 1);
    }).await.expect("plugin restart scenario failure bound");
}

#[tokio::test]
async fn source_import_graphql_recovers_lost_acceptance_after_warmup_is_gone() {
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new().await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx.users.create(User::new_admin("source-refresh-operator")).await.expect("operator");
        let selected = selection(0, false);
        let accepted = ctx.app.start_source_import(&actor, "source-refresh", vec![selected.clone()]).await.expect("accepted");
        ctx.app.wait_source_import(&actor, &accepted.id).await.expect("source completes");
        let query = r#"mutation($requestKey: String!, $input: ExecuteExternalImportInput!) { startExternalImportSources(requestKey:$requestKey,input:$input) { runId ready items { key state reason message } } }"#;
        let input = json!({"sourceWarmupSessionIds":["expired-source-session"],"selectedDownloadClientDedupKeys":[],"selectedIndexerDedupKeys":[selected.key],"downloadClientApiKeyOverrides":[],"downloadClientPasswordOverrides":[],"indexerApiKeyOverrides":[]});
        let response = ctx.schema.execute(async_graphql::Request::new(query).variables(async_graphql::Variables::from_json(json!({"requestKey":"source-refresh","input":input}))).data(actor.clone())).await;
        assert!(response.errors.is_empty(), "accepted job recovery must not read expired warmups: {:?}", response.errors);
        let data = response.data.into_json().expect("response data");
        assert_eq!(data["startExternalImportSources"]["runId"], accepted.id);
        assert_eq!(data["startExternalImportSources"]["ready"], true);
        assert!(!data.to_string().contains("synthetic-private-source-key"));
        assert_eq!(ctx.app.list_indexer_configs(&actor, Some("newznab".into())).await.expect("configs").iter().filter(|config| config.name == selected.name).count(), 1);
    }).await.expect("source recovery scenario failure bound");
}

struct BarrierProvider {
    events: mpsc::UnboundedSender<usize>,
    releases: Vec<Arc<Semaphore>>,
    base: Arc<dyn IndexerPluginProvider>,
}
struct BarrierClient {
    index: usize,
    events: mpsc::UnboundedSender<usize>,
    release: Arc<Semaphore>,
}
#[async_trait::async_trait]
impl IndexerManagementClient for BarrierClient {
    async fn validate_connection(&self) -> scryer_application::AppResult<IndexerValidationResult> {
        self.events
            .send(self.index)
            .expect("probe observer is alive");
        timeout(Duration::from_secs(120), self.release.acquire())
            .await
            .expect("probe released")
            .expect("release semaphore remains open")
            .forget();
        Ok(IndexerValidationResult {
            status: "valid".into(),
            message: None,
            ..Default::default()
        })
    }
    fn name(&self) -> &str {
        "source-import-fixture"
    }
}
impl IndexerPluginProvider for BarrierProvider {
    fn client_for_provider(&self, _: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        None
    }
    fn available_provider_types(&self) -> Vec<String> {
        vec!["newznab".into()]
    }
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        Vec::new()
    }
    fn config_fields_for_provider(&self, provider: &str) -> Vec<ConfigFieldDef> {
        self.base.config_fields_for_provider(provider)
    }
    fn management_capabilities_for_provider(&self, _: &str) -> IndexerManagementCapabilities {
        IndexerManagementCapabilities {
            supports_validate_config: true,
            ..Default::default()
        }
    }
    fn management_client_for_provider(
        &self,
        config: &IndexerConfig,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        let index = config
            .base_url
            .strip_prefix("https://source-")?
            .split('.')
            .next()?
            .parse::<usize>()
            .ok()?;
        Some(Arc::new(BarrierClient {
            index,
            events: self.events.clone(),
            release: self.releases.get(index)?.clone(),
        }))
    }
}

fn selection(index: usize, enabled: bool) -> SourceImportSelection {
    let name = format!("Source {index}");
    SourceImportSelection { key: format!("newznab:https://source-{index}.example.test"), name: name.clone(), input: SourceImportInput::Indexer(NewIndexerConfig {
        name, provider_type: "newznab".into(), config_json: Some(json!({"base_url":format!("https://source-{index}.example.test"),"api_key":"synthetic-private-source-key"}).to_string()),
        rate_limit_seconds: None, rate_limit_burst: None, max_queries_per_minute: None, is_enabled: enabled, enable_interactive_search: true, enable_auto_search: true, proxy_config_id: None, download_client_id: None,
    }) }
}

#[tokio::test]
async fn source_import_accepts_before_probe_and_runs_fifteen_workers_with_safe_routing() {
    timeout(Duration::from_secs(180), async {
        let (events, mut observed) = mpsc::unbounded_channel();
        let releases: Vec<_> = (0..16).map(|_| Arc::new(Semaphore::new(0))).collect();
        let provider = Arc::new(BarrierProvider {
            events,
            releases: releases.clone(),
            base: Arc::new(scryer_plugins::build_indexer_plugin_provider(&[], &[])),
        });
        let ctx = TestContext::new_with_indexer_provider(provider).await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx
            .users
            .create(User::new_admin("source-import-operator"))
            .await
            .expect("operator");
        let selections: Vec<_> = (0..16).map(|index| selection(index, true)).collect();
        let accepted = timeout(
            Duration::from_secs(120),
            ctx.app
                .start_source_import(&actor, "source-worker-barrier", selections.clone()),
        )
        .await
        .expect("acceptance does not wait for probes")
        .expect("accepted");
        let duplicate = ctx
            .app
            .start_source_import(&actor, "source-worker-barrier", selections)
            .await
            .expect("idempotent acceptance");
        assert_eq!(duplicate.id, accepted.id);
        let mut entered = std::collections::HashSet::new();
        for _ in 0..15 {
            entered.insert(
                timeout(Duration::from_secs(120), observed.recv())
                    .await
                    .expect("all fifteen probes enter")
                    .expect("probe event"),
            );
        }
        assert_eq!(entered.len(), 15);
        assert!(observed.try_recv().is_err(), "sixteenth probe is queued");
        let slow = *entered.iter().min().expect("entered probes");
        for index in entered.iter().copied().filter(|index| *index != slow) {
            releases[index].add_permits(1);
        }
        let last = timeout(Duration::from_secs(120), observed.recv())
            .await
            .expect("independent workers continue while one probe is blocked")
            .expect("sixteenth probe");
        assert!(!entered.contains(&last));
        releases[last].add_permits(1);
        releases[slow].add_permits(1);
        let progress = timeout(
            Duration::from_secs(120),
            ctx.app.wait_source_import(&actor, &accepted.id),
        )
        .await
        .expect("source job completes")
        .expect("progress");
        assert_eq!(
            progress
                .items
                .iter()
                .filter(|item| item.state == SourceImportState::Succeeded)
                .count(),
            16
        );
        let configs = ctx
            .app
            .list_indexer_configs(&actor, None)
            .await
            .expect("stored indexers");
        for scope in ["movie", "series", "anime"] {
            let routing = ctx
                .app
                .get_indexer_routing(&actor, scope)
                .await
                .expect("routing");
            for item in &progress.items {
                assert!(configs.iter().any(|config| config.id == item.config_id));
                assert!(
                    routing
                        .iter()
                        .any(|route| route.indexer_id == item.config_id),
                    "every saved source retains routing"
                );
            }
        }
        let row: (Option<String>,) =
            sqlx::query_as("SELECT input_encrypted FROM workflow_private_inputs WHERE run_id = ?")
                .bind(&accepted.id)
                .fetch_one(&ctx.db.pool)
                .await
                .expect("request receipt");
        assert_eq!(row.0, None, "completed imports purge their private input");
        let run = ctx
            .app
            .source_import_status(&actor, &accepted.id)
            .await
            .expect("final status")
            .0;
        assert!(
            !run.progress_json
                .expect("progress")
                .contains("synthetic-private-source-key")
        );
        let other = ctx
            .users
            .create(User::new_admin("other-source-operator"))
            .await
            .expect("other operator");
        assert_eq!(
            ctx.app
                .source_import_status(&other, &accepted.id)
                .await
                .err()
                .expect("ownership refusal")
                .validation_reason(),
            Some(scryer_application::source_import::refusal::NOT_OWNED)
        );
    })
    .await
    .expect("source import scenario completes within its failure bound");
}

#[tokio::test]
async fn source_import_failure_can_be_skipped_and_backend_finish_is_gated() {
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new().await;
        seed_typed_settings_definitions(&ctx).await;
        ctx.settings_store
            .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
                category: "system".into(),
                scope: "system".into(),
                key_name: scryer_application::SETUP_COMPLETE_KEY.into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            }])
            .await
            .expect("setup completion definition");
        let actor = ctx
            .users
            .create(User::new_admin("source-failure-operator"))
            .await
            .expect("operator");
        let mut invalid = selection(0, false);
        let SourceImportInput::Indexer(input) = &mut invalid.input else {
            unreachable!()
        };
        input.name.clear();
        let accepted = ctx
            .app
            .start_source_import(&actor, "source-failure", vec![invalid])
            .await
            .expect("accepted");
        let progress = timeout(
            Duration::from_secs(120),
            ctx.app.wait_source_import(&actor, &accepted.id),
        )
        .await
        .expect("failure settles")
        .expect("progress");
        assert_eq!(progress.items[0].state, SourceImportState::Failed);
        assert_eq!(
            ctx.app
                .complete_setup_for_source_import(&actor, Some(&accepted.id))
                .await
                .expect_err("finish refused")
                .validation_reason(),
            Some(scryer_application::source_import::refusal::UNRESOLVED)
        );
        assert!(
            ctx.app
                .complete_setup_for_source_import(&actor, None)
                .await
                .expect("unrelated manual setup remains compatible")
        );
        let other = ctx
            .users
            .create(User::new_admin("source-finish-other"))
            .await
            .expect("other operator");
        assert_eq!(
            ctx.app
                .complete_setup_for_source_import(&other, Some(&accepted.id))
                .await
                .expect_err("wrong owner refused")
                .validation_reason(),
            Some(scryer_application::source_import::refusal::NOT_OWNED)
        );
        let skipped = ctx
            .app
            .skip_source_import_item(&actor, &accepted.id, &progress.items[0].key)
            .await
            .expect("explicit skip");
        let summary: Value =
            serde_json::from_str(skipped.summary_json.as_deref().expect("updated summary"))
                .expect("summary JSON");
        assert_eq!(summary["failed"], 0);
        assert_eq!(summary["skipped"], 1);
        let private: (Option<String>,) =
            sqlx::query_as("SELECT input_encrypted FROM workflow_private_inputs WHERE run_id = ?")
                .bind(&accepted.id)
                .fetch_one(&ctx.db.pool)
                .await
                .expect("request receipt");
        assert_eq!(
            private.0, None,
            "skipping the last item purges stored credentials"
        );
        assert_eq!(
            ctx.app
                .retry_source_import(&actor, &accepted.id, Vec::new())
                .await
                .expect_err("resolved operation cannot be retried")
                .validation_reason(),
            Some(scryer_application::source_import::refusal::ITEM_NOT_RETRYABLE)
        );
        assert!(
            ctx.app
                .complete_setup_for_source_import(&actor, Some(&accepted.id))
                .await
                .expect("finish after explicit skip")
        );
    })
    .await
    .expect("source import scenario completes within its failure bound");
}

#[tokio::test]
async fn source_import_retry_repairs_reserved_config_after_interrupted_save() {
    timeout(Duration::from_secs(180), async {
    use scryer_application::{JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource};
    use scryer_infrastructure_workflow::workflow::stores::WorkflowOperationStore;
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let actor = ctx.users.create(User::new_admin("source-recovery-operator")).await.expect("operator");
    let mut selected = selection(0, false);
    if let SourceImportInput::Indexer(input) = &mut selected.input { input.name = format!("  {}  ", input.name); }
    let SourceImportInput::Indexer(input) = &selected.input else { unreachable!() };
    let created = ctx.app.create_indexer_config(&actor, input.clone()).await.expect("configuration was committed before interruption");
    for scope in ["movie", "series", "anime"] { ctx.app.update_indexer_routing(&actor, scope, Vec::new()).await.expect("simulate interrupted routing repair"); }
    let now = chrono::Utc::now();
    let run = JobRunRecord { id: scryer_domain::Id::new().0, job_key: JobKey::SourceImport, operation_type: "source_import".into(), status: JobRunStatus::Running, trigger_source: JobTriggerSource::Manual, actor_user_id: Some(actor.id.clone()),
        progress_json: Some(json!({"items":[{"key":selected.key,"name":selected.name,"config_id":created.id,"kind":"indexer","state":"saving","reason":null}]}).to_string()), summary_json: None, summary_text: None, error_text: None, started_at: now, completed_at: None, created_at: now, updated_at: now };
    let store = WorkflowOperationStore::new(ctx.db.datastore()).with_encryption_key(ctx.db.encryption_key_state());
    let private = json!([{"selection":selected,"config_id":created.id,"reuse_existing":false}]).to_string();
    store.accept_private_job(&run, "source-import:source-recovery", &private).await.expect("durable receipt");
    store.reconcile_interrupted_job_runs(&[]).await.expect("startup reconciliation");
    let interrupted = ctx.app.source_import_status(&actor, &run.id).await.expect("restored status");
    assert_eq!(interrupted.1.items[0].state, SourceImportState::Interrupted);
    let rediscovered = ctx.app.existing_source_import(&actor, "source-recovery", vec![selected.key.clone()]).await.expect("lost response recovery").expect("original receipt");
    assert_eq!(rediscovered.id, run.id);
    ctx.app.retry_source_import(&actor, &run.id, Vec::new()).await.expect("retry interrupted item");
    let progress = timeout(Duration::from_secs(120), ctx.app.wait_source_import(&actor, &run.id)).await.expect("retry settles").expect("retry progress");
    assert_eq!(progress.items[0].state, SourceImportState::Succeeded);
    assert_eq!(progress.items[0].config_id, created.id);
    let configs = ctx.app.list_indexer_configs(&actor, Some("newznab".into())).await.expect("configs");
    assert_eq!(configs.iter().filter(|config| config.id == created.id).count(), 1);
    for scope in ["movie", "series", "anime"] { assert!(ctx.app.get_indexer_routing(&actor, scope).await.expect("repaired routing").iter().any(|route| route.indexer_id == created.id)); }
    }).await.expect("source import scenario completes within its failure bound");
}

#[tokio::test]
async fn source_import_purges_credentials_once_resolved_and_housekeeping_prunes_the_run() {
    use scryer_application::HousekeepingRepository;
    use scryer_infrastructure_library::media::libraries::state_store::HousekeepingStore;
    timeout(Duration::from_secs(180), async {
        let ctx = TestContext::new().await;
        seed_typed_settings_definitions(&ctx).await;
        let actor = ctx
            .users
            .create(User::new_admin("source-purge-operator"))
            .await
            .expect("operator");
        let selected = selection(0, false);
        let completed = ctx
            .app
            .start_source_import(&actor, "source-purge", vec![selected.clone()])
            .await
            .expect("accepted");
        let progress = ctx
            .app
            .wait_source_import(&actor, &completed.id)
            .await
            .expect("import settles");
        assert_eq!(progress.items[0].state, SourceImportState::Succeeded);
        let private: (Option<String>,) =
            sqlx::query_as("SELECT input_encrypted FROM workflow_private_inputs WHERE run_id = ?")
                .bind(&completed.id)
                .fetch_one(&ctx.db.pool)
                .await
                .expect("request receipt is kept");
        assert_eq!(
            private.0, None,
            "a resolved import keeps no stored credentials"
        );
        assert_eq!(
            ctx.app
                .start_source_import(&actor, "source-purge", vec![selected.clone()])
                .await
                .expect("replayed request")
                .id,
            completed.id,
            "the receipt still answers a replayed request"
        );
        assert_eq!(
            ctx.app
                .retry_source_import(&actor, &completed.id, Vec::new())
                .await
                .expect_err("nothing to retry")
                .validation_reason(),
            Some(scryer_application::source_import::refusal::ITEM_NOT_RETRYABLE)
        );

        let mut invalid = selection(1, false);
        let SourceImportInput::Indexer(input) = &mut invalid.input else {
            unreachable!()
        };
        input.name.clear();
        let unresolved = ctx
            .app
            .start_source_import(&actor, "source-purge-unresolved", vec![invalid])
            .await
            .expect("accepted");
        assert_eq!(
            ctx.app
                .wait_source_import(&actor, &unresolved.id)
                .await
                .expect("failure settles")
                .items[0]
                .state,
            SourceImportState::Failed
        );
        // A newer terminal run, so neither older run is kept as the job's latest.
        let newest = ctx
            .app
            .start_source_import(&actor, "source-purge-newest", vec![selection(2, false)])
            .await
            .expect("accepted");
        ctx.app
            .wait_source_import(&actor, &newest.id)
            .await
            .expect("newest settles");
        let old = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        for id in [&completed.id, &unresolved.id] {
            sqlx::query("UPDATE workflow_operations SET started_at = ? WHERE id = ?")
                .bind(&old)
                .bind(id)
                .execute(&ctx.db.pool)
                .await
                .expect("age run");
        }
        HousekeepingStore::new(ctx.db.datastore())
            .delete_stale_workflow_operations(30, 30)
            .await
            .expect("housekeeping");
        let remaining = |id: String| {
            let pool = ctx.db.pool.clone();
            async move {
                let runs: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM workflow_operations WHERE id = ?")
                        .bind(&id)
                        .fetch_one(&pool)
                        .await
                        .expect("run count");
                let receipts: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM workflow_private_inputs WHERE run_id = ?")
                        .bind(&id)
                        .fetch_one(&pool)
                        .await
                        .expect("receipt count");
                (runs.0, receipts.0)
            }
        };
        assert_eq!(
            remaining(completed.id.clone()).await,
            (0, 0),
            "a purged run and its receipt are pruned"
        );
        assert_eq!(
            remaining(unresolved.id.clone()).await,
            (1, 1),
            "an unresolved import keeps its run and credentials for retry"
        );
        assert_eq!(
            remaining(newest.id.clone()).await,
            (1, 1),
            "the latest run is retained"
        );
    })
    .await
    .expect("credential purge scenario failure bound");
}
