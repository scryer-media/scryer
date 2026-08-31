use super::*;
use async_trait::async_trait;

struct MarkingDownloadClient {
    marked_client_ids: Mutex<Vec<String>>,
    marked_imported: Mutex<Vec<crate::DownloadClientMarkImportedRequest>>,
    calls: AtomicUsize,
    failures_before_success: usize,
}

impl Default for MarkingDownloadClient {
    fn default() -> Self {
        Self::new(0)
    }
}

impl MarkingDownloadClient {
    fn new(failures_before_success: usize) -> Self {
        Self {
            marked_client_ids: Mutex::new(Vec::new()),
            marked_imported: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            failures_before_success,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl DownloadClient for MarkingDownloadClient {
    async fn submit_download(&self, _: &DownloadClientAddRequest) -> AppResult<DownloadGrabResult> {
        Err(AppError::Repository("not needed in test".to_string()))
    }

    async fn mark_imported_non_destructive_for_client_id(
        &self,
        client_id: &str,
        request: &crate::DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        self.marked_client_ids
            .lock()
            .await
            .push(client_id.to_string());
        self.marked_imported.lock().await.push(request.clone());
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
        if attempt < self.failures_before_success {
            Err(AppError::Repository("transient mark failure".to_string()))
        } else {
            Ok(())
        }
    }
}

struct TorrentOnlyPluginProvider;

impl crate::DownloadClientPluginProvider for TorrentOnlyPluginProvider {
    fn client_for_config(
        &self,
        _: &scryer_domain::DownloadClientConfig,
    ) -> Option<std::sync::Arc<dyn crate::DownloadClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec!["qbittorrent".to_string()]
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        if provider_type.eq_ignore_ascii_case("qbittorrent") {
            vec!["magnet_uri".to_string()]
        } else {
            Vec::new()
        }
    }
}

#[tokio::test]
async fn apply_result_marks_verified_already_present_skip_imported() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("episode already imported".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::Imported);
    assert_eq!(td.status, TrackedDownloadStatus::Ok);
    assert!(td.status_messages.is_empty());
}

#[tokio::test]
async fn apply_result_terminalizes_verified_all_ignored_skip_as_successful_no_op() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let mut episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    episode.monitored = false;
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "ignored",
        )],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let mut result = failed_execution_result("all source videos were intentionally ignored");
    result.decision = ImportDecision::Skipped;
    result.skip_reason = None;

    assert!(apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::Imported);
    assert_eq!(td.status, TrackedDownloadStatus::Ok);
    assert!(td.status_messages.is_empty());
}

#[tokio::test]
async fn unavailable_artifact_evidence_keeps_already_present_import_retryable() {
    let app = build_app_with_import_artifact_repository(
        vec![build_title("title-1", "Show", MediaFacet::Series)],
        vec![build_collection("season-1", "title-1", "1")],
        vec![build_episode("ep-1", "title-1", "season-1", "1", "1", None)],
        Arc::new(UnavailableImportArtifactRepo),
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("episode already imported".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert!(td.import_execution_retry.is_some());
}

#[tokio::test(start_paused = true)]
async fn verified_import_mark_retries_without_rolling_back_import() {
    let marker = Arc::new(MarkingDownloadClient::new(3));
    let app = build_app_with_download_client_configs_submissions_and_settings(
        vec![build_title("title-1", "Show", MediaFacet::Series)],
        vec![build_collection("season-1", "title-1", "1")],
        vec![build_episode("ep-1", "title-1", "season-1", "1", "1", None)],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )],
        TestAppRepositories {
            download_client: marker.clone(),
            download_client_configs: Arc::new(NullDownloadClientConfigRepository),
            download_submissions: Arc::new(
                crate::null_repositories::NullDownloadSubmissionRepository,
            ),
            settings: Arc::new(crate::null_repositories::NullSettingsRepository),
        },
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: Some("/library/Show/Season 01/Show.S01E01.mkv".to_string()),
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("episode already imported".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::Imported);
    tokio::task::yield_now().await;
    assert_eq!(marker.call_count(), 1);

    for (delay, expected_calls) in [(15, 2), (30, 3), (60, 4)] {
        tokio::time::advance(std::time::Duration::from_secs(delay)).await;
        tokio::task::yield_now().await;
        assert_eq!(marker.call_count(), expected_calls);
        assert_eq!(td.state, TrackedDownloadState::Imported);
    }
}

#[tokio::test(start_paused = true)]
async fn verified_import_mark_stops_after_bounded_permanent_failures() {
    let marker = Arc::new(MarkingDownloadClient::new(usize::MAX));
    let app = build_app_with_download_client(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        marker.clone(),
    );
    let td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Imported,
        skip_reason: None,
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: Some("/library/Show/Season 01/Show.S01E01.mkv".to_string()),
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: None,
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    schedule_non_destructive_import_mark(&app, &td, &result, None);
    tokio::task::yield_now().await;
    assert_eq!(marker.call_count(), 1);

    for (delay, expected_calls) in [(15, 2), (30, 3), (60, 4), (120, 5)] {
        tokio::time::advance(std::time::Duration::from_secs(delay)).await;
        tokio::task::yield_now().await;
        assert_eq!(marker.call_count(), expected_calls);
    }
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    tokio::task::yield_now().await;
    assert_eq!(marker.call_count(), 5);
}

#[tokio::test]
async fn verified_import_mark_uses_completed_client_identity() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let client = Arc::new(MarkingDownloadClient::default());
    let mut artifact =
        build_artifact_with_result(hash, Some("ep-1"), "Show.S01E01.mkv", "already_present");
    artifact.source_client_id = Some("rtorrent-primary".to_string());
    artifact.source_system = "rtorrent".to_string();
    let app = build_app_with_download_client(
        vec![title],
        vec![collection],
        vec![episode],
        vec![artifact],
        client.clone(),
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    td.client_id = "rtorrent-primary".to_string();
    td.client_type = "rtorrent".to_string();
    td.client_item.client_id = td.client_id.clone();
    td.client_item.client_type = td.client_type.clone();
    td.client_item.download_client_item_id = hash.to_string();
    td.client_item.download_id = Some(hash.to_ascii_uppercase());
    let mut completed = build_completed_download(
        "Show.S01E01.1080p.WEB-DL",
        "/downloads/Show.S01E01.1080p.WEB-DL",
        Some("series"),
    );
    completed.client_id = "rtorrent-primary".to_string();
    completed.client_type = "rtorrent".to_string();
    completed.download_client_item_id = hash.to_string();
    completed.download_id = Some(hash.to_ascii_uppercase());
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".to_string()),
        source_system: Some("rtorrent".to_string()),
        source_ref: Some(hash.to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: Some("/data/series/Show/Season 01/Show.S01E01.mkv".to_string()),
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: None,
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(
        apply_import_result_with_completed(&app, &mut td, result, 0, Some(&completed), None).await
    );
    tokio::task::yield_now().await;
    assert_eq!(td.state, TrackedDownloadState::Imported);
    assert_eq!(client.call_count(), 1);

    let marked = client.marked_imported.lock().await;
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].client_item_id, hash);
    assert_eq!(marked[0].info_hash.as_deref(), Some(hash));
    assert_eq!(marked[0].title_id.as_deref(), Some("title-1"));
    assert_eq!(marked[0].category.as_deref(), Some("series"));
    assert_eq!(
        marked[0].imported_path.as_deref(),
        Some("/data/series/Show/Season 01/Show.S01E01.mkv")
    );
    drop(marked);
    assert_eq!(
        client.marked_client_ids.lock().await.as_slice(),
        ["rtorrent-primary"]
    );
}

#[tokio::test]
async fn apply_result_does_not_mark_client_when_import_is_rejected() {
    let client = Arc::new(MarkingDownloadClient::default());
    let app = build_app_with_download_client(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        client.clone(),
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let completed = build_completed_download(
        "Show.S01E01.1080p.WEB-DL",
        "/downloads/Show.S01E01.1080p.WEB-DL",
        Some("series"),
    );
    let mut result = failed_execution_result("release language does not match the title");
    result.decision = ImportDecision::Rejected;
    result.release_burned = true;

    assert!(
        !apply_import_result_with_completed(&app, &mut td, result, 0, Some(&completed), None).await
    );
    assert!(client.marked_imported.lock().await.is_empty());
}

#[tokio::test]
async fn apply_result_does_not_mark_client_before_import_verification_succeeds() {
    let client = Arc::new(MarkingDownloadClient::default());
    let app = build_app_with_download_client(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        client.clone(),
    );
    let completed = build_completed_download(
        "Show.S01E01.1080p.WEB-DL",
        "/downloads/Show.S01E01.1080p.WEB-DL",
        Some("series"),
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");

    let mut partial = failed_execution_result("some import work completed");
    partial.decision = ImportDecision::Imported;
    assert!(
        !apply_import_result_with_completed(&app, &mut td, partial, 0, Some(&completed), None)
            .await
    );
    assert_eq!(td.state, TrackedDownloadState::ImportPending);

    assert!(
        !apply_import_result_with_completed(
            &app,
            &mut td,
            failed_execution_result("import execution failed"),
            0,
            Some(&completed),
            None,
        )
        .await
    );

    let mut unverified = failed_execution_result("no eligible video files found");
    unverified.decision = ImportDecision::Skipped;
    unverified.skip_reason = Some(ImportSkipReason::NoVideoFiles);
    assert!(
        !apply_import_result_with_completed(&app, &mut td, unverified, 0, Some(&completed), None)
            .await
    );
    assert!(client.marked_imported.lock().await.is_empty());
}

#[tokio::test]
async fn apply_result_keeps_rejected_already_imported_result_blocked() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Rejected,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("existing episode file is equal or better".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert_eq!(
        td.status_messages,
        vec!["existing episode file is equal or better".to_string()]
    );
}

#[tokio::test]
async fn apply_result_marks_burned_rejection_failed() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let mut result = failed_execution_result("release language does not match the title");
    result.decision = ImportDecision::Rejected;
    result.release_burned = true;

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::Failed);
    assert_eq!(td.status, TrackedDownloadStatus::Error);
    assert_eq!(
        td.status_messages,
        vec!["release language does not match the title".to_string()]
    );
    assert!(td.burned_by_import_gate);
}

#[tokio::test]
async fn apply_result_treats_burned_rule_block_as_final() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let mut result =
        failed_execution_result("post-download rule(s) blocked import: language policy");
    result.decision = ImportDecision::Rejected;
    result.release_burned = true;

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::Failed);
    assert_eq!(td.status, TrackedDownloadStatus::Error);
    assert!(td.import_execution_retry.is_none());
    assert_eq!(
        td.status_messages,
        vec!["post-download rule(s) blocked import: language policy".to_string()]
    );
}

#[tokio::test]
async fn burned_usenet_rejection_deletes_the_mapped_job_directory() {
    let settings = Arc::new(super::route_gate::RoutingSettingsRepo::default());
    settings
        .set_routing(
            "movie",
            r#"{"client-1":{"enabled":true,"removeCompleted":true,"removeFailed":true}}"#,
        )
        .await;
    let app = build_app_with_download_client_configs_submissions_and_settings(
        vec![build_title("title-1", "Show", MediaFacet::Movie)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TestAppRepositories {
            download_client: Arc::new(NullDownloadClient),
            download_client_configs: Arc::new(NullDownloadClientConfigRepository),
            download_submissions: Arc::new(
                crate::null_repositories::NullDownloadSubmissionRepository,
            ),
            settings,
        },
    );
    let mut td = build_tracked_download("title-1", "movie", "Show.S01E01.1080p.WEB-DL");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let job_dir = temp_dir.path().join("completed/job");
    let source = job_dir.join("release/episode.mkv");
    std::fs::create_dir_all(source.parent().expect("source parent")).expect("create source");
    std::fs::write(&source, b"video").expect("write source");
    let completed = build_completed_download(
        "Show.S01E01.1080p.WEB-DL",
        job_dir.to_string_lossy().as_ref(),
        None,
    );
    let mut result = failed_execution_result("release language does not match the title");
    result.decision = ImportDecision::Rejected;
    result.release_burned = true;
    result.source_path = source.to_string_lossy().into_owned();

    assert!(
        !apply_import_result_with_completed(&app, &mut td, result, 0, Some(&completed), None).await
    );
    assert_eq!(td.state, TrackedDownloadState::Failed);
    assert!(!job_dir.exists());
}

#[tokio::test]
async fn burned_usenet_rejection_keeps_data_when_remove_failed_is_off() {
    let settings = Arc::new(super::route_gate::RoutingSettingsRepo::default());
    settings
        .set_routing(
            "movie",
            r#"{"client-1":{"enabled":true,"removeCompleted":true,"removeFailed":false}}"#,
        )
        .await;
    let app = build_app_with_download_client_configs_submissions_and_settings(
        vec![build_title("title-1", "Show", MediaFacet::Movie)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TestAppRepositories {
            download_client: Arc::new(NullDownloadClient),
            download_client_configs: Arc::new(NullDownloadClientConfigRepository),
            download_submissions: Arc::new(
                crate::null_repositories::NullDownloadSubmissionRepository,
            ),
            settings,
        },
    );
    let mut td = build_tracked_download("title-1", "movie", "Show.S01E01.1080p.WEB-DL");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let job_dir = temp_dir.path().join("completed/job");
    let source = job_dir.join("release/episode.mkv");
    std::fs::create_dir_all(source.parent().expect("source parent")).expect("create source");
    std::fs::write(&source, b"video").expect("write source");
    let completed = build_completed_download(
        "Show.S01E01.1080p.WEB-DL",
        job_dir.to_string_lossy().as_ref(),
        None,
    );
    let mut result = failed_execution_result("release language does not match the title");
    result.decision = ImportDecision::Rejected;
    result.release_burned = true;
    result.source_path = source.to_string_lossy().into_owned();

    assert!(
        !apply_import_result_with_completed(&app, &mut td, result, 0, Some(&completed), None).await
    );
    assert_eq!(td.state, TrackedDownloadState::Failed);
    assert!(job_dir.exists());
}

#[tokio::test]
async fn burned_torrent_rejection_does_not_delete_download_data() {
    let mut app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let plugin_provider: std::sync::Arc<dyn crate::DownloadClientPluginProvider> =
        std::sync::Arc::new(TorrentOnlyPluginProvider);
    app.services.integrations.download_client_plugin_provider =
        crate::RuntimeFeature::enabled(plugin_provider);
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    td.client_type = "qbittorrent".to_string();
    td.client_item.client_type = "qbittorrent".to_string();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let job_dir = temp_dir.path().join("completed/job");
    let source = job_dir.join("release/episode.mkv");
    std::fs::create_dir_all(source.parent().expect("source parent")).expect("create source");
    std::fs::write(&source, b"video").expect("write source");
    let mut completed = build_completed_download(
        "Show.S01E01.1080p.WEB-DL",
        job_dir.to_string_lossy().as_ref(),
        None,
    );
    completed.client_type = "qbittorrent".to_string();
    let mut result = failed_execution_result("release language does not match the title");
    result.decision = ImportDecision::Rejected;
    result.release_burned = true;
    result.source_path = source.to_string_lossy().into_owned();

    assert!(
        !apply_import_result_with_completed(&app, &mut td, result, 0, Some(&completed), None).await
    );
    assert_eq!(td.state, TrackedDownloadState::Failed);
    assert!(job_dir.exists());
    assert!(source.exists());
}

#[tokio::test]
async fn apply_result_does_not_verify_unresolved_identity_rejection_as_imported() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact("dl-1", "ep-1", "Show.S01E01.mkv")],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Rejected,
        skip_reason: Some(ImportSkipReason::UnresolvedIdentity),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("download identity is unresolved".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
}

#[tokio::test]
async fn apply_result_blocks_cancelled_import_for_manual_review() {
    let app = build_app(vec![], vec![], vec![], vec![]);
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let result = ImportResult {
        import_id: "import-cancelled".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: None,
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("Import was cancelled. Use Manual Import to resume it.".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert_eq!(
        td.status_messages,
        vec!["Import was cancelled. Use Manual Import to resume it.".to_string()]
    );
    assert!(td.import_execution_retry.is_none());
}

#[tokio::test]
async fn apply_result_keeps_ambiguous_obfuscated_episode_blocked_with_actionable_reason() {
    let app = build_app(vec![], vec![], vec![], vec![]);
    let release =
        "[Erai-raws].Yuki-sama.Kagami.no.Toki.Desu-09.[1080p][Multiple.Subtitle][AA7AC7E5]";
    let reason = "Automatic import could not choose a season for episode 9: the release name does not include a season and the downloaded filename is obfuscated. Open Manual Import and assign the correct season and episode.";
    let mut td = build_tracked_download("title-1", "anime", release);
    let result = ImportResult {
        import_id: "import-ambiguous-season".to_string(),
        decision: ImportDecision::Rejected,
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        title_id: Some("title-1".to_string()),
        source_system: Some("weaver".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some(release.to_string()),
        source_path: "/downloads/obfuscated".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some(reason.to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert_eq!(td.status_messages, vec![reason.to_string()]);
}

#[tokio::test]
async fn apply_result_backs_off_no_video_import_before_blocking() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::NoVideoFiles),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: temp_dir.path().to_string_lossy().into_owned(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("no eligible video files found".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result.clone(), 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert!(td.status_messages[0].contains("Retrying automatically"));
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 1);

    assert!(!apply_import_result(&app, &mut td, result.clone(), 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 2);

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    assert!(td.no_video_import_retry.is_none());
    assert!(td.status_messages[0].contains("Manual review required"));
}

#[tokio::test]
async fn apply_result_resets_no_video_retry_when_source_signature_changes() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let result = ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::NoVideoFiles),
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: temp_dir.path().to_string_lossy().into_owned(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some("no eligible video files found".to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };

    assert!(!apply_import_result(&app, &mut td, result.clone(), 0).await);
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 1);
    std::fs::write(temp_dir.path().join("sample.txt"), b"not video").expect("write sample");

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.no_video_import_retry.as_ref().unwrap().attempts, 1);
}

fn failed_execution_result(error_message: &str) -> ImportResult {
    ImportResult {
        import_id: "import-1".to_string(),
        decision: ImportDecision::Failed,
        skip_reason: None,
        title_id: Some("title-1".to_string()),
        source_system: Some("nzbget".to_string()),
        source_ref: Some("dl-1".to_string()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".to_string()),
        source_path: "/downloads/Show.S01E01.1080p.WEB-DL".to_string(),
        dest_path: None,
        quality: None,
        episode_ids: vec![],
        file_size_bytes: None,
        link_type: None,
        error_message: Some(error_message.to_string()),
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    }
}

#[tokio::test]
async fn apply_result_retries_failed_execution_with_capped_backoff_and_never_blocks() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    // Deliberately NOT one of the allow-listed transient phrases: the phase
    // rule (approved import failed to execute) is what makes this retryable.
    let result = failed_execution_result(
        "0 imported, 0 skipped, 0 rejected, 1 failed. Last error: unexpected hardlink failure",
    );

    let expected_delays = [30i64, 120, 300, 900, 900];
    for (index, expected_delay_secs) in expected_delays.into_iter().enumerate() {
        let attempt = index as u32 + 1;
        let before = Utc::now();
        assert!(!apply_import_result(&app, &mut td, result.clone(), 0).await);
        assert_eq!(
            td.state,
            TrackedDownloadState::ImportPending,
            "attempt {attempt}"
        );
        assert_eq!(
            td.status,
            TrackedDownloadStatus::Warning,
            "attempt {attempt}"
        );
        let retry = td
            .import_execution_retry
            .as_ref()
            .unwrap_or_else(|| panic!("attempt {attempt} must schedule a retry"));
        assert_eq!(retry.attempts, attempt);
        let delay = (retry.next_retry_at - before).num_seconds();
        assert!(
            (expected_delay_secs..=expected_delay_secs + 5).contains(&delay),
            "attempt {attempt}: expected ~{expected_delay_secs}s backoff, got {delay}s"
        );
        assert!(
            td.status_messages[0].contains(&format!("Retrying automatically (attempt {attempt})")),
            "attempt {attempt}: {:?}",
            td.status_messages
        );
        assert!(td.status_messages[0].starts_with("0 imported, 0 skipped"));
        assert!(td.no_video_import_retry.is_none());
    }
}

#[tokio::test]
async fn apply_result_blocks_password_required_failure_without_retry() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let mut result = failed_execution_result("archive requires a password");
    result.skip_reason = Some(ImportSkipReason::PasswordRequired);

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Error);
    assert!(td.import_execution_retry.is_none());
    assert_eq!(
        td.status_messages,
        vec!["archive requires a password".to_string()]
    );
}

#[tokio::test]
async fn apply_result_blocks_archive_extraction_timeout_without_retry() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let mut result = failed_execution_result("archive plugin timed out after 3600 seconds");
    result.skip_reason = Some(ImportSkipReason::ArchiveExtractionTimedOut);

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Error);
    assert!(td.import_execution_retry.is_none());
}

#[tokio::test]
async fn apply_result_blocks_missing_archive_extractor_without_retry() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let mut result = failed_execution_result("archive extractor plugin is required");
    result.skip_reason = Some(ImportSkipReason::ArchiveExtractionPluginRequired);

    assert!(!apply_import_result(&app, &mut td, result, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(td.status, TrackedDownloadStatus::Error);
    assert!(td.import_execution_retry.is_none());
}

#[tokio::test]
async fn apply_result_retries_disk_full_skip_and_clears_counter_on_import() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let app = build_app(
        vec![title],
        vec![collection],
        vec![episode],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )],
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    let mut disk_full = failed_execution_result("insufficient disk space for import");
    disk_full.decision = ImportDecision::Skipped;
    disk_full.skip_reason = Some(ImportSkipReason::DiskFull);

    assert!(!apply_import_result(&app, &mut td, disk_full, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert_eq!(td.import_execution_retry.as_ref().unwrap().attempts, 1);

    let mut imported = failed_execution_result("episode already imported");
    imported.decision = ImportDecision::Skipped;
    imported.skip_reason = Some(ImportSkipReason::AlreadyImported);
    assert!(apply_import_result(&app, &mut td, imported, 0).await);
    assert_eq!(td.state, TrackedDownloadState::Imported);
    assert!(td.import_execution_retry.is_none());
}

#[tokio::test]
async fn apply_result_clears_execution_retry_when_a_later_attempt_is_rejected() {
    let app = build_app(Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    assert!(!apply_import_result(&app, &mut td, failed_execution_result("io failure"), 0).await);
    assert_eq!(td.import_execution_retry.as_ref().unwrap().attempts, 1);

    let mut rejected = failed_execution_result("existing episode file is equal or better");
    rejected.decision = ImportDecision::Rejected;
    rejected.skip_reason = Some(ImportSkipReason::PolicyMismatch);
    assert!(!apply_import_result(&app, &mut td, rejected, 0).await);
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert!(td.import_execution_retry.is_none());
}
