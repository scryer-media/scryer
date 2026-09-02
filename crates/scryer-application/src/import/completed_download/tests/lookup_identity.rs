use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CompletedLookupRegistry {
    ids: HashMap<String, scryer_domain::download_identity::DownloadId>,
    failing_item_ids: HashSet<String>,
}

#[async_trait]
impl crate::DownloadRegistryRepository for CompletedLookupRegistry {
    async fn resolve_observation(
        &self,
        observation: &crate::ObservedClientJob,
    ) -> AppResult<crate::ObservationResolution> {
        if self.failing_item_ids.contains(&observation.locator.item_id) {
            return Err(AppError::Repository(
                "injected completed lookup registry failure".to_string(),
            ));
        }
        let download_id = self
            .ids
            .get(&observation.locator.item_id)
            .copied()
            .ok_or_else(|| AppError::Repository("missing completed lookup registry id".into()))?;
        Ok(crate::ObservationResolution::Resolved {
            download_id,
            newly_foreign: false,
            attached: false,
        })
    }

    async fn load_download(
        &self,
        _: &scryer_domain::download_identity::DownloadId,
    ) -> AppResult<Option<crate::DownloadRecord>> {
        Ok(None)
    }

    async fn load_binding(
        &self,
        _: &scryer_domain::download_identity::DownloadId,
    ) -> AppResult<Option<crate::DownloadClientBindingRecord>> {
        Ok(None)
    }

    async fn find_active_binding_by_locator(
        &self,
        _: &crate::ClientJobLocator,
    ) -> AppResult<Option<crate::DownloadClientBindingRecord>> {
        Ok(None)
    }

    async fn end_binding(&self, _: &scryer_domain::download_identity::DownloadId) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct DownloadIdentityResolverWarningRecorder {
    warnings: Arc<AtomicUsize>,
}

impl tracing::Subscriber for DownloadIdentityResolverWarningRecorder {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "download_identity_resolver"
            && *metadata.level() == tracing::Level::WARN
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        if event.metadata().target() == "download_identity_resolver"
            && *event.metadata().level() == tracing::Level::WARN
        {
            self.warnings.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn completed_download_lookup_keeps_same_native_id_from_different_clients() {
    let first = build_completed_download("Paper.Lantern.2012.1080p", "/downloads/a", Some("movie"));
    let mut second =
        build_completed_download("Paper.Lantern.2012.1080p", "/downloads/b", Some("movie"));
    second.client_id = "client-2".to_string();

    let lookup =
        index_completed_downloads(vec![first, second], CompletedDownloadLookupCoverage::Recent);

    assert_eq!(lookup.by_source.len(), 2);
    assert!(
        lookup
            .by_source
            .contains_key(&completed_download_lookup_key(
                Some("client-1"),
                "nzbget",
                "dl-1"
            ))
    );
    assert!(
        lookup
            .by_source
            .contains_key(&completed_download_lookup_key(
                Some("client-2"),
                "nzbget",
                "dl-1"
            ))
    );
}

#[tokio::test]
async fn check_with_lookup_remaps_remote_completed_download_paths_before_readiness_checks() {
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let local_root = std::env::temp_dir().join(format!("scryer-remote-path-map-{}", Id::new().0));
    let local_download_dir = local_root.join("Paper.Lantern.2012.1080p");
    std::fs::create_dir_all(&local_download_dir).expect("create mapped download directory");

    let remote_completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        "/downloads/Paper.Lantern.2012.1080p",
        Some("movie"),
    );
    let lookup = index_completed_downloads(
        vec![remote_completed],
        CompletedDownloadLookupCoverage::Recent,
    );
    let download_client = Arc::new(TestDownloadClient::default());
    let app = build_app_with_download_client_and_configs(
        vec![title.clone()],
        vec![],
        vec![],
        vec![],
        download_client,
        Arc::new(TestDownloadClientConfigRepo {
            configs: vec![DownloadClientConfig {
                id: "client-1".to_string(),
                name: "qBittorrent".to_string(),
                client_type: "qbittorrent".to_string(),
                config_json: format!(
                    r#"{{"remote_path_mappings":"/downloads => {}"}}"#,
                    local_root.to_string_lossy()
                ),
                is_enabled: true,
                status: scryer_domain::DownloadClientStatus::Healthy,
                last_error: None,
                last_seen_at: None,
                client_priority: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                proxy_config_id: None,
            }],
        }),
    );
    let mut td = build_tracked_download(&title.id, "movie", "Paper.Lantern.2012.1080p");

    check_with_lookup(&app, &mut td, Some(&lookup)).await;

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert!(td.status_messages.is_empty());

    std::fs::remove_dir_all(&local_root).expect("remove mapped download directory");
}

#[tokio::test]
async fn check_requires_canonical_completion_evidence_for_title_parse_observation() {
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let completed_dir = tempfile::tempdir().expect("create completed directory");
    let mut completed = build_completed_download(
        "downloader display label",
        completed_dir.path().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.release_name = Some("unrecognized-completed-release".to_string());
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let app = build_app(vec![title.clone()], vec![], vec![], vec![]);
    let mut td = build_tracked_download(&title.id, "movie", "Paper.Lantern.2012.1080p");
    td.match_type = TitleMatchType::TitleParse;
    td.client_item.is_scryer_origin = false;

    check_with_lookup(&app, &mut td, Some(&lookup)).await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert!(
        td.status_messages
            .iter()
            .any(|message| { message.contains("completed release name no longer proves") })
    );
}

#[tokio::test]
async fn check_with_lookup_matches_qbit_torrent_hash_download_id() {
    let title = build_title("title-1", "Paperman", MediaFacet::Movie);
    let release_title = "Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb";
    let info_hash = "5b4ba671c3729e34718de86e3372be2ecb527b15";
    let accepted_identity = crate::download_identity::accepted_download_submission_identity(
        crate::download_identity::AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:test-qbit"),
            source_kind: Some(crate::DownloadSourceKind::TorrentFile),
            source_hint: Some("http://torrent-indexer/download/paperman.torrent"),
            info_hash_hint: None,
            client_type: Some("qbittorrent"),
            client_item_id: Some(info_hash),
            accepted_info_hash: Some(info_hash),
        },
    );
    assert_eq!(accepted_identity.download_id.as_deref(), Some(info_hash));

    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission_with_identity(
            DownloadSubmission {
    download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: title.facet.as_str().to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "qbittorrent".to_string(),
                download_client_item_id: info_hash.to_string(),
                source_hint: Some("http://torrent-indexer/download/paperman.torrent".to_string()),
                source_provider_id: None,
                source_provider_name: None,
                source_kind: Some(crate::DownloadSourceKind::TorrentFile),
                source_title: Some(release_title.to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: Some(
                    "torrent_file|http://torrent-indexer/download/paperman.torrent|Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb"
                        .to_string(),
                ),
                scope: crate::SubmissionScope::Title,
            },
            accepted_identity,
            None,
        )
        .await
        .expect("record submission");

    let completed_dir = std::env::temp_dir().join(format!("scryer-qbit-completed-{}", Id::new().0));
    std::fs::create_dir_all(&completed_dir).expect("create completed dir");
    let mut completed = build_completed_download(
        "qBit display label",
        completed_dir.to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.release_name = Some("downloader-provided-name".to_string());
    completed.client_type = "qbittorrent".to_string();
    completed.client_id = "client-1".to_string();
    completed.download_client_item_id = info_hash.to_string();
    completed.download_id = Some(info_hash.to_string());
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);

    let app = build_app_with_download_client_configs_and_submissions(
        vec![title.clone()],
        vec![],
        vec![],
        vec![],
        Arc::new(TestDownloadClient::default()),
        Arc::new(NullDownloadClientConfigRepository),
        submission_repo,
    );
    let mut td = build_tracked_download(&title.id, "movie", release_title);
    td.id = format!("download:{info_hash}");
    td.client_type = "qbittorrent".to_string();
    td.client_id = "client-1".to_string();
    td.client_item.client_id = "client-1".to_string();
    td.client_item.client_name = "qBittorrent".to_string();
    td.client_item.client_type = "qbittorrent".to_string();
    td.client_item.download_client_item_id = info_hash.to_string();
    td.client_item.download_id = Some(info_hash.to_string());

    check_with_lookup(&app, &mut td, Some(&lookup)).await;

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert!(td.status_messages.is_empty());
    let evidence = crate::import_workflow::resolve_release_evidence_for_completed_download(
        &app,
        td.completed_source.as_ref().expect("completed source"),
        None,
    )
    .await
    .expect("resolve persisted qBit submission evidence");
    assert_eq!(evidence.release_title(None).as_deref(), Some(release_title));

    std::fs::remove_dir_all(&completed_dir).expect("remove completed dir");
}

fn assert_lookup_matches_tracked_download(
    lookup: &CompletedDownloadLookup,
    td: &TrackedDownload,
    expected: bool,
) {
    assert_eq!(
        find_completed_download_in_lookup(lookup, td).is_some(),
        expected
    );
    assert_eq!(lookup.matches_tracked_download(td), expected);
}

#[test]
fn lookup_identity_download_id_miss_falls_back_to_exact_source_without_completed_identity() {
    let mut completed = build_completed_download(
        "Paperman.2012.720p.WEB-DL",
        "/downloads/Paperman.2012.720p.WEB-DL",
        Some("movie"),
    );
    completed.download_id = None;
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download("title-1", "movie", "Paperman.2012.720p.WEB-DL");
    td.client_item.download_id = Some("scryer-download:managed".to_string());

    assert_lookup_matches_tracked_download(&lookup, &td, true);
}

#[test]
fn lookup_identity_download_id_miss_falls_back_to_exact_source_with_matching_identity() {
    let download_id = "scryer-download:managed";
    let mut completed = build_completed_download(
        "Paperman.2012.720p.WEB-DL",
        "/downloads/Paperman.2012.720p.WEB-DL",
        Some("movie"),
    );
    completed.download_id = Some(download_id.to_string());
    let mut lookup = CompletedDownloadLookup::empty_recent();
    lookup.by_source.insert(
        completed_download_lookup_key(Some("client-1"), "nzbget", "dl-1"),
        completed,
    );
    let mut td = build_tracked_download("title-1", "movie", "Paperman.2012.720p.WEB-DL");
    td.client_item.download_id = Some(download_id.to_string());

    assert_lookup_matches_tracked_download(&lookup, &td, true);
}

#[test]
fn lookup_identity_download_id_miss_rejects_exact_source_with_conflicting_identity() {
    let mut completed = build_completed_download(
        "Paperman.2012.720p.WEB-DL",
        "/downloads/Paperman.2012.720p.WEB-DL",
        Some("movie"),
    );
    completed.download_id = Some("scryer-download:other".to_string());
    let mut lookup = CompletedDownloadLookup::empty_recent();
    lookup.by_source.insert(
        completed_download_lookup_key(Some("client-1"), "nzbget", "dl-1"),
        completed,
    );
    let mut td = build_tracked_download("title-1", "movie", "Paperman.2012.720p.WEB-DL");
    td.client_item.download_id = Some("scryer-download:managed".to_string());

    assert_lookup_matches_tracked_download(&lookup, &td, false);
}

#[test]
fn lookup_identity_download_id_conflict_suppresses_exact_source_fallback() {
    let download_id = "scryer-download:managed";
    let mut first =
        build_completed_download("First.Release.2026", "/downloads/first", Some("movie"));
    first.download_id = Some(download_id.to_string());
    first.download_client_item_id = "other-1".to_string();
    let mut second =
        build_completed_download("Second.Release.2026", "/downloads/second", Some("movie"));
    second.download_id = Some(download_id.to_string());
    second.download_client_item_id = "other-2".to_string();
    let mut source = build_completed_download(
        "Paperman.2012.720p.WEB-DL",
        "/downloads/Paperman.2012.720p.WEB-DL",
        Some("movie"),
    );
    source.download_id = None;
    let lookup = index_completed_downloads(
        vec![first, second, source],
        CompletedDownloadLookupCoverage::Recent,
    );
    let mut td = build_tracked_download("title-1", "movie", "Paperman.2012.720p.WEB-DL");
    td.client_item.download_id = Some(download_id.to_string());

    assert_lookup_matches_tracked_download(&lookup, &td, false);
}

#[test]
fn lookup_identity_matches_exact_source_with_mixed_case_client_type() {
    let mut completed = build_completed_download(
        "Paperman.2012.720p.WEB-DL",
        "/downloads/Paperman.2012.720p.WEB-DL",
        Some("movie"),
    );
    completed.client_type = "WeAvEr".to_string();
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download("title-1", "movie", "Paperman.2012.720p.WEB-DL");
    td.client_type = "weaver".to_string();
    td.client_item.client_type = "weaver".to_string();

    assert_lookup_matches_tracked_download(&lookup, &td, true);
}

#[tokio::test]
async fn check_with_lookup_retries_scryer_origin_when_completed_history_is_missing() {
    let title = build_title("title-1", "Paperman", MediaFacet::Movie);
    let download_id = "cc025b54883bbdc61258e9d5627b3bd1613241b2";
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    let app = build_app_with_download_client_configs_and_submissions(
        vec![title.clone()],
        vec![],
        vec![],
        vec![],
        Arc::new(TestDownloadClient::default()),
        Arc::new(NullDownloadClientConfigRepository),
        submission_repo.clone(),
    );
    let lookup = index_completed_downloads(vec![], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download(&title.id, "movie", "Paperman.2012.720p.WEB-DL");
    td.id = format!("download:client-1:qbittorrent:{download_id}");
    td.client_type = "qbittorrent".to_string();
    td.client_item.client_type = "qbittorrent".to_string();
    td.client_item.client_name = "qBittorrent".to_string();
    td.client_item.download_client_item_id = "2".to_string();
    td.client_item.download_id = Some(download_id.to_string());
    td.client_item.is_scryer_origin = true;
    td.match_type = TitleMatchType::Submission;

    check_with_lookup(&app, &mut td, Some(&lookup)).await;

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert!(
        td.status_messages
            .iter()
            .any(|message| message.contains("waiting for client history"))
    );
    assert!(
        submission_repo
            .identity_tracked_states
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn check_with_recent_lookup_retries_local_identity_miss_without_manual_block() {
    let title = build_title("title-1", "Paperman", MediaFacet::Movie);
    let download_id = "10010";
    let identity = DownloadSubmissionIdentity {
        download_id: Some(download_id.to_string()),
    };
    let source_identity = ClientJobLocator::new(Some("client-1"), "nzbget", "dl-1");
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    let app = build_app_with_download_client_configs_and_submissions(
        vec![title.clone()],
        vec![],
        vec![],
        vec![],
        Arc::new(TestDownloadClient::default()),
        Arc::new(NullDownloadClientConfigRepository),
        submission_repo.clone(),
    );
    let mut td = build_tracked_download(&title.id, "movie", "Paperman.2012.720p.WEB-DL");
    td.id = format!("download:{download_id}");
    td.client_item.download_id = Some(download_id.to_string());

    check_with_lookup(
        &app,
        &mut td,
        Some(&CompletedDownloadLookup::empty_recent()),
    )
    .await;

    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert!(
        td.status_messages
            .iter()
            .any(|message| message.contains("waiting for client history"))
    );
    let recorded_state = submission_repo
        .get_identity_tracked_state(&identity, Some(&source_identity))
        .await
        .expect("identity state lookup");
    assert!(recorded_state.is_none());
}

#[tokio::test]
async fn check_with_lookup_uses_durable_terminal_state_before_redispatch() {
    let title = build_title("title-1", "Paperman", MediaFacet::Movie);
    let download_id = "scryer-download:terminal";
    let identity = DownloadSubmissionIdentity {
        download_id: Some(download_id.to_string()),
    };
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_identity_tracked_state(
            &identity,
            Some(&ClientJobLocator::new(Some("client-1"), "nzbget", "dl-1")),
            TrackedDownloadState::Imported.as_str(),
            None,
            None,
        )
        .await
        .expect("record identity state");

    let app = build_app_with_download_client_configs_and_submissions(
        vec![title.clone()],
        vec![],
        vec![],
        vec![],
        Arc::new(TestDownloadClient::default()),
        Arc::new(NullDownloadClientConfigRepository),
        submission_repo,
    );
    let mut td = build_tracked_download(&title.id, "movie", "Paperman.2012.720p.WEB-DL");
    td.id = format!("download:{download_id}");
    td.client_item.download_id = Some(download_id.to_string());

    check_with_lookup(
        &app,
        &mut td,
        Some(&CompletedDownloadLookup::empty_recent()),
    )
    .await;

    assert_eq!(td.state, TrackedDownloadState::Imported);
    assert_eq!(td.status, TrackedDownloadStatus::Ok);
    assert!(td.status_messages.is_empty());
}

#[tokio::test]
async fn check_with_lookup_does_not_apply_client_local_terminal_state_from_other_client() {
    let title = build_title("title-1", "Paperman", MediaFacet::Movie);
    let download_id = "10010";
    let identity = DownloadSubmissionIdentity {
        download_id: Some(download_id.to_string()),
    };
    let other_client_source = ClientJobLocator::new(Some("client-2"), "nzbget", "dl-1");
    let current_client_source = ClientJobLocator::new(Some("client-1"), "nzbget", "dl-1");
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_identity_tracked_state(
            &identity,
            Some(&other_client_source),
            TrackedDownloadState::Imported.as_str(),
            None,
            None,
        )
        .await
        .expect("record other client identity state");

    let app = build_app_with_download_client_configs_and_submissions(
        vec![title.clone()],
        vec![],
        vec![],
        vec![],
        Arc::new(TestDownloadClient::default()),
        Arc::new(NullDownloadClientConfigRepository),
        submission_repo.clone(),
    );
    let mut td = build_tracked_download(&title.id, "movie", "Paperman.2012.720p.WEB-DL");
    td.id = format!("download:{download_id}");
    td.client_item.download_id = Some(download_id.to_string());

    check_with_lookup(&app, &mut td, Some(&CompletedDownloadLookup::empty_full())).await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    let current_client_state = submission_repo
        .get_identity_tracked_state(&identity, Some(&current_client_source))
        .await
        .expect("current client state lookup");
    let other_client_state = submission_repo
        .get_identity_tracked_state(&identity, Some(&other_client_source))
        .await
        .expect("other client state lookup");
    assert_eq!(
        current_client_state.as_deref(),
        Some(TrackedDownloadState::ImportBlocked.as_str())
    );
    assert_eq!(
        other_client_state.as_deref(),
        Some(TrackedDownloadState::Imported.as_str())
    );
}

#[tokio::test]
async fn load_completed_download_lookup_for_items_fetches_client_history_once_per_cycle() {
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    let download_client = Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![completed.clone()])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let first = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    let mut second = build_tracked_download("title-2", "movie", "Paper.Lantern.2012.1080p.REPACK");
    second.client_item.download_client_item_id = "dl-2".to_string();
    second.client_item.title_id = Some("title-2".to_string());

    let lookup = load_completed_download_lookup_for_items(
        &app,
        &[first.client_item.clone(), second.client_item.clone()],
        100,
    )
    .await
    .expect("completed lookup should load");

    assert_eq!(lookup.by_source.len(), 1);
    assert_eq!(
        download_client
            .completed_download_calls
            .load(Ordering::SeqCst),
        0
    );
    assert_eq!(
        download_client
            .recent_completed_download_calls
            .load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn load_completed_download_lookup_for_items_scopes_to_completed_item_clients() {
    let mut completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.client_id = "qbit-client".to_string();
    completed.client_type = "qbittorrent".to_string();
    let download_client = Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![completed])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let mut qbit = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    qbit.client_item.client_id = "qbit-client".to_string();
    qbit.client_item.client_type = "qbittorrent".to_string();
    let mut nzbget = build_tracked_download("title-2", "movie", "Other.Release.2012.1080p");
    nzbget.client_item.client_id = "nzbget-client".to_string();
    nzbget.client_item.client_type = "nzbget".to_string();
    nzbget.client_item.state = DownloadQueueState::Downloading;

    let lookup = load_completed_download_lookup_for_items(
        &app,
        &[qbit.client_item.clone(), nzbget.client_item.clone()],
        100,
    )
    .await
    .expect("completed lookup should load");

    assert_eq!(lookup.by_source.len(), 1);
    let calls = download_client.scoped_recent_completed_calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, vec!["qbit-client".to_string()]);
    assert!(calls[0].1.is_empty());
    assert_eq!(
        download_client
            .recent_completed_download_calls
            .load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn load_completed_download_lookup_for_items_scopes_import_pending_items() {
    let mut completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.client_id = "qbit-client".to_string();
    completed.client_type = "qbittorrent".to_string();
    let download_client = Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![completed])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let mut qbit = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    qbit.client_item.client_id = "qbit-client".to_string();
    qbit.client_item.client_type = "qbittorrent".to_string();
    qbit.client_item.state = DownloadQueueState::ImportPending;

    let lookup = load_completed_download_lookup_for_items(&app, &[qbit.client_item.clone()], 100)
        .await
        .expect("completed lookup should load");

    assert_eq!(lookup.by_source.len(), 1);
    let calls = download_client.scoped_recent_completed_calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, vec!["qbit-client".to_string()]);
    assert!(calls[0].1.is_empty());
    assert_eq!(
        download_client
            .recent_completed_download_calls
            .load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn load_completed_download_lookup_for_items_uses_exact_id_when_client_id_is_present() {
    let mut completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.client_id = "default".to_string();
    completed.client_type = "qbittorrent".to_string();
    let download_client = Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![completed])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let mut qbit = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    qbit.client_item.client_id = "default".to_string();
    qbit.client_item.client_type = "qbittorrent".to_string();
    qbit.client_item.state = DownloadQueueState::ImportPending;

    let lookup = load_completed_download_lookup_for_items(&app, &[qbit.client_item.clone()], 100)
        .await
        .expect("completed lookup should load");

    assert_eq!(lookup.by_source.len(), 1);
    let calls = download_client.scoped_recent_completed_calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, vec!["default".to_string()]);
    assert!(calls[0].1.is_empty());
}

#[tokio::test]
async fn load_completed_download_lookup_for_items_uses_type_scope_for_idless_items() {
    let mut completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        std::env::temp_dir().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.client_id = String::new();
    completed.client_type = "qbittorrent".to_string();
    let download_client = Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![completed])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let mut qbit = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    qbit.client_item.client_id = String::new();
    qbit.client_item.client_type = "qbittorrent".to_string();
    qbit.client_item.state = DownloadQueueState::ImportPending;

    let lookup = load_completed_download_lookup_for_items(&app, &[qbit.client_item.clone()], 100)
        .await
        .expect("completed lookup should load");

    assert_eq!(lookup.by_source.len(), 1);
    let calls = download_client.scoped_recent_completed_calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].0.is_empty());
    assert_eq!(calls[0].1, vec!["qbittorrent".to_string()]);
}

#[tokio::test]
async fn completed_lookup_indexes_token_locator_and_legacy_identity_observations_by_one_canonical_id()
 {
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    let registry = Arc::new(CompletedLookupRegistry {
        ids: HashMap::from([
            ("token-observation".to_string(), canonical_download_id),
            ("locator-observation".to_string(), canonical_download_id),
            ("legacy-observation".to_string(), canonical_download_id),
        ]),
        failing_item_ids: HashSet::new(),
    });
    let app = build_app(vec![], vec![], vec![], vec![])
        .with_test_overrides(|services| services.with_download_registry(registry));

    let mut token = build_completed_download("Token", "/downloads/token", Some("movie"));
    token.download_client_item_id = "token-observation".to_string();
    token.download_id = Some(canonical_download_id.to_wire());
    let mut locator = build_completed_download("Locator", "/downloads/locator", Some("movie"));
    locator.download_client_item_id = "locator-observation".to_string();
    locator.download_id = None;
    let mut legacy = build_completed_download("Legacy", "/downloads/legacy", Some("movie"));
    legacy.download_client_item_id = "legacy-observation".to_string();
    legacy.download_id = Some("legacy-client-identity".to_string());
    let completed_downloads = vec![token, locator, legacy];

    let resolutions = resolve_completed_download_observations(&app, &completed_downloads).await;
    assert_eq!(
        resolutions,
        vec![
            crate::download_identity::ObservedClientJobResolution::Resolved(canonical_download_id,),
            crate::download_identity::ObservedClientJobResolution::Resolved(canonical_download_id,),
            crate::download_identity::ObservedClientJobResolution::Resolved(canonical_download_id,),
        ]
    );
    let canonical_download_ids = resolutions
        .into_iter()
        .map(|resolution| match resolution {
            crate::download_identity::ObservedClientJobResolution::Resolved(download_id) => {
                Some(download_id)
            }
            crate::download_identity::ObservedClientJobResolution::Conflict
            | crate::download_identity::ObservedClientJobResolution::Unavailable => {
                panic!("canonical observations should resolve")
            }
        })
        .collect();
    let lookup = index_completed_downloads_with_canonical_download_ids(
        completed_downloads,
        canonical_download_ids,
        CompletedDownloadLookupCoverage::Recent,
    );
    assert_eq!(lookup.by_canonical.len(), 1);

    for observation_shape in ["token", "locator", "legacy"] {
        let mut td = build_tracked_download("title-1", "movie", observation_shape);
        td.download_id = canonical_download_id;
        td.client_item.download_client_item_id = format!("tracked-{observation_shape}");

        let found = find_completed_download_in_lookup(&lookup, &td)
            .expect("canonical lookup should resolve the completed download");
        assert_eq!(found.download_client_item_id, "legacy-observation");
    }
}

#[test]
fn completed_lookup_without_canonical_id_uses_the_legacy_route() {
    let completed = build_completed_download("Legacy", "/downloads/legacy", Some("movie"));
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download("title-1", "movie", "Legacy");
    td.download_id =
        scryer_domain::download_identity::DownloadId::parse("00000000-0000-0000-0000-000000000000")
            .expect("legacy fallback id");

    assert_lookup_matches_tracked_download(&lookup, &td, true);
}

#[tokio::test]
async fn completed_lookup_registry_failure_keeps_that_item_available_to_legacy_matching() {
    let registry = Arc::new(CompletedLookupRegistry {
        ids: HashMap::new(),
        failing_item_ids: HashSet::from(["failed-observation".to_string()]),
    });
    let app = build_app(vec![], vec![], vec![], vec![])
        .with_test_overrides(|services| services.with_download_registry(registry));
    let mut completed = build_completed_download("Legacy", "/downloads/legacy", Some("movie"));
    completed.download_client_item_id = "failed-observation".to_string();
    let resolutions = resolve_completed_download_observations(&app, &[completed.clone()]).await;
    assert_eq!(
        resolutions,
        vec![crate::download_identity::ObservedClientJobResolution::Unavailable]
    );
    let canonical_download_ids = resolutions
        .into_iter()
        .map(|resolution| match resolution {
            crate::download_identity::ObservedClientJobResolution::Unavailable => None,
            crate::download_identity::ObservedClientJobResolution::Resolved(_)
            | crate::download_identity::ObservedClientJobResolution::Conflict => {
                panic!("registry failure should remain unavailable")
            }
        })
        .collect();
    let lookup = index_completed_downloads_with_canonical_download_ids(
        vec![completed],
        canonical_download_ids,
        CompletedDownloadLookupCoverage::Recent,
    );
    let mut td = build_tracked_download("title-1", "movie", "Legacy");
    td.download_id =
        scryer_domain::download_identity::DownloadId::parse("00000000-0000-0000-0000-000000000000")
            .expect("legacy fallback id");
    td.client_item.download_client_item_id = "failed-observation".to_string();

    assert_lookup_matches_tracked_download(&lookup, &td, true);
}

#[test]
fn completed_lookup_divergence_uses_legacy_result_and_warns() {
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    let legacy = build_completed_download("Legacy", "/downloads/legacy", Some("movie"));
    let mut canonical =
        build_completed_download("Canonical", "/downloads/canonical", Some("movie"));
    canonical.download_client_item_id = "canonical-observation".to_string();
    let mut lookup =
        index_completed_downloads(vec![legacy], CompletedDownloadLookupCoverage::Recent);
    lookup.by_canonical.insert(canonical_download_id, canonical);
    let mut td = build_tracked_download("title-1", "movie", "Legacy");
    td.download_id = canonical_download_id;

    let recorder = DownloadIdentityResolverWarningRecorder::default();
    let warnings = recorder.warnings.clone();
    let found = tracing::subscriber::with_default(recorder, || {
        find_completed_download_in_lookup(&lookup, &td)
            .expect("the legacy result should be selected")
    });

    assert_eq!(found.download_client_item_id, "dl-1");
    assert_eq!(warnings.load(Ordering::SeqCst), 1);
}
