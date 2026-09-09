//! Removal-gate and import-mode wiring for torrent seeding.
//!
//! The gate's own decision table is unit-tested in
//! `crate::seeding_gate`; these tests cover the wiring: which downloads reach
//! the gate, what the terminal-cleanup path does with each verdict, and that a
//! configured `Move` is downgraded while a torrent is still seeding.

use super::*;
use crate::import::import::TerminalDownloadCleanupOutcome;
use crate::tracked_downloads::{TrackedDownload, tracked_download_id};
use scryer_domain::{
    DownloadQueueState, DownloadSeedingSnapshot, ImportMode, MediaFacet, NewTitle,
};

/// A plugin provider that reports torrent inputs, so `client_type_is_torrent`
/// classifies the fixture clients the way a real install would. Without one,
/// only the built-in usenet clients are known and every plugin client would
/// look protocol-less.
struct TorrentPluginProvider {
    torrent_types: Vec<String>,
}

impl TorrentPluginProvider {
    fn new(types: &[&str]) -> Self {
        Self {
            torrent_types: types.iter().map(|value| (*value).to_string()).collect(),
        }
    }
}

impl DownloadClientPluginProvider for TorrentPluginProvider {
    fn client_for_config(
        &self,
        config: &scryer_domain::DownloadClientConfig,
    ) -> Option<Arc<dyn DownloadClient>> {
        Some(Arc::new(StubDownloadClient {
            native_data_removal: config.client_type == "qbittorrent",
            ..Default::default()
        }))
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.torrent_types.clone()
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        if self
            .torrent_types
            .iter()
            .any(|value| value.eq_ignore_ascii_case(provider_type))
        {
            vec!["magnet_uri".to_string(), "torrent_file".to_string()]
        } else {
            vec![]
        }
    }
}

fn bootstrap_with_torrent_clients(
    download_client: Arc<StubDownloadClient>,
) -> (AppUseCase, User, Arc<TrackingDownloadSubmissionRepo>) {
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let (mut app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );
    // Protocol classification comes from the client's declared accepted
    // inputs, so the fixture needs a provider that declares torrent inputs;
    // without one every plugin client would look protocol-less and the gate
    // would never engage.
    app.services.integrations.download_client_plugin_provider =
        crate::RuntimeFeature::enabled(Arc::new(TorrentPluginProvider::new(&[
            "qbittorrent",
            "rtorrent",
            "downloadstation",
            "aria2",
            "torrent-blackhole",
        ])) as Arc<dyn DownloadClientPluginProvider>);
    (app, user, download_submissions)
}

async fn movie_title(app: &AppUseCase, user: &User, name: &str) -> scryer_domain::Title {
    app.add_title(
        user,
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
    )
    .await
    .expect("create monitored movie title")
}

fn tracked_for(
    client_id: &str,
    client_type: &str,
    item_id: &str,
    title: &scryer_domain::Title,
    state: TrackedDownloadState,
    is_trackable: bool,
) -> TrackedDownload {
    let mut client_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    client_item.client_id = client_id.to_string();
    client_item.client_type = client_type.to_string();
    client_item.title_id = Some(title.id.clone());
    client_item.title_name = title.name.clone();
    client_item.facet = Some("movie".to_string());
    TrackedDownload {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        id: tracked_download_id(Some(client_id), client_type, item_id),
        client_id: client_id.to_string(),
        client_type: client_type.to_string(),
        client_item,
        completed_source: None,
        state,
        status: scryer_domain::TrackedDownloadStatus::Ok,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some(title.name.clone()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable,
        import_attempted: true,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        burned_by_import_gate: false,
        snapshot_missing_since: None,
    }
}

#[tokio::test]
async fn an_imported_torrent_is_held_instead_of_removed_when_seeding_cannot_be_proven_done() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seeding Hold").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-hold-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(
        !crate::import::import::terminal_download_cleanup_is_complete(outcome.outcome),
        "a held torrent must not settle: it has to re-enter the gate next poll"
    );
    assert!(
        download_client.deleted_requests.lock().await.is_empty(),
        "the client entry must not be removed while the torrent may still owe seeding"
    );
}

#[tokio::test]
async fn an_already_held_torrent_re_enters_the_gate_and_is_still_not_removed() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seeding Rehold").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-hold-2",
        &title,
        TrackedDownloadState::ImportedSeeding,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::ImportedSeeding,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn a_torrent_that_left_the_client_settles_without_a_removal_call() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seeding Vanished").await;

    // `is_trackable: false` is how the tracker records "absent from the
    // client's snapshot past the grace window" — a `removes_on_seed_limit`
    // client, or an operator who pulled it by hand.
    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-gone-1",
        &title,
        TrackedDownloadState::ImportedSeeding,
        false,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::ImportedSeeding,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::AlreadyGone);
    assert!(crate::import::import::terminal_download_cleanup_is_complete(outcome.outcome));
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn torrent_blackhole_entries_are_never_auto_removed() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config =
        create_enabled_download_client_config(&app, &user, "Watch Folder", "torrent-blackhole")
            .await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Blackhole Movie").await;

    let tracked = tracked_for(
        &config.id,
        "torrent-blackhole",
        "watch-entry-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    // Removal here is `remove_dir_all` against a directory an external client
    // is still seeding from, so the entry settles without being touched.
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::SeedingEntryKept);
    assert!(crate::import::import::terminal_download_cleanup_is_complete(outcome.outcome));
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

async fn host_payload_fixture(
    app: &AppUseCase,
    user: &scryer_domain::User,
    client: &StubDownloadClient,
    tracked: &TrackedDownload,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source_title = tracked.source_title.as_deref().unwrap_or("Release");
    let payload = root.path().join(source_title);
    record_host_job_ownership(app, tracked, source_title).await;
    configure_host_cleanup_root(app, user, &tracked.client_id, root.path()).await;
    std::fs::create_dir(&payload).unwrap();
    std::fs::write(payload.join("movie.mkv"), b"media").unwrap();
    client
        .set_client_status(crate::DownloadClientStatus {
            remote_output_roots: vec![root.path().display().to_string()],
            ..Default::default()
        })
        .await;
    client
        .completed_downloads
        .lock()
        .await
        .push(scryer_domain::CompletedDownload {
            client_type: tracked.client_type.clone(),
            client_id: tracked.client_id.clone(),
            download_client_item_id: tracked.client_item.download_client_item_id.clone(),
            download_id: Some(tracked.download_id.to_wire()),
            name: source_title.into(),
            release_name: None,
            dest_dir: payload.display().to_string(),
            category: Some("scryer-movies".into()),
            size_bytes: None,
            completed_at: None,
            parameters: vec![],
        });
    (root, payload)
}

async fn configure_host_cleanup_root(
    app: &AppUseCase,
    user: &scryer_domain::User,
    client_id: &str,
    root: &std::path::Path,
) {
    app.update_download_client_config(
        user,
        crate::DownloadClientConfigUpdate {
            id: client_id.into(),
            config_json: Some(
                serde_json::json!({
                    "remote_path_mappings": format!("{} => {}", root.display(), root.display())
                })
                .to_string(),
            ),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

async fn record_host_job_ownership(
    app: &AppUseCase,
    tracked: &TrackedDownload,
    source_title: &str,
) {
    app.services
        .workflow
        .download_submissions
        .record_submission(DownloadSubmission {
            download_id: tracked.download_id,
            title_id: tracked.title_id.clone().unwrap_or_default(),
            purpose: DownloadSubmissionPurpose::Standard,
            facet: tracked.facet.clone().unwrap_or_default(),
            download_client_id: Some(tracked.client_id.clone()),
            download_client_type: tracked.client_type.clone(),
            download_client_item_id: tracked.client_item.download_client_item_id.clone(),
            source_title: Some(source_title.into()),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .unwrap();
}

async fn record_file_import_ownership(
    app: &mut AppUseCase,
    tracked: &TrackedDownload,
    path: &Path,
) {
    let imports = Arc::new(TrackingImportRepo::default());
    let import_id = imports
        .queue_import_request(
            ClientJobLocator::new(
                Some(&tracked.client_id),
                &tracked.client_type,
                &tracked.client_item.download_client_item_id,
            ),
            "completed_download".into(),
            "{}".into(),
        )
        .await
        .unwrap();
    imports
        .update_import_status(
            &import_id,
            ImportStatus::Completed,
            Some(serde_json::json!({"decision": "imported", "source_path": path}).to_string()),
        )
        .await
        .unwrap();
    imports
        .canonical_ids
        .lock()
        .await
        .insert(import_id, tracked.download_id);
    app.services.workflow.imports = imports;
}

fn cleanup_record_for(tracked: &TrackedDownload) -> crate::DownloadCleanupRecord {
    crate::DownloadCleanupRecord {
        download_id: tracked.download_id,
        client_id: tracked.client_id.clone(),
        client_type: tracked.client_type.clone(),
        item_id: tracked.client_item.download_client_item_id.clone(),
        tracked_state: tracked.state.as_str().into(),
        title_id: tracked.title_id.clone(),
        facet: tracked.facet.clone(),
        source_title: tracked.source_title.clone(),
        attempts: 1,
        history_offset: 0,
        payload_checkpoint: None,
    }
}

#[tokio::test]
async fn durable_payload_checkpoint_does_not_delete_recreated_content() {
    for client_type in ["nzbget", "sabnzbd"] {
        let client = Arc::new(StubDownloadClient::default());
        let (app, user, repository) = bootstrap_with_torrent_clients(client.clone());
        let config =
            create_enabled_download_client_config(&app, &user, client_type, client_type).await;
        set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
        let title = movie_title(&app, &user, "Checkpoint Recovery").await;
        let mut tracked = tracked_for(
            &config.id,
            client_type,
            "checkpoint-job",
            &title,
            TrackedDownloadState::Imported,
            true,
        );
        tracked.client_item.download_id = Some(tracked.download_id.to_wire());
        client
            .history_items
            .lock()
            .await
            .push(tracked.client_item.clone());
        let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
        let mut record = cleanup_record_for(&tracked);
        client.set_delete_error(Some("entry delete failed")).await;
        let first = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
            .await
            .unwrap();
        assert_eq!(first.2, TerminalDownloadCleanupOutcome::RetryableFailure);
        assert!(!payload.exists());
        let checkpoint = repository.cleanup_checkpoints.lock().await[&record.download_id].clone();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&checkpoint).unwrap()["payload_removed"],
            true
        );

        std::fs::create_dir(&payload).unwrap();
        std::fs::write(payload.join("unrelated.txt"), b"keep this new content").unwrap();
        record.payload_checkpoint = Some(checkpoint.clone());
        record.attempts = 2;
        client.set_delete_error(None).await;
        // Payload proof also avoids depending on a mount/status read after restart.
        *client.client_status.lock().await = None;
        let retry = crate::import::import::run_claimed_download_cleanup(&app, record, None)
            .await
            .unwrap();
        assert_eq!(retry.2, TerminalDownloadCleanupOutcome::Removed);
        assert_eq!(
            std::fs::read(payload.join("unrelated.txt")).unwrap(),
            b"keep this new content"
        );
        assert_eq!(
            repository.cleanup_checkpoints.lock().await[&tracked.download_id],
            checkpoint
        );
        assert!(
            !client.deleted_requests.lock().await.last().unwrap().4,
            "entry retries must not repeat backend payload deletion either"
        );
    }
}

#[tokio::test]
async fn durable_pause_failure_stays_retryable_until_pause_succeeds() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        client.clone(),
        "Pause Retry",
        "pause-retry",
        Some(PersistedSeedGoals {
            goal_met_action: Some(scryer_domain::SeedGoalMetAction::StopSeeding),
            ..persisted_goals(false)
        }),
    )
    .await;
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(2.5),
            ..Default::default()
        },
        &mut tracked,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let record = cleanup_record_for(&tracked);
    *client.pause_error.lock().await = Some("temporary client outage".into());
    let first = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(first.2, TerminalDownloadCleanupOutcome::RetryableFailure);
    assert!(!crate::import::import::terminal_download_cleanup_is_complete(first.2.outcome));
    *client.pause_error.lock().await = None;
    let retry = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(retry.2, TerminalDownloadCleanupOutcome::SeedingEntryKept);
    assert!(crate::import::import::terminal_download_cleanup_is_complete(retry.2.outcome));
    assert_eq!(client.paused_requests.lock().await.len(), 1);
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn host_cleanup_preserves_custom_recycle_roots_and_defers_on_settings_failure() {
    let client = Arc::new(StubDownloadClient::default());
    let (mut app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let settings = Arc::new(StoredSettingsRepo::default());
    app.services.config.settings = settings.clone();
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Recycle Protection").await;
    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "recycle-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
    app.services.catalog.libraries = Arc::new(MockLibraryRepo::default());
    *settings.read_error_key.lock().await =
        Some(crate::settings::keys::RECYCLE_BIN_PATH_KEY.into());
    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
    assert!(payload.join("movie.mkv").exists());
    assert!(client.deleted_requests.lock().await.is_empty());

    *settings.read_error_key.lock().await = None;
    settings
        .set_value(
            crate::settings::keys::SETTINGS_SCOPE_MEDIA,
            crate::settings::keys::RECYCLE_BIN_PATH_KEY,
            &payload.display().to_string(),
        )
        .await;
    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
    assert!(payload.join("movie.mkv").exists());
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn durable_torrent_cleanup_uses_frozen_info_hash_when_the_internal_name_differs() {
    for client_type in ["qbittorrent", "rtorrent"] {
        let client = Arc::new(StubDownloadClient::default());
        let (app, user, submissions) = bootstrap_with_torrent_clients(client.clone());
        let config =
            create_enabled_download_client_config(&app, &user, client_type, client_type).await;
        set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
        let title = movie_title(&app, &user, "Paperman.2012.720p.WEB-DL").await;
        let hash = "5b4ba671c3729e34718de86e3372be2ecb527b15";
        let mut tracked = tracked_for(
            &config.id,
            client_type,
            hash,
            &title,
            TrackedDownloadState::Imported,
            false,
        );
        tracked.client_item.download_id = None;
        tracked.client_item.title_name = format!("{}.mkv", title.name);
        tracked.client_item.seeding = Some(DownloadSeedingSnapshot {
            is_private: Some(true),
            can_remove: Some(true),
            ..Default::default()
        });
        record_host_job_ownership(&app, &tracked, &title.name).await;
        submissions.store.lock().await[0].info_hash = Some(hash.into());
        app.services
            .workflow
            .download_registry
            .resolve_observation(&ObservedClientJob {
                locator: ClientJobLocator::new(Some(&config.id), client_type, hash),
                wire_token: Some(tracked.download_id.to_wire()),
                observed_name: Some(title.name.clone()),
                observed_at: Utc::now(),
            })
            .await
            .unwrap();
        client
            .history_items
            .lock()
            .await
            .push(tracked.client_item.clone());
        let record = cleanup_record_for(&tracked);
        let recovered =
            crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
                .await
                .expect("the same torrent must reach the seeding gate despite its internal name");
        assert_eq!(recovered.2, TerminalDownloadCleanupOutcome::HeldForSeeding);
        assert_eq!(
            recovered.2.seeding.unwrap().reason,
            "private_torrent_without_resolved_goals"
        );
        assert!(client.deleted_requests.lock().await.is_empty());

        // A different persisted content identity is rejected even with the same name.
        let mut items = client.history_items.lock().await;
        items[0].title_name = title.name.clone();
        drop(items);
        submissions.store.lock().await[0].info_hash = Some("a".repeat(40));
        assert!(
            crate::import::import::run_claimed_download_cleanup(&app, record, None)
                .await
                .is_none()
        );
        assert!(client.deleted_requests.lock().await.is_empty());
    }
}

#[tokio::test]
async fn durable_cleanup_recovers_an_untrackable_import_and_rejects_a_reused_locator() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Recovered Import").await;
    let mut tracked = tracked_for(
        &config.id,
        "weaver",
        "123",
        &title,
        TrackedDownloadState::Imported,
        false,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let record = cleanup_record_for(&tracked);
    let recovered = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(recovered.2, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(recovered.1.unwrap().id, tracked.id);
    assert_eq!(client.deleted_requests.lock().await.len(), 1);
    client.history_items.lock().await[0].download_id =
        Some(scryer_domain::download_identity::DownloadId::new().to_wire());
    assert!(
        crate::import::import::run_claimed_download_cleanup(&app, record, None)
            .await
            .is_none()
    );
    assert_eq!(client.deleted_requests.lock().await.len(), 1);
}

#[tokio::test]
async fn durable_entry_only_cleanup_recovers_absence_but_not_unknown_or_reused_identity() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, repository) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Failed entry recovery").await;
    let mut tracked = tracked_for(
        &config.id,
        "weaver",
        "failed-entry",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let mut record = cleanup_record_for(&tracked);
    client.set_delete_error(Some("response lost")).await;
    let first = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(first.2, TerminalDownloadCleanupOutcome::RetryableFailure);
    record.payload_checkpoint =
        Some(repository.cleanup_checkpoints.lock().await[&record.download_id].clone());
    let checkpoint: serde_json::Value =
        serde_json::from_str(record.payload_checkpoint.as_ref().unwrap()).unwrap();
    assert_eq!(checkpoint["entry_only_remove_requested"], true);
    assert_eq!(checkpoint["native_remove_requested"], false);
    client.history_items.lock().await.clear();
    *client.observation_error.lock().await = Some("offline".into());
    assert!(
        crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
            .await
            .is_none()
    );
    *client.observation_error.lock().await = None;
    // A checkpoint minted for another job identity is never credited as this
    // row's recovery: absence still settles the row (nothing can change on
    // retry), but as an unverified payload, not a recovered removal.
    let mut wrong = record.clone();
    wrong.item_id = "different-job".into();
    let unverified = crate::import::import::run_claimed_download_cleanup(&app, wrong, None)
        .await
        .unwrap();
    assert_eq!(unverified.2, TerminalDownloadCleanupOutcome::AlreadyGone);
    let last_finish =
        |finished: &[(scryer_domain::download_identity::DownloadId, String, bool)]| {
            finished
                .last()
                .map(|(_, outcome, complete)| (outcome.clone(), *complete))
        };
    assert_eq!(
        last_finish(&repository.finished_cleanup.lock().await),
        Some(("entry_absent_payload_unverified".to_string(), true))
    );
    let recovered = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(recovered.2, TerminalDownloadCleanupOutcome::AlreadyGone);
    assert_eq!(
        last_finish(&repository.finished_cleanup.lock().await),
        Some(("entry_absent".to_string(), true))
    );
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn host_cleanup_preserves_unowned_files_inside_an_authorized_root() {
    for client_type in ["nzbget", "sabnzbd"] {
        let client = Arc::new(StubDownloadClient::default());
        let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
        let config =
            create_enabled_download_client_config(&app, &user, client_type, client_type).await;
        set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
        let title = movie_title(&app, &user, "File ownership").await;
        let tracked = tracked_for(
            &config.id,
            client_type,
            "owned-job",
            &title,
            TrackedDownloadState::Imported,
            true,
        );
        let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
        let file = payload.join("movie.mkv");
        client.completed_downloads.lock().await[0].dest_dir = file.display().to_string();
        let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            TrackedDownloadState::Imported,
            None,
        )
        .await;
        assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
        assert_eq!(std::fs::read(&file).unwrap(), b"media");
        assert!(client.deleted_requests.lock().await.is_empty());
        let other = payload.parent().unwrap().join("Unrelated job");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("keep.mkv"), b"other media").unwrap();
        {
            let mut completed = client.completed_downloads.lock().await;
            completed[0].name = "Unrelated job".into();
            completed[0].dest_dir = other.display().to_string();
        }
        let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            TrackedDownloadState::Imported,
            None,
        )
        .await;
        assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
        assert!(other.join("keep.mkv").exists());
        assert!(client.deleted_requests.lock().await.is_empty());
    }
}

#[tokio::test]
async fn host_file_cleanup_requires_a_successful_import_for_the_same_canonical_job() {
    let client = Arc::new(StubDownloadClient::default());
    let (mut app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let imports = Arc::new(TrackingImportRepo::default());
    app.services.workflow.imports = imports.clone();
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Verified source file").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "file-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
    let file = payload.join("movie.mkv");
    client.completed_downloads.lock().await[0].dest_dir = file.display().to_string();
    let import_id = imports
        .queue_import_request(
            ClientJobLocator::new(Some(&config.id), "nzbget", "file-job"),
            "completed_download".into(),
            "{}".into(),
        )
        .await
        .unwrap();
    imports
        .update_import_status(
            &import_id,
            ImportStatus::Completed,
            Some(serde_json::json!({"decision": "imported", "source_path": file}).to_string()),
        )
        .await
        .unwrap();
    imports.canonical_ids.lock().await.insert(
        import_id.clone(),
        scryer_domain::download_identity::DownloadId::new(),
    );
    let record = cleanup_record_for(&tracked);
    let wrong_job = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(
        wrong_job.2,
        TerminalDownloadCleanupOutcome::RetryableFailure
    );
    assert!(file.exists());
    assert!(client.deleted_requests.lock().await.is_empty());
    imports
        .canonical_ids
        .lock()
        .await
        .insert(import_id, record.download_id);
    let owned_job = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(owned_job.2, TerminalDownloadCleanupOutcome::Removed);
    assert!(!file.exists());
    assert_eq!(client.deleted_requests.lock().await.len(), 1);
}

#[tokio::test]
async fn durable_native_cleanup_recovers_a_lost_response_without_redeleting() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, repository) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Lost native response").await;
    let mut tracked = tracked_for(
        &config.id,
        "weaver",
        "125",
        &title,
        TrackedDownloadState::Imported,
        false,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let mut record = cleanup_record_for(&tracked);
    client.set_delete_error(Some("response lost")).await;
    let first = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(first.2, TerminalDownloadCleanupOutcome::RetryableFailure);
    record.payload_checkpoint =
        Some(repository.cleanup_checkpoints.lock().await[&record.download_id].clone());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(record.payload_checkpoint.as_ref().unwrap())
            .unwrap()["native_remove_requested"],
        true
    );
    client.history_items.lock().await.clear();
    *client.observation_error.lock().await = Some("offline".into());
    assert!(
        crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
            .await
            .is_none()
    );
    *client.observation_error.lock().await = None;
    let recovered = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(recovered.2, TerminalDownloadCleanupOutcome::AlreadyGone);
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn durable_cleanup_replays_retained_outcomes_without_client_reads() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, repository) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    let title = movie_title(&app, &user, "Retained entry").await;
    let tracked = tracked_for(
        &config.id,
        "weaver",
        "127",
        &title,
        TrackedDownloadState::Imported,
        false,
    );
    *client.observation_error.lock().await = Some("offline".into());
    for (outcome, expected) in [
        (
            "policy_retained",
            TerminalDownloadCleanupOutcome::NotConfigured,
        ),
        ("handed_off", TerminalDownloadCleanupOutcome::HandedOff),
        (
            "seeding_entry_kept",
            TerminalDownloadCleanupOutcome::SeedingEntryKept,
        ),
    ] {
        repository
            .settled_cleanup
            .lock()
            .await
            .insert(tracked.download_id, outcome.into());
        let result = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            tracked.state,
            None,
        )
        .await;
        assert_eq!(result, expected);
        assert!(crate::import::import::terminal_download_cleanup_is_complete(result.outcome));
    }
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn durable_ignored_orphan_cleanup_removes_only_entry() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    let title = movie_title(&app, &user, "Ignored orphan").await;
    let mut tracked = tracked_for(
        &config.id,
        "weaver",
        "126",
        &title,
        TrackedDownloadState::Ignored,
        false,
    );
    tracked.title_id = None;
    tracked.facet = None;
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let record = cleanup_record_for(&tracked);
    let result = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(result.2, TerminalDownloadCleanupOutcome::Removed);
    assert!(!client.deleted_requests.lock().await[0].4);
    client.history_items.lock().await.clear();
    let result = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(result.2, TerminalDownloadCleanupOutcome::AlreadyGone);
}

#[tokio::test]
async fn durable_cleanup_settles_an_absent_entry_without_a_payload_checkpoint() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Lost Response").await;
    let tracked = tracked_for(
        &config.id,
        "weaver",
        "124",
        &title,
        TrackedDownloadState::Imported,
        false,
    );
    let mut record = cleanup_record_for(&tracked);
    // Absence is the adapter's authoritative answer and there is nothing left
    // to verify: the row settles as gone on its first attempt with the payload
    // reported unverified, rather than burning a retry budget (an upgrade
    // backfills every historical download through here).
    let settled = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(settled.2, TerminalDownloadCleanupOutcome::AlreadyGone);
    record.payload_checkpoint = Some(r#"{"payload_removed":true}"#.into());
    let recovered = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
        .await
        .unwrap();
    assert_eq!(recovered.2, TerminalDownloadCleanupOutcome::AlreadyGone);
    *client.observation_error.lock().await = Some("client offline".into());
    assert!(
        crate::import::import::run_claimed_download_cleanup(&app, record, None)
            .await
            .is_none()
    );
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn host_cleanup_verifies_payload_files_without_operator_roots() {
    // Deletion authority comes from the per-file inventory check, not from an
    // operator-configured root: an install with no path mappings (or
    // mappings that do not contain the payload) still cleans up its own job.
    for client_type in ["nzbget", "sabnzbd", "rtorrent", "downloadstation", "aria2"] {
        let client = Arc::new(StubDownloadClient::default());
        let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
        let config =
            create_enabled_download_client_config(&app, &user, client_type, client_type).await;
        set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
        let title = movie_title(&app, &user, "Root authority").await;
        let mut tracked = tracked_for(
            &config.id,
            client_type,
            "root-authority",
            &title,
            TrackedDownloadState::Imported,
            true,
        );
        observed(
            DownloadSeedingSnapshot {
                can_remove: Some(true),
                can_move_files: Some(true),
                ..Default::default()
            },
            &mut tracked,
        );
        let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
        let configured = tempfile::tempdir().unwrap();
        for mapping in [
            String::new(),
            format!(
                "{} => {}",
                configured.path().display(),
                configured.path().display()
            ),
        ] {
            if !payload.exists() {
                std::fs::create_dir(&payload).unwrap();
                std::fs::write(payload.join("movie.mkv"), b"media").unwrap();
            }
            client.deleted_requests.lock().await.clear();
            app.update_download_client_config(
                &user,
                crate::DownloadClientConfigUpdate {
                    id: config.id.clone(),
                    config_json: Some(
                        serde_json::json!({"remote_path_mappings":mapping}).to_string(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
                &app,
                &tracked,
                tracked.state,
                None,
            )
            .await;
            assert_eq!(
                outcome,
                TerminalDownloadCleanupOutcome::Removed,
                "{client_type} mapping={mapping:?}"
            );
            assert!(!payload.exists(), "{client_type} mapping={mapping:?}");
            let requests = client.deleted_requests.lock().await;
            assert_eq!(requests.len(), 1, "{client_type}");
            assert_eq!(requests[0].4, client_type == "sabnzbd", "{client_type}");
        }
    }
}

#[tokio::test]
async fn host_cleanup_retains_unrecognised_files_and_reports_a_partial_payload() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, repository) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Partial Payload").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "partial-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
    std::fs::create_dir(payload.join("Sample")).unwrap();
    std::fs::write(payload.join("Sample").join("sample.mkv"), b"sample").unwrap();
    std::fs::write(payload.join("release.nfo"), b"nfo").unwrap();
    // Not a payload class Scryer knows: whatever it is, it is not Scryer's to
    // delete, and neither is the directory holding it.
    std::fs::write(payload.join("notes.bin"), b"operator data").unwrap();
    let record = cleanup_record_for(&tracked);
    let result = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(result.2, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        result.2.payload,
        Some(crate::import::import::HostPayloadDisposition::PartiallyRetained)
    );
    assert!(!payload.join("movie.mkv").exists());
    assert!(!payload.join("release.nfo").exists());
    assert!(!payload.join("Sample").exists());
    assert_eq!(
        std::fs::read(payload.join("notes.bin")).unwrap(),
        b"operator data"
    );
    let checkpoint: serde_json::Value =
        serde_json::from_str(&repository.cleanup_checkpoints.lock().await[&tracked.download_id])
            .unwrap();
    assert_eq!(checkpoint["payload_removed"], true);
    assert_eq!(checkpoint["disposition"], "partially_retained");
    let requests = client.deleted_requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].4);
}

#[tokio::test]
async fn host_cleanup_skips_files_that_changed_since_the_inventory() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Changed Payload").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "changed-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
    let completed = client.completed_downloads.lock().await[0].clone();
    // An inventory persisted before a crash that no longer matches what is on
    // disk: the file was rewritten since. It must be left alone.
    let mut record = cleanup_record_for(&tracked);
    record.attempts = 2;
    record.payload_checkpoint = Some(
        serde_json::json!({
            "completed": completed,
            "payload_removed": false,
            "target": payload,
            "inventory": {
                "target": payload,
                "entries": [{
                    "path": "movie.mkv", "len": 5, "modified_unix_nanos": 1,
                    "dev": null, "ino": null, "sample_bytes": 5,
                    "sample_blake3": "stale"
                }],
                "retained": [],
                "directories": []
            }
        })
        .to_string(),
    );
    let result = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(result.2, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        result.2.payload,
        Some(crate::import::import::HostPayloadDisposition::PartiallyRetained)
    );
    assert_eq!(std::fs::read(payload.join("movie.mkv")).unwrap(), b"media");
    assert!(payload.exists());
    assert_eq!(client.deleted_requests.lock().await.len(), 1);
}

#[tokio::test]
async fn host_cleanup_never_follows_a_symlinked_ancestor_planted_after_the_inventory() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Moved Subdirectory").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "moved-sub-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
    std::fs::create_dir(payload.join("Sub")).unwrap();
    std::fs::write(payload.join("Sub").join("clip.mkv"), b"clip").unwrap();
    let inventory = crate::import::import::inventory_payload_directory_blocking(&payload).unwrap();
    assert_eq!(inventory.entries.len(), 2);
    let completed = client.completed_downloads.lock().await[0].clone();

    // Between the inventory and the retry the subdirectory was moved out (same
    // inodes, sizes, hashes) and a symlink left in its place.
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::rename(payload.join("Sub"), elsewhere.path().join("Sub")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join("Sub"), payload.join("Sub")).unwrap();

    let mut record = cleanup_record_for(&tracked);
    record.attempts = 2;
    record.payload_checkpoint = Some(
        serde_json::json!({
            "completed": completed,
            "payload_removed": false,
            "target": payload,
            "inventory": inventory,
        })
        .to_string(),
    );
    let result = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(result.2, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        result.2.payload,
        Some(crate::import::import::HostPayloadDisposition::PartiallyRetained)
    );
    assert!(!payload.join("movie.mkv").exists());
    assert_eq!(
        std::fs::read(elsewhere.path().join("Sub").join("clip.mkv")).unwrap(),
        b"clip",
        "a file reached through a symlinked ancestor is never deleted"
    );
    assert!(payload.exists());
}

#[tokio::test]
async fn host_cleanup_in_an_unproven_directory_removes_only_scryer_imported_files() {
    let client = Arc::new(StubDownloadClient::default());
    let (mut app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Shared Folder").await;
    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "shared-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let (root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
    // The client reports a directory that is not named for the release (a
    // shared completed folder, or a client-renamed duplicate). Only the file
    // Scryer's own import record proves it imported from there may go.
    let shared = root.path().join("Completed Jobs");
    std::fs::rename(&payload, &shared).unwrap();
    std::fs::write(shared.join("keep.mkv"), b"someone else's").unwrap();
    client.completed_downloads.lock().await[0].dest_dir = shared.display().to_string();
    record_file_import_ownership(&mut app, &tracked, &shared.join("movie.mkv")).await;
    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        tracked.state,
        None,
    )
    .await;
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert!(!shared.join("movie.mkv").exists());
    assert_eq!(
        std::fs::read(shared.join("keep.mkv")).unwrap(),
        b"someone else's"
    );
    assert!(shared.exists());
    assert_eq!(client.deleted_requests.lock().await.len(), 1);
}

#[tokio::test]
async fn host_cleanup_gives_up_after_the_retry_budget_and_removes_the_entry() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, repository) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Unverifiable Payload").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "unverifiable-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    tracked.client_item.download_id = Some(tracked.download_id.to_wire());
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    let (root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
    let unrelated = root.path().join("Unrelated job");
    std::fs::rename(&payload, &unrelated).unwrap();
    client.completed_downloads.lock().await[0].dest_dir = unrelated.display().to_string();
    let mut record = cleanup_record_for(&tracked);
    // The budget counts host deletion failures, not cleanup claims: a row that
    // was claimed many times for seeding holds still gets its full retries.
    record.attempts = 40;
    for failure in 1..crate::import::import::HOST_PAYLOAD_CLEANUP_ATTEMPTS_BEFORE_ENTRY_REMOVAL {
        let held = crate::import::import::run_claimed_download_cleanup(&app, record.clone(), None)
            .await
            .unwrap();
        assert_eq!(
            held.2,
            TerminalDownloadCleanupOutcome::RetryableFailure,
            "failure {failure}"
        );
        assert!(client.deleted_requests.lock().await.is_empty());
        let checkpoint = repository.cleanup_checkpoints.lock().await[&record.download_id].clone();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&checkpoint).unwrap()["host_payload_failures"],
            failure
        );
        record.payload_checkpoint = Some(checkpoint);
        record.attempts += 1;
    }

    // Sonarr parity: after the budget the entry goes and the payload stays for
    // the operator, reported as unverified rather than silently.
    let released = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(released.2, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        released.2.payload,
        Some(crate::import::import::HostPayloadDisposition::Unverified)
    );
    assert_eq!(
        std::fs::read(unrelated.join("movie.mkv")).unwrap(),
        b"media"
    );
    let requests = client.deleted_requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert!(
        !requests[0].4,
        "an unverified payload is never deleted through the client either"
    );
}

#[tokio::test]
async fn durable_cleanup_abandons_unattributable_rows_at_once() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Orphan Row").await;
    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "orphan-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let mut record = cleanup_record_for(&tracked);
    record.title_id = None;
    record.facet = None;
    // Missing attribution never heals by retrying (manual import reopens the
    // row when it attributes it), so the row is abandoned on its first attempt.
    let settled = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(settled.2, TerminalDownloadCleanupOutcome::NotConfigured);
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn durable_cleanup_settles_when_the_client_no_longer_exists() {
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Deleted Client").await;
    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "deleted-client-job",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let mut record = cleanup_record_for(&tracked);
    record.client_id = "no-such-client".into();
    // Nothing is observed or deleted through a client that is gone.
    *client.observation_error.lock().await = Some("must not be consulted".into());
    let settled = crate::import::import::run_claimed_download_cleanup(&app, record, None)
        .await
        .unwrap();
    assert_eq!(settled.2, TerminalDownloadCleanupOutcome::NotConfigured);
    assert!(client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn host_cleanup_removes_payload_for_all_entry_only_client_families() {
    for client_type in ["nzbget", "sabnzbd", "rtorrent", "downloadstation", "aria2"] {
        let client = Arc::new(StubDownloadClient::default());
        let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
        let config =
            create_enabled_download_client_config(&app, &user, client_type, client_type).await;
        set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
        let title = movie_title(&app, &user, "Host Cleanup").await;
        let mut tracked = tracked_for(
            &config.id,
            client_type,
            "host-job",
            &title,
            TrackedDownloadState::Imported,
            true,
        );
        observed(
            DownloadSeedingSnapshot {
                can_remove: Some(true),
                can_move_files: Some(true),
                ..Default::default()
            },
            &mut tracked,
        );
        let (_root, payload) = host_payload_fixture(&app, &user, &client, &tracked).await;
        let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            TrackedDownloadState::Imported,
            None,
        )
        .await;
        assert_eq!(
            outcome,
            TerminalDownloadCleanupOutcome::Removed,
            "{client_type}"
        );
        assert!(!payload.exists(), "{client_type}");
        let requests = client.deleted_requests.lock().await;
        assert_eq!(requests.len(), 1, "{client_type}");
        assert_eq!(
            requests[0].4,
            client_type == "sabnzbd",
            "SAB-compatible backends also receive their supported deletion flags"
        );
    }
}

#[tokio::test]
async fn weaver_import_cleanup_removes_entry_and_payload() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Usenet Movie").await;

    let tracked = tracked_for(
        &config.id,
        "weaver",
        "nzb-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "nzb-1".to_string(),
            true,
            true,
        )]
    );
}

#[tokio::test]
async fn a_failed_torrent_without_removal_eligibility_keeps_its_entry_and_data() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Failed Torrent").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-failed-1",
        &title,
        TrackedDownloadState::Failed,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

/// Sonarr parity: a torrent warning is never auto-failed, profile or not.
///
/// This test used to pin the opposite for a profile-less torrent — timeout,
/// FailedPending, client-failure cleanup. That path removed a client entry
/// without the seeding gate ever seeing it: a completed torrent warned on a
/// recoverable condition (disk full, permissions, a tracker hiccup) lost its
/// entry after a day, bypassing the private rail for exactly the torrents that
/// had no profile to protect them. The gate holds on unknown; the timeout must
/// not fail on it. Sonarr's `FailedDownloadService` acts only on
/// `Failed`/`IsEncrypted` and lets warnings persist for the operator.
#[tokio::test]
async fn a_warned_torrent_without_a_profile_is_never_timed_out() {
    let download_client = Arc::new(StubDownloadClient::default());
    // No seeding profile behind this grab — the case the old rule timed out.
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Timed Out Downloading Warning",
        "torrent-warning-downloading",
        None,
    )
    .await;
    tracked.state = TrackedDownloadState::Downloading;
    tracked.client_item.state = DownloadQueueState::Warning;
    tracked.client_item.attention_reason = Some("disk full".to_string());
    let id = tracked.id.clone();
    let timeout_applies = crate::app_usecase_integration::warning_timeout_applies(&app, &tracked);
    assert!(
        !timeout_applies,
        "a torrent warning must never be auto-failed, with or without a profile"
    );
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);
    let now = chrono::Utc::now();

    assert!(!tracker.fail_persistent_warning(&id, now, timeout_applies));
    assert!(!tracker.fail_persistent_warning(
        &id,
        now + crate::tracked_downloads::TrackedDownloadService::WARNING_FAILURE_TIMEOUT * 3,
        timeout_applies,
    ));
    let still_warned = tracker.find(&id).expect("warned torrent");
    assert_eq!(still_warned.state, TrackedDownloadState::Downloading);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

/// Usenet keeps the 24 h warning timeout: no seeding obligation exists, and a
/// stuck usenet download is exactly what failed-download handling is for.
#[tokio::test]
async fn a_warned_usenet_download_keeps_the_warning_timeout() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Warned Usenet Download",
        "usenet-warning-1",
        None,
    )
    .await;
    tracked.client_type = "sabnzbd".to_string();
    tracked.state = TrackedDownloadState::Downloading;
    tracked.client_item.state = DownloadQueueState::Warning;
    tracked.client_item.attention_reason = Some("unpack failed".to_string());

    assert!(
        crate::app_usecase_integration::warning_timeout_applies(&app, &tracked),
        "usenet downloads keep the warning timeout"
    );
}

/// The other half of Sonarr's failed-download rule: once the client itself says
/// the torrent is free to go, the data goes with the entry.
#[tokio::test]
async fn a_failed_torrent_its_client_will_release_takes_its_data_with_it() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Failed But Releasable").await;

    let mut tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-failed-2",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "torrent-failed-2".to_string(),
            true,
            true
        )]
    );
}

#[tokio::test]
async fn a_burned_usenet_failure_removes_history_with_data() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Burned Usenet").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-burned-1",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    tracked.burned_by_import_gate = true;

    let (_root, payload) = host_payload_fixture(&app, &user, &download_client, &tracked).await;
    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "nzb-burned-1".to_string(),
            true,
            false,
        )]
    );
    assert!(!payload.exists());
}

#[tokio::test]
async fn a_restart_recovers_burned_usenet_failure_cleanup_origin() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, download_submissions) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Restart Burned Usenet").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-burned-restart-1",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    tracked.client_item.download_id = Some("scryer-download:nzb-burned-restart-1".to_string());
    let (_root, payload) = host_payload_fixture(&app, &user, &download_client, &tracked).await;
    let identity = crate::tracked_downloads::observed_queue_item_identity(&tracked.client_item);
    let source_identity =
        crate::ClientJobLocator::new(Some(config.id.as_str()), "nzbget", "nzb-burned-restart-1");
    download_submissions
        .record_identity_tracked_state(
            &identity,
            Some(&source_identity),
            TrackedDownloadState::Failed.as_str(),
            Some("import_gate_rejected"),
            Some("release language does not match the title"),
        )
        .await
        .expect("record durable import-gate failure");
    assert_eq!(
        download_submissions
            .get_identity_tracked_state_reason(&identity, Some(&source_identity))
            .await
            .expect("read durable import-gate failure")
            .as_deref(),
        Some("import_gate_rejected")
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "nzb-burned-restart-1".to_string(),
            true,
            false,
        )]
    );
    assert!(!payload.exists());
}

#[tokio::test]
async fn a_restart_without_burned_origin_keeps_client_failure_cleanup() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Restart Client Failure").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-client-failure-restart-1",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    tracked.client_item.download_id =
        Some("scryer-download:nzb-client-failure-restart-1".to_string());

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "nzb-client-failure-restart-1".to_string(),
            true,
            false,
        )]
    );
}

#[tokio::test]
async fn a_burned_usenet_failure_respects_remove_failed_routing() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;
    let title = movie_title(&app, &user, "Keep Burned Usenet").await;
    let mut tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-burned-2",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    tracked.burned_by_import_gate = true;

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::NotConfigured);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn a_burned_torrent_failure_holds_until_its_seed_goal_is_met() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Burned Torrent Hold",
        "torrent-burned-hold",
        Some(persisted_goals(false)),
    )
    .await;
    tracked.state = TrackedDownloadState::Failed;
    tracked.burned_by_import_gate = true;
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            can_move_files: Some(true),
            seed_ratio: Some(0.7),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

/// Operator rule: the 24 h warning timeout applies only when no seeding
/// profile is attached. A torrent grabbed under a profile (here resolved from
/// its indexer) is that profile's business — failing and removing it after a
/// day would expose a private-tracker user to hit-and-run penalties — so the
/// warning stays visible and nothing is failed, however long it persists.
#[tokio::test]
async fn a_warned_torrent_under_a_seeding_profile_is_never_timed_out() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Warned Under A Seeding Profile",
        "torrent-warning-completed",
        Some(persisted_goals(false)),
    )
    .await;
    tracked.state = TrackedDownloadState::ImportPending;
    tracked.client_item.state = DownloadQueueState::Warning;
    tracked.client_item.attention_reason = Some("files are missing".to_string());
    let id = tracked.id.clone();
    let timeout_applies = crate::app_usecase_integration::warning_timeout_applies(&app, &tracked);
    assert!(
        !timeout_applies,
        "a torrent grabbed under a seeding profile must not be subject to the warning timeout"
    );
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);
    let now = chrono::Utc::now();

    assert!(!tracker.fail_persistent_warning(&id, now, timeout_applies));
    assert!(!tracker.fail_persistent_warning(
        &id,
        now + crate::tracked_downloads::TrackedDownloadService::WARNING_FAILURE_TIMEOUT * 3,
        timeout_applies,
    ));
    let still_warned = tracker.find(&id).expect("warned torrent");
    assert_eq!(still_warned.state, TrackedDownloadState::ImportPending);
    assert!(!still_warned.burned_by_import_gate);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn a_burned_torrent_failure_removes_entry_and_data_when_its_seed_goal_is_met() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Burned Torrent Released",
        "torrent-burned-release",
        Some(persisted_goals(false)),
    )
    .await;
    tracked.state = TrackedDownloadState::Failed;
    tracked.burned_by_import_gate = true;
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(2.4),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client
            .deleted_requests
            .lock()
            .await
            .iter()
            .map(|(_, _, item_id, is_history, remove_data)| {
                (item_id.clone(), *is_history, *remove_data)
            })
            .collect::<Vec<_>>(),
        vec![("torrent-burned-release".to_string(), true, true)]
    );
}

/// A client that can point at an unmet limit is not overruled by a failure.
#[tokio::test]
async fn a_failed_torrent_its_client_refuses_to_release_keeps_its_data() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Failed Still Seeding").await;

    let mut tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-failed-3",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(false),
            seed_ratio: Some(0.2),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

/// A blackhole "remove" is `remove_dir_all` on a watch folder an *external*
/// client is seeding from. Imported and failed states both preserve it,
/// whatever the client says.
#[tokio::test]
async fn a_failed_blackhole_entry_never_asks_for_its_watch_folder_to_be_deleted() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config =
        create_enabled_download_client_config(&app, &user, "Watch Folder", "torrent-blackhole")
            .await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Blackhole Failure").await;

    let mut tracked = tracked_for(
        &config.id,
        "torrent-blackhole",
        "/downloads/watch/blackhole-failed-1",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::SeedingEntryKept);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn a_torrent_with_removal_disabled_is_left_alone_without_engaging_the_gate() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, false, true).await;
    let title = movie_title(&app, &user, "Keep Everything").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-keep-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    // Nothing was ever going to be removed, so the download settles as
    // `Imported` rather than being parked in `ImportedSeeding` forever.
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::NotConfigured);
    assert!(crate::import::import::terminal_download_cleanup_is_complete(outcome.outcome));
}

// ── import mode ───────────────────────────────────────────────────────────

fn completed_for(client_id: &str, client_type: &str, item_id: &str) -> CompletedDownload {
    CompletedDownload {
        client_type: client_type.to_string(),
        client_id: client_id.to_string(),
        download_client_item_id: item_id.to_string(),
        download_id: None,
        name: "Example.Release.2024.1080p".to_string(),
        release_name: None,
        dest_dir: "/downloads/complete/example".to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![],
    }
}

#[tokio::test]
async fn a_configured_move_is_downgraded_to_copy_while_a_torrent_may_still_be_seeding() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    let title = movie_title(&app, &user, "Move Guard").await;
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("configure move import mode");
    assert_eq!(
        app.resolve_import_mode(Some(&title.library_id), &title.facet)
            .await
            .expect("resolve configured import mode"),
        ImportMode::Move,
        "the fixture must actually be configured for Move, or this test proves nothing"
    );

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "qbittorrent", "torrent-move-1")),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::HardlinkOrCopy);
}

/// What the plugin-host trust floor hands the gate, and what the gate must do
/// with it.
///
/// A pre-audit torrent plugin reports `can_remove: Some(true)` (registry
/// qBittorrent 1.0.5 hardcodes it) and computes `can_move_files` under the old
/// "safe to move while seeding" rule. `crates/scryer-plugins/src/seeding_trust.rs`
/// rewrites that pair into `can_remove: None` while leaving a refusal to move
/// alone; this is the receiving end of that hand-off — the shape reaches the
/// gate through the domain snapshot on the client row.
///
/// Both halves matter: the unknown verdict must Hold rather than release, and
/// the surviving `Some(false)` must still force a copy. Erasing that refusal
/// instead would have made the floor *upgrade* a stale plugin's import to
/// `Move`, since the gate reads stability as "not explicitly false".
#[tokio::test]
async fn the_shape_the_trust_floor_produces_holds_the_entry_and_keeps_the_import_a_copy() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config =
        create_enabled_download_client_config(&app, &user, "Stale qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Pre-Audit Plugin").await;
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("configure move import mode");

    let item_id = "torrent-below-floor-1";
    let mut tracked = tracked_for(
        &config.id,
        "qbittorrent",
        item_id,
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    observed(
        DownloadSeedingSnapshot {
            // Post-floor: the plugin said `Some(true)`, the host does not
            // believe it.
            can_remove: None,
            // Post-floor: the plugin's own refusal to move, kept verbatim.
            can_move_files: Some(false),
            seed_ratio: Some(0.3),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(
        outcome,
        TerminalDownloadCleanupOutcome::HeldForSeeding,
        "an unknown client verdict holds instead of releasing the entry"
    );
    assert_eq!(
        outcome.seeding.as_ref().map(|report| report.reason),
        Some("no_resolved_goals_and_client_verdict_unknown")
    );
    assert!(download_client.deleted_requests.lock().await.is_empty());

    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked.id.clone(),
            crate::tracked_downloads::TrackedDownloadQueueMetadata::from(&tracked),
        );

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "qbittorrent", item_id)),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(
        effective,
        ImportMode::HardlinkOrCopy,
        "a plugin that says the data is not stable still forces a copy"
    );
}

#[tokio::test]
async fn a_configured_move_survives_for_usenet_imports() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    let title = movie_title(&app, &user, "Move Usenet").await;
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("configure move import mode");

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "nzbget", "nzb-move-1")),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::Move);
}

#[tokio::test]
async fn a_configured_hardlink_or_copy_is_never_upgraded_by_the_gate() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    let title = movie_title(&app, &user, "Copy Stays Copy").await;

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "qbittorrent", "torrent-copy-1")),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::HardlinkOrCopy);
}

// ── tracked-state transition ──────────────────────────────────────────────

#[tokio::test]
async fn a_restarted_burned_torrent_stays_failed_while_held_for_seeding() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, submissions) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Restarted Burned Torrent").await;
    let message = "post-download rule(s) blocked import: language policy";
    let mut before_restart = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-burned-restart-1",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    before_restart.client_item.download_id =
        Some("scryer-download:torrent-burned-restart-1".to_string());
    before_restart.status = scryer_domain::TrackedDownloadStatus::Error;
    before_restart.status_messages = vec![message.to_string()];
    let id = crate::tracked_downloads::tracked_download_id_for_item(&before_restart.client_item);
    let identity =
        crate::tracked_downloads::observed_queue_item_identity(&before_restart.client_item);
    let source_identity = crate::ClientJobLocator::new(
        Some(before_restart.client_id.as_str()),
        &before_restart.client_type,
        &before_restart.client_item.download_client_item_id,
    );
    submissions
        .record_identity_tracked_state(
            &identity,
            Some(&source_identity),
            TrackedDownloadState::Failed.as_str(),
            Some("import_gate_rejected"),
            Some(message),
        )
        .await
        .expect("persist burned failure before restart");

    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker
        .track(&app, before_restart.client_item.clone())
        .await;

    let restored = tracker.find(&id).expect("reconstructed tracked download");
    assert_eq!(restored.state, TrackedDownloadState::Failed);
    assert!(restored.burned_by_import_gate);
    assert_eq!(restored.status, scryer_domain::TrackedDownloadStatus::Error);
    assert_eq!(restored.status_messages, vec![message.to_string()]);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Failed,
    )
    .await;

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Failed,
    )
    .await;

    let held = tracker
        .find(&id)
        .expect("held burned torrent stays tracked");
    assert_eq!(held.state, TrackedDownloadState::Failed);
    assert!(held.burned_by_import_gate);
    assert_eq!(held.status, scryer_domain::TrackedDownloadStatus::Error);
    assert_eq!(
        held.status_messages,
        vec![
            message.to_string(),
            "Kept in the download client until its seeding goal is met; the entry and its data are removed then."
                .to_string(),
        ]
    );
    assert!(download_client.deleted_requests.lock().await.is_empty());
    assert_eq!(
        submissions
            .get_identity_tracked_state_reason(&identity, Some(&source_identity))
            .await
            .expect("read durable failure reason")
            .as_deref(),
        Some("import_gate_rejected")
    );
}

#[tokio::test]
async fn a_held_torrent_is_parked_in_imported_seeding_and_stays_tracked() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, submissions) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Park Me").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-park-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    let parked = tracker
        .find(&id)
        .expect("a held torrent must stay tracked so it re-enters the gate and stays visible");
    assert_eq!(parked.state, TrackedDownloadState::ImportedSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
    assert_eq!(
        submissions
            .tracked_states
            .lock()
            .await
            .values()
            .next()
            .cloned(),
        Some(TrackedDownloadState::ImportedSeeding.as_str().to_string()),
        "the parked state must be persisted so a restart does not re-derive and remove it"
    );
}

#[tokio::test]
async fn a_usenet_download_still_stops_being_tracked_once_it_is_removed() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Settle Me").await;

    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-settle-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let (_root, payload) = host_payload_fixture(&app, &user, &download_client, &tracked).await;
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    assert!(tracker.find(&id).is_none());
    assert!(!payload.exists());
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "nzb-settle-1".to_string(),
            true,
            false
        )]
    );
}

// ── persisted seed-goal read ──────────────────────────────────────────────

/// Serves only the seed-goal reads; every other repository method is inert.
/// This pins the exact lookup the gate performs against the grab-time
/// persistence contract.
#[derive(Default)]
struct SeedGoalOnlySubmissionRepo {
    by_canonical_download_id:
        std::sync::Mutex<HashMap<scryer_domain::download_identity::DownloadId, PersistedSeedGoals>>,
    by_identity: std::sync::Mutex<HashMap<String, PersistedSeedGoals>>,
    by_info_hash: std::sync::Mutex<HashMap<String, PersistedSeedGoals>>,
    canonical_lookups: std::sync::Mutex<Vec<Option<scryer_domain::download_identity::DownloadId>>>,
    identity_lookups: std::sync::Mutex<Vec<ClientJobLocator>>,
    info_hash_lookups: std::sync::Mutex<Vec<String>>,
    /// One entry per batched read, holding the identities it was asked for.
    batch_lookups: std::sync::Mutex<Vec<Vec<ClientJobLocator>>>,
    /// When set, the batched read fails — the prefetch must then degrade to
    /// per-row reads, never to "these torrents have no obligation".
    batch_fails: std::sync::atomic::AtomicBool,
    identity_fails: std::sync::atomic::AtomicBool,
    info_hash_fails: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl DownloadSubmissionRepository for SeedGoalOnlySubmissionRepo {
    async fn record_submission(&self, _: DownloadSubmission) -> AppResult<()> {
        Ok(())
    }

    async fn record_ambiguous_submission(&self, _: DownloadSubmission) -> AppResult<()> {
        Ok(())
    }

    async fn record_submission_with_identity(
        &self,
        _: DownloadSubmission,
        _: crate::DownloadSubmissionIdentity,
        _: Option<PersistedSeedGoals>,
    ) -> AppResult<crate::CanonicalDownloadIdentityDisposition> {
        Ok(crate::CanonicalDownloadIdentityDisposition::Requested)
    }

    async fn find_by_client_item_id(
        &self,
        _: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }

    async fn list_for_client_items(
        &self,
        _: &[ClientJobLocator],
    ) -> AppResult<Vec<DownloadSubmission>> {
        Ok(vec![])
    }

    async fn list_for_title(&self, _: &str) -> AppResult<Vec<DownloadSubmission>> {
        Ok(vec![])
    }

    async fn find_by_title_and_request_signature(
        &self,
        _: &str,
        _: &str,
        _: DownloadSubmissionPurpose,
        _: &SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }

    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn delete_by_client_item_id(&self, _: &ClientJobLocator) -> AppResult<()> {
        Ok(())
    }

    async fn update_tracked_state(&self, _: &ClientJobLocator, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn get_tracked_state(&self, _: &ClientJobLocator) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_seed_goals(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        if self.identity_fails.load(Ordering::Relaxed) {
            return Err(AppError::Repository("seed goal read unavailable".into()));
        }
        self.identity_lookups
            .lock()
            .expect("identity lookup log")
            .push(identity.clone());
        Ok(self
            .by_identity
            .lock()
            .expect("seed goals by identity")
            .get(&identity.item_id)
            .cloned())
    }

    async fn get_seed_goals_for_download(
        &self,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        self.canonical_lookups
            .lock()
            .expect("canonical lookup log")
            .push(canonical_download_id.cloned());
        let canonical = canonical_download_id.and_then(|canonical_download_id| {
            self.by_canonical_download_id
                .lock()
                .expect("seed goals by canonical download id")
                .get(canonical_download_id)
                .cloned()
        });
        let legacy = self.get_seed_goals(identity).await?;
        Ok(legacy.or(canonical))
    }

    async fn find_seed_goals_by_info_hash(
        &self,
        info_hash: &str,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        if self.info_hash_fails.load(Ordering::Relaxed) {
            return Err(AppError::Repository("seed hash read unavailable".into()));
        }
        self.info_hash_lookups
            .lock()
            .expect("info hash lookup log")
            .push(info_hash.to_string());
        Ok(self
            .by_info_hash
            .lock()
            .expect("seed goals by info hash")
            .get(info_hash)
            .cloned())
    }

    async fn list_seed_goals_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<(ClientJobLocator, PersistedSeedGoals)>> {
        self.batch_lookups
            .lock()
            .expect("batch lookup log")
            .push(client_items.to_vec());
        if self.batch_fails.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(crate::AppError::Repository("batch read failed".into()));
        }
        let by_identity = self.by_identity.lock().expect("seed goals by identity");
        Ok(client_items
            .iter()
            .filter_map(|identity| {
                by_identity
                    .get(&identity.item_id)
                    .cloned()
                    .map(|goals| (identity.clone(), goals))
            })
            .collect())
    }
}

fn persisted_goals(never_remove: bool) -> PersistedSeedGoals {
    PersistedSeedGoals {
        seeding_profile_id: Some("profile-1".to_string()),
        seed_goal_ratio: Some(2.0),
        seed_goal_seconds: None,
        never_remove,
        goal_met_action: Some(scryer_domain::SeedGoalMetAction::RemoveEntry),
        post_import_tracking: scryer_domain::PostImportTracking::Park,
        resolution_source: crate::SeedGoalResolutionSource::Indexer,
        info_hash: None,
    }
}

/// The same grab, but under a profile whose post-import tracking is `HandOff`.
fn handed_off_goals() -> PersistedSeedGoals {
    PersistedSeedGoals {
        post_import_tracking: scryer_domain::PostImportTracking::HandOff,
        ..persisted_goals(false)
    }
}

#[tokio::test]
async fn the_gate_reads_the_goals_a_grab_was_persisted_under() {
    use crate::seeding_gate::{SeedGoalLookupKey, SeedGoalsRead};

    let item_id = "abcdef0123456789abcdef0123456789abcdef01";
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_identity
        .lock()
        .expect("seed goals by identity")
        .insert(item_id.to_string(), persisted_goals(true));

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let key = SeedGoalLookupKey {
        canonical_download_id: None,
        client_id: "client-1".to_string(),
        client_type: "qbittorrent".to_string(),
        client_item_id: item_id.to_string(),
        info_hash: Some(item_id.to_string()),
    };
    let goals = app
        .resolved_seed_goals(&key, None)
        .await
        .expect("goal lookup should succeed")
        .expect("persisted goals should be found by client identity");

    assert_eq!(goals.seed_goal_ratio, Some(2.0));
    assert!(goals.never_remove);
    assert_eq!(
        repo.identity_lookups
            .lock()
            .expect("identity lookup log")
            .len(),
        1
    );
    assert!(
        repo.info_hash_lookups
            .lock()
            .expect("info hash lookup log")
            .is_empty(),
        "the info-hash fallback must not run when client identity already answered"
    );
}

#[tokio::test]
async fn the_gate_falls_back_to_the_info_hash_when_the_client_item_id_moved() {
    use crate::seeding_gate::{SeedGoalLookupKey, SeedGoalsRead};

    let info_hash = "abcdef0123456789abcdef0123456789abcdef01";
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_info_hash
        .lock()
        .expect("seed goals by info hash")
        .insert(info_hash.to_string(), persisted_goals(false));

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let key = SeedGoalLookupKey {
        canonical_download_id: None,
        client_id: "client-1".to_string(),
        client_type: "qbittorrent".to_string(),
        client_item_id: "some-other-item-id".to_string(),
        info_hash: Some(info_hash.to_string()),
    };
    let goals = app
        .resolved_seed_goals(&key, None)
        .await
        .expect("goal lookup should succeed")
        .expect("persisted goals should be found by info hash");

    assert_eq!(goals.seed_goal_ratio, Some(2.0));
    assert_eq!(
        repo.info_hash_lookups
            .lock()
            .expect("info hash lookup log")
            .clone(),
        vec![info_hash.to_string()]
    );
}

#[tokio::test]
async fn canonical_seed_policy_overrides_a_tuple_batch_and_missing_plugins_hold() {
    use crate::seeding_gate::{SeedGoalBatch, SeedGoalLookupKey, SeedGoalsRead};
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    let id = scryer_domain::download_identity::DownloadId::new();
    repo.by_canonical_download_id
        .lock()
        .unwrap()
        .insert(id, persisted_goals(true));
    repo.by_identity
        .lock()
        .unwrap()
        .insert("job".into(), persisted_goals(false));
    let (mut app, _, _) = bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();
    let locator = ClientJobLocator::new(Some("client"), "qbittorrent", "job");
    let batch = SeedGoalBatch::prefetch(&app, &[locator]).await;
    repo.by_identity.lock().unwrap().clear();
    let mut key = SeedGoalLookupKey {
        canonical_download_id: Some(id),
        client_id: "client".into(),
        client_type: "qbittorrent".into(),
        client_item_id: "job".into(),
        ..Default::default()
    };
    assert!(
        app.resolved_seed_goals(&key, Some(&batch))
            .await
            .unwrap()
            .unwrap()
            .never_remove
    );
    key.client_type = "unavailable-plugin".into();
    assert!(crate::seeding_gate::client_type_is_torrent(
        &app,
        &key.client_type
    ));
    let decision = crate::seeding_gate::evaluate_seeding_gate_with(
        &app,
        &key,
        true,
        Some(crate::seeding_gate::TorrentSeedingObservation {
            can_remove: Some(true),
            ..Default::default()
        }),
        None,
    )
    .await;
    assert_eq!(
        decision.outcome,
        crate::seeding_gate::SeedingGateOutcome::Hold
    );
    assert_eq!(
        decision.import_mode(ImportMode::Move),
        ImportMode::HardlinkOrCopy
    );
}

#[tokio::test]
async fn unavailable_seed_goals_hold_even_when_the_client_allows_removal() {
    for hash_failure in [false, true] {
        let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
        repo.identity_fails.store(!hash_failure, Ordering::Relaxed);
        repo.info_hash_fails.store(hash_failure, Ordering::Relaxed);
        let (mut app, _, _) =
            bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
        app.services.workflow.download_submissions = repo;
        let key = crate::seeding_gate::SeedGoalLookupKey {
            client_type: "qbittorrent".into(),
            client_id: "client".into(),
            client_item_id: "job".into(),
            info_hash: Some("hash".into()),
            ..Default::default()
        };
        let decision = crate::seeding_gate::evaluate_seeding_gate_with(
            &app,
            &key,
            true,
            Some(crate::seeding_gate::TorrentSeedingObservation {
                can_remove: Some(true),
                can_move_files: Some(true),
                ..Default::default()
            }),
            None,
        )
        .await;
        assert_eq!(
            decision.outcome,
            crate::seeding_gate::SeedingGateOutcome::Hold
        );
        assert_eq!(
            decision.import_mode(ImportMode::Move),
            ImportMode::HardlinkOrCopy
        );
    }
}

#[tokio::test]
async fn failed_torrents_honor_persisted_profiles_and_unknown_fresh_observations() {
    let client = Arc::new(StubDownloadClient::default());
    let (mut app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Failed but still obligated").await;
    let mut tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "seed-job",
        &title,
        TrackedDownloadState::Failed,
        true,
    );
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            can_move_files: Some(true),
            seed_ratio: Some(0.5),
            ..Default::default()
        },
        &mut tracked,
    );
    for never_remove in [false, true] {
        app.services.workflow.download_submissions =
            goals_repo("seed-job", persisted_goals(never_remove));
        let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            TrackedDownloadState::Failed,
            None,
        )
        .await;
        assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    }
    // A fresh unknown observation must not borrow the old cached release.
    app.services.workflow.download_submissions = Arc::new(SeedGoalOnlySubmissionRepo::default());
    tracked.state = TrackedDownloadState::Imported;
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked.id.clone(),
            crate::tracked_downloads::TrackedDownloadQueueMetadata::from(&tracked),
        );
    tracked.client_item.seeding = None;
    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(client.deleted_requests.lock().await.is_empty());
    assert!(client.paused_requests.lock().await.is_empty());
}

#[tokio::test]
async fn a_never_remove_profile_holds_a_torrent_the_client_says_is_removable() {
    let item_id = "abcdef0123456789abcdef0123456789abcdef02";
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_identity
        .lock()
        .expect("seed goals by identity")
        .insert(item_id.to_string(), persisted_goals(true));

    let download_client = Arc::new(StubDownloadClient::default());
    let (mut app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seed Forever").await;
    app.services.workflow.download_submissions = repo;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        item_id,
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

// ── the gate goes live: real observations flip each decision-table row ─────
//
// Every one of these torrents is held on a build with no observation plumbing
// (the gate's row 10, "no resolved goals and client verdict unknown"). They
// reach a different verdict here only because the download-client adapter now
// carries `can_remove` / `can_move_files` / the torrent projection onto the
// client-item snapshot, and the reconcile tick hands that snapshot to the gate.

fn observed(snapshot: DownloadSeedingSnapshot, tracked: &mut TrackedDownload) {
    tracked.client_item.seeding = Some(snapshot);
}

fn goals_repo(item_id: &str, goals: PersistedSeedGoals) -> Arc<SeedGoalOnlySubmissionRepo> {
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_identity
        .lock()
        .expect("seed goals by identity")
        .insert(item_id.to_string(), goals);
    repo
}

async fn torrent_cleanup_fixture(
    download_client: Arc<StubDownloadClient>,
    name: &str,
    item_id: &str,
    goals: Option<PersistedSeedGoals>,
) -> (AppUseCase, TrackedDownload) {
    let (mut app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, name).await;
    if let Some(goals) = goals {
        app.services.workflow.download_submissions = goals_repo(item_id, goals);
    }
    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        item_id,
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    (app, tracked)
}

async fn rtorrent_cleanup_fixture(
    download_client: Arc<StubDownloadClient>,
    name: &str,
    item_id: &str,
) -> (
    AppUseCase,
    User,
    scryer_domain::DownloadClientConfig,
    TrackedDownload,
) {
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "rTorrent", "rtorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, name).await;
    let tracked = tracked_for(
        &config.id,
        "rtorrent",
        item_id,
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    record_host_job_ownership(&app, &tracked, "Release").await;
    (app, user, config, tracked)
}

#[tokio::test]
async fn a_client_that_reports_its_obligation_met_now_releases_the_entry() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Client Says Done",
        "torrent-live-1",
        None,
    )
    .await;

    // Without the observation this is the fail-closed row: held forever.
    let held = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;
    assert_eq!(held, TerminalDownloadCleanupOutcome::HeldForSeeding);

    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            can_move_files: Some(true),
            seed_ratio: Some(3.1),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client
            .deleted_requests
            .lock()
            .await
            .iter()
            .map(|(_, _, item_id, _, remove_data)| (item_id.clone(), *remove_data))
            .collect::<Vec<_>>(),
        // A released torrent's payload goes with its entry, Sonarr's
        // `RemoveItem(item, deleteData: true)`. The import already produced the
        // library file; leaving the client's copy behind would orphan it.
        vec![("torrent-live-1".to_string(), true)]
    );
}

#[tokio::test]
async fn rtorrent_cleanup_retries_entry_removal_after_deleting_the_mapped_payload() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, config, mut tracked) = rtorrent_cleanup_fixture(
        download_client.clone(),
        "rTorrent Cleanup",
        "rtorrent-cleanup-1",
    )
    .await;
    let tempdir = tempfile::tempdir().expect("temporary download root");
    let local_root = tempdir.path().join("downloads");
    let payload = local_root.join("Release");
    std::fs::create_dir_all(&payload).expect("create payload directory");
    std::fs::write(payload.join("movie.mkv"), b"media").expect("create payload file");

    app.update_download_client_config(
        &user,
        crate::DownloadClientConfigUpdate {
            id: config.id.clone(),
            config_json: Some(format!(
                r#"{{"remote_path_mappings":"/remote/downloads => {}\n{} => /only-if-remapped-twice"}}"#,
                local_root.display(),
                local_root.display()
            )),
            ..Default::default()
        },
    )
    .await
    .expect("configure remote path mapping");
    download_client
        .set_client_status(crate::DownloadClientStatus {
            remote_output_roots: vec![local_root.display().to_string()],
            ..Default::default()
        })
        .await;
    download_client
        .completed_downloads
        .lock()
        .await
        .push(scryer_domain::CompletedDownload {
            client_type: "rtorrent".to_string(),
            client_id: config.id.clone(),
            download_client_item_id: "rtorrent-cleanup-1".to_string(),
            download_id: None,
            name: "Release".to_string(),
            release_name: None,
            dest_dir: "/remote/downloads/Release".to_string(),
            category: Some("scryer-movies".to_string()),
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
        });
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );
    download_client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());

    *download_client.delete_error.lock().await = Some("transient delete failure".to_string());
    let first_outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(
        first_outcome,
        TerminalDownloadCleanupOutcome::RetryableFailure
    );
    assert!(!payload.exists(), "the host must delete the mapped payload");
    assert_eq!(
        std::fs::read_dir(&local_root)
            .expect("read output root after payload deletion")
            .count(),
        1,
        "a durable checkpoint must keep the mounted root distinguishable from an empty mountpoint"
    );

    *download_client.delete_error.lock().await = None;
    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        std::fs::read_dir(&local_root)
            .expect("read output root after cleanup completion")
            .count(),
        0,
        "the checkpoint must be removed with the rTorrent entry"
    );
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id),
            None,
            "rtorrent-cleanup-1".to_string(),
            true,
            false,
        )],
        "rTorrent receives entry-only removal after host payload cleanup"
    );
}

#[tokio::test]
async fn rtorrent_cleanup_refuses_to_delete_an_output_root() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, _user, config, mut tracked) = rtorrent_cleanup_fixture(
        download_client.clone(),
        "rTorrent Root Guard",
        "rtorrent-root-1",
    )
    .await;
    let tempdir = tempfile::tempdir().expect("temporary download root");
    let root = tempdir.path().join("downloads");
    std::fs::create_dir_all(&root).expect("create output root");
    std::fs::write(root.join(".mounted"), b"mounted").expect("make root available");
    configure_host_cleanup_root(&app, &_user, &config.id, &root).await;
    download_client
        .set_client_status(crate::DownloadClientStatus {
            remote_output_roots: vec![root.display().to_string()],
            ..Default::default()
        })
        .await;
    download_client
        .completed_downloads
        .lock()
        .await
        .push(scryer_domain::CompletedDownload {
            client_type: "rtorrent".to_string(),
            client_id: config.id,
            download_client_item_id: "rtorrent-root-1".to_string(),
            download_id: None,
            name: "downloads".to_string(),
            release_name: None,
            dest_dir: root.display().to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
        });
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
    assert!(root.exists());
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn rtorrent_cleanup_refuses_a_symlinked_payload_ancestor() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, _user, config, mut tracked) = rtorrent_cleanup_fixture(
        download_client.clone(),
        "rTorrent Symlink Guard",
        "rtorrent-symlink-1",
    )
    .await;
    let tempdir = tempfile::tempdir().expect("temporary download root");
    let root = tempdir.path().join("downloads");
    let outside = tempdir.path().join("library");
    let payload = outside.join("Release");
    std::fs::create_dir_all(&root).expect("create output root");
    std::fs::write(root.join(".mounted"), b"mounted").expect("make root available");
    std::fs::create_dir_all(&payload).expect("create outside payload");
    std::fs::write(payload.join("movie.mkv"), b"media").expect("create outside media");
    std::os::unix::fs::symlink(&outside, root.join("escape"))
        .expect("create payload ancestor symlink");
    configure_host_cleanup_root(&app, &_user, &config.id, &root).await;
    download_client
        .set_client_status(crate::DownloadClientStatus {
            remote_output_roots: vec![root.display().to_string()],
            ..Default::default()
        })
        .await;
    download_client
        .completed_downloads
        .lock()
        .await
        .push(scryer_domain::CompletedDownload {
            client_type: "rtorrent".to_string(),
            client_id: config.id,
            download_client_item_id: "rtorrent-symlink-1".to_string(),
            download_id: None,
            name: "Release".to_string(),
            release_name: None,
            dest_dir: root.join("escape/Release").display().to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
        });
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::RetryableFailure);
    assert!(payload.exists(), "the symlink target must remain intact");
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn rtorrent_cleanup_allows_an_output_root_below_a_symlinked_ancestor() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (mut app, _user, config, mut tracked) = rtorrent_cleanup_fixture(
        download_client.clone(),
        "rTorrent Symlinked Root Ancestor",
        "rtorrent-ancestor-1",
    )
    .await;
    let tempdir = tempfile::tempdir().expect("temporary download root");
    let real_parent = tempdir.path().join("private");
    let real_root = real_parent.join("downloads");
    let alias = tempdir.path().join("var");
    let reported_root = alias.join("downloads");
    let payload = reported_root.join("release.mkv");
    std::fs::create_dir_all(&real_root).expect("create output root");
    std::fs::write(real_root.join(".mounted"), b"mounted").expect("make root available");
    std::os::unix::fs::symlink(&real_parent, &alias).expect("create root ancestor alias");
    std::fs::write(&payload, b"media").expect("create payload file through alias");
    record_file_import_ownership(&mut app, &tracked, &payload).await;
    configure_host_cleanup_root(&app, &_user, &config.id, &reported_root).await;
    download_client
        .set_client_status(crate::DownloadClientStatus {
            remote_output_roots: vec![reported_root.display().to_string()],
            ..Default::default()
        })
        .await;
    download_client
        .completed_downloads
        .lock()
        .await
        .push(scryer_domain::CompletedDownload {
            client_type: "rtorrent".to_string(),
            client_id: config.id.clone(),
            download_client_item_id: "rtorrent-ancestor-1".to_string(),
            download_id: None,
            name: "release.mkv".to_string(),
            release_name: None,
            dest_dir: payload.display().to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
        });
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert!(!real_root.join("release.mkv").exists());
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id),
            None,
            "rtorrent-ancestor-1".to_string(),
            true,
            false,
        )]
    );
}

#[tokio::test]
async fn rtorrent_cleanup_deletes_a_direct_completed_file() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (mut app, _user, config, mut tracked) = rtorrent_cleanup_fixture(
        download_client.clone(),
        "rTorrent Direct File Cleanup",
        "rtorrent-file-1",
    )
    .await;
    let tempdir = tempfile::tempdir().expect("temporary download root");
    let root = tempdir.path().join("downloads");
    let payload = root.join("release.mkv");
    std::fs::create_dir_all(&root).expect("create output root");
    std::fs::write(root.join(".mounted"), b"mounted").expect("make root available");
    std::fs::write(&payload, b"media").expect("create payload file");
    record_file_import_ownership(&mut app, &tracked, &payload).await;
    configure_host_cleanup_root(&app, &_user, &config.id, &root).await;
    download_client
        .set_client_status(crate::DownloadClientStatus {
            remote_output_roots: vec![root.display().to_string()],
            ..Default::default()
        })
        .await;
    download_client
        .completed_downloads
        .lock()
        .await
        .push(scryer_domain::CompletedDownload {
            client_type: "rtorrent".to_string(),
            client_id: config.id.clone(),
            download_client_item_id: "rtorrent-file-1".to_string(),
            download_id: None,
            name: "release.mkv".to_string(),
            release_name: None,
            dest_dir: payload.display().to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
        });
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert!(!payload.exists());
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id),
            None,
            "rtorrent-file-1".to_string(),
            true,
            false,
        )]
    );
}

#[tokio::test]
async fn an_ignored_torrent_is_removed_without_deleting_its_data() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Ignored Torrent").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-ignored-1",
        &title,
        TrackedDownloadState::Ignored,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Ignored,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        // Ignoring a download says "stop tracking this", not "delete what you
        // downloaded" — the operator may still want the payload.
        vec![(
            Some(config.id.clone()),
            None,
            "torrent-ignored-1".to_string(),
            true,
            false
        )]
    );
}

#[tokio::test]
async fn a_torrent_released_by_its_persisted_goal_takes_its_data_with_it() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Goal Met With Data",
        "torrent-live-9",
        Some(persisted_goals(false)),
    )
    .await;

    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(2.4),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::ImportedSeeding,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    let requests = download_client.deleted_requests.lock().await.clone();
    assert_eq!(
        requests
            .iter()
            .map(|(_, _, item_id, _, remove_data)| (item_id.clone(), *remove_data))
            .collect::<Vec<_>>(),
        vec![("torrent-live-9".to_string(), true)]
    );
}

#[tokio::test]
async fn an_observed_ratio_past_the_persisted_goal_beats_a_client_saying_no() {
    let download_client = Arc::new(StubDownloadClient::default());
    // Persisted goal: ratio 2.0, remove the entry when met.
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Goal Met",
        "torrent-live-2",
        Some(persisted_goals(false)),
    )
    .await;

    observed(
        DownloadSeedingSnapshot {
            // The client is asserting one of *its* limits is unmet. That is a
            // different question from the profile goal Scryer was told to
            // enforce, and it is not a veto.
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(2.4),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
}

#[tokio::test]
async fn an_observed_ratio_short_of_the_persisted_goal_still_holds() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Goal Unmet",
        "torrent-live-3",
        Some(persisted_goals(false)),
    )
    .await;

    observed(
        DownloadSeedingSnapshot {
            // Even a client volunteering "yes, remove it" cannot discharge a
            // Scryer goal that is demonstrably unmet.
            can_remove: Some(true),
            can_move_files: Some(true),
            seed_ratio: Some(0.7),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn an_observed_private_torrent_without_goals_is_held_forever() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Private Rail",
        "torrent-live-4",
        None,
    )
    .await;

    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            can_move_files: Some(true),
            is_private: Some(true),
            seed_ratio: Some(9.0),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    // The same observation on a public (or unknown) torrent releases it; the
    // private flag is the whole difference.
    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);

    tracked
        .client_item
        .seeding
        .as_mut()
        .expect("observation")
        .is_private = None;
    let released = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;
    assert_eq!(released, TerminalDownloadCleanupOutcome::Removed);
}

#[tokio::test]
async fn an_absent_observation_holds_exactly_as_it_did_before_the_plumbing() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Nothing Observed",
        "torrent-live-5",
        None,
    )
    .await;
    assert!(tracked.client_item.seeding.is_none());

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn every_tick_re_reads_the_observation_rather_than_the_one_that_parked_the_row() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Fresh Every Tick",
        "torrent-live-6",
        Some(persisted_goals(false)),
    )
    .await;

    // Tick 1: ratio well short of the goal.
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(0.2),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );
    assert_eq!(
        crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            TrackedDownloadState::ImportedSeeding,
            None,
        )
        .await,
        TerminalDownloadCleanupOutcome::HeldForSeeding
    );

    // Tick 2: the poller refreshed the client item and the ratio has moved.
    // A gate reading a cached first sighting would still hold here.
    tracked
        .client_item
        .seeding
        .as_mut()
        .expect("observation")
        .seed_ratio = Some(2.0);

    assert_eq!(
        crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            TrackedDownloadState::ImportedSeeding,
            None,
        )
        .await,
        TerminalDownloadCleanupOutcome::Removed
    );
}

#[tokio::test]
async fn the_wall_clock_fallback_covers_clients_with_no_seed_time_counter() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Wall Clock",
        "torrent-live-7",
        Some(PersistedSeedGoals {
            seed_goal_ratio: None,
            seed_goal_seconds: Some(3_600),
            ..persisted_goals(false)
        }),
    )
    .await;

    observed(
        DownloadSeedingSnapshot {
            can_remove: None,
            can_move_files: Some(true),
            // No ratio, no seed-time counter — freebox, hadouken, aria2.
            completed_at: Some(
                (chrono::Utc::now() - chrono::Duration::seconds(7_200)).to_rfc3339(),
            ),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    assert_eq!(
        crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            &tracked,
            TrackedDownloadState::ImportedSeeding,
            None,
        )
        .await,
        TerminalDownloadCleanupOutcome::Removed
    );
}

#[tokio::test]
async fn a_stop_seeding_profile_pauses_the_torrent_instead_of_removing_it() {
    // Only reachable now that an observation can release a torrent: before the
    // plumbing every wiring-level path either held or vanished, so the
    // `StopSeeding` action had no end-to-end coverage.
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, mut tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Stop Seeding",
        "torrent-live-8",
        Some(PersistedSeedGoals {
            goal_met_action: Some(scryer_domain::SeedGoalMetAction::StopSeeding),
            ..persisted_goals(false)
        }),
    )
    .await;

    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(2.5),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::ImportedSeeding,
        None,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::SeedingEntryKept);
    assert!(
        download_client.deleted_requests.lock().await.is_empty(),
        "stop-seeding must leave the entry in the client"
    );
    assert_eq!(
        download_client
            .paused_requests
            .lock()
            .await
            .iter()
            .map(|(_, item_id)| item_id.clone())
            .collect::<Vec<_>>(),
        vec!["torrent-live-8".to_string()]
    );
}

// ── queue projection: goals joined beside the observation ─────────────────

#[tokio::test]
async fn queue_enrichment_joins_the_persisted_goals_onto_the_observed_row() {
    let item_id = "abcdef0123456789abcdef0123456789abcdef09";
    let repo = goals_repo(item_id, persisted_goals(false));
    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo;

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 100);
    item.client_id = "client-1".to_string();
    item.client_type = "qbittorrent".to_string();
    item.progress_percent = 100;
    item.seeding = Some(DownloadSeedingSnapshot {
        can_remove: Some(false),
        can_move_files: Some(true),
        seed_ratio: Some(0.8),
        seed_time_seconds: Some(3_600),
        is_private: Some(false),
        ..DownloadSeedingSnapshot::default()
    });
    let mut items = vec![item];

    crate::enrich_download_queue_items_from_submissions(&app, &mut items).await;

    let seeding = items[0]
        .seeding
        .clone()
        .expect("the observation must survive enrichment");
    // Observation untouched, goals joined beside it.
    assert_eq!(seeding.seed_ratio, Some(0.8));
    assert_eq!(seeding.seed_time_seconds, Some(3_600));
    assert_eq!(seeding.seed_goal_ratio, Some(2.0));
    assert!(!seeding.never_remove);
    assert_eq!(
        crate::derive_download_seeding_state(&items[0]),
        Some(crate::DownloadSeedingState::Seeding)
    );
}

#[tokio::test]
async fn a_torrent_in_plain_seeding_carries_progress_before_it_is_ever_imported() {
    // Pre-import, post-completion: no tracked state at all, and the row still
    // has to show what it is waiting on.
    let item_id = "abcdef0123456789abcdef0123456789abcdef0a";
    let repo = goals_repo(item_id, persisted_goals(false));
    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo;

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 100);
    item.client_id = "client-1".to_string();
    item.client_type = "qbittorrent".to_string();
    item.progress_percent = 100;
    item.tracked_state = None;
    item.seeding = Some(DownloadSeedingSnapshot {
        can_remove: Some(false),
        seed_ratio: Some(2.2),
        ..DownloadSeedingSnapshot::default()
    });
    let mut items = vec![item];

    crate::enrich_download_queue_items_from_submissions(&app, &mut items).await;

    assert_eq!(
        crate::derive_download_seeding_state(&items[0]),
        Some(crate::DownloadSeedingState::GoalMet)
    );
    assert_eq!(
        items[0]
            .seeding
            .as_ref()
            .and_then(|seeding| seeding.seed_goal_ratio),
        Some(2.0)
    );
}

#[tokio::test]
async fn the_import_mode_check_requires_fresh_client_evidence_even_with_a_releasing_snapshot() {
    let item_id = "torrent-move-lookup-1";
    let client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    let title = movie_title(&app, &user, "Move After Seeding").await;
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("configure move import mode");

    let mut tracked = tracked_for(
        &config.id,
        "qbittorrent",
        item_id,
        &title,
        TrackedDownloadState::ImportedSeeding,
        true,
    );
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            can_move_files: Some(true),
            seed_ratio: Some(5.0),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );
    client
        .history_items
        .lock()
        .await
        .push(tracked.client_item.clone());
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked.id.clone(),
            crate::tracked_downloads::TrackedDownloadQueueMetadata::from(&tracked),
        );

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "qbittorrent", item_id)),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(
        effective,
        ImportMode::Move,
        "a torrent the client says is done, with stable data, may be moved"
    );

    // The published snapshot still permits a move; only the fresh verdict changes.
    client.history_items.lock().await[0]
        .seeding
        .as_mut()
        .unwrap()
        .can_remove = Some(false);

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "qbittorrent", item_id)),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::HardlinkOrCopy);
}

// ── per-tick batching ─────────────────────────────────────────────────────

/// The reconcile tick re-offers every settled row on every poll, so the reads
/// each row used to do for itself are the shape being removed here.
#[tokio::test]
async fn the_reconcile_tick_reads_every_settled_row_s_goals_in_one_batch() {
    use crate::seeding_gate::{SeedGoalBatch, SeedGoalLookupKey, SeedGoalsRead};

    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    for item_id in ["torrent-a", "torrent-b", "torrent-c"] {
        repo.by_identity
            .lock()
            .expect("seed goals by identity")
            .insert(item_id.to_string(), persisted_goals(false));
    }

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let identities: Vec<ClientJobLocator> = ["torrent-a", "torrent-b", "torrent-c"]
        .iter()
        .map(|item_id| ClientJobLocator::new(Some("client-1"), "qbittorrent", item_id))
        .collect();
    let batch = SeedGoalBatch::prefetch(&app, &identities).await;

    assert_eq!(
        repo.batch_lookups.lock().expect("batch lookup log").len(),
        1,
        "one batched read for the whole tick"
    );
    assert!(
        repo.identity_lookups
            .lock()
            .expect("identity lookup log")
            .is_empty(),
        "the prefetch must not fall back to per-row identity reads"
    );

    for item_id in ["torrent-a", "torrent-b", "torrent-c"] {
        let key = SeedGoalLookupKey {
            canonical_download_id: None,
            client_id: "client-1".to_string(),
            client_type: "qbittorrent".to_string(),
            client_item_id: item_id.to_string(),
            info_hash: None,
        };
        let goals = app
            .resolved_seed_goals(&key, Some(&batch))
            .await
            .expect("goal lookup should succeed")
            .expect("the batch answers for every row it covered");
        assert_eq!(goals.seed_goal_ratio, Some(2.0));
    }

    assert!(
        repo.identity_lookups
            .lock()
            .expect("identity lookup log")
            .is_empty(),
        "a row the batch answered must not repeat the identity query"
    );
    assert_eq!(
        repo.batch_lookups.lock().expect("batch lookup log").len(),
        1,
        "resolving rows must not trigger further batched reads"
    );
}

#[tokio::test]
async fn a_covered_row_without_goals_skips_the_identity_query_but_keeps_the_info_hash_fallback() {
    use crate::seeding_gate::{SeedGoalBatch, SeedGoalLookupKey, SeedGoalsRead};

    let info_hash = "abcdef0123456789abcdef0123456789abcdef01";
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_info_hash
        .lock()
        .expect("seed goals by info hash")
        .insert(info_hash.to_string(), persisted_goals(false));

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let identity = ClientJobLocator::new(Some("client-1"), "qbittorrent", "moved-item-id");
    let batch = SeedGoalBatch::prefetch(&app, std::slice::from_ref(&identity)).await;

    let key = SeedGoalLookupKey {
        canonical_download_id: None,
        client_id: "client-1".to_string(),
        client_type: "qbittorrent".to_string(),
        client_item_id: "moved-item-id".to_string(),
        info_hash: Some(info_hash.to_string()),
    };
    let goals = app
        .resolved_seed_goals(&key, Some(&batch))
        .await
        .expect("goal lookup should succeed")
        .expect("the info-hash fallback still runs for a covered row with no identity match");

    assert_eq!(goals.seed_goal_ratio, Some(2.0));
    assert!(
        repo.identity_lookups
            .lock()
            .expect("identity lookup log")
            .is_empty(),
        "the batch already answered the identity question"
    );
    assert_eq!(
        repo.info_hash_lookups
            .lock()
            .expect("info hash lookup log")
            .clone(),
        vec![info_hash.to_string()]
    );
}

#[tokio::test]
async fn a_row_the_batch_never_covered_still_takes_the_per_row_path() {
    use crate::seeding_gate::{SeedGoalBatch, SeedGoalLookupKey, SeedGoalsRead};

    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_identity
        .lock()
        .expect("seed goals by identity")
        .insert("uncovered".to_string(), persisted_goals(false));

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let covered = ClientJobLocator::new(Some("client-1"), "qbittorrent", "covered");
    let batch = SeedGoalBatch::prefetch(&app, std::slice::from_ref(&covered)).await;

    let key = SeedGoalLookupKey {
        canonical_download_id: None,
        client_id: "client-1".to_string(),
        client_type: "qbittorrent".to_string(),
        client_item_id: "uncovered".to_string(),
        info_hash: None,
    };
    let goals = app
        .resolved_seed_goals(&key, Some(&batch))
        .await
        .expect("goal lookup should succeed")
        .expect("an uncovered row falls back to its own identity read");

    assert_eq!(goals.seed_goal_ratio, Some(2.0));
    assert_eq!(
        repo.identity_lookups
            .lock()
            .expect("identity lookup log")
            .len(),
        1
    );
}

/// A failed prefetch must degrade to per-row reads. Reading it as "no goals"
/// would release torrents that are still under obligation.
#[tokio::test]
async fn a_failed_prefetch_falls_back_to_per_row_reads() {
    use crate::seeding_gate::{SeedGoalBatch, SeedGoalLookupKey, SeedGoalsRead};

    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_identity
        .lock()
        .expect("seed goals by identity")
        .insert("torrent-a".to_string(), persisted_goals(true));
    repo.batch_fails
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let identity = ClientJobLocator::new(Some("client-1"), "qbittorrent", "torrent-a");
    let batch = SeedGoalBatch::prefetch(&app, std::slice::from_ref(&identity)).await;

    let key = SeedGoalLookupKey {
        canonical_download_id: None,
        client_id: "client-1".to_string(),
        client_type: "qbittorrent".to_string(),
        client_item_id: "torrent-a".to_string(),
        info_hash: None,
    };
    let goals = app
        .resolved_seed_goals(&key, Some(&batch))
        .await
        .expect("goal lookup should succeed")
        .expect("a failed prefetch must not read as 'no obligation'");

    assert!(goals.never_remove);
    assert_eq!(
        repo.identity_lookups
            .lock()
            .expect("identity lookup log")
            .len(),
        1
    );
}

/// Two settled rows of the same title, on the same client, in one tick: the
/// title lookup and the routing-entry read happen once, not once per row.
#[tokio::test]
async fn identical_routing_scopes_resolve_once_per_tick() {
    use crate::import::import::TerminalCleanupTickCache;

    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Shared Scope").await;

    let rows: Vec<TrackedDownload> = ["torrent-scope-1", "torrent-scope-2"]
        .iter()
        .map(|item_id| {
            tracked_for(
                &config.id,
                "qbittorrent",
                item_id,
                &title,
                TrackedDownloadState::ImportedSeeding,
                true,
            )
        })
        .collect();

    let identities: Vec<ClientJobLocator> = rows
        .iter()
        .map(|tracked| {
            ClientJobLocator::new(
                Some(tracked.client_id.as_str()),
                &tracked.client_type,
                &tracked.client_item.download_client_item_id,
            )
        })
        .collect();
    let cache = TerminalCleanupTickCache::prefetch(&app, &identities).await;

    for tracked in &rows {
        let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
            &app,
            tracked,
            TrackedDownloadState::ImportedSeeding,
            Some(&cache),
        )
        .await;
        assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    }

    assert_eq!(
        cache.memo_reads(),
        (1, 1),
        "the title lookup and the removal-policy read are hoisted out of the per-row loop"
    );
}

// ── seeding history events ────────────────────────────────────────────────

async fn seeding_history_events(app: &AppUseCase) -> Vec<scryer_domain::DomainEvent> {
    let mut events = app
        .services
        .events
        .domain_events
        .list(&scryer_domain::DomainEventFilter {
            event_types: Some(vec![
                scryer_domain::DomainEventType::SeedingStarted,
                scryer_domain::DomainEventType::SeedingCompleted,
            ]),
            title_id: None,
            facet: None,
            after_sequence: None,
            before_sequence: None,
            limit: 0,
        })
        .await
        .expect("list seeding history events");
    events.sort_by_key(|event| event.sequence);
    events
}

/// A held torrent is re-offered to the gate on every poll; only the transition
/// that actually parked it is history.
#[tokio::test]
async fn parking_a_torrent_records_one_history_event_however_many_ticks_it_is_held() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "History Park").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-history-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    let events = seeding_history_events(&app).await;
    assert_eq!(events.len(), 1, "one park, one event");
    let scryer_domain::DomainEventPayload::SeedingStarted(data) = &events[0].payload else {
        panic!(
            "expected a seeding_started event, got {:?}",
            events[0].payload
        );
    };
    assert_eq!(data.download_client_item_id, "torrent-history-1");
    assert_eq!(data.client_id.as_deref(), Some(config.id.as_str()));
    assert_eq!(data.client_type.as_deref(), Some("qbittorrent"));
    assert_eq!(data.reason, "no_resolved_goals_and_client_verdict_unknown");
    assert_eq!(events[0].title_id.as_deref(), Some(title.id.as_str()));

    // Two more polls of the same held torrent.
    for _ in 0..2 {
        crate::app_usecase_integration::finalize_tracked_terminal_state(
            &app,
            &mut tracker,
            &id,
            TrackedDownloadState::ImportedSeeding,
        )
        .await;
    }

    assert_eq!(
        seeding_history_events(&app).await.len(),
        1,
        "a held torrent must not record an event per tick"
    );
}

#[tokio::test]
async fn releasing_a_parked_torrent_records_the_action_and_the_observed_progress() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "History Release").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-history-2",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    // Tick 1: nothing observed, so the entry is parked.
    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;
    assert_eq!(seeding_history_events(&app).await.len(), 1);

    // Tick 2: the client now reports its obligation discharged.
    tracker
        .find_mut(&id)
        .expect("parked row")
        .client_item
        .seeding = Some(DownloadSeedingSnapshot {
        can_remove: Some(true),
        can_move_files: Some(true),
        seed_ratio: Some(2.75),
        seed_time_seconds: Some(90_000),
        ..DownloadSeedingSnapshot::default()
    });
    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::ImportedSeeding,
    )
    .await;

    let events = seeding_history_events(&app).await;
    assert_eq!(events.len(), 2, "the park is closed by exactly one release");
    let scryer_domain::DomainEventPayload::SeedingCompleted(data) = &events[1].payload else {
        panic!(
            "expected a seeding_completed event, got {:?}",
            events[1].payload
        );
    };
    assert_eq!(data.action, "removed");
    assert_eq!(data.reason, "client_reports_seeding_obligation_met");
    assert_eq!(data.seed_ratio, Some(2.75));
    assert_eq!(data.seed_time_seconds, Some(90_000));
    assert_eq!(data.download_client_item_id, "torrent-history-2");
    assert!(tracker.find(&id).is_none());
}

/// A `StopSeeding` profile reports what actually happened to the entry, not the
/// profile's intent.
#[tokio::test]
async fn a_paused_release_is_recorded_as_paused() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (mut app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "History Pause").await;
    app.services.workflow.download_submissions = goals_repo(
        "torrent-history-3",
        PersistedSeedGoals {
            goal_met_action: Some(scryer_domain::SeedGoalMetAction::StopSeeding),
            ..persisted_goals(false)
        },
    );

    let mut tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-history-3",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked.clone());

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(4.0),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );
    tracker
        .find_mut(&id)
        .expect("parked row")
        .client_item
        .seeding = tracked.client_item.seeding.clone();

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::ImportedSeeding,
    )
    .await;

    let events = seeding_history_events(&app).await;
    assert_eq!(events.len(), 2);
    let scryer_domain::DomainEventPayload::SeedingCompleted(data) = &events[1].payload else {
        panic!(
            "expected a seeding_completed event, got {:?}",
            events[1].payload
        );
    };
    assert_eq!(data.action, "paused");
    assert_eq!(data.reason, "profile_goal_met");
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

/// A torrent the gate never held has no retention window, so it records no
/// seeding history at all.
#[tokio::test]
async fn a_torrent_that_was_never_held_records_no_seeding_history() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Never Held").await;

    let mut tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-history-4",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    observed(
        DownloadSeedingSnapshot {
            can_remove: Some(true),
            can_move_files: Some(true),
            ..DownloadSeedingSnapshot::default()
        },
        &mut tracked,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    assert!(tracker.find(&id).is_none());
    assert!(seeding_history_events(&app).await.is_empty());
}

/// Usenet never reaches the gate, so it never produces seeding history.
#[tokio::test]
async fn usenet_imports_record_no_seeding_history() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Usenet History").await;

    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-history-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    assert!(seeding_history_events(&app).await.is_empty());
}

// ── post-import handoff ───────────────────────────────────────────────────

/// The whole point of the feature: an imported torrent under a handoff profile
/// settles immediately, with nothing removed and nothing paused, and stops
/// being tracked.
#[tokio::test]
async fn a_handoff_profile_settles_an_imported_torrent_without_touching_the_client() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Hand Off").await;
    let mut app = app;
    app.services.workflow.download_submissions =
        goals_repo("torrent-handoff-1", handed_off_goals());

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-handoff-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    assert!(
        tracker.find(&id).is_none(),
        "a handed-off torrent stops being tracked and leaves the queue"
    );
    assert!(
        download_client.deleted_requests.lock().await.is_empty(),
        "a handoff never removes the client entry"
    );
    assert!(
        download_client.paused_requests.lock().await.is_empty(),
        "a handoff never touches the torrent at all"
    );

    let events = seeding_history_events(&app).await;
    assert_eq!(events.len(), 1, "one handoff, one event");
    let scryer_domain::DomainEventPayload::SeedingCompleted(data) = &events[0].payload else {
        panic!(
            "expected a seeding_completed event, got {:?}",
            events[0].payload
        );
    };
    assert_eq!(data.action, "handed_off");
    assert_eq!(data.reason, "post_import_handoff");
    assert_eq!(data.download_client_item_id, "torrent-handoff-1");
    assert_eq!(events[0].title_id.as_deref(), Some(title.id.as_str()));
}

/// A handed-off entry stays in the client, so the poller re-creates and
/// re-offers its row on every tick. The one-shot history must not become a
/// per-tick history.
#[tokio::test]
async fn a_re_offered_handed_off_row_settles_again_without_recording_another_event() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (mut app, tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Hand Off Again",
        "torrent-handoff-2",
        Some(handed_off_goals()),
    )
    .await;
    let _ = &mut app;
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked.clone());

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;
    assert_eq!(seeding_history_events(&app).await.len(), 1);

    // The next poll rebuilds the row from the client listing and the reconcile
    // tick re-offers it.
    tracker.insert_for_tests(tracked);
    crate::app_usecase_integration::finalize_tracked_terminal_state_with(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
        crate::app_usecase_integration::TerminalSettleTrigger::Reconcile,
        None,
    )
    .await;

    assert!(tracker.find(&id).is_none());
    assert!(download_client.deleted_requests.lock().await.is_empty());
    assert_eq!(
        seeding_history_events(&app).await.len(),
        1,
        "re-offering a settled handed-off row must not record the handoff again"
    );
}

/// The handoff outcome overrides the removal rails, not the import mode: a
/// torrent that may still be seeding is still imported by hardlink-or-copy.
#[tokio::test]
async fn a_configured_move_is_still_downgraded_for_a_handed_off_torrent() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    let title = movie_title(&app, &user, "Hand Off Move").await;
    let mut app = app;
    app.services.workflow.download_submissions =
        goals_repo("torrent-handoff-3", handed_off_goals());
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("configure move import mode");

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(
            &config.id,
            "qbittorrent",
            "torrent-handoff-3",
        )),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::HardlinkOrCopy);
}

/// The fail-closed rail stays: handoff is opt-in per profile, so a download
/// with no profile (and one under a parking profile) still parks.
#[tokio::test]
async fn a_download_without_a_handoff_profile_still_parks() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, tracked) = torrent_cleanup_fixture(
        download_client.clone(),
        "Still Parks",
        "torrent-handoff-4",
        Some(persisted_goals(false)),
    )
    .await;

    let cleanup = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
        None,
    )
    .await;

    assert_eq!(cleanup, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}
