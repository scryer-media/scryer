use super::*;

type DownloadBindingTimes = HashMap<
    scryer_domain::download_identity::DownloadId,
    (chrono::DateTime<Utc>, Option<chrono::DateTime<Utc>>),
>;

#[derive(Default)]
struct RecordingDownloadRegistry {
    rows: Arc<Mutex<HashMap<ClientJobLocator, scryer_domain::download_identity::DownloadId>>>,
    binding_times: Arc<Mutex<DownloadBindingTimes>>,
    ended: Arc<Mutex<HashSet<scryer_domain::download_identity::DownloadId>>>,
    terminal: Arc<Mutex<HashSet<scryer_domain::download_identity::DownloadId>>>,
    reconcile_candidates: Arc<Mutex<Vec<DownloadClientBindingRecord>>>,
    failing_bindings: Arc<Mutex<HashSet<ClientJobLocator>>>,
    strict_conflicts: bool,
}

fn fixed_time(value: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .expect("valid fixed test timestamp")
        .with_timezone(&Utc)
}

impl RecordingDownloadRegistry {
    async fn contains(&self, locator: &ClientJobLocator) -> bool {
        self.rows.lock().await.contains_key(locator)
    }

    async fn bind(
        &self,
        locator: ClientJobLocator,
        download_id: scryer_domain::download_identity::DownloadId,
    ) {
        self.rows.lock().await.insert(locator, download_id);
    }

    async fn bind_at(
        &self,
        locator: ClientJobLocator,
        download_id: scryer_domain::download_identity::DownloadId,
        created_at: chrono::DateTime<Utc>,
        last_seen_at: Option<chrono::DateTime<Utc>>,
    ) {
        self.bind(locator, download_id).await;
        self.binding_times
            .lock()
            .await
            .insert(download_id, (created_at, last_seen_at));
    }

    async fn fail_binding_lookup(&self, locator: ClientJobLocator) {
        self.failing_bindings.lock().await.insert(locator);
    }

    async fn set_reconcile_candidates(&self, candidates: Vec<DownloadClientBindingRecord>) {
        *self.reconcile_candidates.lock().await = candidates;
    }

    async fn mark_terminal(&self, download_id: scryer_domain::download_identity::DownloadId) {
        self.terminal.lock().await.insert(download_id);
    }
}

#[async_trait]
impl DownloadRegistryRepository for RecordingDownloadRegistry {
    async fn resolve_observation(
        &self,
        observation: &ObservedClientJob,
    ) -> AppResult<ObservationResolution> {
        let mut rows = self.rows.lock().await;
        let ended = self.ended.lock().await;
        let known = rows
            .get(&observation.locator)
            .copied()
            .filter(|download_id| !ended.contains(download_id));
        let token = observation
            .wire_token
            .as_deref()
            .and_then(scryer_domain::download_identity::DownloadId::from_wire);
        if self.strict_conflicts
            && let (Some(binding_download_id), Some(token_id)) = (known, token)
            && binding_download_id != token_id
        {
            return Ok(ObservationResolution::Conflict {
                token_id,
                binding_download_id,
            });
        }
        let download_id = token
            .or(known)
            .unwrap_or_else(scryer_domain::download_identity::DownloadId::new);
        let newly_foreign = known.is_none() && token.is_none();
        rows.insert(observation.locator.clone(), download_id);
        Ok(ObservationResolution::Resolved {
            download_id,
            newly_foreign,
            attached: false,
        })
    }

    async fn load_download(
        &self,
        id: &scryer_domain::download_identity::DownloadId,
    ) -> AppResult<Option<DownloadRecord>> {
        let is_terminal = self.terminal.lock().await.contains(id);
        Ok(self
            .rows
            .lock()
            .await
            .values()
            .any(|existing| existing == id)
            .then(|| DownloadRecord {
                id: *id,
                origin: DownloadOrigin::ForeignObservation,
                created_at: Utc::now(),
                first_observed_at: None,
                last_observed_at: None,
                terminal_at: is_terminal.then(Utc::now),
            }))
    }

    async fn load_binding(
        &self,
        id: &scryer_domain::download_identity::DownloadId,
    ) -> AppResult<Option<DownloadClientBindingRecord>> {
        let binding = self
            .rows
            .lock()
            .await
            .iter()
            .find_map(|(locator, existing)| (existing == id).then(|| locator.clone()));
        let Some(locator) = binding else {
            return Ok(None);
        };
        let ended_at = self.ended.lock().await.contains(id).then(Utc::now);
        let (created_at, last_seen_at) = self
            .binding_times
            .lock()
            .await
            .get(id)
            .copied()
            .unwrap_or_else(|| (Utc::now(), None));
        Ok(Some(DownloadClientBindingRecord {
            download_id: *id,
            client_config_id: locator.client_id,
            client_type_snapshot: Some(locator.client_type),
            client_name_snapshot: None,
            native_item_id: Some(locator.item_id),
            created_at,
            last_seen_at,
            ended_at,
        }))
    }

    async fn find_active_binding_by_locator(
        &self,
        locator: &ClientJobLocator,
    ) -> AppResult<Option<DownloadClientBindingRecord>> {
        if self.failing_bindings.lock().await.contains(locator) {
            return Err(AppError::Repository(
                "injected registry binding lookup failure".to_string(),
            ));
        }
        let Some(download_id) = self.rows.lock().await.get(locator).copied() else {
            return Ok(None);
        };
        if self.ended.lock().await.contains(&download_id) {
            return Ok(None);
        }
        let (created_at, last_seen_at) = self
            .binding_times
            .lock()
            .await
            .get(&download_id)
            .copied()
            .unwrap_or_else(|| (Utc::now(), None));
        Ok(Some(DownloadClientBindingRecord {
            download_id,
            client_config_id: locator.client_id.clone(),
            client_type_snapshot: Some(locator.client_type.clone()),
            client_name_snapshot: None,
            native_item_id: Some(locator.item_id.clone()),
            created_at,
            last_seen_at,
            ended_at: None,
        }))
    }

    async fn list_active_bindings_for_client_before(
        &self,
        client_config_id: &str,
        client_type: &str,
        observed_before: chrono::DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<DownloadClientBindingRecord>> {
        let ended = self.ended.lock().await;
        Ok(self
            .reconcile_candidates
            .lock()
            .await
            .iter()
            .filter(|binding| {
                binding.ended_at.is_none()
                    && binding.client_config_id.as_deref() == Some(client_config_id)
                    && binding
                        .client_type_snapshot
                        .as_deref()
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(client_type))
                    && binding.created_at <= observed_before
                    && binding
                        .last_seen_at
                        .is_none_or(|last_seen| last_seen <= observed_before)
                    && !ended.contains(&binding.download_id)
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn end_binding(
        &self,
        id: &scryer_domain::download_identity::DownloadId,
    ) -> AppResult<()> {
        self.ended.lock().await.insert(*id);
        Ok(())
    }
}

async fn publish_test_download_queue_snapshot(app: &AppUseCase, items: Vec<DownloadQueueItem>) {
    app.runtime
        .acquisition
        .download_queue_snapshot
        .stage_success(items)
        .await;
    sleep(crate::services::DOWNLOAD_QUEUE_SNAPSHOT_COALESCE_WINDOW + Duration::from_millis(50))
        .await;
}

#[tokio::test]
async fn push_snapshot_observation_is_recorded_by_the_registry_resolver() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));
    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            cancellation.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_secs(60),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let mut item = queue_history_fixture_item("weaver-push-1", DownloadQueueState::Downloading, 1);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    let locator = ClientJobLocator::new(
        Some(config.id.as_str()),
        item.client_type.as_str(),
        item.download_client_item_id.as_str(),
    );
    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("push snapshot should publish");

    timeout(Duration::from_secs(5), async {
        loop {
            if registry.contains(&locator).await {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("push snapshot should resolve a registry row");

    cancellation.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn authoritative_absence_fails_and_ends_an_incomplete_binding_before_reobservation() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));
    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    download_client
        .set_snapshot_authoritative_client_ids([config.id.clone()])
        .await;
    let first_download_id = scryer_domain::download_identity::DownloadId::new();
    let source_identity =
        ClientJobLocator::new(Some(config.id.as_str()), "nzbget", "removed-incomplete-1");
    registry
        .bind(source_identity.clone(), first_download_id)
        .await;
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: first_download_id,
            title_id: "title-removed-incomplete".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "removed-incomplete-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Removed.Incomplete.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed download submission");

    let mut item =
        queue_history_fixture_item("removed-incomplete-1", DownloadQueueState::Downloading, 1);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "nzbget".to_string();
    item.download_id = Some(first_download_id.to_wire());
    item.is_scryer_origin = true;
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.track(&app, item).await;

    crate::app_usecase_integration::reconcile_authoritatively_absent_source(
        &app,
        &mut tracker,
        &source_identity,
    )
    .await;

    assert!(registry.ended.lock().await.contains(&first_download_id));
    assert_eq!(
        download_submissions
            .get_tracked_state(&source_identity)
            .await
            .expect("failed state should load"),
        Some(TrackedDownloadState::Failed.as_str().to_string())
    );
    assert!(
        tracker
            .get_all()
            .iter()
            .any(|tracked| tracked.state == TrackedDownloadState::Failed)
    );

    let reobserved = registry
        .resolve_observation(&ObservedClientJob {
            locator: source_identity,
            wire_token: None,
            observed_name: Some("Removed.Incomplete.2026.1080p.WEB-DL".to_string()),
            observed_at: Utc::now(),
        })
        .await
        .expect("re-observation should resolve after the binding was ended");
    let ObservationResolution::Resolved {
        download_id: reobserved_download_id,
        ..
    } = reobserved
    else {
        panic!("re-observation should resolve to a fresh identity");
    };
    assert_ne!(reobserved_download_id, first_download_id);
}

#[tokio::test]
async fn authoritative_absence_ends_a_terminal_binding_without_replacing_its_state() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));
    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let download_id = scryer_domain::download_identity::DownloadId::new();
    let source_identity =
        ClientJobLocator::new(Some(config.id.as_str()), "nzbget", "removed-imported-1");
    registry.bind(source_identity.clone(), download_id).await;
    registry.mark_terminal(download_id).await;
    download_submissions
        .update_tracked_state(&source_identity, TrackedDownloadState::Imported.as_str())
        .await
        .expect("seed imported state");

    crate::app_usecase_integration::reconcile_authoritatively_absent_source(
        &app,
        &mut crate::tracked_downloads::TrackedDownloadService::new(),
        &source_identity,
    )
    .await;

    assert!(registry.ended.lock().await.contains(&download_id));
    assert_eq!(
        download_submissions
            .get_tracked_state(&source_identity)
            .await
            .expect("imported state should load"),
        Some(TrackedDownloadState::Imported.as_str().to_string())
    );
}

#[tokio::test]
async fn restart_ghost_reconcile_preserves_durable_post_queue_bindings() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));
    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;

    for (item_id, state) in [
        ("ghost-import-pending", TrackedDownloadState::ImportPending),
        ("ghost-import-blocked", TrackedDownloadState::ImportBlocked),
    ] {
        let download_id = scryer_domain::download_identity::DownloadId::new();
        let source_identity = ClientJobLocator::new(Some(config.id.as_str()), "nzbget", item_id);
        registry.bind(source_identity.clone(), download_id).await;
        download_submissions
            .record_identity_tracked_state(
                &DownloadSubmissionIdentity {
                    download_id: Some(download_id.to_wire()),
                },
                Some(&source_identity),
                state.as_str(),
                None,
                None,
            )
            .await
            .expect("seed preserved durable state");

        crate::app_usecase_integration::reconcile_authoritatively_absent_source(
            &app,
            &mut crate::tracked_downloads::TrackedDownloadService::new(),
            &source_identity,
        )
        .await;

        assert!(
            !registry.ended.lock().await.contains(&download_id),
            "{state:?} restart ghost must remain bound"
        );
        assert_eq!(
            download_submissions
                .get_identity_tracked_state_for_download(
                    Some(&download_id),
                    &DownloadSubmissionIdentity::default(),
                    Some(&source_identity),
                )
                .await
                .expect("preserved state should load"),
            Some(state.as_str().to_string())
        );
    }
}

#[tokio::test]
async fn full_snapshot_reconciles_a_restart_ghost_binding_but_respects_the_recency_floor() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));
    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let old_download_id = scryer_domain::download_identity::DownloadId::new();
    download_client
        .set_snapshot_authoritative_client_ids([config.id.clone()])
        .await;
    let old_source = ClientJobLocator::new(Some(config.id.as_str()), "nzbget", "restart-ghost-1");
    registry.bind(old_source, old_download_id).await;
    registry
        .set_reconcile_candidates(vec![DownloadClientBindingRecord {
            download_id: old_download_id,
            client_config_id: Some(config.id.clone()),
            client_type_snapshot: Some("nzbget".to_string()),
            client_name_snapshot: Some(config.name.clone()),
            native_item_id: Some("restart-ghost-1".to_string()),
            created_at: Utc::now() - chrono::Duration::minutes(11),
            last_seen_at: Some(Utc::now() - chrono::Duration::minutes(11)),
            ended_at: None,
        }])
        .await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(1);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                ..Default::default()
            },
        ),
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if registry.ended.lock().await.contains(&old_download_id) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("full snapshot should end an old restart-ghost binding");

    token.cancel();
    poller.await.expect("poller should stop cleanly");

    let fresh_client = Arc::new(StubDownloadClient::default());
    let fresh_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let fresh_pending = Arc::new(TrackingPendingReleaseRepo::default());
    let (fresh_base_app, fresh_user) =
        bootstrap_with_cleanup_tracking(fresh_client.clone(), fresh_submissions, fresh_pending);
    let fresh_registry = Arc::new(RecordingDownloadRegistry::default());
    let fresh_app = fresh_base_app
        .with_test_overrides(|services| services.with_download_registry(fresh_registry.clone()));
    let fresh_config =
        create_enabled_download_client_config(&fresh_app, &fresh_user, "Primary NZBGet", "nzbget")
            .await;
    fresh_client
        .set_snapshot_authoritative_client_ids([fresh_config.id.clone()])
        .await;
    let fresh_download_id = scryer_domain::download_identity::DownloadId::new();
    let fresh_source =
        ClientJobLocator::new(Some(fresh_config.id.as_str()), "nzbget", "fresh-binding-1");
    fresh_registry.bind(fresh_source, fresh_download_id).await;
    fresh_registry
        .set_reconcile_candidates(vec![DownloadClientBindingRecord {
            download_id: fresh_download_id,
            client_config_id: Some(fresh_config.id.clone()),
            client_type_snapshot: Some("nzbget".to_string()),
            client_name_snapshot: Some(fresh_config.name.clone()),
            native_item_id: Some("fresh-binding-1".to_string()),
            created_at: Utc::now(),
            last_seen_at: None,
            ended_at: None,
        }])
        .await;

    let (_command_tx, fresh_tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, fresh_snapshot_rx) = tokio::sync::mpsc::channel(1);
    let fresh_token = tokio_util::sync::CancellationToken::new();
    let fresh_poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            fresh_app,
            fresh_token.child_token(),
            fresh_tracked_download_rx,
            fresh_snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                ..Default::default()
            },
        ),
    );
    sleep(Duration::from_millis(250)).await;
    assert!(
        !fresh_registry
            .ended
            .lock()
            .await
            .contains(&fresh_download_id),
        "a binding younger than the recency floor must not be ended"
    );
    fresh_token.cancel();
    fresh_poller
        .await
        .expect("fresh-binding poller should stop cleanly");
}

#[tokio::test]
async fn failed_client_poll_does_not_end_bindings_or_clean_manual_import_records() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_queue_error(Some("client unavailable"))
        .await;
    download_client
        .set_recent_activity_error(Some("client unavailable"))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let imports = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_registry(registry.clone())
            .with_imports(imports.clone())
    });
    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let download_id = scryer_domain::download_identity::DownloadId::new();
    let source_identity =
        ClientJobLocator::new(Some(config.id.as_str()), "nzbget", "failed-poll-1");
    registry.bind(source_identity.clone(), download_id).await;
    registry
        .set_reconcile_candidates(vec![DownloadClientBindingRecord {
            download_id,
            client_config_id: Some(config.id.clone()),
            client_type_snapshot: Some("nzbget".to_string()),
            client_name_snapshot: Some(config.name.clone()),
            native_item_id: Some("failed-poll-1".to_string()),
            created_at: Utc::now() - chrono::Duration::minutes(11),
            last_seen_at: None,
            ended_at: None,
        }])
        .await;
    let import_id = app
        .services
        .workflow
        .imports
        .queue_import_request(
            source_identity,
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue manual import record");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(1);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app,
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                ..Default::default()
            },
        ),
    );
    sleep(Duration::from_millis(250)).await;
    assert!(!registry.ended.lock().await.contains(&download_id));
    assert!(
        imports
            .records
            .lock()
            .await
            .iter()
            .any(|record| record.id == import_id && record.status == ImportStatus::Pending),
        "a failed poll must not clean the manual import record"
    );
    token.cancel();
    poller.await.expect("poller should stop cleanly");
}

#[tokio::test]
async fn download_identity_shapes_keep_current_queue_and_history_projections() {
    const TORRENT_INFO_HASH: &str = "abcdef0123456789abcdef0123456789abcdef01";

    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    let mut nzbget =
        queue_history_fixture_item("42", DownloadQueueState::Downloading, 1_700_000_000);
    nzbget.client_id = "nzbget-primary".to_string();
    nzbget.client_name = "NZBGet".to_string();
    nzbget.client_type = "nzbget".to_string();
    nzbget.download_id = Some("scryer-download:nzbget-token".to_string());
    nzbget.is_scryer_origin = true;

    let mut sabnzbd = queue_history_fixture_item(
        "SABnzbd_nzo_abc123",
        DownloadQueueState::Downloading,
        1_700_000_000,
    );
    sabnzbd.client_id = "sabnzbd-primary".to_string();
    sabnzbd.client_name = "SABnzbd".to_string();
    sabnzbd.client_type = "sabnzbd".to_string();
    sabnzbd.download_id = Some("SABnzbd_nzo_abc123".to_string());
    sabnzbd.is_scryer_origin = false;

    let mut torrent = queue_history_fixture_item(
        "native-torrent-item",
        DownloadQueueState::Downloading,
        1_700_000_000,
    );
    torrent.id = format!("qbittorrent:{TORRENT_INFO_HASH}");
    torrent.client_id = "torrent-primary".to_string();
    torrent.client_name = "qBittorrent".to_string();
    torrent.client_type = "qbittorrent".to_string();
    torrent.download_id = Some(TORRENT_INFO_HASH.to_string());
    torrent.is_scryer_origin = false;

    let mut plugin = queue_history_fixture_item(
        "plugin-item-1",
        DownloadQueueState::Downloading,
        1_700_000_000,
    );
    plugin.id = "plugin-client:plugin-item-1".to_string();
    plugin.client_id = "plugin-primary".to_string();
    plugin.client_name = "Plugin client".to_string();
    plugin.client_type = "plugin-client".to_string();
    plugin.download_id = Some("scryer-download:plugin-token".to_string());
    plugin.is_scryer_origin = false;

    let queue_items = vec![nzbget, sabnzbd, torrent, plugin];
    publish_test_download_queue_snapshot(&app, queue_items.clone()).await;

    let assert_identity_projection = |items: &[DownloadQueueItem]| {
        for (client_item_id, id, download_id, is_scryer_origin) in [
            ("42", "42", "scryer-download:nzbget-token", true),
            (
                "SABnzbd_nzo_abc123",
                "SABnzbd_nzo_abc123",
                "SABnzbd_nzo_abc123",
                false,
            ),
            (
                "native-torrent-item",
                "qbittorrent:abcdef0123456789abcdef0123456789abcdef01",
                "abcdef0123456789abcdef0123456789abcdef01",
                false,
            ),
            (
                "plugin-item-1",
                "plugin-client:plugin-item-1",
                "scryer-download:plugin-token",
                false,
            ),
        ] {
            let item = items
                .iter()
                .find(|item| item.download_client_item_id == client_item_id)
                .unwrap_or_else(|| panic!("missing {client_item_id} identity projection"));
            assert_eq!(item.id, id);
            assert_eq!(item.download_id.as_deref(), Some(download_id));
            assert_eq!(item.download_client_item_id, client_item_id);
            assert_eq!(item.is_scryer_origin, is_scryer_origin);
        }
    };

    let queue = app
        .list_download_queue(&user, true, false, false, DownloadActivityFilter::All)
        .await
        .expect("queue should load");
    assert_identity_projection(&queue);

    let history_items = queue_items
        .into_iter()
        .map(|mut item| {
            item.state = DownloadQueueState::Completed;
            item.last_updated_at = Some("1700000000".to_string());
            item
        })
        .collect();
    publish_test_download_queue_snapshot(&app, history_items).await;

    let history = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("history should load");
    assert_identity_projection(&history.items);
}

#[tokio::test]
async fn list_download_queue_reads_cached_observed_items_without_client_calls() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "SABnzbd".to_string(),
            client_type: "sabnzbd".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: String::new(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "observed-stub".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Observed Download".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record stub submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "observed-stub".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Observed Download".to_string(),
        facet: None,
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "sabnzbd".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "observed-stub".to_string(),
        download_id: None,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        source_provider: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
        seeding: None,
    }];
    app.runtime
        .acquisition
        .download_queue_snapshot
        .stage_success(download_client.queue_items.lock().await.clone())
        .await;
    sleep(crate::services::DOWNLOAD_QUEUE_SNAPSHOT_COALESCE_WINDOW + Duration::from_millis(50))
        .await;

    let items = app
        .list_download_queue(&user, true, false, false, DownloadActivityFilter::All)
        .await
        .expect("list queue");

    assert_eq!(items.len(), 1);
    assert!(!items[0].is_scryer_origin);
    assert!(items[0].title_id.is_none());
    assert!(items[0].facet.is_none());
    assert_eq!(*download_client.queue_calls.lock().await, 0);
}

#[tokio::test]
async fn list_download_queue_uses_live_queue_only_for_all_activity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    *download_client.history_items.lock().await = vec![DownloadQueueItem {
        id: "history-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "History Download".to_string(),
        facet: None,
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Completed,
        progress_percent: 100,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: Some("100".to_string()),
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "history-1".to_string(),
        download_id: None,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        source_provider: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
        seeding: None,
    }];

    let items = app
        .list_download_queue(&user, true, false, false, DownloadActivityFilter::All)
        .await
        .expect("list queue should succeed");

    assert!(items.is_empty());
    assert_eq!(*download_client.history_calls.lock().await, 0);
    assert!(
        download_client
            .recent_activity_calls
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn list_download_queue_for_title_filters_the_shared_cache() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    app.services
        .catalog
        .titles
        .create(make_due_hydration_title("title-1", MediaFacet::Series, 1))
        .await
        .expect("seed title");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title-1".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "job-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Title Scoped Download".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "job-1".to_string(),
        title_id: Some("title-1".to_string()),
        episode_id: None,
        title_name: "Title Scoped Download".to_string(),
        facet: Some("series".to_string()),
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "job-1".to_string(),
        download_id: None,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        source_provider: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
        seeding: None,
    }];
    app.runtime
        .acquisition
        .download_queue_snapshot
        .stage_success(download_client.queue_items.lock().await.clone())
        .await;
    sleep(crate::services::DOWNLOAD_QUEUE_SNAPSHOT_COALESCE_WINDOW + Duration::from_millis(50))
        .await;

    let items = app
        .list_download_queue_for_title(
            &user,
            "title-1",
            false,
            false,
            false,
            DownloadActivityFilter::All,
        )
        .await
        .expect("title-scoped queue should load");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].download_client_item_id, "job-1");
    assert_eq!(items[0].title_id.as_deref(), Some("title-1"));
    assert_eq!(*download_client.queue_calls.lock().await, 0);
    assert!(
        download_client
            .queue_for_title_calls
            .lock()
            .await
            .is_empty()
    );
    assert!(
        download_client
            .recent_activity_for_title_calls
            .lock()
            .await
            .is_empty()
    );

    let mut history_items = (0..60)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("other-{index:02}"),
                DownloadQueueState::Completed,
                100 + index,
            );
            item.title_id = None;
            item
        })
        .collect::<Vec<_>>();
    let mut title_history =
        queue_history_fixture_item("title-history", DownloadQueueState::Completed, 1);
    title_history.title_id = Some("title-1".to_string());
    history_items.push(title_history);
    publish_test_download_queue_snapshot(&app, history_items).await;

    let title_history = app
        .list_download_queue_for_title(
            &user,
            "title-1",
            true,
            true,
            false,
            DownloadActivityFilter::All,
        )
        .await
        .expect("title history should apply the legacy limit after title filtering");
    assert_eq!(title_history.len(), 1);
    assert_eq!(title_history[0].download_client_item_id, "title-history");
}

#[tokio::test]
async fn list_download_queue_page_clamps_filters_and_uses_stable_identity_ordering() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    let items = (0..250)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("job-{index:03}"),
                match index % 3 {
                    0 => DownloadQueueState::Queued,
                    1 => DownloadQueueState::Downloading,
                    _ => DownloadQueueState::Paused,
                },
                index,
            );
            item.title_id = None;
            item.client_id = if index % 2 == 0 {
                "client-a".to_string()
            } else {
                "client-b".to_string()
            };
            item.client_name = item.client_id.clone();
            item.client_type = "qbittorrent".to_string();
            item.progress_percent = (index % 100) as u8;
            item.size_bytes = Some(1);
            item
        })
        .collect::<Vec<_>>();
    publish_test_download_queue_snapshot(&app, items).await;

    let all = app
        .list_download_queue_page(
            &user,
            500,
            0,
            None,
            None,
            false,
            None,
            DownloadHistorySort {
                key: DownloadHistorySortKey::Size,
                direction: SortDirection::Asc,
            },
        )
        .await
        .expect("paged queue should load");
    assert_eq!(all.items.len(), 200);
    assert_eq!(all.total_count, 250);
    assert!(all.has_more);
    assert_eq!(all.available_clients.len(), 2);
    assert!(
        all.items[..125]
            .iter()
            .all(|item| item.client_id == "client-a")
    );

    let empty_clients = app
        .list_download_queue_page(
            &user,
            50,
            0,
            None,
            Some(Vec::new()),
            false,
            None,
            DownloadHistorySort {
                key: DownloadHistorySortKey::Status,
                direction: SortDirection::Asc,
            },
        )
        .await
        .expect("empty client selection should load");
    assert!(empty_clients.items.is_empty());
    assert_eq!(empty_clients.total_count, 0);
    assert_eq!(empty_clients.available_clients.len(), 2);

    let empty_statuses = app
        .list_download_queue_page(
            &user,
            50,
            0,
            Some(Vec::new()),
            None,
            false,
            None,
            DownloadHistorySort {
                key: DownloadHistorySortKey::Status,
                direction: SortDirection::Asc,
            },
        )
        .await
        .expect("empty status selection should load");
    assert!(empty_statuses.items.is_empty());
    assert_eq!(empty_statuses.total_count, 0);
    assert!(empty_statuses.available_clients.is_empty());
}

#[tokio::test]
async fn list_download_import_page_returns_only_import_rows_for_selected_filter() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Running);

    let mut pending =
        queue_history_fixture_item("pending-1", DownloadQueueState::ImportPending, 30);
    pending.tracked_state = Some(TrackedDownloadState::ImportPending);

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let mut observed_blocked =
        queue_history_fixture_item("observed-blocked-1", DownloadQueueState::Completed, 25);
    observed_blocked.is_scryer_origin = false;
    observed_blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let failed = queue_history_fixture_item("failed-1", DownloadQueueState::Failed, 10);
    let completed = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 5);

    *download_client.history_items.lock().await = vec![
        completed,
        failed,
        observed_blocked,
        blocked.clone(),
        pending,
        importing,
    ];
    let snapshot_items = download_client.history_items.lock().await.clone();
    publish_test_download_queue_snapshot(&app, snapshot_items).await;

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
        .await
        .expect("import page should load");

    // A non-Scryer-origin row is still eligible for manual assignment unless
    // the runtime classifier or configured category ownership proves that it
    // belongs to another app.
    assert!(!page.has_more);
    assert_eq!(page.total_count, 2);
    assert_eq!(page.items.len(), 2);
    let blocked_ids = page
        .items
        .iter()
        .map(|item| item.download_client_item_id.as_str())
        .collect::<Vec<_>>();
    assert!(blocked_ids.contains(&"blocked-1"));
    assert!(blocked_ids.contains(&"observed-blocked-1"));
    for item in &page.items {
        assert_eq!(
            crate::integration::derive_download_queue_display_state(item),
            DownloadDisplayState::ImportBlocked
        );
    }

    let count = app
        .count_download_import_items(&user, DownloadImportFilter::Blocked)
        .await
        .expect("import count should load");
    // Must agree with page.total_count or the import badge drifts from the list.
    assert_eq!(count, 2);
}

#[tokio::test]
async fn count_download_import_items_matches_selected_filter() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Running);

    let mut pending =
        queue_history_fixture_item("pending-1", DownloadQueueState::ImportPending, 30);
    pending.tracked_state = Some(TrackedDownloadState::ImportPending);

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let failed = queue_history_fixture_item("failed-1", DownloadQueueState::Failed, 10);
    let completed = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 5);

    *download_client.history_items.lock().await =
        vec![completed, failed, blocked, pending.clone(), importing];
    let snapshot_items = download_client.history_items.lock().await.clone();
    publish_test_download_queue_snapshot(&app, snapshot_items).await;

    let all_page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("all import page");
    let all_count = app
        .count_download_import_items(&user, DownloadImportFilter::All)
        .await
        .expect("all import count");
    let attention_page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::Attention)
        .await
        .expect("attention import page");
    let attention_count = app
        .count_download_import_items(&user, DownloadImportFilter::Attention)
        .await
        .expect("attention import count");
    let pending_count = app
        .count_download_import_items(&user, DownloadImportFilter::Pending)
        .await
        .expect("pending import count");

    assert_eq!(all_count, all_page.total_count as i64);
    assert_eq!(attention_count, attention_page.total_count as i64);
    assert_eq!(attention_count, 2);
    assert!(
        attention_page
            .items
            .iter()
            .all(|item| item.download_client_item_id != "importing-1")
    );
    assert_eq!(pending_count, 1);
    assert_eq!(pending.download_client_item_id, "pending-1");
}

#[tokio::test]
async fn download_import_blocked_includes_snapshot_only_item_when_history_is_empty() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;

    let mut blocked =
        queue_history_fixture_item("blocked-snapshot-1", DownloadQueueState::Completed, 20);
    insert_tracked_download_snapshot(
        &app,
        "blocked-snapshot-1",
        TrackedDownloadState::ImportBlocked,
        blocked.clone(),
    )
    .await;
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    blocked.tracked_status = Some(scryer_domain::TrackedDownloadStatus::Warning);
    blocked.tracked_status_messages = vec!["tracked import_blocked".to_string()];
    publish_test_download_queue_snapshot(&app, vec![blocked]).await;

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
        .await
        .expect("blocked import page should include snapshot-only tracked rows");
    let count = app
        .count_download_import_items(&user, DownloadImportFilter::Blocked)
        .await
        .expect("blocked import count should include snapshot-only tracked rows");

    assert_eq!(page.total_count, 1);
    assert_eq!(count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "blocked-snapshot-1");
    assert_eq!(page.items[0].state, DownloadQueueState::Completed);
    assert_eq!(page.items[0].import_status, None);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::ImportBlocked)
    );
    assert_eq!(
        crate::integration::derive_download_queue_display_state(&page.items[0]),
        DownloadDisplayState::ImportBlocked
    );
}

#[tokio::test]
async fn download_import_all_includes_snapshot_only_pending_and_importing_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;

    let pending_client =
        queue_history_fixture_item("pending-snapshot-1", DownloadQueueState::Completed, 30);
    insert_tracked_download_snapshot(
        &app,
        "pending-snapshot-1",
        TrackedDownloadState::ImportPending,
        pending_client.clone(),
    )
    .await;
    let importing_client =
        queue_history_fixture_item("importing-snapshot-1", DownloadQueueState::Completed, 40);
    insert_tracked_download_snapshot(
        &app,
        "importing-snapshot-1",
        TrackedDownloadState::Importing,
        importing_client.clone(),
    )
    .await;
    let mut pending_projection = pending_client;
    pending_projection.state = DownloadQueueState::ImportPending;
    pending_projection.tracked_state = Some(TrackedDownloadState::ImportPending);
    let mut importing_projection = importing_client;
    importing_projection.import_status = Some(ImportStatus::Running);
    importing_projection.tracked_state = Some(TrackedDownloadState::Importing);
    publish_test_download_queue_snapshot(&app, vec![pending_projection, importing_projection])
        .await;

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("all import page should include snapshot-only tracked rows");

    assert_eq!(page.total_count, 2);
    let pending = page
        .items
        .iter()
        .find(|item| item.download_client_item_id == "pending-snapshot-1")
        .expect("pending snapshot row");
    assert_eq!(pending.state, DownloadQueueState::ImportPending);
    assert_eq!(pending.import_status, None);
    assert_eq!(
        crate::integration::derive_download_queue_display_state(pending),
        DownloadDisplayState::ImportPending
    );

    let importing = page
        .items
        .iter()
        .find(|item| item.download_client_item_id == "importing-snapshot-1")
        .expect("importing snapshot row");
    assert_eq!(importing.state, DownloadQueueState::Completed);
    assert_eq!(importing.import_status, Some(ImportStatus::Running));
    assert_eq!(
        crate::integration::derive_download_queue_display_state(importing),
        DownloadDisplayState::Importing
    );
}

#[tokio::test]
async fn synthetic_download_import_rows_are_enriched_from_submissions_before_permission_filtering()
{
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, admin) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );

    create_enabled_download_client_config(&app, &admin, "NZBGet", "nzbget").await;
    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Manual Import Visibility".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title for scoped visibility");
    let scoped_actor = library_permission_user(
        "resolver",
        &title.library_id,
        &[scryer_domain::LibraryPermission::ResolveImports],
    );

    let mut blocked =
        queue_history_fixture_item("blocked-submission-1", DownloadQueueState::Completed, 20);
    blocked.title_id = None;
    blocked.facet = None;
    insert_tracked_download_snapshot(
        &app,
        "blocked-submission-1",
        TrackedDownloadState::ImportBlocked,
        blocked.clone(),
    )
    .await;
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "blocked-submission-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Manual Import Visibility".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record download submission");
    blocked.title_id = Some(title.id.clone());
    blocked.facet = Some("movie".to_string());
    blocked.title_name = "Manual Import Visibility".to_string();
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    publish_test_download_queue_snapshot(&app, vec![blocked]).await;

    let page = app
        .list_download_import_page(&scoped_actor, 50, 0, DownloadImportFilter::Blocked)
        .await
        .expect("scoped actor should see submission-enriched snapshot row");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].download_client_item_id,
        "blocked-submission-1"
    );
    assert_eq!(page.items[0].title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(page.items[0].facet.as_deref(), Some("movie"));
}

#[tokio::test]
async fn find_download_queue_scope_ignores_stale_submission_titles() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Scope Regression Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create visible title");

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.title_id = Some(title.id.clone());
    blocked.title_name = title.name.clone();
    blocked.facet = Some("movie".to_string());
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    *download_client.history_items.lock().await = vec![blocked.clone()];
    publish_test_download_queue_snapshot(&app, vec![blocked]).await;

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "missing-title".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "blocked-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Fixture blocked-1".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record stale submission");

    let scope = app
        .find_download_queue_scope(&user, Some("primary"), "nzbget", "blocked-1")
        .await
        .expect("stale scope lookup should not fail");
    assert!(scope.is_none());

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("download import page should still load");
    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "blocked-1");
}

#[tokio::test]
async fn find_download_queue_scope_returns_orphan_without_title_lookup() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("weaver-primary".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "observed-10000".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Observed Weaver Download".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record orphan submission");

    let scope = app
        .find_download_queue_scope(&user, Some("weaver-primary"), "weaver", "observed-10000")
        .await
        .expect("orphan scope lookup should not require a title");

    assert!(matches!(scope, Some(SubmissionScope::Orphan)));
}

#[tokio::test]
async fn manual_import_source_allows_orphan_submission_but_rejects_managed_reassignment() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let selected_title = app
        .add_title(
            &user,
            NewTitle {
                name: "Observed Manual Import Target".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create selected title");
    let managed_title = app
        .add_title(
            &user,
            NewTitle {
                name: "Managed Manual Import Target".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create managed title");
    let source_dir = tempfile::tempdir().expect("source tempdir");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("weaver-primary".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "observed-manual-import".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Observed.Manual.Import.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record observed submission");
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: managed_title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("weaver-primary".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "managed-manual-import".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Managed.Manual.Import.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record managed submission");

    let mut observed_completed = completed_download_fixture_item(
        "observed-manual-import",
        &selected_title.id,
        "Observed.Manual.Import.2026.1080p.WEB-DL",
        source_dir.path().to_string_lossy().as_ref(),
    );
    observed_completed.client_id = "weaver-primary".to_string();
    observed_completed.client_type = "weaver".to_string();
    let mut managed_completed = completed_download_fixture_item(
        "managed-manual-import",
        &managed_title.id,
        "Managed.Manual.Import.2026.1080p.WEB-DL",
        source_dir.path().to_string_lossy().as_ref(),
    );
    managed_completed.client_id = "weaver-primary".to_string();
    managed_completed.client_type = "weaver".to_string();
    *download_client.completed_downloads.lock().await = vec![observed_completed, managed_completed];

    let observed = crate::import::workflow::resolve_current_manual_import_source(
        &app,
        &user,
        "weaver-primary",
        "weaver",
        "observed-manual-import",
        &selected_title.id,
    )
    .await
    .expect("an authorized target title may resolve a observed submission");
    assert_eq!(observed.download_client_item_id, "observed-manual-import");

    let catalog_viewer = app
        .create_user(
            &user,
            "observed_manual_import_catalog_viewer".to_string(),
            "password123".to_string(),
            scryer_domain::AppPermissionMask::NONE,
            vec![scryer_domain::LibraryGrant {
                user_id: String::new(),
                library_id: selected_title.library_id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::VIEW,
            }],
        )
        .await
        .expect("create view-only user");
    let catalog_viewer_token = app
        .issue_access_token(&catalog_viewer)
        .await
        .expect("issue view-only user token");
    let catalog_viewer = app
        .authenticate_token(&catalog_viewer_token)
        .await
        .expect("authenticate view-only user");
    let permission_error = crate::import::workflow::resolve_current_manual_import_source(
        &app,
        &catalog_viewer,
        "weaver-primary",
        "weaver",
        "observed-manual-import",
        &selected_title.id,
    )
    .await
    .expect_err("users without ResolveImports cannot select a target title");
    assert!(
        permission_error
            .to_string()
            .contains("do not have access to this library"),
        "unexpected permission error: {permission_error}"
    );

    let reassignment_error = crate::import::workflow::resolve_current_manual_import_source(
        &app,
        &user,
        "weaver-primary",
        "weaver",
        "managed-manual-import",
        &selected_title.id,
    )
    .await
    .expect_err("managed submission must remain title-bound");
    assert!(
        reassignment_error
            .to_string()
            .contains("download is no longer available for manual import"),
        "unexpected reassignment error: {reassignment_error}"
    );

    let unavailable_error = crate::import::workflow::resolve_current_manual_import_source(
        &app,
        &user,
        "weaver-primary",
        "weaver",
        "missing-manual-import",
        &selected_title.id,
    )
    .await
    .expect_err("missing completed source must remain unavailable");
    assert!(
        unavailable_error
            .to_string()
            .contains("download is no longer available for manual import"),
        "unexpected unavailable-source error: {unavailable_error}"
    );
}

#[tokio::test]
async fn manual_import_source_uses_retained_tracked_source_when_live_history_is_empty() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (command_tx, mut command_rx) = tokio::sync::mpsc::channel(1);
    let tracked_handle = crate::tracked_downloads::TrackedDownloadHandle::new(command_tx);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        tracked_handle,
    );
    let selected_title = app
        .add_title(
            &user,
            NewTitle {
                name: "Retained Manual Import Target".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create selected title");
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("qbittorrent-primary".to_string()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: "retained-manual-import".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Retained.Manual.Import.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record orphan submission");

    let source_dir = tempfile::tempdir().expect("create source directory");
    let mut retained = completed_download_fixture_item(
        "retained-manual-import",
        &selected_title.id,
        "Retained.Manual.Import.2026.1080p.WEB-DL",
        source_dir.path().to_string_lossy().as_ref(),
    );
    retained.client_id = "qbittorrent-primary".to_string();
    retained.client_type = "qbittorrent".to_string();
    let responder = tokio::spawn(async move {
        match command_rx.recv().await.expect("completed-source command") {
            crate::tracked_downloads::TrackedDownloadCommand::CompletedSource {
                identity,
                reply,
            } => {
                assert_eq!(identity.client_id.as_deref(), Some("qbittorrent-primary"));
                assert_eq!(identity.client_type, "qbittorrent");
                assert_eq!(identity.item_id, "retained-manual-import");
                let _ = reply.send(Some(retained));
            }
            _ => panic!("unexpected tracked-download command"),
        }
    });

    let resolved = crate::import::workflow::resolve_current_manual_import_source(
        &app,
        &user,
        "qbittorrent-primary",
        "qbittorrent",
        "retained-manual-import",
        &selected_title.id,
    )
    .await
    .expect("retained tracked source should resolve");
    responder.await.expect("completed-source responder");

    assert_eq!(resolved.download_client_item_id, "retained-manual-import");
    assert!(
        download_client
            .targeted_completed_download_calls
            .lock()
            .await
            .is_empty(),
        "retained resolution must not re-query live completed history"
    );
}

#[tokio::test]
async fn queued_manual_import_rejects_observed_targets_before_consuming_or_queueing() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));
    let bound_title = app
        .add_title(
            &user,
            NewTitle {
                name: "Bound Manual Import Title".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create bound title");
    let observed_title = app
        .add_title(
            &user,
            NewTitle {
                name: "Observed Manual Import Title".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create observed title");
    let observed_episode = scryer_domain::Episode {
        id: Id::new().0,
        title_id: observed_title.id.clone(),
        collection_id: None,
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Observed Episode".to_string()),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: Utc::now(),
    };
    app.services
        .catalog
        .shows
        .create_episode(observed_episode.clone())
        .await
        .expect("create observed episode");
    let observed_series_movie = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &observed_title.id,
            "Observed Manual Import Movie",
            Some(2026),
            None,
            None,
        ))
        .await
        .expect("create observed series-movie link");
    let selection = crate::ManualImportSelection {
        id: Id::new().0,
        actor_user_id: user.id.clone(),
        title_id: bound_title.id.clone(),
        source_identity: ClientJobLocator {
            client_id: Some("qbittorrent-primary".to_string()),
            client_type: "qbittorrent".to_string(),
            item_id: "observed-target-selection".to_string(),
        },
        canonical_download_id: None,
        release_evidence_json: None,
        trusted_source_root: "/private/tmp".to_string(),
        archive_workspace_root: None,
        candidates: vec![crate::ManualImportSelectionCandidate {
            id: "candidate-1".to_string(),
            canonical_path: "/private/tmp/observed-target-selection.mkv".to_string(),
        }],
    };
    *import_repo.manual_import_selection.lock().await = Some(selection.clone());

    for mapping in [
        crate::ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: Some(observed_episode.id),
            series_movie_link_id: None,
        },
        crate::ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: None,
            series_movie_link_id: Some(observed_series_movie.id),
        },
    ] {
        let error = app
            .queue_manual_import_selection(&user, selection.id.clone(), vec![mapping])
            .await
            .expect_err("observed target should be rejected before queueing");
        assert!(
            error.to_string().contains("does not belong to title"),
            "unexpected validation error: {error}"
        );
    }

    assert_eq!(
        import_repo
            .manual_import_selection_consume_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "scope validation must happen before consuming the selection"
    );
    assert!(import_repo.records.lock().await.is_empty());
    assert_eq!(
        import_repo
            .manual_import_selection
            .lock()
            .await
            .as_ref()
            .map(|stored| stored.id.as_str()),
        Some(selection.id.as_str()),
        "rejected mappings must leave the selection available"
    );
}

#[tokio::test]
async fn queued_manual_import_reports_prior_automatic_import_after_source_cleanup() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Already Imported Manual Target".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let mut permissions = scryer_domain::LibraryPermissionMask::NONE;
    permissions.insert(scryer_domain::LibraryPermissionMask::RESOLVE_IMPORTS);
    let actor = app
        .create_user(
            &user,
            "manual_import_race_actor".to_string(),
            "password123".to_string(),
            scryer_domain::AppPermissionMask::NONE,
            vec![scryer_domain::LibraryGrant {
                user_id: String::new(),
                library_id: title.library_id.clone(),
                permissions,
            }],
        )
        .await
        .expect("create manual import actor");
    let client_id = "weaver-primary";
    let item_id = "manual-race-source";
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(client_id.to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Already.Imported.Manual.Target.2026".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");

    let manual_import_id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    import_repo.records.lock().await.extend([
        ImportRecord {
            id: Id::new().0,
            source_client_id: Some(client_id.to_string()),
            source_system: "weaver".to_string(),
            source_ref: item_id.to_string(),
            import_type: ImportType::MovieDownload,
            status: ImportStatus::Completed,
            payload_json: String::new(),
            result_json: None,
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: Some(now.clone()),
            finished_at: Some(now.clone()),
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        ImportRecord {
            id: manual_import_id.clone(),
            source_client_id: Some(client_id.to_string()),
            source_system: "weaver".to_string(),
            source_ref: item_id.to_string(),
            import_type: ImportType::ManualImport,
            status: ImportStatus::Pending,
            payload_json: String::new(),
            result_json: None,
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: None,
            finished_at: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    ]);
    let payload = crate::import_workflow::ManualImportRequestPayload {
        requested_by_user_id: Some(actor.id),
        title_id: Some(title.id),
        download_client_item_id: item_id.to_string(),
        client_id: Some(client_id.to_string()),
        client_type: "weaver".to_string(),
        files: Vec::new(),
        selection_id: None,
        release_evidence: None,
        trusted_source_root: None,
        archive_workspace_root: None,
        requested_at: now,
    };

    let (status, result_json) =
        crate::import_workflow::execute_queued_manual_import(&app, &manual_import_id, &payload)
            .await
            .expect("prior import should win without a live source lookup");

    assert_eq!(status, ImportStatus::Skipped);
    let result: scryer_domain::ImportResult = serde_json::from_str(
        result_json
            .as_deref()
            .expect("already-imported result json"),
    )
    .expect("parse already-imported result");
    assert_eq!(result.decision, scryer_domain::ImportDecision::Skipped);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::AlreadyImported));
}

struct TrackedTitleAssignmentFixture {
    app: AppUseCase,
    user: User,
    submissions: Arc<TrackingDownloadSubmissionRepo>,
    title: Title,
    tracker: crate::tracked_downloads::TrackedDownloadService,
    tracked_id: String,
    submission: DownloadSubmission,
}

async fn tracked_title_assignment_fixture() -> TrackedTitleAssignmentFixture {
    let download_client = Arc::new(StubDownloadClient::default());
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, submissions.clone(), pending_releases);
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Assignment".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create assignment title");
    let client_id = "weaver-primary";
    let item_id = "observed-10000";
    let mut queue_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 100);
    queue_item.client_id = client_id.to_string();
    queue_item.client_name = "Weaver".to_string();
    queue_item.client_type = "weaver".to_string();
    queue_item.title_id = None;
    queue_item.facet = None;
    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some(client_id), "weaver", item_id);
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(crate::tracked_downloads::TrackedDownload {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        id: tracked_id.clone(),
        client_id: client_id.to_string(),
        client_type: "weaver".to_string(),
        client_item: queue_item,
        completed_source: None,
        state: TrackedDownloadState::ImportBlocked,
        status: scryer_domain::TrackedDownloadStatus::Warning,
        status_messages: vec!["title required".to_string()],
        title_id: None,
        facet: None,
        source_title: Some("Observed.Download".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Unmatched,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        burned_by_import_gate: false,
        snapshot_missing_since: None,
    });
    let submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: title.id.clone(),
        purpose: DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some(client_id.to_string()),
        download_client_type: "weaver".to_string(),
        download_client_item_id: item_id.to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some(title.name.clone()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::Title,
    };

    TrackedTitleAssignmentFixture {
        app,
        user,
        submissions,
        title,
        tracker,
        tracked_id,
        submission,
    }
}

#[tokio::test]
async fn assign_tracked_download_title_serializes_submission_and_runtime_assignment() {
    let mut fixture = tracked_title_assignment_fixture().await;
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();

    crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &HashSet::new(),
        fixture.tracked_id.clone(),
        fixture.title.clone(),
        fixture.submission.clone(),
        actor_snapshot,
    )
    .await
    .expect("assignment command should succeed");

    let submissions = fixture.submissions.store.lock().await;
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, fixture.submission.title_id);
    assert_eq!(
        submissions[0].download_client_item_id,
        fixture.submission.download_client_item_id
    );
    // The assignment names the requested scope and keeps the release name it
    // was given (an operator assignment is an explicit identity, recorded like
    // a grab).
    assert_eq!(submissions[0].scope, SubmissionScope::Title);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        fixture.submission.source_title.as_deref()
    );
    drop(submissions);
    let tracked = fixture
        .tracker
        .find(&fixture.tracked_id)
        .expect("tracked download remains available");
    assert_eq!(tracked.title_id.as_deref(), Some(fixture.title.id.as_str()));
    assert_eq!(tracked.facet.as_deref(), Some("movie"));
    assert_eq!(
        tracked.match_type,
        scryer_domain::TitleMatchType::Submission
    );
    // Assignment records the movie target but does not itself start an import.
    // The operator's manual-import action remains the transition out of the
    // blocked state.
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn assign_tracked_download_title_preserves_the_grab_time_release_name_and_honors_scope() {
    // A reassignment must not destroy the indexer release name the grab
    // persisted — it is still the release evidence the import parses and
    // scores — and it must record the scope the operator asked for.
    let mut fixture = tracked_title_assignment_fixture().await;
    let identity = ClientJobLocator::from_submission(&fixture.submission);
    let mut grabbed = fixture.submission.clone();
    grabbed.title_id = "some-other-title".to_string();
    grabbed.source_title = Some("Grabbed.Release.2026.1080p.WEB-DL-GRP".to_string());
    fixture
        .submissions
        .record_submission(grabbed)
        .await
        .expect("seed the original grab row");
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();
    let mut assignment = fixture.submission.clone();
    assignment.source_title = None;
    assignment.scope = SubmissionScope::Episode {
        episode_id: "ep-7".to_string(),
    };

    crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &HashSet::new(),
        fixture.tracked_id.clone(),
        fixture.title.clone(),
        assignment,
        actor_snapshot,
    )
    .await
    .expect("assignment command should succeed");

    let row = fixture
        .submissions
        .find_by_client_item_id(&identity)
        .await
        .expect("lookup")
        .expect("assignment row");
    assert_eq!(row.title_id, fixture.title.id);
    assert_eq!(
        row.source_title.as_deref(),
        Some("Grabbed.Release.2026.1080p.WEB-DL-GRP"),
        "the grab-time release name survives the reassignment"
    );
    assert_eq!(
        row.scope,
        SubmissionScope::Episode {
            episode_id: "ep-7".to_string()
        }
    );
}

#[tokio::test]
async fn assign_tracked_download_title_keeps_series_blocked_for_manual_mapping() {
    // The counterpart to the movie case above: for a series the mapping
    // decision (which episode is this file?) is real and still owed by the
    // user, so assignment must NOT push it back into auto-import.
    let mut fixture = tracked_title_assignment_fixture().await;
    fixture.title.facet = scryer_domain::MediaFacet::Series;
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();

    crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &HashSet::new(),
        fixture.tracked_id.clone(),
        fixture.title.clone(),
        fixture.submission.clone(),
        actor_snapshot,
    )
    .await
    .expect("assignment command should succeed");

    let tracked = fixture
        .tracker
        .find(&fixture.tracked_id)
        .expect("tracked download remains available");
    assert_eq!(
        tracked.match_type,
        scryer_domain::TitleMatchType::Submission
    );
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn assign_tracked_download_title_busy_rejection_persists_nothing() {
    let mut fixture = tracked_title_assignment_fixture().await;
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();
    let in_flight = HashSet::from([fixture.tracked_id.clone()]);

    let error = crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &in_flight,
        fixture.tracked_id.clone(),
        fixture.title,
        fixture.submission,
        actor_snapshot,
    )
    .await
    .expect_err("busy assignment should fail");

    assert!(matches!(error, AppError::Validation(_)));
    assert!(fixture.submissions.store.lock().await.is_empty());
    assert!(
        fixture
            .tracker
            .find(&fixture.tracked_id)
            .is_some_and(|tracked| tracked.title_id.is_none())
    );
}

#[tokio::test]
async fn assign_tracked_download_title_missing_rejection_persists_nothing() {
    let fixture = tracked_title_assignment_fixture().await;
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();
    let mut empty_tracker = crate::tracked_downloads::TrackedDownloadService::new();

    let error = crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut empty_tracker,
        &HashSet::new(),
        fixture.tracked_id,
        fixture.title,
        fixture.submission,
        actor_snapshot,
    )
    .await
    .expect_err("missing assignment should fail");

    assert!(matches!(error, AppError::NotFound(_)));
    assert!(fixture.submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn assign_tracked_download_title_persistence_failure_keeps_runtime_unchanged() {
    let mut fixture = tracked_title_assignment_fixture().await;
    *fixture.submissions.record_submission_error.lock().await =
        Some("submission store unavailable".to_string());
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();

    let error = crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &HashSet::new(),
        fixture.tracked_id.clone(),
        fixture.title,
        fixture.submission,
        actor_snapshot,
    )
    .await
    .expect_err("persistence failure should fail assignment");

    assert!(matches!(error, AppError::Repository(_)));
    assert!(fixture.submissions.store.lock().await.is_empty());
    let tracked = fixture
        .tracker
        .find(&fixture.tracked_id)
        .expect("tracked download remains available");
    assert!(tracked.title_id.is_none());
    assert_eq!(tracked.match_type, scryer_domain::TitleMatchType::Unmatched);
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn list_download_import_page_returns_promptly_when_tracked_snapshot_handle_never_replies() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    *download_client.history_items.lock().await = vec![queue_history_fixture_item(
        "pending-1",
        DownloadQueueState::ImportPending,
        40,
    )];
    let snapshot_items = download_client.history_items.lock().await.clone();
    publish_test_download_queue_snapshot(&app, snapshot_items).await;

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All),
    )
    .await
    .expect("download import page should stay responsive even when the tracked snapshot handle is wedged")
    .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "pending-1");
}

#[tokio::test]
async fn list_download_import_page_uses_runtime_tracked_snapshot_cache() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let history_item = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 40);
    *download_client.history_items.lock().await = vec![history_item.clone()];

    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some("primary"), "nzbget", "completed-1");
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: history_item.clone(),
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some("series".to_string()),
                source_title: Some("Cached Release".to_string()),
                state: TrackedDownloadState::ImportBlocked,
                status: scryer_domain::TrackedDownloadStatus::Warning,
                status_messages: vec!["moving files to nas".to_string()],
                match_type: scryer_domain::TitleMatchType::Submission,
                import_hold: None,
            },
        );
    let mut projected_item = history_item;
    projected_item.title_id = Some("title-1".to_string());
    projected_item.facet = Some("series".to_string());
    projected_item.title_name = "Cached Release".to_string();
    projected_item.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    projected_item.tracked_status = Some(scryer_domain::TrackedDownloadStatus::Warning);
    projected_item.tracked_status_messages = vec!["moving files to nas".to_string()];
    publish_test_download_queue_snapshot(&app, vec![projected_item]).await;

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All),
    )
    .await
    .expect("download import page should stay responsive with cached tracked metadata")
    .expect("download import page should load");

    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::ImportBlocked)
    );
    assert_eq!(
        page.items[0].tracked_status,
        Some(scryer_domain::TrackedDownloadStatus::Warning)
    );
    assert_eq!(
        page.items[0].tracked_status_messages,
        vec!["moving files to nas".to_string()]
    );
    assert_eq!(page.items[0].title_id.as_deref(), Some("title-1"));
}

#[tokio::test]
async fn list_download_import_page_degrades_promptly_for_limit_one_count_reads_when_snapshot_cache_is_contended()
 {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Processing);
    *download_client.history_items.lock().await = vec![importing.clone()];
    publish_test_download_queue_snapshot(&app, vec![importing]).await;

    let _snapshot_guard = app
        .runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await;

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 1, 0, DownloadImportFilter::All),
    )
    .await
    .expect("limit-one count-style download import read should degrade instead of blocking")
    .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "importing-1");
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Processing));
    assert_eq!(page.items[0].tracked_state, None);
}

#[tokio::test]
async fn download_import_page_renders_importing_state_from_runtime_snapshot() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let item_id = "blocked-worker-1";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    history_item.import_status = Some(ImportStatus::Processing);
    *download_client.history_items.lock().await = vec![history_item.clone()];

    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some("primary"), "nzbget", item_id);
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: history_item.clone(),
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some("movie".to_string()),
                source_title: Some("Fixture blocked-worker-1".to_string()),
                state: TrackedDownloadState::Importing,
                status: scryer_domain::TrackedDownloadStatus::Ok,
                status_messages: vec!["Moving files to library.".to_string()],
                match_type: scryer_domain::TitleMatchType::Submission,
                import_hold: None,
            },
        );
    history_item.tracked_state = Some(TrackedDownloadState::Importing);
    history_item.tracked_status = Some(scryer_domain::TrackedDownloadStatus::Ok);
    history_item.tracked_status_messages = vec!["Moving files to library.".to_string()];
    publish_test_download_queue_snapshot(&app, vec![history_item]).await;

    let page = app
        .list_download_import_page(&user, 1, 0, DownloadImportFilter::All)
        .await
        .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, item_id);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::Importing)
    );
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Processing));
    assert_eq!(
        crate::integration::derive_download_queue_display_state(&page.items[0]),
        DownloadDisplayState::Importing
    );
    assert_eq!(
        page.items[0].tracked_status_messages,
        vec!["Moving files to library.".to_string()]
    );
}

#[tokio::test]
async fn download_queue_poller_retries_imported_cleanup_from_facet_routing_until_delete_succeeds() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Imported Cleanup Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "imported-cleanup-1";
    let download_id = "download-id-imported-cleanup-1";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    history_item.client_id = config.id.clone();
    history_item.client_name = config.name.clone();
    history_item.title_id = Some(title.id.clone());
    history_item.title_name = title.name.clone();
    history_item.facet = Some("movie".to_string());
    history_item.download_id = Some(download_id.to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&history_item);
    *download_client.history_items.lock().await = vec![history_item];

    let submission_identity = DownloadSubmissionIdentity {
        download_id: Some(download_id.to_string()),
    };
    let submission_source_identity =
        ClientJobLocator::new(Some(config.id.as_str()), "nzbget", item_id);
    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: Some(config.id.clone()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some(title.name.clone()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            submission_identity,
            None,
        )
        .await
        .expect("seed owned download submission identity");
    download_submissions
        .update_tracked_state(
            &submission_source_identity,
            TrackedDownloadState::Imported.as_str(),
        )
        .await
        .expect("seed imported tracked state");

    download_client
        .set_delete_error(Some("repository: delete failed"))
        .await;

    let (_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(1);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                ..Default::default()
            },
        ),
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::Imported)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tracked imported item should stay visible after retryable delete failure");
    assert!(download_client.deleted_items.lock().await.is_empty());

    let mut pushed_out_history = (0..105)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("recent-history-{index}"),
                DownloadQueueState::Completed,
                1_000 - index as i64,
            );
            item.client_id = config.id.clone();
            item.client_name = config.name.clone();
            item
        })
        .collect::<Vec<_>>();
    let mut hidden_target = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 1);
    hidden_target.client_id = config.id.clone();
    hidden_target.client_name = config.name.clone();
    hidden_target.title_id = Some(title.id.clone());
    hidden_target.title_name = title.name.clone();
    hidden_target.facet = Some("movie".to_string());
    hidden_target.download_id = Some(download_id.to_string());
    pushed_out_history.push(hidden_target);
    *download_client.history_items.lock().await = pushed_out_history;

    download_client.set_delete_error(None).await;
    // A real download client's history drops an entry the moment its
    // deletion actually succeeds; this stub only records the call and never
    // mutates `history_items`, so a still-listed row keeps being re-tracked
    // and re-offered to the cleanup gate on every subsequent poll tick
    // (`process_tracked_download_snapshot`'s per-tick refresh has no notion
    // of "already deleted"). Left unmutated, the live poller below can string
    // together more than one successful delete for this id before the
    // assertions run — worse, if the test process stalls for a moment (CI
    // contention), `tokio::time::interval`'s default burst catch-up fires the
    // backlog of missed ticks back-to-back, each one rediscovering the row
    // through this same stale entry. The tracked row itself does not need
    // this listing to be reconciled — `reconcile_terminal_tracked_downloads`
    // drives off the poller's own persisted tracker cache, which keeps the
    // row (well within its 150s absence grace) regardless of what the client
    // reports this tick — so dropping it here, before the retry it unblocks
    // can run, is what makes "exactly one delete" a guarantee instead of a
    // race against scheduling.
    download_client
        .history_items
        .lock()
        .await
        .retain(|item| item.download_client_item_id != item_id);

    timeout(Duration::from_secs(5), async {
        loop {
            if !download_client.deleted_requests.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("poller should retry imported cleanup on the next cycle");

    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        // Usenet: the entry goes, the data stays the client's business.
        vec![(
            Some(config.id.clone()),
            None,
            item_id.to_string(),
            true,
            false,
        )]
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if !app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .contains_key(&tracked_id)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tracked imported item should disappear once cleanup succeeds");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_failed_snapshot_dispatches_failure_worker_without_completed_rows() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let blocklist_repo = Arc::new(MockBlocklistRepo::default());
    let app = base_app
        .with_test_overrides(|services| services.with_blocklist_repo(blocklist_repo.clone()));
    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failed External Snapshot".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");
    let item_id = "weaver-external-failed-1";
    let release_title = "Failed.External.Snapshot.2026.1080p.WEB-DL";
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some(release_title.to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed download submission");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_secs(60),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Downloading, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = release_title.to_string();
    item.facet = Some("movie".to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish initial downloading update");
    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::Downloading)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("initial external snapshot should be tracked as downloading");

    item.state = DownloadQueueState::Failed;
    item.progress_percent = 100;
    item.attention_reason = Some("download verification failed".to_string());
    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish failed-only external update");

    let expected_source_title = crate::normalize_release_name(Some(release_title));
    timeout(Duration::from_secs(5), async {
        loop {
            if blocklist_repo.entries.lock().await.iter().any(|entry| {
                entry.title_id == title.id
                    && Some(&entry.normalized_release_name) == expected_source_title.as_ref()
            }) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("failed-only external snapshot should run the failure worker");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_snapshot_uses_tracked_runtime_and_provided_completed_rows() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_secs(60),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-external-1";
    let download_id = "weaver-download-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::ImportPending)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("external snapshot should enter retryable waiting state");

    let mut completed =
        completed_download_fixture_item(item_id, "title-1", item.title_name.as_str(), "");
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = Some(download_id.to_string());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item],
            completed_downloads: vec![completed],
            actor_id: None,
        })
        .await
        .expect("publish provided completed update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::Downloading
                        && metadata.status_messages.iter().any(|message| {
                            message.contains("Completed download path is not available yet")
                        })
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("provided completed row should drive normal tracked evaluation");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_missing_history_retries_from_tracked_runtime() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Managed Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-external-retry-1";
    let download_id = "weaver-download-retry-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot_state = app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .map(|metadata| metadata.state);
            let retry_called = !download_client
                .recent_completed_download_calls
                .lock()
                .await
                .is_empty();
            if snapshot_state == Some(TrackedDownloadState::ImportPending) && retry_called {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("missing history should remain retryable and be retried centrally");

    timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = app
                .runtime
                .acquisition
                .download_queue_snapshot
                .snapshot()
                .await;
            if snapshot.items.iter().any(|item| {
                item.download_client_item_id == item_id
                    && item.tracked_state == Some(TrackedDownloadState::ImportPending)
            }) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("runtime queue cache should receive the reconciled item");

    let queue_events = app
        .services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::DownloadQueueItemUpserted]),
            title_id: None,
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 100,
        })
        .await
        .expect("queue upsert events should load");
    assert!(queue_events.is_empty());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("central retry should dispatch once exact source history appears");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);
    assert!(
        !download_client
            .recent_completed_download_calls
            .lock()
            .await
            .is_empty()
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_aged_out_history_recovers_via_widened_batch_lookup() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Aged Out Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-aged-out-1";
    let download_id = "weaver-download-aged-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    // The recent window never contains the row — it aged out of the bounded
    // listing while the item sat in the waiting state.
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::ImportPending)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("missing history should enter the retryable waiting state");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    // Age the row out of the RECENT window: pad the client's completed list so
    // the real entry sits past DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT. Only a
    // widened re-read can reach it.
    {
        let mut recent = Vec::new();
        for index in 0..crate::DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT {
            let mut filler = completed.clone();
            filler.download_client_item_id = format!("filler-{index}");
            filler.download_id = None;
            recent.push(filler);
        }
        recent.push(completed.clone());
        *download_client.recent_completed_downloads.lock().await = Some(recent);
    }

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("widened batch lookup should recover an item missing from the recent window");

    // The recovery must cost ONE widened batch read, not one client call per
    // stuck item: for clients whose API cannot address a single row (nzbget
    // has no history id filter), the per-item form was a full history download
    // each, scaling with exactly the population that grows while completions
    // are being missed.
    assert!(
        download_client
            .targeted_completed_download_calls
            .lock()
            .await
            .is_empty(),
        "retry must not issue per-item targeted lookups"
    );
    assert!(
        download_client
            .recent_completed_download_calls
            .lock()
            .await
            .contains(&crate::DOWNLOAD_QUEUE_STUCK_COMPLETED_LOOKUP_LIMIT),
        "retry should widen the batch read once to reach aged-out rows"
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn every_poll_tick_reads_recent_history_alongside_the_queue() {
    // A download client's queue only shows work IN FLIGHT, so a completion is a
    // queue-ABSENCE event — unobservable on its own. History is where
    // completions appear, and it used to be sampled on a separate, slower
    // window (30s against a 1s queue tick), leaving 29 of every 30 ticks
    // structurally unable to notice one. Anything finishing between two history
    // reads was stranded; weaver finishes small jobs in ~200ms and nzbget in
    // ~1s, so that was the common case, not the edge. Every tick must now carry
    // both reads.
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(20),
                ..Default::default()
            },
        ),
    );

    // Wait for several ticks, then require history reads to have kept pace with
    // them rather than trailing on a slower cadence of their own.
    timeout(Duration::from_secs(5), async {
        loop {
            if download_client.recent_activity_calls.lock().await.len() >= 3 {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("every tick should read recent history, not just the queue");

    let history_reads = download_client.recent_activity_calls.lock().await.len();
    let queue_reads = *download_client.queue_calls.lock().await;
    assert!(
        history_reads * 2 >= queue_reads,
        "history must ride the queue tick (queue reads: {queue_reads}, history reads: {history_reads})"
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn excluded_client_history_reconciliation_imports_missed_completion() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Reconciled Weaver Import".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "weaver-swallowed-1";
    let download_id = "weaver-download-swallowed-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;

    // The completion event was never delivered: no bridge delta is published.
    // Only the client's history knows about the item.
    *download_client.history_items.lock().await = vec![item.clone()];
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("history reconciliation should import a completion the bridge never announced");

    assert!(
        !download_client
            .recent_activity_calls
            .lock()
            .await
            .is_empty(),
        "reconciliation should list the excluded client's recent history"
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn excluded_client_history_reconciliation_skips_stale_completions() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stale Weaver Backlog".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "weaver-stale-backlog-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    // Retained history from long before this Scryer version was deployed: the
    // sweep must leave it for an explicit backfill instead of importing it.
    completed.completed_at = Some(Utc::now() - chrono::Duration::days(30));

    *download_client.history_items.lock().await = vec![item];
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    // Wait for the sweep to actually read the client's history, then give the
    // dispatch path time to act on it. The positive-path test shows an eligible
    // row dispatches within the same cycle, so a still-empty import repo here
    // means the age filter — not a missed sweep — kept it out.
    timeout(Duration::from_secs(5), async {
        loop {
            if !download_client
                .recent_activity_calls
                .lock()
                .await
                .is_empty()
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("reconciliation should list the excluded client's history");
    sleep(Duration::from_millis(500)).await;

    assert!(
        import_repo.records.lock().await.is_empty(),
        "a completion older than the reconcile window must not be auto-imported"
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn blocked_import_outcome_is_persisted_durably() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let tracked_handle = crate::tracked_downloads::TrackedDownloadHandle::new(command_tx);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_secs(60),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-observed-junk-1";
    let download_id = "ext-dl-junk-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = None;
    item.title_name = "Zxqv Unknown Show S01E01".to_string();
    item.facet = None;
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);

    let source_dir = tempfile::tempdir().expect("source tempdir");
    // A non-Scryer-origin download whose directory holds no video is
    // classified NoImportableVideo and parked at Downloading, so it never
    // reaches the manual-review block this test waits for.
    std::fs::write(source_dir.path().join("fixture.mkv"), b"video").expect("write fixture video");
    let mut completed = completed_download_fixture_item(
        item_id,
        "",
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = Some(download_id.to_string());
    completed.parameters.clear();
    let initial_revision = app
        .runtime
        .acquisition
        .download_queue_snapshot
        .snapshot()
        .await
        .revision;
    let sync_rx = app.runtime.acquisition.download_queue_snapshot.subscribe();

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item],
            completed_downloads: vec![completed],
            actor_id: None,
        })
        .await
        .expect("publish unmatched completed update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::ImportBlocked)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unmatched observed completion should block for manual review");

    timeout(Duration::from_secs(5), async {
        loop {
            let tracked_recorded = download_submissions
                .tracked_states
                .lock()
                .await
                .values()
                .any(|state| state == "import_blocked");
            let identity_recorded = download_submissions
                .identity_states
                .lock()
                .await
                .values()
                .any(|state| state == "import_blocked");
            if tracked_recorded && identity_recorded {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("blocked outcome should be persisted to submissions and identity states");

    let reads_before_visibility = (
        *download_client.queue_calls.lock().await,
        *download_client.history_calls.lock().await,
        download_client.recent_activity_calls.lock().await.len(),
    );
    let page = timeout(Duration::from_secs(5), async {
        loop {
            let page = app
                .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
                .await
                .expect("blocked import page should remain readable");
            if page.items.iter().any(|item| {
                item.download_client_item_id == item_id
                    && item.tracked_state == Some(TrackedDownloadState::ImportBlocked)
            }) {
                break page;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("blocked import should become visible from the queue cache");

    assert_eq!(page.total_count, 1);
    let blocked = page
        .items
        .iter()
        .find(|item| item.download_client_item_id == item_id)
        .expect("blocked Weaver row should be returned");
    assert_eq!(blocked.client_id, config.id.as_str());
    assert_eq!(blocked.client_type, "weaver");
    assert_eq!(
        blocked.tracked_state,
        Some(TrackedDownloadState::ImportBlocked)
    );
    assert_eq!(
        blocked.tracked_status,
        Some(scryer_domain::TrackedDownloadStatus::Warning)
    );
    assert!(!blocked.tracked_status_messages.is_empty());
    assert!(blocked.attention_required);
    assert_eq!(
        crate::integration::derive_download_queue_display_state(blocked),
        DownloadDisplayState::ImportBlocked
    );
    assert_eq!(
        app.count_download_import_items(&user, DownloadImportFilter::All)
            .await
            .expect("all import count"),
        1
    );
    assert_eq!(
        app.count_download_import_items(&user, DownloadImportFilter::Blocked)
            .await
            .expect("blocked import count"),
        1
    );
    assert_eq!(
        reads_before_visibility,
        (
            *download_client.queue_calls.lock().await,
            *download_client.history_calls.lock().await,
            download_client.recent_activity_calls.lock().await.len(),
        ),
        "cache-backed import reads must not call the download client"
    );
    let snapshot = app
        .runtime
        .acquisition
        .download_queue_snapshot
        .snapshot()
        .await;
    assert!(snapshot.revision > initial_revision);
    assert_eq!(sync_rx.borrow().revision, snapshot.revision);

    let revision_before_source_omission = snapshot.revision;
    let mut unrelated_source_item =
        queue_history_fixture_item("weaver-other-active-1", DownloadQueueState::Downloading, 1);
    unrelated_source_item.client_id = config.id.clone();
    unrelated_source_item.client_name = config.name.clone();
    unrelated_source_item.client_type = "weaver".to_string();
    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::AuthoritativeForClient {
                client_id: Some(config.id.clone()),
                client_type: "weaver".to_string(),
            },
            items: vec![unrelated_source_item],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish source snapshot without the blocked row");
    timeout(Duration::from_secs(5), async {
        loop {
            let page = app
                .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
                .await
                .expect("blocked import page after source omission");
            let revision = app
                .runtime
                .acquisition
                .download_queue_snapshot
                .snapshot()
                .await
                .revision;
            if revision > revision_before_source_omission
                && page
                    .items
                    .iter()
                    .any(|item| item.download_client_item_id == item_id)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("authoritative source omission must retain unresolved blocked rows");

    tracked_handle
        .ignore(tracked_id.clone())
        .await
        .expect("ignore blocked tracked download");
    timeout(Duration::from_secs(5), async {
        loop {
            let page = app
                .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
                .await
                .expect("blocked import page after resolution");
            if page
                .items
                .iter()
                .all(|item| item.download_client_item_id != item_id)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("resolved tracked download should leave the blocked filter");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn queued_manual_import_on_blocked_bridged_row_renders_as_pending() {
    // Prod 2026-08-18: five Shōgun manual imports queued against blocked Weaver
    // rows ran for minutes while every row still read "Import Blocked" with all
    // actions live — the operator could re-queue or cancel an import that was
    // copying. Two gaps: bridged (Weaver) rows never received import-record
    // state, and the ImportBlocked projection dropped whatever import status
    // the row had. A live (pending/running) manual import must win the display
    // over the block; a finished (failed) one must not.
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));
    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let (command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let _tracked_handle = crate::tracked_downloads::TrackedDownloadHandle::new(command_tx);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_secs(60),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-blocked-manual-queued-1";
    let download_id = "ext-dl-manual-queued-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = None;
    item.title_name = "Zxqv Unknown Show S01E01".to_string();
    item.facet = None;
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    let source_dir = tempfile::tempdir().expect("source tempdir");
    std::fs::write(source_dir.path().join("fixture.mkv"), b"video").expect("write fixture video");
    let mut completed = completed_download_fixture_item(
        item_id,
        "",
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = Some(download_id.to_string());
    completed.parameters.clear();
    let bridge_scope =
        || crate::tracked_downloads::TrackedDownloadSnapshotScope::AuthoritativeForClient {
            client_id: Some(config.id.clone()),
            client_type: "weaver".to_string(),
        };

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: vec![completed],
            actor_id: None,
        })
        .await
        .expect("publish unmatched completed update");
    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::ImportBlocked)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unmatched observed completion should block for manual review");

    // The operator queues a manual import for the blocked row.
    let source_identity = ClientJobLocator::new(Some(config.id.as_str()), "weaver", item_id);
    let manual_import_id = app
        .services
        .workflow
        .imports
        .queue_import_request(
            source_identity.clone(),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue manual import record");

    // The bridge's next authoritative snapshot must render the queued import.
    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: bridge_scope(),
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish bridge snapshot with the blocked row");
    let queued_row = timeout(Duration::from_secs(5), async {
        loop {
            let page = app
                .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
                .await
                .expect("import page while a manual import is queued");
            if let Some(row) = page.items.iter().find(|row| {
                row.download_client_item_id == item_id
                    && row.import_status == Some(ImportStatus::Pending)
            }) {
                break row.clone();
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a queued manual import must reach the bridged row");
    assert_eq!(
        crate::integration::derive_download_queue_display_state(&queued_row),
        DownloadDisplayState::ImportPending,
        "a live manual import wins the display over the block: {queued_row:?}"
    );

    // A finished import record is not live state: the row must stop rendering
    // as ImportPending (the tracker may meanwhile re-evaluate the re-published
    // completed item, so only the import-side outcome is asserted here; the
    // block-vs-status precedence itself is pinned by
    // `import_blocked_projection_keeps_a_live_manual_import_and_drops_a_finished_one`).
    app.services
        .workflow
        .imports
        .update_import_status(&manual_import_id, ImportStatus::Failed, None)
        .await
        .expect("fail the manual import record");
    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: bridge_scope(),
            items: vec![item],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish bridge snapshot after the manual import finished");
    timeout(Duration::from_secs(5), async {
        loop {
            let page = app
                .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
                .await
                .expect("import page after the manual import finished");
            if page.items.iter().any(|row| {
                row.download_client_item_id == item_id
                    && !matches!(
                        row.import_status,
                        Some(
                            ImportStatus::Pending
                                | ImportStatus::Running
                                | ImportStatus::Processing
                        )
                    )
                    && crate::integration::derive_download_queue_display_state(row)
                        != DownloadDisplayState::Importing
            }) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a finished manual import must not keep the row in Importing");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_idless_missing_history_retries_and_dispatches_without_second_delta() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Idless Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-idless-retry-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = None;
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish idless missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await;
            let waiting = snapshot.get(&tracked_id).is_some_and(|metadata| {
                metadata.state == TrackedDownloadState::ImportPending
                    && metadata
                        .status_messages
                        .iter()
                        .any(|message| message.contains("waiting for client history"))
            });
            drop(snapshot);
            let retry_called = !download_client
                .recent_completed_download_calls
                .lock()
                .await
                .is_empty();
            if waiting && retry_called {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("idless completed item should wait for recent completed history");
    assert!(import_repo.records.lock().await.is_empty());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("idless retry should revalidate and dispatch import without another delta");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_path_wait_retries_from_tracked_runtime_without_second_delta() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Path Wait Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-path-wait-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = None;
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    let missing_root = tempfile::tempdir().expect("missing root tempdir");
    let missing_path = missing_root.path().join("not-visible-yet");
    let mut missing_completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        missing_path.to_string_lossy().as_ref(),
    );
    missing_completed.client_id = config.id.clone();
    missing_completed.client_type = "weaver".to_string();
    missing_completed.download_id = None;

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: vec![missing_completed],
            actor_id: None,
        })
        .await
        .expect("publish path-wait completed update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::Downloading
                        && metadata.status_messages.iter().any(|message| {
                            message.contains("Completed download path is not available yet")
                        })
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("completed item should enter path wait");
    assert!(import_repo.records.lock().await.is_empty());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut ready_completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    ready_completed.client_id = config.id.clone();
    ready_completed.client_type = "weaver".to_string();
    ready_completed.download_id = None;
    *download_client.recent_completed_downloads.lock().await = Some(vec![ready_completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .filter(|record| record.source_ref == item_id && record.source_system == "weaver")
                .count()
                == 1
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("path-wait retry should dispatch once path appears without second delta");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_idless_bad_observed_item_blocks_after_history_retry() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
                ..Default::default()
            },
        ),
    );

    let item_id = "weaver-idless-bad-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = None;
    item.title_id = None;
    item.title_name = "Unmatched.Observed.Download.2026.1080p".to_string();
    item.facet = None;
    item.category = Some("movie".to_string());
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish unsafe idless missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::ImportPending
                        && metadata
                            .status_messages
                            .iter()
                            .any(|message| message.contains("waiting for client history"))
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unsafe idless item should wait for history first");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    // A non-Scryer-origin download whose directory holds no video is
    // classified NoImportableVideo and parked at Downloading, so it never
    // reaches the manual-review block this test waits for.
    std::fs::write(source_dir.path().join("fixture.mkv"), b"video").expect("write fixture video");
    let mut completed = completed_download_fixture_item(
        item_id,
        "",
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    // This remains an untracked observation, so it must use a configured
    // category before the ID-less safety path may expose it for manual review.
    completed.category = Some("movie".to_string());
    completed.parameters.clear();
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::ImportBlocked
                        && metadata
                            .status_messages
                            .iter()
                            .any(|message| message.contains("couldn't be matched"))
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unsafe idless item should revalidate to manual block, not auto-import");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn failed_tracked_cleanup_uses_facet_routing_and_exact_client_id() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Series NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, false, true).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failed Cleanup".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "failed-cleanup-1";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Failed, 40);
    history_item.client_id = config.id.clone();
    history_item.client_name = config.name.clone();
    history_item.title_id = Some(title.id.clone());
    history_item.title_name = title.name.clone();
    history_item.facet = Some("movie".to_string());
    let tracked = crate::tracked_downloads::TrackedDownload {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        id: crate::tracked_downloads::tracked_download_id(
            Some(config.id.as_str()),
            "nzbget",
            item_id,
        ),
        client_id: config.id.clone(),
        client_type: "nzbget".to_string(),
        client_item: history_item,
        completed_source: None,
        state: TrackedDownloadState::Failed,
        status: scryer_domain::TrackedDownloadStatus::Ok,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some(title.name.clone()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: true,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        burned_by_import_gate: false,
        snapshot_missing_since: None,
    };

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
        None,
    )
    .await;

    assert_eq!(
        outcome,
        crate::import::import::TerminalDownloadCleanupOutcome::Removed
    );
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        // Usenet: the entry goes, the data stays the client's business.
        vec![(
            Some(config.id.clone()),
            None,
            item_id.to_string(),
            true,
            false,
        )]
    );
}

#[tokio::test]
async fn import_completed_download_ignores_stale_item_id_import_when_request_identity_is_fresh() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fresh Identity".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let item_id = "10010";
    let client_id = "weaver-client";
    let now = Utc::now().to_rfc3339();
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some(client_id.to_string()),
        source_system: "weaver".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        download_id: None,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: Some(client_id.to_string()),
                download_client_type: "weaver".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Fresh.Identity.2026.1080p.WEB-DL".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            DownloadSubmissionIdentity {
                download_id: Some("scryer-download:fresh".to_string()),
            },
            None,
        )
        .await
        .expect("record fresh identity");

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Fresh.Identity.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = client_id.to_string();
    completed.client_type = "weaver".to_string();
    completed.parameters.push((
        "*scryer_download_id".to_string(),
        "scryer-download:fresh".to_string(),
    ));

    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed import should run");

    assert_ne!(result.skip_reason, Some(ImportSkipReason::AlreadyImported));
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
}

#[tokio::test]
async fn import_series_duplicate_destination_requires_catalog_for_already_imported() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let media_files = Arc::new(MockMediaFileRepo::default());
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo)
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files)
    });

    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Import Series Skip");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Import Series Skip".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season 1".into()),
            None,
            Some("1".into()),
            Some("1".into()),
        )
        .await
        .expect("create season collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("S01E01".into()),
            Some("Pilot".into()),
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create episode");

    let source_root = tempfile::tempdir().expect("source tempdir");
    let source_dir = source_root.path().join("download");
    tokio::fs::create_dir_all(&source_dir)
        .await
        .expect("create source directory");
    let release_name = "Import.Series.Skip.S01E01.1080p.WEB-DL";
    let source_path = source_dir.join(format!("{release_name}.mkv"));
    let duplicate_size = 128 * 1024 * 1024;
    std::fs::File::create(&source_path)
        .and_then(|file| file.set_len(duplicate_size))
        .expect("write source video");

    let stored_title = app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load stored title")
        .expect("stored title exists");
    let path_settings = crate::import_workflow::resolve_import_paths(&app, &stored_title)
        .await
        .expect("resolve import paths");
    let parsed = crate::parse_release_metadata(release_name);
    let dest_path = crate::import_workflow::episode_import_dest_path(
        &stored_title,
        true,
        &parsed,
        "mkv",
        &source_path,
        &title_folder,
        path_settings.rename_enabled,
        &path_settings.rename_template,
        &path_settings.season_folder_template,
        &path_settings.specials_folder_template,
        1,
        "1",
        None,
        episode.title.as_deref(),
        parsed.quality.as_deref(),
    );
    tokio::fs::create_dir_all(dest_path.parent().expect("destination parent"))
        .await
        .expect("create destination parent");
    std::fs::File::create(&dest_path)
        .and_then(|file| file.set_len(duplicate_size))
        .expect("write duplicate destination");

    let mut completed = completed_download_fixture_item(
        "series-skip-1",
        &title.id,
        release_name,
        &source_path.to_string_lossy(),
    );
    completed.client_type = "weaver".to_string();
    completed.client_id = "weaver-client".to_string();
    completed.category = Some("series".to_string());
    completed.parameters = vec![
        ("*scryer_title_id".to_string(), title.id.clone()),
        ("*scryer_facet".to_string(), "series".to_string()),
    ];

    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed series import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Skipped,
        "unexpected import result: {result:?}"
    );
    assert_eq!(
        result.episode_ids,
        vec![episode.id.clone()],
        "unexpected import result: {result:?}"
    );
    assert_eq!(result.skip_reason, Some(ImportSkipReason::DuplicateFile));
    assert!(
        result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("destination exists with identical size")),
        "expected duplicate destination message, got {:?}",
        result.error_message
    );

    let retry_result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("uncataloged duplicate retry should run");
    assert_eq!(
        retry_result.skip_reason,
        Some(ImportSkipReason::DuplicateFile),
        "uncataloged duplicate retry should not become already imported: {retry_result:?}"
    );

    app.services
        .library
        .media_files
        .insert_media_file(&crate::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: crate::stored_paths::path_to_stored_string(&dest_path),
            size_bytes: duplicate_size as i64,
            role: crate::MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert catalog row for duplicate destination");

    let mut cataloged_completed = completed.clone();
    cataloged_completed.download_client_item_id = "series-skip-2".to_string();
    let cataloged_result =
        crate::import::import::import_completed_download(&app, &user, &cataloged_completed)
            .await
            .expect("cataloged completed series import should run");

    assert_eq!(
        cataloged_result.decision,
        scryer_domain::ImportDecision::Skipped,
        "unexpected import result: {cataloged_result:?}"
    );
    assert_eq!(
        cataloged_result.episode_ids,
        vec![episode.id.clone()],
        "unexpected import result: {cataloged_result:?}"
    );
    assert_eq!(
        cataloged_result.skip_reason,
        Some(ImportSkipReason::AlreadyImported)
    );
}

/// Every video file that landed under `root`, so a test can prove that a
/// rejected pack member never reached the library.
fn library_video_file_names(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("mkv")
                && let Some(name) = path.file_name().and_then(|name| name.to_str())
            {
                names.push(name.to_string());
            }
        }
    }
    names
}

struct FailClosedPackFixture {
    app: AppUseCase,
    user: User,
    title: scryer_domain::Title,
    episode: Episode,
    library_dir: tempfile::TempDir,
    import_repo: Arc<TrackingImportRepo>,
    import_artifacts: Arc<RecordingImportArtifactRepo>,
}

/// A monitored series with exactly one catalogued episode (S01E01), wired to
/// recording import repositories so pack members can be asserted one by one.
async fn fail_closed_pack_fixture() -> FailClosedPackFixture {
    fail_closed_pack_fixture_with_submissions().await.0
}

/// [`fail_closed_pack_fixture`] plus the submission repository, for tests that
/// record the durable Scryer grab (release title + scope) behind a download.
async fn fail_closed_pack_fixture_with_submissions()
-> (FailClosedPackFixture, Arc<TrackingDownloadSubmissionRepo>) {
    build_fail_closed_pack_fixture(FailClosedPackFixtureOptions::default()).await
}

struct FailClosedPackFixtureOptions {
    file_importer: Arc<dyn FileImporter>,
    /// Register the fixture's temp library dir as the series library root
    /// (needed by upgrades, which recycle the old file under a configured
    /// root) — before the title is created, so the title's root id matches.
    series_root_at_library_dir: bool,
}

impl Default for FailClosedPackFixtureOptions {
    fn default() -> Self {
        Self {
            file_importer: Arc::new(CopyingFileImporter),
            series_root_at_library_dir: false,
        }
    }
}

async fn build_fail_closed_pack_fixture(
    options: FailClosedPackFixtureOptions,
) -> (FailClosedPackFixture, Arc<TrackingDownloadSubmissionRepo>) {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let import_artifacts = Arc::new(RecordingImportArtifactRepo::default());
    let file_importer = options.file_importer.clone();
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_import_artifacts(import_artifacts.clone())
            .with_file_importer(file_importer)
            .with_media_files(Arc::new(MockMediaFileRepo::default()))
    });
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("seed import actor");

    let library_dir = tempfile::tempdir().expect("library tempdir");
    if options.series_root_at_library_dir {
        let current_paths = app.get_library_paths(&user).await.expect("library paths");
        app.update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: current_paths.movie_path.clone(),
                series_path: library_dir.path().to_string_lossy().into_owned(),
                anime_path: Some(current_paths.anime_path.clone()),
            },
        )
        .await
        .expect("point the series library root at the fixture dir");
    }
    let title_folder = library_dir.path().join("Fail Closed Pack");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fail Closed Pack".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season 1".into()),
            None,
            Some("1".into()),
            Some("1".into()),
        )
        .await
        .expect("create season collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("S01E01".into()),
            Some("Pilot".into()),
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create episode");

    (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_repo,
            import_artifacts,
        },
        download_submissions,
    )
}

/// Sparse stand-in episode file, sized like a real episode.
///
/// It used to be 51 MiB, one megabyte over the import sample threshold — the
/// only gate a file this small has to clear, now that size scoring penalises
/// implausible smallness (`size_tiny_for_quality`) instead of refusing it. The
/// size is kept where it is so these fixtures score in the ordinary part of the
/// curve rather than at its bottom anchor, where a profile minimum could refuse
/// them for reasons the tests are not about. The file is sparse, so the byte
/// count is close to free.
const PACK_VIDEO_SIZE_BYTES: u64 = 256 * 1024 * 1024;

fn write_pack_video(dir: &Path, file_name: &str) -> std::path::PathBuf {
    let path = dir.join(file_name);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scryer-mediainfo/tests/media/h264_aac.mkv");
    std::fs::copy(fixture, &path).expect("copy source video fixture");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open source video fixture")
        .set_len(PACK_VIDEO_SIZE_BYTES)
        .expect("size source video above the import sample threshold");
    path
}

fn series_pack_completed_download(
    item_id: &str,
    title_id: &str,
    release_name: &str,
    source_dir: &Path,
) -> CompletedDownload {
    let mut completed = completed_download_fixture_item(
        item_id,
        title_id,
        release_name,
        &source_dir.to_string_lossy(),
    );
    completed.category = Some("series".to_string());
    completed.parameters = vec![
        ("*scryer_title_id".to_string(), title_id.to_string()),
        ("*scryer_facet".to_string(), "series".to_string()),
    ];
    completed
}

/// An upgrade the admission ladder refuses ("existing file is better") is a
/// fair loss, not a lie: canonical scoring disposes it as `Skip`, so the
/// release is **not** burned and the download parks as import-blocked rather
/// than failing (design D17). Only a `Blocklist` disposition — a truth verdict
/// that the release misrepresented itself — sets `release_burned`; that path
/// needs runtime media analysis and is covered at the `result_state` level.
#[tokio::test]
async fn automatic_episode_upgrade_rejection_is_not_burned() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            library_dir: _library_dir,
            ..
        },
        _submissions,
    ) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
        series_root_at_library_dir: true,
        ..Default::default()
    })
    .await;

    let initial_source = tempfile::tempdir().expect("initial source tempdir");
    write_pack_video(
        initial_source.path(),
        "Fail.Closed.Pack.S01E01.1080p.WEB-DL.mkv",
    );
    let initial = series_pack_completed_download(
        "burned-upgrade-initial",
        &title.id,
        "Fail.Closed.Pack.S01E01.1080p.WEB-DL",
        initial_source.path(),
    );
    let initial_result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &initial)
            .await
            .expect("initial completed import should run")
    };
    assert_eq!(
        initial_result.decision,
        scryer_domain::ImportDecision::Imported
    );

    let rejected_source = tempfile::tempdir().expect("rejected source tempdir");
    write_pack_video(
        rejected_source.path(),
        "Fail.Closed.Pack.S01E01.720p.WEB-DL.mkv",
    );
    let rejected = series_pack_completed_download(
        "burned-upgrade-rejected",
        &title.id,
        "Fail.Closed.Pack.S01E01.720p.WEB-DL",
        rejected_source.path(),
    );
    let result = {
        let _probe = probe_agrees_with_the_name(1280, 720);
        crate::import::import::import_completed_download(&app, &user, &rejected)
            .await
            .expect("lower-quality completed import should run")
    };

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(
        !result.release_burned,
        "a release that merely lost the upgrade comparison must not be burned: {result:?}"
    );
}

#[tokio::test]
async fn automatic_multi_file_import_prefers_rejection_over_another_import() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir: _library_dir,
            ..
        },
        _submissions,
    ) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
        series_root_at_library_dir: true,
        ..Default::default()
    })
    .await;
    create_second_pack_episode(&app, &user, &title.id, episode.collection_id.clone()).await;

    let initial_source = tempfile::tempdir().expect("initial source tempdir");
    write_pack_video(
        initial_source.path(),
        "Fail.Closed.Pack.S01E01.1080p.WEB-DL.mkv",
    );
    let initial = series_pack_completed_download(
        "mixed-burn-initial",
        &title.id,
        "Fail.Closed.Pack.S01E01.1080p.WEB-DL",
        initial_source.path(),
    );
    let initial_result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &initial)
            .await
            .expect("initial completed import should run")
    };
    assert_eq!(
        initial_result.decision,
        scryer_domain::ImportDecision::Imported
    );

    let mixed_source = tempfile::tempdir().expect("mixed source tempdir");
    write_pack_video(
        mixed_source.path(),
        "Fail.Closed.Pack.S01E01.720p.WEB-DL.mkv",
    );
    write_pack_video(
        mixed_source.path(),
        "Fail.Closed.Pack.S01E02.1080p.WEB-DL.mkv",
    );
    let mixed = series_pack_completed_download(
        "mixed-burn-release",
        &title.id,
        "Fail.Closed.Pack.S01.1080p.WEB-DL",
        mixed_source.path(),
    );
    let result = {
        let _probes = probe_sequence_agrees_with_the_names([(1280, 720), (1920, 1080)]);
        crate::import::import::import_completed_download(&app, &user, &mixed)
            .await
            .expect("mixed completed import should run")
    };

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(!result.release_burned, "{result:?}");
}

#[tokio::test]
async fn automatic_season_pack_import_rejects_member_without_a_matching_episode() {
    let FailClosedPackFixture {
        app,
        user,
        title,
        episode,
        library_dir,
        import_repo,
        import_artifacts,
    } = fail_closed_pack_fixture().await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let matched_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E01.1080p.WEB-DL.mkv",
    );
    let unmatched_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL.mkv",
    );

    let completed = series_pack_completed_download(
        "fail-closed-pack-1",
        &title.id,
        "Fail.Closed.Pack.S01.1080p.WEB-DL",
        source_dir.path(),
    );
    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("completed season pack import should run")
    };

    // The unmatched member is rejected on its own, without transfer, and
    // without any record that could later be resolved as a library file.
    assert!(
        unmatched_file.exists(),
        "rejected file must stay in the completed-download directory"
    );
    let library_files = library_video_file_names(library_dir.path());
    assert!(
        library_files.iter().all(|name| !name.contains("S02E09")),
        "unmatched episode reached the library: {library_files:?}"
    );

    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert!(
        media_files
            .iter()
            .all(|file| !file.file_path.contains("S02E09")),
        "unmatched episode was catalogued: {media_files:?}"
    );
    assert!(
        media_files.iter().all(|file| file.episode_id.is_some()),
        "unlinked media file was created: {media_files:?}"
    );

    let rejected_artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s02e09.1080p.web-dl.mkv")
        .await;
    assert_eq!(
        rejected_artifacts.len(),
        1,
        "unexpected artifacts: {rejected_artifacts:?}"
    );
    assert_eq!(rejected_artifacts[0].result, "rejected");
    assert_eq!(
        rejected_artifacts[0].reason_code.as_deref(),
        Some("episode_not_found_for_title")
    );
    assert_eq!(rejected_artifacts[0].episode_id, None);
    assert_eq!(rejected_artifacts[0].imported_media_file_id, None);

    // The safe member still imports, but the rejected member keeps the overall
    // download blocked for operator review.
    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "unexpected import result: {result:?}"
    );
    assert_eq!(result.skip_reason, Some(ImportSkipReason::PolicyMismatch));
    assert_eq!(
        result.episode_ids,
        vec![episode.id.clone()],
        "unexpected import result: {result:?}"
    );
    assert!(
        result.error_message.as_deref().is_some_and(|message| {
            message.contains("1 imported, 0 ignored, 0 skipped, 1 rejected, 0 failed")
        }),
        "expected partial pack summary, got {:?}",
        result.error_message
    );
    assert_eq!(
        library_files.len(),
        1,
        "matched episode should be the only library file: {library_files:?}"
    );
    assert!(
        library_files[0].contains("S01E01"),
        "unexpected library file: {library_files:?}"
    );
    assert_eq!(
        media_files.len(),
        1,
        "unexpected media files: {media_files:?}"
    );
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(episode.id.as_str()),
        "unexpected media files: {media_files:?}"
    );

    let imported_artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s01e01.1080p.web-dl.mkv")
        .await;
    assert_eq!(
        imported_artifacts.len(),
        1,
        "unexpected artifacts: {imported_artifacts:?}"
    );
    assert_eq!(imported_artifacts[0].result, "imported");
    assert_eq!(
        imported_artifacts[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );

    // A rejected pack member must not leave the import pending for a later
    // sweep to pick up again.
    let statuses: Vec<ImportStatus> = import_repo
        .records
        .lock()
        .await
        .iter()
        .map(|record| record.status)
        .collect();
    assert_eq!(statuses, vec![ImportStatus::Failed]);
    assert!(matched_file.exists());
}

#[tokio::test]
async fn automatic_import_rejects_download_whose_only_file_matches_no_episode() {
    let FailClosedPackFixture {
        app,
        user,
        title,
        library_dir,
        import_repo,
        import_artifacts,
        ..
    } = fail_closed_pack_fixture().await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let unmatched_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL.mkv",
    );

    let completed = series_pack_completed_download(
        "fail-closed-pack-2",
        &title.id,
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL",
        source_dir.path(),
    );
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed unmatched import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "unexpected import result: {result:?}"
    );
    assert_eq!(result.skip_reason, Some(ImportSkipReason::PolicyMismatch));
    assert!(
        result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("file resolves to no episode of this title")),
        "expected fail-closed rejection message, got {:?}",
        result.error_message
    );
    assert!(
        result.episode_ids.is_empty(),
        "unexpected result: {result:?}"
    );

    assert!(unmatched_file.exists(), "rejected file must not be moved");
    assert!(
        library_video_file_names(library_dir.path()).is_empty(),
        "no file may reach the library"
    );
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );

    let rejected_artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s02e09.1080p.web-dl.mkv")
        .await;
    assert_eq!(
        rejected_artifacts.len(),
        1,
        "unexpected artifacts: {rejected_artifacts:?}"
    );
    assert_eq!(rejected_artifacts[0].result, "rejected");
    assert_eq!(
        rejected_artifacts[0].reason_code.as_deref(),
        Some("episode_not_found_for_title")
    );

    // Terminal: the rejection is permanent, so nothing stays pending.
    let statuses: Vec<ImportStatus> = import_repo
        .records
        .lock()
        .await
        .iter()
        .map(|record| record.status)
        .collect();
    assert_eq!(statuses, vec![ImportStatus::Failed]);
}

#[tokio::test]
async fn manual_import_still_accepts_file_whose_name_matches_no_episode() {
    // Manual imports resolve the target from the operator's mapping, not from
    // the file name, so the automatic fail-closed rejection must not reach them.
    let FailClosedPackFixture {
        app,
        user,
        title,
        episode,
        ..
    } = fail_closed_pack_fixture().await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL.mkv",
    );

    let results = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-no-episode-match",
        &title.id,
        None,
        vec![ManualImportFileMapping {
            file_path: source_file.to_string_lossy().into_owned(),
            episode_id: Some(episode.id.clone()),
            series_movie_link_id: None,
        }],
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root")),
    )
    .await
    .expect("execute manual import");
    assert!(results.iter().all(|result| result.success), "{results:?}");

    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_files.len(),
        1,
        "unexpected media files: {media_files:?}"
    );
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
}

#[tokio::test]
async fn manual_import_of_a_file_already_in_place_is_satisfied_not_failed() {
    // Prod 2026-08-18: an operator re-ran a manual import whose file an earlier
    // import had already landed byte-for-byte; the mapping came back as a
    // failed import ("destination exists with identical size", code unknown),
    // so the download stayed blocked in Activity and every retry failed the
    // same way. The identical file being in place IS the import: the mapping
    // is satisfied, recorded as `already_present` like the automatic path, and
    // the manual import completes.
    let FailClosedPackFixture {
        app,
        user,
        title,
        episode,
        import_artifacts,
        ..
    } = fail_closed_pack_fixture().await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let release_name = "Fail.Closed.Pack.S01E01.1080p.WEB-DL";
    let source_file = write_pack_video(source_dir.path(), &format!("{release_name}.mkv"));
    let completed = series_pack_completed_download(
        "pack-already-in-place",
        &title.id,
        release_name,
        source_dir.path(),
    );
    let mappings = vec![ManualImportFileMapping {
        file_path: source_file.to_string_lossy().into_owned(),
        episode_id: Some(episode.id.clone()),
        series_movie_link_id: None,
    }];
    let source_root =
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root"));

    let first = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-lands-the-file",
        &title.id,
        Some(&completed),
        mappings.clone(),
        source_root.clone(),
    )
    .await
    .expect("first manual import");
    assert!(first.iter().all(|result| result.success), "{first:?}");

    // The copying importer leaves the source in place; importing the same
    // mapping again finds the identical file already at the destination.
    let second = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-already-in-place",
        &title.id,
        Some(&completed),
        mappings,
        source_root,
    )
    .await
    .expect("second manual import");
    assert_eq!(second.len(), 1, "{second:?}");
    assert!(
        second[0].success && !second[0].skipped && second[0].error_code.is_none(),
        "an identical file already in place must satisfy the mapping: {second:?}"
    );

    let artifacts = import_artifacts
        .artifacts_for_file(&format!("{release_name}.mkv"))
        .await;
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact.result == "already_present"),
        "the re-run must record the unit as already present: {artifacts:?}"
    );

    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_files.len(),
        1,
        "the re-run must not duplicate the catalog row: {media_files:?}"
    );
}

#[tokio::test]
async fn manual_import_upgrade_reports_transfer_progress_on_its_record() {
    // Prod 2026-08-18: every manual import that replaced an existing episode
    // file finished with NULL `import_transfer_*` columns — the upgrade path
    // copied through the raw importer, so the row never showed "Copying".
    // A replacement must go through the record-progress importer exactly like
    // a first import.
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            import_repo,
            ..
        },
        _submissions,
    ) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
        file_importer: Arc::new(ProgressReportingFileImporter),
        series_root_at_library_dir: true,
    })
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_root =
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root"));
    let source_identity =
        ClientJobLocator::new(Some("weaver-client"), "weaver", "pack-upgrade-progress");
    let manual_import = |release_name: &str| {
        let source_file = write_pack_video(source_dir.path(), &format!("{release_name}.mkv"));
        let completed = series_pack_completed_download(
            "pack-upgrade-progress",
            &title.id,
            release_name,
            source_dir.path(),
        );
        (
            completed,
            vec![ManualImportFileMapping {
                file_path: source_file.to_string_lossy().into_owned(),
                episode_id: Some(episode.id.clone()),
                series_movie_link_id: None,
            }],
        )
    };

    // First import lands a 720p file.
    let (completed, mappings) = manual_import("Fail.Closed.Pack.S01E01.720p.WEB-DL");
    let first_id = app
        .services
        .workflow
        .imports
        .queue_import_request(
            source_identity.clone(),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue first manual import record");
    let first = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        &first_id,
        &title.id,
        Some(&completed),
        mappings,
        source_root.clone(),
    )
    .await
    .expect("first manual import");
    assert!(first.iter().all(|result| result.success), "{first:?}");

    // Second import of a better release replaces it (the upgrade path).
    let (completed, mappings) = manual_import("Fail.Closed.Pack.S01E01.1080p.WEB-DL");
    let second_id = app
        .services
        .workflow
        .imports
        .queue_import_request(
            source_identity,
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue second manual import record");
    let second = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        &second_id,
        &title.id,
        Some(&completed),
        mappings,
        source_root,
    )
    .await
    .expect("second manual import");
    assert!(second.iter().all(|result| result.success), "{second:?}");
    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1, "{media_files:?}");
    assert!(
        media_files[0].file_path.contains("1080p"),
        "the second import must have replaced the 720p file: {media_files:?}"
    );

    let record = import_repo
        .get_import_by_id(&second_id)
        .await
        .expect("read second import record")
        .expect("second import record exists");
    assert!(
        record.import_transfer_updated_at.is_some(),
        "the upgrade copy must report transfer progress on its import record: {record:?}"
    );
    assert_eq!(
        record.import_transfer_total_bytes,
        Some(PACK_VIDEO_SIZE_BYTES as i64),
        "{record:?}"
    );
    assert_eq!(
        record.import_transfer_bytes,
        record.import_transfer_total_bytes
    );
}

#[tokio::test]
async fn scryer_manual_import_defaults_to_grabbed_scope_but_accepts_same_title_override() {
    let FailClosedPackFixture {
        app,
        user,
        title,
        episode,
        ..
    } = fail_closed_pack_fixture().await;
    let grabbed_episode = app
        .create_episode(
            &user,
            title.id.clone(),
            episode.collection_id.clone(),
            "standard".into(),
            Some("3".into()),
            Some("1".into()),
            Some("S01E03".into()),
            Some("Grabbed Episode".into()),
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create grabbed episode");
    let selected_episode = app
        .create_episode(
            &user,
            title.id.clone(),
            episode.collection_id,
            "standard".into(),
            Some("4".into()),
            Some("1".into()),
            Some("S01E04".into()),
            Some("Selected Episode".into()),
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create selected episode");
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(source_dir.path(), "Tokan — S01E03 2160p WEB-DL.mkv");
    let evidence = crate::import_workflow::ReleaseEvidence::ScryerSubmission {
        title_id: title.id.clone(),
        facet: "series".to_string(),
        source_title: Some("Fail.Closed.Pack.S01E03.1080p.WEB-DL.DDP5.1.H.264-GRP".to_string()),
        observed_release_name: None,
        release_size_bytes: None,
        purpose: DownloadSubmissionPurpose::Standard,
        scope: SubmissionScope::Episode {
            episode_id: grabbed_episode.id,
        },
    };

    let results = crate::import_workflow::execute_manual_import_with_release_evidence(
        &app,
        &user,
        "manual-import-same-title-override",
        &title.id,
        None,
        &evidence,
        vec![ManualImportFileMapping {
            file_path: source_file.to_string_lossy().into_owned(),
            episode_id: Some(selected_episode.id.clone()),
            series_movie_link_id: None,
        }],
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root")),
    )
    .await
    .expect("same-title manual override should import");
    assert!(results.iter().all(|result| result.success), "{results:?}");

    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_files.len(),
        1,
        "unexpected media files: {media_files:?}"
    );
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(selected_episode.id.as_str())
    );
    assert_eq!(media_files[0].quality_label.as_deref(), Some("1080p"));
}

// ── episode identity for multi-file downloads and season-pack members ────────
//
// Sonarr's `OtherVideoFiles` rule: the release name's numbering is applied to
// a file only when it is the download's sole video and the release names
// concrete episodes; season-pack members and files with siblings identify
// themselves, and an obfuscated one is parked for manual import.

const PACK_IDENTITY_EPISODE_RELEASE: &str = "Fail.Closed.Pack.S01E01.720p.WEB-DL.AV1.AAC2.0-GRP";
const PACK_IDENTITY_SEASON_RELEASE: &str = "Fail.Closed.Pack.S01.720p.WEB-DL.AV1.AAC2.0-GRP";

/// Record the durable Scryer grab behind `item_id` (release title + scope) so
/// the completed download imports as a `ReleaseEvidence::ScryerSubmission`.
async fn record_pack_identity_submission(
    download_submissions: &TrackingDownloadSubmissionRepo,
    title_id: &str,
    item_id: &str,
    source_title: &str,
    scope: SubmissionScope,
) {
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title_id.to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some(source_title.to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope,
        })
        .await
        .expect("record series submission");
}

async fn create_second_pack_episode(
    app: &AppUseCase,
    user: &User,
    title_id: &str,
    collection_id: Option<String>,
) -> Episode {
    app.create_episode(
        user,
        title_id.to_string(),
        collection_id,
        "standard".into(),
        Some("2".into()),
        Some("1".into()),
        Some("S01E02".into()),
        Some("Second".into()),
        None,
        Some(1_500),
        false,
        false,
    )
    .await
    .expect("create second episode")
}

async fn create_absolute_pack_episode(
    app: &AppUseCase,
    template: &Episode,
    episode_number: u32,
) -> Episode {
    let mut episode = template.clone();
    episode.id = Id::new().0;
    episode.episode_number = Some(episode_number.to_string());
    episode.episode_label = Some(format!("S01E{episode_number:02}"));
    episode.title = Some(format!("Episode {episode_number}"));
    episode.absolute_number = Some(episode_number.to_string());
    app.services
        .catalog
        .shows
        .create_episode(episode.clone())
        .await
        .expect("create absolute-numbered episode");
    episode
}

/// A catalogued episode in `season_number` that also carries a title-wide
/// absolute number. `create_episode` has no absolute-number argument, so the
/// row is written through the repository.
async fn create_absolute_numbered_pack_episode(
    app: &AppUseCase,
    user: &User,
    title_id: &str,
    template: &Episode,
    season_number: u32,
    episode_number: u32,
    absolute_number: u32,
) -> Episode {
    let collection = app
        .create_collection(
            user,
            title_id.to_string(),
            "season".into(),
            season_number.to_string(),
            Some(format!("Season {season_number}")),
            None,
            Some(episode_number.to_string()),
            Some(episode_number.to_string()),
        )
        .await
        .expect("create pack season collection");
    let mut episode = template.clone();
    episode.id = Id::new().0;
    episode.collection_id = Some(collection.id);
    episode.season_number = Some(season_number.to_string());
    episode.episode_number = Some(episode_number.to_string());
    episode.episode_label = Some(format!("S{season_number:02}E{episode_number:02}"));
    episode.title = Some(format!("Season {season_number} Episode {episode_number}"));
    episode.absolute_number = Some(absolute_number.to_string());
    app.services
        .catalog
        .shows
        .create_episode(episode.clone())
        .await
        .expect("create absolute-numbered pack episode");
    episode
}

async fn create_pack_episode_in_season(
    app: &AppUseCase,
    user: &User,
    title_id: &str,
    season_number: u32,
    episode_number: u32,
    absolute_number: Option<u32>,
    episode_type: &str,
) -> Episode {
    let collection = app
        .create_collection(
            user,
            title_id.to_string(),
            "season".into(),
            season_number.to_string(),
            Some(format!("Season {season_number}")),
            None,
            Some(season_number.to_string()),
            Some(season_number.to_string()),
        )
        .await
        .expect("create pack season collection");
    app.create_episode(
        user,
        title_id.to_string(),
        Some(collection.id),
        episode_type.into(),
        Some(episode_number.to_string()),
        Some(season_number.to_string()),
        Some(format!("S{season_number:02}E{episode_number:02}")),
        Some(format!("Season {season_number} Episode {episode_number}")),
        absolute_number.map(|number| number.to_string()),
        Some(1_500),
        false,
        false,
    )
    .await
    .expect("create pack episode")
}

fn media_file_episode_ids(files: &[crate::TitleMediaFile]) -> std::collections::BTreeSet<String> {
    files
        .iter()
        .filter_map(|file| file.episode_id.clone())
        .collect()
}

#[tokio::test]
async fn automatic_season_pack_import_identifies_each_member_by_its_own_name() {
    // Release-gate regression (`bluey-s01-pack-av1`): the pack title parsed to
    // a whole-season episode, so every member resolved to the entire season
    // and the runtime gate rejected all of them. Each member must import to
    // the episode its own name says.
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_repo,
            import_artifacts,
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let second_episode =
        create_second_pack_episode(&app, &user, &title.id, episode.collection_id.clone()).await;
    let collection_id = episode.collection_id.clone().expect("season collection");

    let item_id = "season-pack-identity-1";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        PACK_IDENTITY_SEASON_RELEASE,
        SubmissionScope::Collection { collection_id },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let season_dir = source_dir.path().join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let first_member = write_pack_video(
        &season_dir,
        "Fail.Closed.Pack.S01E01.720p.WEB-DL.AV1.AAC2.0-GRP.mkv",
    );
    let second_member = write_pack_video(
        &season_dir,
        "Fail.Closed.Pack.S01E02.720p.WEB-DL.AV1.AAC2.0-GRP.mkv",
    );
    let completed = series_pack_completed_download(
        item_id,
        &title.id,
        PACK_IDENTITY_SEASON_RELEASE,
        source_dir.path(),
    );

    let _probe = probe_sequence_agrees_with_the_names([(1280, 720), (1280, 720)]);
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed season pack import should run");

    // Neither member may be judged as the whole season: no member is skipped
    // as unparseable and no member is rejected for its runtime against a
    // season-long expectation.
    assert_ne!(
        result.skip_reason,
        Some(ImportSkipReason::UnparseableEpisode),
        "unexpected import result: {result:?}"
    );
    assert!(
        result
            .error_message
            .as_deref()
            .is_none_or(|message| !message.contains("expected about")),
        "member judged against a season-long runtime: {result:?}"
    );

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "unexpected import result: {result:?}"
    );
    assert_eq!(
        result.error_message, None,
        "every member should import cleanly: {result:?}"
    );
    assert_eq!(
        result
            .episode_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([episode.id.clone(), second_episode.id.clone()])
    );

    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_files.len(),
        2,
        "one library file per member: {media_files:?}"
    );
    assert_eq!(
        media_file_episode_ids(&media_files),
        std::collections::BTreeSet::from([episode.id.clone(), second_episode.id.clone()]),
        "each member links to exactly its own episode: {media_files:?}"
    );
    let library_files = library_video_file_names(library_dir.path());
    assert_eq!(
        library_files.len(),
        2,
        "unexpected library files: {library_files:?}"
    );
    assert!(
        library_files.iter().any(|name| name.contains("S01E01"))
            && library_files.iter().any(|name| name.contains("S01E02")),
        "unexpected library files: {library_files:?}"
    );

    for (file_name, expected_episode_id) in [
        (
            "fail.closed.pack.s01e01.720p.web-dl.av1.aac2.0-grp.mkv",
            episode.id.as_str(),
        ),
        (
            "fail.closed.pack.s01e02.720p.web-dl.av1.aac2.0-grp.mkv",
            second_episode.id.as_str(),
        ),
    ] {
        let artifacts = import_artifacts.artifacts_for_file(file_name).await;
        assert_eq!(artifacts.len(), 1, "unexpected artifacts: {artifacts:?}");
        assert_eq!(artifacts[0].result, "imported", "{artifacts:?}");
        assert_eq!(
            artifacts[0].episode_id.as_deref(),
            Some(expected_episode_id)
        );
    }
    let statuses: Vec<ImportStatus> = import_repo
        .records
        .lock()
        .await
        .iter()
        .map(|record| record.status)
        .collect();
    assert_eq!(statuses, vec![ImportStatus::Completed]);
    assert!(first_member.exists());
    assert!(second_member.exists());
}

#[tokio::test]
async fn automatic_multi_season_pack_imports_explicit_members_from_each_declared_season() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: first_episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let second_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 2, 1, Some(2), "standard").await;
    let item_id = "multi-season-explicit-members";
    let release_name = "Fail.Closed.Pack.S01-S02.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: first_episode
                .collection_id
                .clone()
                .expect("season one collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let season_one_dir = source_dir.path().join("Season 01");
    let season_two_dir = source_dir.path().join("Season 02");
    std::fs::create_dir_all(&season_one_dir).expect("create first season directory");
    std::fs::create_dir_all(&season_two_dir).expect("create second season directory");
    write_pack_video(&season_one_dir, "Fail.Closed.Pack.S01E01.720p.WEB-DL.mkv");
    write_pack_video(&season_two_dir, "Fail.Closed.Pack.S02E01.720p.WEB-DL.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());

    let result = {
        let _probe = probe_sequence_agrees_with_the_names([(1280, 720), (1280, 720)]);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("multi-season pack import should run")
    };

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "{result:?}"
    );
    assert_eq!(
        media_file_episode_ids(
            &app.services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await
                .expect("list imported media files")
        ),
        std::collections::BTreeSet::from([first_episode.id.clone(), second_episode.id.clone()])
    );
    assert_eq!(library_video_file_names(library_dir.path()).len(), 2);
    for (file_name, episode_id) in [
        ("fail.closed.pack.s01e01.720p.web-dl.mkv", &first_episode.id),
        (
            "fail.closed.pack.s02e01.720p.web-dl.mkv",
            &second_episode.id,
        ),
    ] {
        let artifacts = import_artifacts.artifacts_for_file(file_name).await;
        assert_eq!(artifacts.len(), 1, "{artifacts:?}");
        assert_eq!(artifacts[0].result, "imported", "{artifacts:?}");
        assert_eq!(
            artifacts[0].episode_id.as_deref(),
            Some(episode_id.as_str())
        );
    }
}

/// Monitoring decides what Scryer searches for, never what it keeps: like
/// Sonarr, a pack member that resolves to an unmonitored catalog episode
/// imports with the rest of the pack.
#[tokio::test]
async fn automatic_pack_imports_unmonitored_catalog_members() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: monitored_episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let unmonitored_episode = create_second_pack_episode(
        &app,
        &user,
        &title.id,
        monitored_episode.collection_id.clone(),
    )
    .await;
    app.set_episode_monitored(&user, &unmonitored_episode.id, false)
        .await
        .expect("unmonitor second pack episode");
    let item_id = "unmonitored-pack-member";
    let release_name = "Fail.Closed.Pack.S01.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: monitored_episode
                .collection_id
                .clone()
                .expect("season collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let monitored_file =
        write_pack_video(source_dir.path(), "Fail.Closed.Pack.S01E01.720p.WEB-DL.mkv");
    let unmonitored_file =
        write_pack_video(source_dir.path(), "Fail.Closed.Pack.S01E02.720p.WEB-DL.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    let result = {
        let _probe = probe_sequence_agrees_with_the_names([(1280, 720), (1280, 720)]);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("pack import should run")
    };

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "{result:?}"
    );
    assert_eq!(library_video_file_names(library_dir.path()).len(), 2);
    assert!(monitored_file.exists() && unmonitored_file.exists());
    let imported = import_artifacts
        .artifacts_for_file("fail.closed.pack.s01e02.720p.web-dl.mkv")
        .await;
    assert_eq!(imported.len(), 1, "{imported:?}");
    assert_eq!(imported[0].result, "imported", "{imported:?}");
    assert_eq!(
        imported[0].episode_id.as_deref(),
        Some(unmonitored_episode.id.as_str())
    );
    assert_eq!(
        media_file_episode_ids(
            &app.services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await
                .expect("list imported media files")
        ),
        std::collections::BTreeSet::from([
            monitored_episode.id.clone(),
            unmonitored_episode.id.clone()
        ])
    );
}

#[tokio::test]
async fn automatic_pack_ignores_recognized_large_ncop_video_without_creating_media() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let item_id = "auxiliary-pack-member";
    let release_name = "Fail.Closed.Pack.S01.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: episode.collection_id.clone().expect("season collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let auxiliary_file =
        write_pack_video(source_dir.path(), "Fail.Closed.Pack.NCOP.720p.WEB-DL.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("auxiliary-only pack import should run");

    assert!(auxiliary_file.exists(), "ignored source must stay in place");
    assert!(library_video_file_names(library_dir.path()).is_empty());
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list imported media files")
            .is_empty()
    );
    let ignored = import_artifacts
        .artifacts_for_file("fail.closed.pack.ncop.720p.web-dl.mkv")
        .await;
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    assert_eq!(ignored[0].result, "ignored", "{ignored:?}");
    assert_eq!(ignored[0].reason_code.as_deref(), Some("auxiliary_video"));
}

#[tokio::test]
async fn automatic_verified_pack_imports_monitored_special_omitted_from_episode_set() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: first_standard,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let second_standard =
        create_second_pack_episode(&app, &user, &title.id, first_standard.collection_id.clone())
            .await;
    let special =
        create_pack_episode_in_season(&app, &user, &title.id, 0, 1, None, "special").await;
    let item_id = "monitored-special-outside-episode-set";
    let release_name = "Fail.Closed.Pack.S01.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::EpisodeSet {
            episode_ids: vec![first_standard.id.clone(), second_standard.id.clone()],
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    write_pack_video(source_dir.path(), "Fail.Closed.Pack.S00E01.720p.WEB-DL.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    let result = {
        let _probe = probe_agrees_with_the_name(1280, 720);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("special pack member import should run")
    };

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "{result:?}"
    );
    assert_eq!(library_video_file_names(library_dir.path()).len(), 1);
    let artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s00e01.720p.web-dl.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "imported", "{artifacts:?}");
    assert_eq!(
        artifacts[0].episode_id.as_deref(),
        Some(special.id.as_str())
    );
}

#[tokio::test]
async fn automatic_pack_holds_nested_different_title_numeric_member() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: first_episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    create_pack_episode_in_season(&app, &user, &title.id, 2, 1, Some(2), "standard").await;
    let item_id = "nested-different-title-member";
    let release_name = "Fail.Closed.Pack.S01-S02.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: first_episode
                .collection_id
                .clone()
                .expect("season one collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let member_dir = source_dir.path().join("Different Title").join("Season 02");
    std::fs::create_dir_all(&member_dir).expect("create nested title directory");
    let source_file = write_pack_video(&member_dir, "01.720p.WEB-DL.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("mixed-title pack import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(source_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    let artifacts = import_artifacts
        .artifacts_for_file("01.720p.web-dl.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
    assert_eq!(
        artifacts[0].reason_code.as_deref(),
        Some("member_title_mismatch")
    );
}

#[tokio::test]
async fn automatic_pack_holds_bdmv_stream_payload() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let item_id = "bdmv-stream-payload";
    let release_name = "Fail.Closed.Pack.S01.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: episode.collection_id.clone().expect("season collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let stream_dir = source_dir.path().join("BDMV").join("STREAM");
    std::fs::create_dir_all(&stream_dir).expect("create BDMV stream directory");
    let source_file = write_pack_video(&stream_dir, "00001.m2ts");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("BDMV pack import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(source_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    let artifacts = import_artifacts.artifacts_for_file("00001.m2ts").await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
    assert_eq!(
        artifacts[0].reason_code.as_deref(),
        Some("disc_layout_member")
    );
}

#[tokio::test]
async fn automatic_pack_holds_unmonitored_standard_outside_declared_season() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: first_episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let outside_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 2, 1, Some(2), "standard").await;
    app.set_episode_monitored(&user, &outside_episode.id, false)
        .await
        .expect("unmonitor out-of-range catalog episode");
    let item_id = "unmonitored-outside-declared-season";
    let release_name = "Fail.Closed.Pack.S01.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: first_episode
                .collection_id
                .clone()
                .expect("season one collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file =
        write_pack_video(source_dir.path(), "Fail.Closed.Pack.S02E01.720p.WEB-DL.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("out-of-range pack import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(source_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    let artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s02e01.720p.web-dl.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
    assert_eq!(
        artifacts[0].reason_code.as_deref(),
        Some("episode_outside_declared_pack_seasons")
    );
}

#[tokio::test]
async fn automatic_pack_holds_ambiguous_season_local_and_absolute_numbering() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: _first_episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let season_two_first =
        create_pack_episode_in_season(&app, &user, &title.id, 2, 1, Some(2), "standard").await;
    let mut season_two_second = season_two_first.clone();
    season_two_second.id = Id::new().0;
    season_two_second.episode_number = Some("2".to_string());
    season_two_second.episode_label = Some("S02E02".to_string());
    season_two_second.title = Some("Season 2 Episode 2".to_string());
    season_two_second.absolute_number = Some("1".to_string());
    app.services
        .catalog
        .shows
        .create_episode(season_two_second)
        .await
        .expect("create alternate absolute-numbered episode");
    let item_id = "ambiguous-season-numbering";
    let release_name = "Fail.Closed.Pack.S02.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: season_two_first
                .collection_id
                .clone()
                .expect("season two collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let season_dir = source_dir.path().join("Season 02");
    std::fs::create_dir_all(&season_dir).expect("create season directory");
    let source_file = write_pack_video(&season_dir, "Fail.Closed.Pack - 01 720p WEB-DL.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("ambiguous pack import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(source_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    let artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack - 01 720p web-dl.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
    assert_eq!(
        artifacts[0].reason_code.as_deref(),
        Some("ambiguous_pack_numbering")
    );
}

#[tokio::test]
async fn automatic_pack_holds_every_member_with_duplicate_catalog_mapping() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let item_id = "duplicate-pack-episode-mapping";
    let release_name = "Fail.Closed.Pack.S01.720p.WEB-DL";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_name,
        SubmissionScope::Collection {
            collection_id: episode.collection_id.clone().expect("season collection"),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let first_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E01.720p.WEB-DL-A.mkv",
    );
    let second_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E01.720p.WEB-DL-B.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_name, source_dir.path());
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("duplicate pack import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(first_file.exists() && second_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    for file_name in [
        "fail.closed.pack.s01e01.720p.web-dl-a.mkv",
        "fail.closed.pack.s01e01.720p.web-dl-b.mkv",
    ] {
        let artifacts = import_artifacts.artifacts_for_file(file_name).await;
        assert_eq!(artifacts.len(), 1, "{artifacts:?}");
        assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
        assert_eq!(
            artifacts[0].reason_code.as_deref(),
            Some("duplicate_pack_episode_mapping")
        );
    }
}

#[tokio::test]
async fn automatic_import_parks_obfuscated_multi_file_episode_download_for_manual_import() {
    // Release-gate regression (`bluey-s01e01-manual-import`): a single-episode
    // release extracted to two identical obfuscated videos and S01E01 was
    // applied to both, importing whichever came first. Neither file can be
    // identified, so nothing imports and the tracked download blocks with the
    // actionable manual-import message.
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_repo,
            import_artifacts,
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;

    let item_id = "obfuscated-multi-file-1";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        PACK_IDENTITY_EPISODE_RELEASE,
        SubmissionScope::Episode {
            episode_id: episode.id.clone(),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let first_file = write_pack_video(source_dir.path(), "4f8e2c7a91b6d3e0.mkv");
    let second_file = write_pack_video(source_dir.path(), "7b2c41d8e5f609aa.mkv");
    let completed = series_pack_completed_download(
        item_id,
        &title.id,
        PACK_IDENTITY_EPISODE_RELEASE,
        source_dir.path(),
    );

    // Track the completed queue item the way the poller does (title and
    // release resolved from the recorded submission), then run the tracked
    // import step against the client history holding this completion.
    let mut client_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    client_item.title_id = Some(title.id.clone());
    client_item.title_name = title.name.clone();
    client_item.facet = Some("series".to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&client_item);
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.track(&app, client_item).await;
    let tracked = tracker
        .find_mut(&tracked_id)
        .expect("completed item is tracked");
    assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(
        tracked.match_type,
        scryer_domain::TitleMatchType::Submission,
        "{tracked:?}"
    );
    tracked.state = TrackedDownloadState::ImportPending;
    let lookup =
        crate::import::completed_download::CompletedDownloadLookup::from_recent_downloads(vec![
            completed,
        ]);

    let imported =
        crate::import::completed_download::import_with_lookup(&app, &user, tracked, &lookup).await;

    let expected_message = "Automatic import could not identify the episode for this file: this download contains 2 video files and this file's name is obfuscated. Open Manual Import and assign the correct season and episode.";
    assert!(!imported, "nothing may import: {tracked:?}");
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
    assert_eq!(
        tracked.status,
        scryer_domain::TrackedDownloadStatus::Warning
    );
    assert_eq!(tracked.status_messages, vec![expected_message.to_string()]);

    // The import itself ended skipped as unparseable — not rejected, so the
    // release is not burned and the operator can map the files.
    let records = import_repo.records.lock().await.clone();
    assert_eq!(records.len(), 1, "unexpected import records: {records:?}");
    assert_eq!(records[0].status, ImportStatus::Skipped);
    let result: scryer_domain::ImportResult = serde_json::from_str(
        records[0]
            .result_json
            .as_deref()
            .expect("skipped import records its result"),
    )
    .expect("import result json");
    assert_eq!(result.decision, scryer_domain::ImportDecision::Skipped);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::UnparseableEpisode)
    );
    assert_eq!(result.error_message.as_deref(), Some(expected_message));
    assert!(result.episode_ids.is_empty(), "{result:?}");

    // Nothing moved, nothing catalogued, nothing recorded as imported.
    assert!(first_file.exists() && second_file.exists());
    assert!(
        library_video_file_names(library_dir.path()).is_empty(),
        "no file may reach the library"
    );
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );
    for file_name in ["4f8e2c7a91b6d3e0.mkv", "7b2c41d8e5f609aa.mkv"] {
        assert!(
            import_artifacts
                .artifacts_for_file(file_name)
                .await
                .iter()
                .all(|artifact| artifact.result != "imported"),
            "{file_name} must not be recorded as imported"
        );
    }
}

#[tokio::test]
async fn automatic_import_applies_single_episode_release_to_its_sole_obfuscated_video() {
    // Guards the single-video half of the rule: with no other video files the
    // release name's numbering still identifies an obfuscated file.
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;

    let item_id = "obfuscated-single-file-1";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        PACK_IDENTITY_EPISODE_RELEASE,
        SubmissionScope::Episode {
            episode_id: episode.id.clone(),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(source_dir.path(), "4f8e2c7a91b6d3e0.mkv");
    let completed = series_pack_completed_download(
        item_id,
        &title.id,
        PACK_IDENTITY_EPISODE_RELEASE,
        source_dir.path(),
    );

    let result = {
        let _probe = probe_agrees_with_the_name(1280, 720);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("completed single-file import should run")
    };

    // Identity resolved to the grabbed episode whatever the gate did with the
    // synthetic file afterwards.
    assert_ne!(
        result.skip_reason,
        Some(ImportSkipReason::UnparseableEpisode),
        "unexpected import result: {result:?}"
    );
    assert_eq!(
        result.episode_ids,
        vec![episode.id.clone()],
        "unexpected import result: {result:?}"
    );

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "unexpected import result: {result:?}"
    );
    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_files.len(),
        1,
        "unexpected media files: {media_files:?}"
    );
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(media_files[0].quality_label.as_deref(), Some("720p"));
    let library_files = library_video_file_names(library_dir.path());
    assert_eq!(
        library_files.len(),
        1,
        "unexpected library files: {library_files:?}"
    );
    assert!(
        library_files[0].contains("S01E01"),
        "unexpected library file: {library_files:?}"
    );
    let artifacts = import_artifacts
        .artifacts_for_file("4f8e2c7a91b6d3e0.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "unexpected artifacts: {artifacts:?}");
    assert_eq!(artifacts[0].result, "imported");
    assert_eq!(
        artifacts[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert!(source_file.exists());
}

#[tokio::test]
async fn automatic_import_uses_grabbed_episode_for_absolute_numbered_release() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: first_episode,
            library_dir,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let episode = create_absolute_pack_episode(&app, &first_episode, 19).await;
    let release_title = "Fail Closed Pack - 19 (WEB 1080p x264 10-bit AAC) [A1B2C3D4]";
    let item_id = "absolute-numbered-release-19";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Episode {
            episode_id: episode.id.clone(),
        },
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(source_dir.path(), &format!("{release_title}.mkv"));
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("absolute-numbered completed import should run")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Imported);
    assert_eq!(result.episode_ids, vec![episode.id.clone()]);
    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list imported absolute-numbered file");
    assert_eq!(media_files.len(), 1, "{media_files:?}");
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(media_files[0].quality_label.as_deref(), Some("1080p"));
    let library_files = library_video_file_names(library_dir.path());
    assert_eq!(library_files.len(), 1, "{library_files:?}");
    assert!(
        library_files[0].contains("S01E19"),
        "catalog season/episode numbering must drive the destination: {library_files:?}"
    );
    assert!(source_file.exists());
}

#[tokio::test]
async fn alternate_numbered_sole_scene_file_reconciles_from_typoed_episode_title() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            library_dir,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let scoped_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 17, 42, Some(408), "standard").await;

    let release_title = "Fail.Closed.Pack.S17E42.1080p.WEB-DL.x264";
    let item_id = "alternate-numbered-sole-scene-file";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Episode {
            episode_id: scoped_episode.id.clone(),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E42.Season.17.Episod.42.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("alternate-numbered scene file should import")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Imported);
    assert_eq!(result.episode_ids, vec![scoped_episode.id.clone()]);
    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list imported file");
    assert_eq!(media_files.len(), 1, "{media_files:?}");
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(scoped_episode.id.as_str())
    );
    assert!(
        library_video_file_names(library_dir.path())
            .iter()
            .any(|file_name| file_name.contains("S17E42")),
        "catalog numbering must drive the destination"
    );
}

#[tokio::test]
async fn manual_import_preview_reconciles_alternate_numbered_typoed_episode_title() {
    let (
        FailClosedPackFixture {
            app, user, title, ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let scoped_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 17, 42, Some(408), "standard").await;
    let release_title = "Fail.Closed.Pack.S17E42.1080p.WEB-DL.x264";
    let item_id = "manual-preview-alternate-numbering";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Episode {
            episode_id: scoped_episode.id.clone(),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E42.Season.17.Episod.42.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());
    let release_evidence =
        crate::import::workflow::resolve_release_evidence_for_completed_download(
            &app, &completed, None,
        )
        .await
        .expect("resolve durable release evidence");

    let suggested_episode_ids =
        crate::import::workflow::preview_manual_import_suggested_episode_ids_for_tests(
            &app,
            source_dir.path(),
            &title,
            &release_evidence,
            std::slice::from_ref(&scoped_episode),
        )
        .await
        .expect("preview manual import");

    assert_eq!(suggested_episode_ids.len(), 1, "{suggested_episode_ids:?}");
    assert_eq!(
        suggested_episode_ids[0].as_deref(),
        Some(scoped_episode.id.as_str())
    );
}

#[tokio::test]
async fn alternate_numbered_verified_pack_member_reconciles_from_typoed_episode_title() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let scoped_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 17, 42, Some(408), "standard").await;
    let release_title = "Fail.Closed.Pack.S17.1080p.WEB-DL.x264";
    let item_id = "alternate-numbered-verified-pack-member";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Collection {
            collection_id: scoped_episode
                .collection_id
                .clone()
                .expect("season seventeen collection"),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E42.Season.17.Episod.42.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("alternate-numbered pack member should import")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Imported);
    assert_eq!(result.episode_ids, vec![scoped_episode.id.clone()]);
    let artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s01e42.season.17.episod.42.1080p.web-dl.x264.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "imported", "{artifacts:?}");
    assert_eq!(
        artifacts[0].episode_id.as_deref(),
        Some(scoped_episode.id.as_str())
    );
    assert!(
        library_video_file_names(library_dir.path())
            .iter()
            .any(|file_name| file_name.contains("S17E42")),
        "catalog numbering must drive the destination"
    );
}

/// A pack member named with a bare `E###` token parses as an inferred season 1
/// that no catalog episode answers. The pack's declared season plus the
/// catalog's absolute numbering are what identify it.
#[tokio::test]
async fn bare_episode_token_pack_member_imports_through_catalog_absolute_numbering() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let scoped_episode =
        create_absolute_numbered_pack_episode(&app, &user, &title.id, &episode, 17, 42, 408).await;
    let release_title = "Fail.Closed.Pack.S17.1080p.WEB-DL.x264";
    let item_id = "bare-episode-token-pack-member";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Collection {
            collection_id: scoped_episode
                .collection_id
                .clone()
                .expect("season seventeen collection"),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    write_pack_video(source_dir.path(), "E408.mkv");
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("bare episode-token pack member should import")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Imported);
    assert_eq!(result.episode_ids, vec![scoped_episode.id.clone()]);
    let artifacts = import_artifacts.artifacts_for_file("e408.mkv").await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "imported", "{artifacts:?}");
    assert_eq!(
        artifacts[0].episode_id.as_deref(),
        Some(scoped_episode.id.as_str())
    );
    assert!(
        library_video_file_names(library_dir.path())
            .iter()
            .any(|file_name| file_name.contains("S17E42")),
        "catalog numbering must drive the destination"
    );
}

#[tokio::test]
async fn manual_import_preview_reconciles_alternate_numbered_verified_pack_member_title() {
    let (
        FailClosedPackFixture {
            app, user, title, ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let scoped_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 17, 42, Some(408), "standard").await;
    let release_title = "Fail.Closed.Pack.S17.1080p.WEB-DL.x264";
    let item_id = "manual-preview-alternate-numbered-pack-member";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Collection {
            collection_id: scoped_episode
                .collection_id
                .clone()
                .expect("season seventeen collection"),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E42.Season.17.Episod.42.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());
    let release_evidence =
        crate::import::workflow::resolve_release_evidence_for_completed_download(
            &app, &completed, None,
        )
        .await
        .expect("resolve durable release evidence");

    let suggested_episode_ids =
        crate::import::workflow::preview_manual_import_suggested_episode_ids_for_tests(
            &app,
            source_dir.path(),
            &title,
            &release_evidence,
            std::slice::from_ref(&scoped_episode),
        )
        .await
        .expect("preview manual import");

    assert_eq!(suggested_episode_ids.len(), 1, "{suggested_episode_ids:?}");
    assert_eq!(
        suggested_episode_ids[0].as_deref(),
        Some(scoped_episode.id.as_str())
    );
}

#[tokio::test]
async fn alternate_numbered_pack_member_holds_when_scoped_episode_number_is_ambiguous() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let first_candidate =
        create_pack_episode_in_season(&app, &user, &title.id, 17, 42, Some(408), "standard").await;
    let second_candidate =
        create_pack_episode_in_season(&app, &user, &title.id, 18, 42, Some(409), "standard").await;
    let release_title = "Fail.Closed.Pack.S17-S18.1080p.WEB-DL.x264";
    let item_id = "ambiguous-alternate-numbered-pack-member";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::EpisodeSet {
            episode_ids: vec![first_candidate.id, second_candidate.id],
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E42.Season.17.Episod.42.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("ambiguous alternate-numbered pack member should run")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Rejected);
    assert!(source_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    let artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s01e42.season.17.episod.42.1080p.web-dl.x264.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
    assert_eq!(
        artifacts[0].reason_code.as_deref(),
        Some("ambiguous_pack_alternate_numbering")
    );
}

#[tokio::test]
async fn alternate_numbered_multi_episode_pack_member_does_not_use_scoped_fallback() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let scoped_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 17, 42, Some(408), "standard").await;
    let release_title = "Fail.Closed.Pack.S17.1080p.WEB-DL.x264";
    let item_id = "multi-episode-alternate-numbered-pack-member";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Collection {
            collection_id: scoped_episode
                .collection_id
                .clone()
                .expect("season seventeen collection"),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E42-E43.Season.17.Episod.42.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("multi-episode pack member should run")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Rejected);
    assert!(source_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    let artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s01e42-e43.season.17.episod.42.1080p.web-dl.x264.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
    assert_eq!(
        artifacts[0].reason_code.as_deref(),
        Some("episode_not_found_for_title")
    );
}

#[tokio::test]
async fn alternate_numbered_pack_member_with_typoed_series_title_is_held() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            library_dir,
            import_artifacts,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let scoped_episode =
        create_pack_episode_in_season(&app, &user, &title.id, 17, 42, Some(408), "standard").await;
    let release_title = "Fail.Closed.Pack.S17.1080p.WEB-DL.x264";
    let item_id = "typoed-series-title-pack-member";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Collection {
            collection_id: scoped_episode
                .collection_id
                .clone()
                .expect("season seventeen collection"),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pock.S01E42.Season.17.Episode.42.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("typoed-series-title pack member should run")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Rejected);
    assert!(source_file.exists());
    assert!(library_video_file_names(library_dir.path()).is_empty());
    let artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pock.s01e42.season.17.episode.42.1080p.web-dl.x264.mkv")
        .await;
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(artifacts[0].result, "rejected", "{artifacts:?}");
    assert_eq!(
        artifacts[0].reason_code.as_deref(),
        Some("member_title_mismatch")
    );
}

#[tokio::test]
async fn unmatched_scene_episode_number_is_not_reconciled_from_scoped_release() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: scoped_episode,
            library_dir,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let release_title = "Fail.Closed.Pack.S01E01.1080p.WEB-DL.x264";
    let item_id = "unmatched-scene-episode-number";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Episode {
            episode_id: scoped_episode.id.clone(),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E99.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("unmatched scene file should be rejected")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Rejected);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::PolicyMismatch));
    assert!(result.episode_ids.is_empty(), "{result:?}");
    assert!(
        library_video_file_names(library_dir.path()).is_empty(),
        "an unmatched file must remain outside the library"
    );
}

#[tokio::test]
async fn conflicting_sole_scene_file_is_rejected_against_grabbed_episode() {
    let (
        FailClosedPackFixture {
            app,
            user,
            title,
            episode: grabbed_episode,
            library_dir,
            ..
        },
        download_submissions,
    ) = fail_closed_pack_fixture_with_submissions().await;
    let file_episode = create_second_pack_episode(
        &app,
        &user,
        &title.id,
        grabbed_episode.collection_id.clone(),
    )
    .await;

    let release_title = "Fail.Closed.Pack.S01E01.1080p.WEB-DL.x264";
    let item_id = "single-file-conflicting-scene-name";
    record_pack_identity_submission(
        &download_submissions,
        &title.id,
        item_id,
        release_title,
        SubmissionScope::Episode {
            episode_id: grabbed_episode.id.clone(),
        },
    )
    .await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E02.1080p.WEB-DL.x264.mkv",
    );
    let completed =
        series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

    let result = {
        let _probe = probe_agrees_with_the_name(1920, 1080);
        crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("conflicting scene-name import should run")
    };

    assert_eq!(result.decision, scryer_domain::ImportDecision::Rejected);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::PolicyMismatch));
    assert_eq!(result.episode_ids, vec![file_episode.id.clone()]);
    assert!(
        result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("outside the grabbed release")),
        "unexpected rejection: {result:?}"
    );
    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list imported file");
    assert!(media_files.is_empty(), "{media_files:?}");
    assert!(
        library_video_file_names(library_dir.path()).is_empty(),
        "a contradictory file must remain outside the library"
    );
    assert!(
        source_file.exists(),
        "the blocked source must remain in place"
    );
}

#[tokio::test]
async fn invalid_exact_submission_episode_falls_back_to_artifact_parsing() {
    for wrong_title in [false, true] {
        let (
            FailClosedPackFixture {
                app,
                user,
                title,
                episode,
                ..
            },
            download_submissions,
        ) = fail_closed_pack_fixture_with_submissions().await;
        let scoped_episode_id = if wrong_title {
            let mut other_title_episode = episode.clone();
            other_title_episode.id = Id::new().0;
            other_title_episode.title_id = "other-title".to_string();
            app.services
                .catalog
                .shows
                .create_episode(other_title_episode.clone())
                .await
                .expect("create wrong-title episode");
            other_title_episode.id
        } else {
            "missing-episode".to_string()
        };

        let release_title = "Fail.Closed.Pack.S01E01.1080p.WEB-DL.x264";
        let item_id = if wrong_title {
            "wrong-title-submission-episode"
        } else {
            "missing-submission-episode"
        };
        record_pack_identity_submission(
            &download_submissions,
            &title.id,
            item_id,
            release_title,
            SubmissionScope::Episode {
                episode_id: scoped_episode_id,
            },
        )
        .await;
        let source_dir = tempfile::tempdir().expect("source tempdir");
        write_pack_video(source_dir.path(), &format!("{release_title}.mkv"));
        let completed =
            series_pack_completed_download(item_id, &title.id, release_title, source_dir.path());

        let result = crate::import::import::import_completed_download(&app, &user, &completed)
            .await
            .expect("invalid acquisition episode import should reach a decision");

        assert_eq!(
            result.skip_reason,
            Some(ImportSkipReason::PolicyMismatch),
            "the parsed episode remains subject to the grabbed-scope gate: {result:?}"
        );
        assert_eq!(
            result.episode_ids,
            vec![episode.id.clone()],
            "artifact parsing must still resolve the file: {result:?}"
        );
    }
}

/// Option c, persisted: a file that landed inside the overhead band of its
/// announced size remembers that announced size on its media-file row (the bar
/// re-derives the import score from it); a real shortfall, or a grab without an
/// announced size, remembers nothing.
#[tokio::test]
async fn an_imported_file_remembers_the_announced_size_it_was_scored_on() {
    let landed = PACK_VIDEO_SIZE_BYTES as i64;
    for (announced, remembered) in [
        // 256 MiB landed against 264 MiB announced: 97 %, inside the band.
        (Some(landed + 8 * 1024 * 1024), true),
        // Half of what was announced: a real shortfall, scored on the landed size.
        (Some(landed * 2), false),
        (None, false),
    ] {
        let (
            FailClosedPackFixture {
                app,
                user,
                title,
                episode,
                library_dir,
                ..
            },
            download_submissions,
        ) = fail_closed_pack_fixture_with_submissions().await;
        let _keep_library = &library_dir;
        let item_id = "announced-size-single-file";
        download_submissions
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "series".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some(PACK_IDENTITY_EPISODE_RELEASE.to_string()),
                info_hash: None,
                release_size_bytes: announced,
                request_signature: None,
                scope: SubmissionScope::Episode {
                    episode_id: episode.id.clone(),
                },
            })
            .await
            .expect("record series submission");

        let source_dir = tempfile::tempdir().expect("source tempdir");
        let _source_file = write_pack_video(
            source_dir.path(),
            &format!("{PACK_IDENTITY_EPISODE_RELEASE}.mkv"),
        );
        let completed = series_pack_completed_download(
            item_id,
            &title.id,
            PACK_IDENTITY_EPISODE_RELEASE,
            source_dir.path(),
        );

        let result = {
            let _probe = probe_agrees_with_the_name(1280, 720);
            crate::import::import::import_completed_download(&app, &user, &completed)
                .await
                .expect("completed import should run")
        };
        assert_eq!(
            result.decision,
            scryer_domain::ImportDecision::Imported,
            "announced={announced:?}: {result:?}"
        );
        let media_files = app
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files");
        assert_eq!(media_files.len(), 1, "{media_files:?}");
        assert_eq!(media_files[0].size_bytes, landed);
        assert_eq!(
            media_files[0].announced_size_bytes,
            if remembered { announced } else { None },
            "announced={announced:?}"
        );
    }
}

#[tokio::test]
async fn manual_import_allows_explicit_title_for_missing_download_id_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Identity Override".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        "manual-missing-identity",
        &title.id,
        "Manual.Identity.Override.2026.1080p.WEB-DL",
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = "weaver-client".to_string();
    completed.client_type = "weaver".to_string();
    completed.parameters.clear();
    completed.parameters.push((
        "*scryer_download_id".to_string(),
        "scryer-e2e-manual-missing".to_string(),
    ));

    let result = app
        .trigger_manual_import(&user, &completed, Some(&title.id))
        .await
        .expect("manual import should honor the explicit title");

    assert_eq!(result.title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
    assert_ne!(
        result.skip_reason,
        Some(ImportSkipReason::UnresolvedIdentity)
    );
    assert_eq!(import_repo.records.lock().await.len(), 1);
    let identity_state = download_submissions
        .get_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some("scryer-e2e-manual-missing".to_string()),
            },
            None,
        )
        .await
        .expect("identity state lookup");
    assert_eq!(identity_state, None);
}

#[tokio::test]
async fn completed_import_imports_additional_series_movie_file_from_submission_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let media_files = Arc::new(MockMediaFileRepo::default());
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
    });

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Additional Series Movie Import");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Series Movie Import".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let specials = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "0".into(),
            Some("Specials".into()),
            None,
            Some("0".into()),
            Some("3".into()),
        )
        .await
        .expect("create specials collection");
    let linked_episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(specials.id),
            "special".into(),
            Some("3".into()),
            Some("0".into()),
            Some("S00E03".into()),
            Some("Movie Special".into()),
            None,
            Some(6_600),
            false,
            false,
        )
        .await
        .expect("create linked special episode");

    let mut link_input = test_series_movie_link(
        &title.id,
        "Additional Series Movie Import: The Movie",
        Some(2026),
        None,
        Some("additional-series-movie-import"),
    );
    link_input.linked_episode_id = Some(linked_episode.id.clone());
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(link_input)
        .await
        .expect("create series movie link");

    let primary_file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: title_folder
                .join("Season 00")
                .join("Additional Series Movie Import - S00E03 - The Movie.mkv")
                .to_string_lossy()
                .into_owned(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert existing primary file");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&primary_file_id, &link.id)
        .await
        .expect("link primary file to series movie");

    let item_id = "additional-series-movie-import-1";
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::AdditionalFile,
            facet: "anime".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some(
                "Additional.Series.Movie.Import.The.Movie.2026.1080p.BluRay.x264-Group".to_string(),
            ),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::SeriesMovie {
                series_movie_link_id: link.id.clone(),
            },
        })
        .await
        .expect("record additional series movie submission");

    let download_dir = tempfile::tempdir().expect("download tempdir");
    let source_file = download_dir
        .path()
        .join("Additional.Series.Movie.Import.The.Movie.2026.1080p.BluRay.x264-Group.mkv");
    std::fs::File::create(&source_file)
        .expect("create source video")
        .set_len(51 * 1024 * 1024)
        .expect("size source video above sample threshold");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Additional.Series.Movie.Import.The.Movie.2026.1080p.BluRay.x264-Group",
        download_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed.clone()];

    let result = crate::import_workflow::import_completed_download(&app, &user, &completed)
        .await
        .expect("series movie additional completed download should import");
    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "{result:?}"
    );
    let import_records = import_repo.records.lock().await.clone();
    assert_eq!(import_records.len(), 1);
    assert_eq!(
        import_records[0].status,
        ImportStatus::Completed,
        "import result: {:?}",
        import_records[0].result_json
    );

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let primary_file = files
        .iter()
        .find(|file| file.id == primary_file_id)
        .expect("primary file remains");
    assert_eq!(primary_file.role, MediaFileRole::Primary);
    let additional_file = files
        .iter()
        .find(|file| file.id != primary_file_id)
        .expect("additional file imported");
    assert_eq!(additional_file.role, MediaFileRole::Additional);
    assert_eq!(
        additional_file.episode_id.as_deref(),
        Some(linked_episode.id.as_str())
    );
    assert_eq!(additional_file.series_movie_link_ids, vec![link.id]);
}

#[tokio::test]
async fn path_manual_import_can_target_series_movie_link() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
    });
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("seed manual import actor");

    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Manual Series Movie Import");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Series Movie Import".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");

    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Manual Series Movie Import: Case 3",
            Some(2026),
            None,
            Some("manual-series-movie-import-case-3"),
        ))
        .await
        .expect("create series movie link");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(
        source_dir.path(),
        "Manual.Series.Movie.Import.Case.3.2026.1080p.WEB-DL.mkv",
    );

    let results = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-series-movie",
        &title.id,
        None,
        vec![ManualImportFileMapping {
            file_path: source_file.to_string_lossy().into_owned(),
            episode_id: None,
            series_movie_link_id: Some(link.id.clone()),
        }],
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root")),
    )
    .await
    .expect("execute manual import");
    assert!(results.iter().all(|result| result.success), "{results:?}");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let imported = files
        .iter()
        .find(|file| file.series_movie_link_ids.contains(&link.id))
        .expect("manual import linked media file to series movie");
    assert_eq!(imported.role, MediaFileRole::Primary);
    assert_eq!(imported.episode_id, None);
}

#[tokio::test]
async fn path_manual_import_rejects_another_title_folder_before_source_mutation() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
    });
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("seed manual import actor");

    let owner = app
        .add_title(
            &user,
            NewTitle {
                name: "Fixture Owner".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create owner title");
    let candidate = app
        .add_title(
            &user,
            NewTitle {
                name: "Fixture Candidate".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create candidate title");
    let import_paths = crate::import_workflow::resolve_import_paths(&app, &candidate)
        .await
        .expect("resolve import paths");
    let candidate_folder = crate::effective_title_folder_path(
        &import_paths.media_root,
        &candidate,
        &import_paths.folder_template,
        None,
    );
    app.services
        .catalog
        .titles
        .set_folder_path(
            &owner.id,
            &crate::stored_paths::path_to_stored_string(&candidate_folder),
        )
        .await
        .expect("assign candidate destination to owner");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = source_dir.path().join("Fixture.Candidate.2026.1080p.mkv");
    std::fs::File::create(&source_file)
        .expect("create source video")
        .set_len(51 * 1024 * 1024)
        .expect("size source video above sample threshold");
    let source_size = std::fs::metadata(&source_file)
        .expect("source metadata")
        .len();

    let error = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-folder-conflict",
        &candidate.id,
        None,
        vec![ManualImportFileMapping {
            file_path: source_file.to_string_lossy().into_owned(),
            episode_id: None,
            series_movie_link_id: None,
        }],
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root")),
    )
    .await
    .expect_err("manual import must reject another title's folder");

    assert!(
        error
            .to_string()
            .contains("already owned by title Fixture Owner")
    );
    assert_eq!(
        std::fs::metadata(&source_file)
            .expect("source remains")
            .len(),
        source_size
    );
    assert!(!candidate_folder.exists());
    assert!(
        media_files
            .list_media_files_for_title(&candidate.id)
            .await
            .expect("list candidate media")
            .is_empty()
    );
    let refreshed_candidate = app
        .services
        .catalog
        .titles
        .get_by_id(&candidate.id)
        .await
        .expect("load candidate title")
        .expect("candidate title exists");
    assert_eq!(refreshed_candidate.folder_path, None);
}

#[tokio::test]
async fn completed_import_uses_durable_scope_over_stale_origin_parameters() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Scope Conflict".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let item_id = "origin-scope-conflict-1";
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Scope.Conflict.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed durable submission");

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "stale-title",
        "Scope.Conflict.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    *download_client.completed_downloads.lock().await = vec![completed.clone()];

    let _ = item;
    // The durable Scryer submission (title, scope) is authoritative over the
    // stale `*scryer_title_id` the client echoed back: the import runs and is
    // recorded once, in the submission's title.
    let result = crate::import_workflow::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed import should run");

    assert_eq!(result.title_id.as_deref(), Some(title.id.as_str()));
    assert!(download_client.deleted_requests.lock().await.is_empty());
    assert_eq!(import_repo.records.lock().await.len(), 1);
}

#[tokio::test]
async fn list_download_history_page_filters_terminal_rows_and_clamps_page_size_to_50() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let mut history_items = (0..120)
        .map(|index| {
            queue_history_fixture_item(
                &format!("completed-{index}"),
                DownloadQueueState::Completed,
                10_000 - index as i64,
            )
        })
        .collect::<Vec<_>>();
    history_items.extend((0..5).map(|index| {
        queue_history_fixture_item(
            &format!("failed-{index}"),
            DownloadQueueState::Failed,
            20_000 - index as i64,
        )
    }));

    let mut blocked =
        queue_history_fixture_item("blocked-import", DownloadQueueState::Completed, 30_000);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    history_items.push(blocked);

    *download_client.history_items.lock().await = history_items.clone();
    publish_test_download_queue_snapshot(&app, history_items).await;

    let failed_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::Failed]),
            None,
            false,
            None,
        )
        .await
        .expect("failed history page should load");
    assert_eq!(failed_page.total_count, 5);
    assert_eq!(failed_page.items.len(), 5);
    assert_eq!(failed_page.available_clients.len(), 1);
    assert!(
        failed_page
            .items
            .iter()
            .all(|item| item.state == DownloadQueueState::Failed)
    );
    assert!(!failed_page.has_more);

    let all_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("all history page should load");
    assert_eq!(all_page.total_count, 125);
    assert_eq!(all_page.items.len(), 50);
    assert!(all_page.has_more);
    assert_eq!(
        all_page.items[0].download_client_item_id, "failed-0",
        "newest terminal rows should be returned first"
    );
    assert!(all_page.items.iter().all(
        |item| crate::integration::derive_download_queue_display_state(item)
            != DownloadDisplayState::ImportBlocked
    ));

    let client_filtered_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::Failed]),
            Some(vec!["primary".to_string()]),
            false,
            None,
        )
        .await
        .expect("client filtered history page should load");
    assert_eq!(client_filtered_page.total_count, 5);
    assert_eq!(client_filtered_page.available_clients.len(), 1);
}

#[tokio::test]
async fn list_download_history_page_includes_tracked_terminal_rows_when_client_history_is_empty() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Tracked History Fixture".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: Some(2012),
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("title should be added");

    let mut tracked_history_item =
        queue_history_fixture_item("tracked-terminal-1", DownloadQueueState::Completed, 50);
    tracked_history_item.client_id = "primary".to_string();
    tracked_history_item.client_name = "NZBGet".to_string();
    tracked_history_item.client_type = "nzbget".to_string();
    tracked_history_item.title_id = Some(title.id.clone());
    tracked_history_item.title_name = "Paper Lantern".to_string();

    let tracked_id = crate::tracked_downloads::tracked_download_id(
        Some("primary"),
        "nzbget",
        "tracked-terminal-1",
    );
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: tracked_history_item.clone(),
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id: Some(title.id.clone()),
                facet: Some("movie".to_string()),
                source_title: Some("Paper.Lantern.2012.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
                state: TrackedDownloadState::Imported,
                status: scryer_domain::TrackedDownloadStatus::Ok,
                status_messages: Vec::new(),
                match_type: scryer_domain::TitleMatchType::Submission,
                import_hold: None,
            },
        );
    tracked_history_item.facet = Some("movie".to_string());
    tracked_history_item.title_name = "Paper.Lantern.2012.720p.WEB-DL.AV1.AAC2.0-NTb".to_string();
    tracked_history_item.tracked_state = Some(TrackedDownloadState::Imported);
    tracked_history_item.tracked_status = Some(scryer_domain::TrackedDownloadStatus::Ok);
    tracked_history_item.import_status = Some(ImportStatus::Completed);
    publish_test_download_queue_snapshot(&app, vec![tracked_history_item]).await;

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("tracked terminal history page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "tracked-terminal-1");
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::Imported)
    );
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Completed));
    assert_eq!(
        page.items[0].title_name,
        "Paper.Lantern.2012.720p.WEB-DL.AV1.AAC2.0-NTb"
    );
}

#[tokio::test]
async fn list_download_history_page_sorts_before_paginating() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let history_items = (0..60)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("sort-{index:02}"),
                DownloadQueueState::Completed,
                10_000 - index as i64,
            );
            item.title_name = format!("Title {:02}", 59 - index);
            item
        })
        .collect::<Vec<_>>();

    *download_client.history_items.lock().await = history_items.clone();
    publish_test_download_queue_snapshot(&app, history_items).await;

    let first_page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            Some(DownloadHistorySort {
                key: DownloadHistorySortKey::Title,
                direction: SortDirection::Asc,
            }),
        )
        .await
        .expect("sorted history page should load");

    let second_page = app
        .list_download_history_page(
            &user,
            50,
            50,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            Some(DownloadHistorySort {
                key: DownloadHistorySortKey::Title,
                direction: SortDirection::Asc,
            }),
        )
        .await
        .expect("second sorted history page should load");

    assert_eq!(first_page.items.len(), 50);
    assert_eq!(second_page.items.len(), 10);
    assert_eq!(first_page.items[0].title_name, "Title 00");
    assert_eq!(first_page.items[49].title_name, "Title 49");
    assert_eq!(second_page.items[0].title_name, "Title 50");
}

#[tokio::test]
async fn list_download_history_page_can_limit_to_scryer_submitted_rows() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let mut scryer_item =
        queue_history_fixture_item("scryer-item", DownloadQueueState::Completed, 100);
    scryer_item.client_id = "primary".to_string();
    scryer_item.client_name = "Primary".to_string();

    let mut external_item =
        queue_history_fixture_item("external-item", DownloadQueueState::Failed, 90);
    external_item.is_scryer_origin = false;
    external_item.client_id = "secondary".to_string();
    external_item.client_name = "Secondary".to_string();

    let history_items = vec![scryer_item, external_item];
    *download_client.history_items.lock().await = history_items.clone();
    publish_test_download_queue_snapshot(&app, history_items).await;

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            true,
            None,
        )
        .await
        .expect("scryer filtered history page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert!(page.items.iter().all(|item| item.is_scryer_origin));
    assert_eq!(page.available_clients.len(), 1);
    assert_eq!(page.available_clients[0].client_id, "primary");
}

#[tokio::test]
async fn recent_activity_and_history_ignore_operational_domain_events() {
    let (app, user) = bootstrap();

    app.add_title(
        &user,
        NewTitle {
            name: "Activity Filter Fixture".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            root_folder_id: None,
            min_availability: None,
            poster_url: None,
            year: None,
            overview: None,
            sort_title: None,
            slug: None,
            runtime_minutes: None,
            language: None,
            content_status: None,
        },
    )
    .await
    .expect("title should be added");

    app.append_domain_event(crate::domain_events::new_global_domain_event(
        None,
        DomainEventPayload::JobRunStarted(JobRunStartedEventData {
            run_id: "job-run-1".to_string(),
            job_key: "rss_sync".to_string(),
            operation_type: "job".to_string(),
            trigger_source: "system_internal".to_string(),
        }),
    ))
    .await
    .expect("operational domain event should append");

    let activities = app
        .recent_activity(&user, 10, 0)
        .await
        .expect("recent activity should load");
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].kind, ActivityKind::TitleAdded);

    let history = app
        .recent_events(&user, None, 10, 0)
        .await
        .expect("recent events should load");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].event_type, EventType::TitleAdded);

    let after_sequence = app
        .list_activity_events_after_sequence(&user, 0, 10)
        .await
        .expect("activity replay should load");
    assert_eq!(after_sequence.len(), 1);
    assert_eq!(after_sequence[0].1.kind, ActivityKind::TitleAdded);
}

#[tokio::test]
async fn download_queue_subscription_bootstraps_from_runtime_cache_without_client_reads() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "queue-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Observed Queue Item".to_string(),
        facet: Some("movie".to_string()),
        category: None,
        client_id: "primary".to_string(),
        client_name: "NZBGet".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 10,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "queue-1".to_string(),
        download_id: None,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        source_provider: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
        seeding: None,
    }];
    app.runtime
        .acquisition
        .download_queue_snapshot
        .stage_success(download_client.queue_items.lock().await.clone())
        .await;
    sleep(crate::services::DOWNLOAD_QUEUE_SNAPSHOT_COALESCE_WINDOW + Duration::from_millis(50))
        .await;

    let mut receiver = app
        .subscribe_download_queue(&user)
        .expect("queue subscription should start");
    let snapshot = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial queue snapshot should arrive")
        .expect("queue subscription should stay open");

    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].download_client_item_id, "queue-1");
    assert_eq!(snapshot[0].title_name, "Observed Queue Item");
    assert_eq!(*download_client.queue_calls.lock().await, 0);
}

#[tokio::test]
async fn download_queue_subscription_sends_empty_bootstrap_snapshot() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let mut receiver = app
        .subscribe_download_queue(&user)
        .expect("queue subscription should start");
    let snapshot = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial empty queue snapshot should arrive")
        .expect("queue subscription should stay open");

    assert!(snapshot.is_empty());
}

#[tokio::test]
async fn mark_tracked_download_failed_allows_orphaned_title_reference() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (command_tx, mut command_rx) = tokio::sync::mpsc::channel(1);
    let tracked_handle = crate::tracked_downloads::TrackedDownloadHandle::new(command_tx);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client,
        download_submissions,
        pending_releases,
        tracked_handle,
    );

    let mut item = queue_history_fixture_item(
        "orphaned-import-pending",
        DownloadQueueState::ImportPending,
        1,
    );
    item.client_id = "client-orphaned".to_string();
    item.title_id = Some("missing-title".to_string());
    publish_test_download_queue_snapshot(&app, vec![item]).await;

    let responder = tokio::spawn(async move {
        match command_rx.recv().await.expect("mark-failed command") {
            crate::tracked_downloads::TrackedDownloadCommand::MarkFailed {
                id,
                skip_reacquire,
                reply,
            } => {
                assert_eq!(
                    id,
                    crate::tracked_downloads::tracked_download_id(
                        Some("client-orphaned"),
                        "nzbget",
                        "orphaned-import-pending",
                    )
                );
                assert!(skip_reacquire);
                let _ = reply.send(Ok(()));
            }
            _ => panic!("unexpected tracked-download command"),
        }
    });

    app.mark_tracked_download_failed(
        &user,
        Some("client-orphaned"),
        "nzbget",
        "orphaned-import-pending",
        true,
    )
    .await
    .expect("orphaned queue item should remain manageable");
    responder.await.expect("mark-failed responder");
}

#[tokio::test]
async fn queued_delete_poller_executes_client_delete() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let command_id = download_queue_commands
        .seed_pending(None, "nzbget", "job-1", true)
        .await;
    let (app, _) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let token = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));

    let record = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(record) = download_queue_commands.get(&command_id).await
                && record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
            {
                break record;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delete should complete");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert_eq!(
        record.status,
        scryer_domain::DownloadQueueDeleteStatus::Completed
    );
    assert_eq!(
        download_client.deleted_items.lock().await.clone(),
        vec![("job-1".to_string(), true)]
    );
}

#[tokio::test]
async fn queue_delete_ends_bound_download_when_client_is_unavailable() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_delete_error(Some("download client unavailable"))
        .await;
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let (base_app, user) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    let locator = ClientJobLocator::new(Some("client-bound"), "nzbget", "bound-job");
    registry.bind(locator, canonical_download_id).await;
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));

    let command = app
        .delete_download_queue_item(&user, Some("client-bound"), "NZBGet", "bound-job", false)
        .await
        .expect("queue delete command");
    assert_eq!(command.canonical_download_id, Some(canonical_download_id));

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if download_queue_commands
                .get(&command.id)
                .await
                .is_some_and(|record| {
                    record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
                })
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bound delete should complete");
    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert!(
        registry
            .load_binding(&canonical_download_id)
            .await
            .expect("load binding")
            .is_some_and(|binding| binding.ended_at.is_some())
    );
    assert!(download_client.deleted_items.lock().await.is_empty());
}

#[tokio::test]
async fn legacy_queue_delete_ends_binding_that_predates_the_command() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_delete_error(Some("download client item not found"))
        .await;
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    let source_identity = ClientJobLocator::new(Some("client-legacy"), "nzbget", "legacy-job");
    let command_id = download_queue_commands
        .seed_pending(Some("client-legacy"), "nzbget", "legacy-job", false)
        .await;
    let command_created_at = fixed_time("2026-08-29T12:00:00Z");
    {
        let mut commands = download_queue_commands.queued.lock().await;
        let command = commands
            .iter_mut()
            .find(|command| command.id == command_id)
            .expect("seeded legacy delete command");
        command.created_at = command_created_at.to_rfc3339();
        command.updated_at = command.created_at.clone();
    }
    registry
        .bind_at(
            source_identity.clone(),
            canonical_download_id,
            command_created_at - chrono::Duration::minutes(2),
            Some(command_created_at - chrono::Duration::minutes(1)),
        )
        .await;
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: canonical_download_id,
            title_id: "title-legacy".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("client-legacy".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "legacy-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Fixture.Release".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record matching submission");
    let (base_app, _) =
        bootstrap_with_delete_queue(download_client, download_queue_commands.clone());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_registry(registry.clone())
            .with_download_submissions(download_submissions.clone())
    });

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if download_queue_commands
                .get(&command_id)
                .await
                .is_some_and(|record| {
                    record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
                })
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("legacy delete should complete");
    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert!(registry.ended.lock().await.contains(&canonical_download_id));
    assert_eq!(
        download_submissions
            .get_identity_tracked_state_for_download(
                Some(&canonical_download_id),
                &DownloadSubmissionIdentity::default(),
                Some(&source_identity),
            )
            .await
            .expect("load canonical ignored state")
            .as_deref(),
        Some(TrackedDownloadState::Ignored.as_str())
    );
}

#[tokio::test]
async fn completed_legacy_delete_heals_a_conflicting_reused_locator() {
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(RecordingDownloadRegistry {
        strict_conflicts: true,
        ..Default::default()
    });
    let locator = ClientJobLocator::new(Some("client-heal"), "weaver", "job-heal");
    let stale_download_id = scryer_domain::download_identity::DownloadId::new();
    let command = download_queue_commands
        .queue_delete_command(
            Some("client-heal"),
            "weaver",
            "job-heal",
            false,
            Some("admin"),
        )
        .await
        .expect("queue legacy delete");
    download_queue_commands
        .mark_delete_command_completed(&command.id)
        .await
        .expect("complete legacy delete");
    let command_created_at = fixed_time("2026-08-29T12:00:00Z");
    {
        let mut commands = download_queue_commands.queued.lock().await;
        let command = commands
            .iter_mut()
            .find(|candidate| candidate.id == command.id)
            .expect("queued legacy delete command");
        command.created_at = command_created_at.to_rfc3339();
    }
    registry
        .bind_at(
            locator.clone(),
            stale_download_id,
            command_created_at - chrono::Duration::minutes(2),
            Some(command_created_at - chrono::Duration::minutes(1)),
        )
        .await;
    let (base_app, _) = bootstrap_with_delete_queue(
        Arc::new(StubDownloadClient::default()),
        download_queue_commands,
    );
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));
    let replacement_download_id = scryer_domain::download_identity::DownloadId::new();

    let resolution = crate::download_identity::resolve_observed_client_job(
        &app,
        ObservedClientJob {
            locator,
            wire_token: Some(replacement_download_id.to_wire()),
            observed_name: Some("Replacement job".to_string()),
            observed_at: Utc::now(),
        },
    )
    .await;

    assert_eq!(
        resolution,
        crate::download_identity::ObservedClientJobResolution::Resolved(replacement_download_id)
    );
    assert!(registry.ended.lock().await.contains(&stale_download_id));
}

#[tokio::test]
async fn identity_conflict_without_completed_delete_evidence_remains_blocked() {
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(RecordingDownloadRegistry {
        strict_conflicts: true,
        ..Default::default()
    });
    let locator = ClientJobLocator::new(Some("client-missing"), "weaver", "job-missing");
    let stale_download_id = scryer_domain::download_identity::DownloadId::new();
    registry.bind(locator.clone(), stale_download_id).await;
    let (base_app, _) = bootstrap_with_delete_queue(
        Arc::new(StubDownloadClient::default()),
        download_queue_commands,
    );
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));

    let resolution = crate::download_identity::resolve_observed_client_job(
        &app,
        ObservedClientJob {
            locator,
            wire_token: Some(scryer_domain::download_identity::DownloadId::new().to_wire()),
            observed_name: Some("Unproven job".to_string()),
            observed_at: Utc::now(),
        },
    )
    .await;

    assert_eq!(
        resolution,
        crate::download_identity::ObservedClientJobResolution::Conflict
    );
    assert!(!registry.ended.lock().await.contains(&stale_download_id));
}

#[tokio::test]
async fn queue_delete_registry_miss_or_error_uses_legacy_command_and_still_executes() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let (base_app, user) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let failing_locator = ClientJobLocator::new(Some("client-error"), "nzbget", "error-job");
    registry.fail_binding_lookup(failing_locator).await;
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));

    let missed = app
        .delete_download_queue_item(&user, Some("client-miss"), "nzbget", "miss-job", false)
        .await
        .expect("queue legacy command after registry miss");
    let failed = app
        .delete_download_queue_item(&user, Some("client-error"), "nzbget", "error-job", false)
        .await
        .expect("queue legacy command after registry error");
    assert_eq!(missed.canonical_download_id, None);
    assert_eq!(failed.canonical_download_id, None);

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let missed_completed =
                download_queue_commands
                    .get(&missed.id)
                    .await
                    .is_some_and(|record| {
                        record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
                    });
            let failed_completed =
                download_queue_commands
                    .get(&failed.id)
                    .await
                    .is_some_and(|record| {
                        record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
                    });
            if missed_completed && failed_completed {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("legacy delete commands should complete");
    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    let deleted = download_client.deleted_items.lock().await.clone();
    assert!(deleted.contains(&("miss-job".to_string(), false)));
    assert!(deleted.contains(&("error-job".to_string(), false)));
}

#[tokio::test]
async fn legacy_delete_command_does_not_end_binding_created_after_command() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let command_id = download_queue_commands
        .seed_pending(Some("client-legacy"), "nzbget", "legacy-job", false)
        .await;
    let command_created_at = fixed_time("2026-08-29T12:00:00Z");
    {
        let mut commands = download_queue_commands.queued.lock().await;
        let command = commands
            .iter_mut()
            .find(|command| command.id == command_id)
            .expect("seeded legacy delete command");
        command.created_at = command_created_at.to_rfc3339();
        command.updated_at = command.created_at.clone();
    }
    let (base_app, _) =
        bootstrap_with_delete_queue(download_client, download_queue_commands.clone());
    let registry = Arc::new(RecordingDownloadRegistry::default());
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    registry
        .bind_at(
            ClientJobLocator::new(Some("client-legacy"), "nzbget", "legacy-job"),
            canonical_download_id,
            command_created_at + chrono::Duration::minutes(1),
            None,
        )
        .await;
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if download_queue_commands
                .get(&command_id)
                .await
                .is_some_and(|record| {
                    record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
                })
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("legacy delete should complete");
    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert!(
        registry
            .load_binding(&canonical_download_id)
            .await
            .expect("load binding")
            .is_some_and(|binding| binding.ended_at.is_none())
    );
}

#[tokio::test]
async fn mark_imported_command_carries_canonical_download_id() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let handle = crate::tracked_downloads::TrackedDownloadHandle::new(tx);
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    let mark = tokio::spawn(async move {
        handle
            .mark_imported_for_download(
                "tracked-mark-imported".to_string(),
                Some(canonical_download_id),
            )
            .await
    });

    let command = rx.recv().await.expect("mark-imported command");
    let crate::tracked_downloads::TrackedDownloadCommand::MarkImported {
        id,
        canonical_download_id: carried_download_id,
        reply,
    } = command
    else {
        panic!("expected a mark-imported command");
    };
    assert_eq!(id, "tracked-mark-imported");
    assert_eq!(carried_download_id, Some(canonical_download_id));
    reply.send(Ok(())).expect("reply to mark-imported command");
    mark.await
        .expect("mark-imported caller task")
        .expect("mark imported");
}

#[tokio::test]
async fn queued_delete_poller_completes_local_delete_when_client_is_unavailable() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_delete_error(Some("download client unavailable"))
        .await;
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let command_id = download_queue_commands
        .seed_pending(None, "nzbget", "job-2", false)
        .await;
    let (app, _) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let token = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));

    let record = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(record) = download_queue_commands.get(&command_id).await
                && record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
            {
                break record;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delete should complete locally");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert_eq!(
        record.status,
        scryer_domain::DownloadQueueDeleteStatus::Completed
    );
    assert_eq!(record.error_text, None);
    assert!(download_client.deleted_items.lock().await.is_empty());
}

#[tokio::test]
async fn queued_delete_removes_orphaned_import_blocked_item_locally() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_delete_error(Some("download client item not found"))
        .await;
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let command_id = download_queue_commands
        .seed_pending(Some("client-blocked"), "nzbget", "blocked-job", false)
        .await;
    let source_identity = ClientJobLocator::new(Some("client-blocked"), "nzbget", "blocked-job");
    download_submissions
        .update_tracked_state(
            &source_identity,
            scryer_domain::TrackedDownloadState::ImportBlocked.as_str(),
        )
        .await
        .expect("seed import-blocked tracked state");

    let (tracked_tx, mut tracked_rx) = tokio::sync::mpsc::channel(1);
    let tracked_handle = crate::tracked_downloads::TrackedDownloadHandle::new(tracked_tx);
    let (base_app, _) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_submissions(download_submissions.clone())
            .with_tracked_download_handle(tracked_handle)
    });
    let mut queue_item =
        queue_history_fixture_item("blocked-job", DownloadQueueState::ImportPending, 1);
    queue_item.client_id = "client-blocked".to_string();
    queue_item.client_type = "nzbget".to_string();
    queue_item.tracked_state = Some(scryer_domain::TrackedDownloadState::ImportBlocked);
    publish_test_download_queue_snapshot(&app, vec![queue_item]).await;

    let tracked_responder = tokio::spawn(async move {
        match tracked_rx.recv().await.expect("forget command") {
            crate::tracked_downloads::TrackedDownloadCommand::Forget { id, reply } => {
                assert_eq!(
                    id,
                    crate::tracked_downloads::tracked_download_id(
                        Some("client-blocked"),
                        "nzbget",
                        "blocked-job",
                    )
                );
                let _ = reply.send(Ok(()));
            }
            _ => panic!("unexpected tracked-download command"),
        }
    });
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(start_background_download_delete_poller(
        app.clone(),
        token.child_token(),
    ));

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let completed = download_queue_commands
                .get(&command_id)
                .await
                .is_some_and(|record| {
                    record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
                });
            let removed_from_snapshot = app
                .runtime
                .acquisition
                .download_queue_snapshot
                .snapshot()
                .await
                .items
                .iter()
                .all(|item| item.download_client_item_id != "blocked-job");
            if completed && removed_from_snapshot {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("orphaned import-blocked delete should complete locally");

    token.cancel();
    poller.await.expect("delete poller should stop cleanly");
    tracked_responder
        .await
        .expect("tracked responder should stop cleanly");
    assert_eq!(
        download_submissions
            .get_tracked_state(&source_identity)
            .await
            .expect("load tracked state"),
        None
    );
    assert!(download_client.deleted_items.lock().await.is_empty());
}

#[tokio::test]
async fn ignore_tracked_download_uses_durable_fallback_idempotently() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Durable Ignore Fallback".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let source_identity = ClientJobLocator::new(None, "nzbget", "evicted-job-1");
    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id,
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "evicted-job-1".to_string(),
                source_hint: Some("https://indexer.example/get?apikey=secret".to_string()),
                source_provider_id: Some("indexer-1".to_string()),
                source_provider_name: Some("Fixture Indexer".to_string()),
                source_kind: None,
                source_title: Some("Durable.Ignore.2026.1080p.WEB-DL".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            DownloadSubmissionIdentity {
                download_id: Some("scryer-download:evicted-job-1".to_string()),
            },
            None,
        )
        .await
        .expect("record submission identity");

    assert!(matches!(
        crate::integration::workflow::finalize_scryer_download_ignored(
            &app,
            crate::domain_events::DomainEventActor::from(&user),
            source_identity.clone(),
        )
        .await
        .expect("durable fallback should ignore the evicted item"),
        crate::integration::workflow::FinalizeIgnoredOutcome::Finalized
    ));
    assert!(matches!(
        crate::integration::workflow::finalize_scryer_download_ignored(
            &app,
            crate::domain_events::DomainEventActor::from(&user),
            source_identity.clone(),
        )
        .await
        .expect("second ignore should be idempotent"),
        crate::integration::workflow::FinalizeIgnoredOutcome::Finalized
    ));

    let states = download_submissions.identity_states.lock().await;
    assert_eq!(states.len(), 1);
    assert!(
        states
            .values()
            .all(|state| state == TrackedDownloadState::Ignored.as_str())
    );
    drop(states);
    assert!(
        download_submissions
            .get_identity_tracked_state(
                &DownloadSubmissionIdentity {
                    download_id: Some("scryer-download:evicted-job-1".to_string()),
                },
                Some(&source_identity),
            )
            .await
            .expect("load durable state")
            .is_some_and(|state| state == TrackedDownloadState::Ignored.as_str())
    );
}

#[tokio::test]
async fn finalize_ignore_preserves_an_imported_outcome() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Imported Then Deleted".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let source_identity = ClientJobLocator::new(None, "nzbget", "done-job-1");
    let identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:done-job-1".to_string()),
    };
    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id,
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "done-job-1".to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: Some("Fixture Indexer".to_string()),
                source_kind: None,
                source_title: Some("Imported.Then.Deleted.2026.1080p.WEB-DL".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            identity.clone(),
            None,
        )
        .await
        .expect("record submission identity");
    download_submissions
        .record_identity_tracked_state(
            &identity,
            Some(&source_identity),
            TrackedDownloadState::Imported.as_str(),
            None,
            None,
        )
        .await
        .expect("record imported outcome");

    // Deleting the client's history entry afterwards is cleanup, not a change
    // of outcome: the imported state must survive and no ignore may be
    // recorded.
    assert!(matches!(
        crate::integration::workflow::finalize_scryer_download_ignored(
            &app,
            crate::domain_events::DomainEventActor::from(&user),
            source_identity.clone(),
        )
        .await
        .expect("finalize should preserve the imported outcome"),
        crate::integration::workflow::FinalizeIgnoredOutcome::PreservedTerminal(state)
            if state == TrackedDownloadState::Imported.as_str()
    ));
    assert!(
        download_submissions
            .get_identity_tracked_state(&identity, Some(&source_identity))
            .await
            .expect("load durable state")
            .is_some_and(|state| state == TrackedDownloadState::Imported.as_str())
    );
}

#[tokio::test]
async fn active_library_scans_and_subscription_use_runtime_tracker_state() {
    let (app, user) = bootstrap();

    let session = app
        .runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "scan-runtime-1".to_string(),
            MediaFacet::Movie,
            LibraryScanMode::Full,
        )
        .await
        .expect("library scan session should start");

    let active = app
        .active_library_scans(&user)
        .await
        .expect("active scans should load");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].session_id, session.session_id);

    let mut receiver = app
        .subscribe_library_scan_progress(&user)
        .await
        .expect("library scan subscription should start");
    let initial = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial library scan snapshot should arrive")
        .expect("library scan subscription should stay open");

    assert_eq!(initial.session_id, session.session_id);
    assert_eq!(initial.facet, session.facet);
}

// ── decide_import: one gate, three dispositions (D17) ────────────────────────
//
// These drive the *real* completed-download path. Until the probe override
// existed the default test build could only produce a `Consistent` verdict —
// `probe_and_validate`'s non-mediainfo body synthesizes its analysis from the
// release name, so the file always agreed with it — and every consequence of a
// verdict (blocklist rows, reopened scopes, which disposition fires) was
// asserted only against hand-built values. See `post_download_gate::probe_override`.

/// Everything the four disposition tests share: a movie title with a folder, a
/// download client, a `Standard` submission, a grabbed scope row, and a source
/// file above the sample threshold.
struct DispositionFixture {
    app: AppUseCase,
    user: User,
    title: scryer_domain::Title,
    title_folder: PathBuf,
    completed: CompletedDownload,
    blocklist_repo: Arc<MockBlocklistRepo>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    import_repo: Arc<TrackingImportRepo>,
    scope_id: String,
    _library_dir: tempfile::TempDir,
    _download_dir: tempfile::TempDir,
}

impl DispositionFixture {
    async fn scope_status(&self) -> AcquisitionScopeStatus {
        self.app
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&self.scope_id)
            .await
            .expect("scope lookup")
            .expect("the seeded scope row survives the import")
            .status
    }

    async fn blocklisted_titles(&self) -> Vec<String> {
        self.blocklist_repo
            .entries
            .lock()
            .await
            .iter()
            .map(|entry| entry.normalized_release_name.clone())
            .collect()
    }

    async fn set_submission_purpose(&self, purpose: crate::DownloadSubmissionPurpose) {
        let mut submissions = self.download_submissions.store.lock().await;
        let submission = submissions
            .iter_mut()
            .find(|submission| {
                submission.download_client_item_id == self.completed.download_client_item_id
            })
            .expect("fixture submission exists");
        submission.purpose = purpose;
    }

    fn tracked_import_pending(&self) -> crate::tracked_downloads::TrackedDownload {
        let mut client_item = queue_history_fixture_item(
            &self.completed.download_client_item_id,
            DownloadQueueState::Completed,
            300 * 1024 * 1024,
        );
        client_item.client_id = self.completed.client_id.clone();
        client_item.client_type = self.completed.client_type.clone();
        client_item.client_name = "Primary NZBGet".to_string();
        client_item.title_id = Some(self.title.id.clone());
        client_item.title_name = self.title.name.clone();
        client_item.facet = Some("movie".to_string());

        crate::tracked_downloads::TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: crate::tracked_downloads::tracked_download_id(
                Some(self.completed.client_id.as_str()),
                &self.completed.client_type,
                &self.completed.download_client_item_id,
            ),
            client_id: self.completed.client_id.clone(),
            client_type: self.completed.client_type.clone(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::ImportPending,
            status: scryer_domain::TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some(self.title.id.clone()),
            facet: Some("movie".to_string()),
            source_title: Some(self.completed.name.clone()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: scryer_domain::TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
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

    async fn latest_import_result(&self) -> crate::ImportResult {
        let records = self.import_repo.records.lock().await;
        let result_json = records
            .iter()
            .rev()
            .find_map(|record| record.result_json.as_deref())
            .expect("completed import stores a result");
        serde_json::from_str(result_json).expect("stored import result deserializes")
    }
}

async fn disposition_fixture(name: &str, release_title: &str) -> DispositionFixture {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let media_files = Arc::new(MockMediaFileRepo::default());
    let blocklist_repo = Arc::new(MockBlocklistRepo::default());
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files)
            .with_blocklist_repo(blocklist_repo.clone())
    });

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join(name);
    // The upgrade path refuses to recycle a file outside the library's
    // configured roots, so the fixture's tempdir has to *be* the root.
    let movie_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Movie)
        .await
        .expect("library lookup")
        .expect("movie library exists");
    app.services
        .catalog
        .libraries
        .update(
            &movie_library.id,
            movie_library.name.clone(),
            movie_library.slug.clone(),
            vec![LibraryRootDraft {
                path: library_dir.path().to_string_lossy().into_owned(),
                is_default: true,
            }],
        )
        .await
        .expect("point the movie library at the fixture root");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: name.to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2026),
                runtime_minutes: Some(40),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");

    let scope_id = Id::new().0;
    app.services
        .workflow
        .acquisition_scope_states
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: scope_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_facet: Some(MediaFacet::Movie.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            media_type: "movie".to_string(),
            // Grabbed, because that is where a completed download leaves it.
            // A reopen puts it back to `wanted`; a skip or a hold must not.
            status: AcquisitionScopeStatus::Grabbed,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            title_slug: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            last_search_at: None,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
        })
        .await
        .expect("seed grabbed scope row");

    let item_id = format!("{}-1", name.to_ascii_lowercase().replace(' ', "-"));
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.clone(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some(release_title.to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");

    let download_dir = tempfile::tempdir().expect("download tempdir");
    let source_file = download_dir.path().join(format!("{release_title}.mkv"));
    std::fs::File::create(&source_file)
        .expect("create source video")
        // Above the sample threshold and inside a plausible band for a 40-minute
        // runtime, and small enough that hashing and copying it stay cheap.
        .set_len(300 * 1024 * 1024)
        .expect("size source video into a plausible band");
    let mut completed = completed_download_fixture_item(
        &item_id,
        &title.id,
        release_title,
        download_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed.clone()];

    DispositionFixture {
        app,
        user,
        title,
        title_folder,
        completed,
        blocklist_repo,
        download_submissions,
        import_repo,
        scope_id,
        _library_dir: library_dir,
        _download_dir: download_dir,
    }
}

async fn seed_primary_movie_file(
    fixture: &DispositionFixture,
    file_name: &str,
    quality_label: &str,
) -> String {
    // On disk as well as in the row: the upgrade path recycles the real file,
    // and refuses outright when its configured root looks empty.
    std::fs::create_dir_all(&fixture.title_folder).expect("create title folder");
    std::fs::File::create(fixture.title_folder.join(file_name))
        .expect("create incumbent file")
        .set_len(300 * 1024 * 1024)
        .expect("size incumbent file");
    fixture
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: fixture
                .title_folder
                .join(file_name)
                .to_string_lossy()
                .into_owned(),
            size_bytes: 300 * 1024 * 1024,
            role: MediaFileRole::Primary,
            quality_label: Some(quality_label.to_string()),
            resolution: Some(quality_label.to_string()),
            scene_name: Some(file_name.trim_end_matches(".mkv").to_string()),
            ..Default::default()
        })
        .await
        .expect("insert incumbent primary file")
}

/// A probe that simply agrees with the release name.
///
/// The feature-off build synthesizes exactly this from the parsed quality, so a
/// test that wants "no contradiction" used to need no hook at all. It needs one
/// now: a workspace build unifies `runtime-media-analysis` on (see
/// `post_download_gate::probe_override`), and the real probe rejects a sparse
/// fixture for having no readable duration. Installing the agreement explicitly
/// makes the same test mean the same thing in both builds.
fn probe_agreement(width: i32, height: i32) -> crate::post_download_gate::ImportedFileAcceptance {
    let mut analysis = crate::post_download_gate::build_stream_pointer_media_file_analysis();
    analysis.video_codec = crate::release_parser::VideoCodec::parse("h264");
    analysis.video_width = Some(width);
    analysis.video_height = Some(height);
    crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(analysis),
        scan_error: None,
        rule_file_doc: None,
        audio_language_warning: None,
    }
}

fn probe_agrees_with_the_name(
    width: i32,
    height: i32,
) -> crate::post_download_gate::probe_override::ProbeOverrideGuard {
    crate::post_download_gate::probe_override::install(probe_agreement(width, height))
}

fn probe_sequence_agrees_with_the_names(
    dimensions: impl IntoIterator<Item = (i32, i32)>,
) -> crate::post_download_gate::probe_override::ProbeOverrideGuard {
    crate::post_download_gate::probe_override::install_sequence(
        dimensions
            .into_iter()
            .map(|(width, height)| probe_agreement(width, height)),
    )
}

async fn primary_movie_files(fixture: &DispositionFixture) -> Vec<crate::TitleMediaFile> {
    fixture
        .app
        .services
        .library
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .expect("list media files")
        .into_iter()
        .filter(|file| file.role.is_primary())
        .collect()
}

/// **`Blocklist`.** The release advertised 1080p and the file measures 720p, in
/// a profile that ranks 1080P above 720P and a scope that already holds a 1080p
/// file. The release lied, so it is burned and the scope re-opened to look for
/// a different candidate.
#[tokio::test]
async fn a_release_that_lied_about_its_quality_is_blocklisted_and_the_scope_reopened() {
    let release_title = "Quality Lie Movie.2026.1080p.WEB-DL.x264-GRP";
    let fixture = disposition_fixture("Quality Lie Movie", release_title).await;
    let incumbent_id =
        seed_primary_movie_file(&fixture, "Quality Lie Movie - 1080p.mkv", "1080p").await;

    let mut analysis = crate::post_download_gate::build_stream_pointer_media_file_analysis();
    analysis.video_codec = crate::release_parser::VideoCodec::parse("h264");
    analysis.video_width = Some(1280);
    analysis.video_height = Some(720);
    let _probe = crate::post_download_gate::probe_override::install(
        crate::post_download_gate::ImportedFileAcceptance {
            analysis: Some(analysis),
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        },
    );

    let result = crate::import_workflow::import_completed_download(
        &fixture.app,
        &fixture.user,
        &fixture.completed,
    )
    .await
    .expect("the import runs to a decision");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    let expected = crate::normalize_release_name(Some(release_title)).unwrap_or_default();
    assert!(
        fixture.blocklisted_titles().await.contains(&expected),
        "a proven quality lie must be blocklisted, got {:?}",
        fixture.blocklisted_titles().await
    );
    assert_eq!(
        fixture.scope_status().await,
        AcquisitionScopeStatus::Wanted,
        "the scope must reopen so convergence looks for a different release"
    );
    let primaries = primary_movie_files(&fixture).await;
    assert_eq!(
        primaries.len(),
        1,
        "the incumbent must stand alone: {primaries:?}"
    );
    assert_eq!(primaries[0].id, incumbent_id);
}

/// A file rule the operator wrote vetoes the file over something the release name
/// never disclosed. That is an import failure, so burn this release and reopen
/// the scope to try another candidate.
#[tokio::test]
async fn a_file_the_profile_vetoes_over_an_undisclosed_property_is_blocklisted() {
    let release_title = "Undisclosed Veto Movie.2026.1080p.WEB-DL-GRP";
    let fixture = disposition_fixture("Undisclosed Veto Movie", release_title).await;

    let policy = scryer_rules::UserPolicy {
        id: "no_probe_files".to_string(),
        name: "No probe files".to_string(),
        rego_source: scryer_rules::rewrite_package_declaration(
            r#"
score_entry["operator_refuses_this_file"] := -10000 if {
    input.file != null
}
"#,
            "no_probe_files",
        ),
        origin: scryer_rules::PolicyOrigin::User,
        applied_facets: vec!["movie".to_string()],
    };
    *fixture
        .app
        .services
        .customization
        .user_rules
        .write()
        .expect("user rules lock should be writable") =
        scryer_rules::UserRulesEngine::build(&[policy]).expect("rule fixture should compile");

    let mut analysis = crate::post_download_gate::build_stream_pointer_media_file_analysis();
    analysis.video_codec = crate::release_parser::VideoCodec::parse("h264");
    analysis.video_width = Some(1920);
    analysis.video_height = Some(1080);
    let rule_file_doc = crate::user_rule_input::file_doc_from_analysis(&analysis);
    let _probe = crate::post_download_gate::probe_override::install(
        crate::post_download_gate::ImportedFileAcceptance {
            analysis: Some(analysis),
            scan_error: None,
            rule_file_doc: Some(rule_file_doc),
            audio_language_warning: None,
        },
    );

    let mut tracked = fixture.tracked_import_pending();
    assert!(
        !crate::completed_download_handler::import(&fixture.app, &fixture.user, &mut tracked).await,
        "a burned import does not report completion"
    );
    let result = fixture.latest_import_result().await;

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );
    assert!(result.release_burned, "{result:?}");
    let expected = crate::normalize_release_name(Some(release_title)).unwrap_or_default();
    assert!(
        fixture.blocklisted_titles().await.contains(&expected),
        "an undisclosed veto must burn this release: {:?}",
        fixture.blocklisted_titles().await
    );
    assert_eq!(
        fixture.scope_status().await,
        AcquisitionScopeStatus::Wanted,
        "a burned veto must reopen the scope so convergence can try another release"
    );
    assert_eq!(tracked.state, TrackedDownloadState::Failed);
    assert!(
        primary_movie_files(&fixture).await.is_empty(),
        "a held file is not imported"
    );
}

/// The same file-rule rejection remains a guard for an operator-chosen release,
/// but it is held for manual import instead of being burned and retried.
#[tokio::test]
async fn operator_queued_file_rule_veto_is_held_without_blocklisting_or_reopening() {
    let release_title = "Undisclosed Veto Movie.2026.1080p.WEB-DL-GRP";
    let fixture = disposition_fixture("Undisclosed Veto Movie", release_title).await;
    fixture
        .set_submission_purpose(crate::DownloadSubmissionPurpose::OperatorQueued)
        .await;

    let policy = scryer_rules::UserPolicy {
        id: "no_probe_files".to_string(),
        name: "No probe files".to_string(),
        rego_source: scryer_rules::rewrite_package_declaration(
            r#"
score_entry["operator_refuses_this_file"] := -10000 if {
    input.file != null
}
"#,
            "no_probe_files",
        ),
        origin: scryer_rules::PolicyOrigin::User,
        applied_facets: vec!["movie".to_string()],
    };
    *fixture
        .app
        .services
        .customization
        .user_rules
        .write()
        .expect("user rules lock should be writable") =
        scryer_rules::UserRulesEngine::build(&[policy]).expect("rule fixture should compile");

    let mut analysis = crate::post_download_gate::build_stream_pointer_media_file_analysis();
    analysis.video_codec = crate::release_parser::VideoCodec::parse("h264");
    analysis.video_width = Some(1920);
    analysis.video_height = Some(1080);
    let rule_file_doc = crate::user_rule_input::file_doc_from_analysis(&analysis);
    let _probe = crate::post_download_gate::probe_override::install(
        crate::post_download_gate::ImportedFileAcceptance {
            analysis: Some(analysis),
            scan_error: None,
            rule_file_doc: Some(rule_file_doc),
            audio_language_warning: None,
        },
    );

    let mut tracked = fixture.tracked_import_pending();
    assert!(
        !crate::completed_download_handler::import(&fixture.app, &fixture.user, &mut tracked).await,
        "a held import does not report completion"
    );
    let result = fixture.latest_import_result().await;

    assert_eq!(
        // A held import is `Skipped` (the manual-import hold), never `Rejected`:
        // `result_state` parks it as ImportBlocked for the operator.
        result.decision,
        scryer_domain::ImportDecision::Skipped,
        "{result:?}"
    );
    assert!(!result.release_burned, "{result:?}");
    assert!(
        result.error_message.as_deref().is_some_and(
            |message| message.starts_with("held for manual import because the file failed")
        ),
        "{result:?}"
    );
    assert!(
        fixture.blocklisted_titles().await.is_empty(),
        "an operator-held release must not be blocklisted"
    );
    assert_eq!(
        fixture.scope_status().await,
        AcquisitionScopeStatus::Grabbed,
        "a held release leaves the scope alone"
    );
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
    assert!(
        tracked
            .status_messages
            .iter()
            .any(|message| message.starts_with("held for manual import because the file failed")),
        "{tracked:?}"
    );
    assert!(
        primary_movie_files(&fixture).await.is_empty(),
        "held files are not imported automatically"
    );
}

/// **`Skip`.** The scope already holds a better tier. The release is perfectly
/// good, it just lost the comparison — so it is not burned, and the scope is not
/// reopened for a search that has nothing new to find.
#[tokio::test]
async fn an_import_that_loses_the_comparison_is_skipped_without_burning_the_release() {
    let release_title = "Loses Comparison Movie.2026.720p.WEB-DL-GRP";
    let fixture = disposition_fixture("Loses Comparison Movie", release_title).await;
    let incumbent_id =
        seed_primary_movie_file(&fixture, "Loses Comparison Movie - 1080p.mkv", "1080p").await;
    let _probe = probe_agrees_with_the_name(1280, 720);

    let result = crate::import_workflow::import_completed_download(
        &fixture.app,
        &fixture.user,
        &fixture.completed,
    )
    .await
    .expect("the import runs to a decision");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Skipped,
        "{result:?}"
    );
    assert!(
        fixture.blocklisted_titles().await.is_empty(),
        "losing a comparison is not a lie: {:?}",
        fixture.blocklisted_titles().await
    );
    assert_eq!(
        fixture.scope_status().await,
        AcquisitionScopeStatus::Grabbed,
        "a skip must not reopen the scope"
    );
    // **The fall-through bug.** `import_movie_download` had no `else` for a
    // refused admission and walked straight into the first-import insert, which
    // defaults `MediaFileRole` to `Primary` — a second primary file for a movie
    // it had just refused. `decide_import` makes that unrepresentable.
    let primaries = primary_movie_files(&fixture).await;
    assert_eq!(
        primaries.len(),
        1,
        "a refused movie import must not insert a second primary file: {primaries:?}"
    );
    assert_eq!(primaries[0].id, incumbent_id);
}

/// **A1, end to end.** A movie incumbent that lives at a path this import would
/// never write (rename disabled, a changed template, `.mp4` → `.mkv`) must still
/// be found and recycled. Resolving by destination path came up empty while
/// admission said the scope was occupied — the condition that panicked the
/// import task, and that a path filter would silently reintroduce.
#[tokio::test]
async fn a_movie_upgrade_finds_its_incumbent_at_another_path() {
    let release_title = "Renamed Incumbent Movie.2026.1080p.WEB-DL-GRP";
    let fixture = disposition_fixture("Renamed Incumbent Movie", release_title).await;
    let incumbent_id =
        seed_primary_movie_file(&fixture, "preserved.original.name.720p.mp4", "720p").await;
    let _probe = probe_agrees_with_the_name(1920, 1080);

    let result = crate::import_workflow::import_completed_download(
        &fixture.app,
        &fixture.user,
        &fixture.completed,
    )
    .await
    .expect("the import runs to a decision");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "{result:?}"
    );
    let primaries = primary_movie_files(&fixture).await;
    assert!(
        !primaries.iter().any(|file| file.id == incumbent_id),
        "the recycled incumbent must be gone: {primaries:?}"
    );
    assert_eq!(
        primaries.len(),
        1,
        "exactly one primary file survives an upgrade: {primaries:?}"
    );
    assert!(
        fixture.blocklisted_titles().await.is_empty(),
        "an honest upgrade burns nothing"
    );
}

/// **A1 through the link path, end to end.** The clone of
/// `completed_import_imports_additional_series_movie_file_from_submission_scope`
/// the review asked for: a `Standard` submission (so it takes
/// `import_series_movie_download`'s main path) whose linked incumbent sits at a
/// path this import would never write. Resolving the row by destination path
/// leaves it empty while admission says the scope is occupied — the condition
/// that used to `.expect()` and panic the import task. Reintroducing a
/// path-scoped incumbent list turns this red.
#[tokio::test]
async fn series_movie_link_upgrade_finds_its_incumbent_at_another_path() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let media_files = Arc::new(MockMediaFileRepo::default());
    let import_repo = Arc::new(TrackingImportRepo::default());
    let blocklist_repo = Arc::new(MockBlocklistRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
            .with_blocklist_repo(blocklist_repo.clone())
    });

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Renamed Link Incumbent");
    let anime_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Anime)
        .await
        .expect("library lookup")
        .expect("anime library exists");
    app.services
        .catalog
        .libraries
        .update(
            &anime_library.id,
            anime_library.name.clone(),
            anime_library.slug.clone(),
            vec![LibraryRootDraft {
                path: library_dir.path().to_string_lossy().into_owned(),
                is_default: true,
            }],
        )
        .await
        .expect("point the anime library at the fixture root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Renamed Link Incumbent".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                runtime_minutes: Some(40),
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let specials = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "0".into(),
            Some("Specials".into()),
            None,
            Some("0".into()),
            Some("3".into()),
        )
        .await
        .expect("create specials collection");
    let linked_episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(specials.id),
            "special".into(),
            Some("3".into()),
            Some("0".into()),
            Some("S00E03".into()),
            Some("Movie Special".into()),
            None,
            Some(2_400),
            false,
            false,
        )
        .await
        .expect("create linked special episode");

    let mut link_input = test_series_movie_link(
        &title.id,
        "Renamed Link Incumbent: The Movie",
        Some(2026),
        None,
        Some("renamed-link-incumbent"),
    );
    link_input.linked_episode_id = Some(linked_episode.id.clone());
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(link_input)
        .await
        .expect("create series movie link");

    // Deliberately not the path the import writes: rename disabled, a template
    // change, `.mp4` instead of `.mkv` — all ordinary, all fatal to a path
    // lookup.
    std::fs::create_dir_all(title_folder.join("Season 00")).expect("create season folder");
    let incumbent_path = title_folder
        .join("Season 00")
        .join("preserved.original.name.720p.mp4");
    std::fs::File::create(&incumbent_path)
        .expect("create incumbent file")
        .set_len(300 * 1024 * 1024)
        .expect("size incumbent file");
    let incumbent_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: incumbent_path.to_string_lossy().into_owned(),
            size_bytes: 300 * 1024 * 1024,
            role: MediaFileRole::Primary,
            quality_label: Some("720p".to_string()),
            resolution: Some("720p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert linked incumbent");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&incumbent_id, &link.id)
        .await
        .expect("link incumbent to series movie");

    let release_title = "Renamed.Link.Incumbent.The.Movie.2026.1080p.BluRay.x264-Group";
    let item_id = "renamed-link-incumbent-1";
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            // `Standard`, not `AdditionalFile`: this must take the main link
            // import path, the one that resolves an incumbent to recycle.
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some(release_title.to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::SeriesMovie {
                series_movie_link_id: link.id.clone(),
            },
        })
        .await
        .expect("record series movie submission");

    let download_dir = tempfile::tempdir().expect("download tempdir");
    let source_file = download_dir.path().join(format!("{release_title}.mkv"));
    std::fs::File::create(&source_file)
        .expect("create source video")
        .set_len(300 * 1024 * 1024)
        .expect("size source video");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        release_title,
        download_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed.clone()];
    let _probe = probe_agrees_with_the_name(1920, 1080);

    let result = crate::import_workflow::import_completed_download(&app, &user, &completed)
        .await
        .expect("the linked upgrade runs to a decision");
    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Imported,
        "{result:?}"
    );

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert!(
        !files.iter().any(|file| file.id == incumbent_id),
        "the recycled incumbent must be gone: {files:?}"
    );
    let imported = files
        .iter()
        .find(|file| file.role.is_primary())
        .expect("the upgrade landed a primary file");
    assert_eq!(imported.series_movie_link_ids, vec![link.id]);
    assert!(
        blocklist_repo.entries.lock().await.is_empty(),
        "an honest upgrade burns nothing"
    );
}

/// **Minor 5.** A manual series-movie-link import does not go through
/// `import_series_movie_download` at all, so it never reaches the verdict gate —
/// whatever the probe reports. Pinned because `operator_initiated_import`'s doc
/// used to claim it covered "all three import paths", which invited someone to
/// rely on the bypass flag for a path that never consults it.
#[tokio::test]
async fn a_manual_series_movie_link_import_never_reaches_the_verdict_gate() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let media_files = Arc::new(MockMediaFileRepo::default());
    let blocklist_repo = Arc::new(MockBlocklistRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
            .with_blocklist_repo(blocklist_repo.clone())
    });
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("seed manual import actor");

    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Manual Link Verdict Bypass");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Link Verdict Bypass".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                runtime_minutes: Some(40),
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Manual Link Verdict Bypass: The Movie",
            Some(2026),
            None,
            Some("manual-link-verdict-bypass"),
        ))
        .await
        .expect("create series movie link");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = source_dir
        .path()
        .join("Manual.Link.Verdict.Bypass.The.Movie.2026.1080p.WEB-DL.mkv");
    std::fs::File::create(&source_file)
        .expect("create source video")
        .set_len(300 * 1024 * 1024)
        .expect("size source video");

    // The probe reports 720p against a 1080p name — exactly the evidence that
    // blocklists a release on the automatic lane.
    let mut analysis = crate::post_download_gate::build_stream_pointer_media_file_analysis();
    analysis.video_codec = crate::release_parser::VideoCodec::parse("h264");
    analysis.video_width = Some(1280);
    analysis.video_height = Some(720);
    let _probe = crate::post_download_gate::probe_override::install(
        crate::post_download_gate::ImportedFileAcceptance {
            analysis: Some(analysis),
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        },
    );

    let results = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-link-verdict-bypass",
        &title.id,
        None,
        vec![ManualImportFileMapping {
            file_path: source_file.to_string_lossy().into_owned(),
            episode_id: None,
            series_movie_link_id: Some(link.id.clone()),
        }],
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root")),
    )
    .await
    .expect("execute manual import");

    assert!(
        results.iter().all(|result| result.success),
        "the operator's own file must import: {results:?}"
    );
    assert!(
        blocklist_repo.entries.lock().await.is_empty(),
        "a manual link import must never blocklist the operator's release: {:?}",
        blocklist_repo.entries.lock().await
    );
}

/// **The link dimension of the reopen (D17 / review m7).** A refused link import
/// used to reopen nothing: `reset_wanted_items_for_retry` had only an episode
/// list and a title fallback, and a series-movie link's scope row is keyed on
/// neither — so the link sat permanently un-searched after one bad release.
/// The reopen now reads the same `BlocklistAttribution` the blocklist entry is
/// filed under.
#[tokio::test]
async fn a_refused_link_import_blocklists_and_reopens_the_link_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let media_files = Arc::new(MockMediaFileRepo::default());
    let blocklist_repo = Arc::new(MockBlocklistRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(Arc::new(TrackingImportRepo::default()))
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
            .with_blocklist_repo(blocklist_repo.clone())
    });

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Refused Link Import");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Refused Link Import".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                runtime_minutes: Some(40),
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Refused Link Import: The Movie",
            Some(2026),
            None,
            Some("refused-link-import"),
        ))
        .await
        .expect("create series movie link");

    // Occupied at 1080p, so a landed 720p is a refusal rather than an
    // import-and-blocklist.
    std::fs::create_dir_all(&title_folder).expect("create title folder");
    let incumbent_path = title_folder.join("Refused Link Import - 1080p.mkv");
    std::fs::File::create(&incumbent_path)
        .expect("create incumbent file")
        .set_len(300 * 1024 * 1024)
        .expect("size incumbent file");
    let incumbent_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: incumbent_path.to_string_lossy().into_owned(),
            size_bytes: 300 * 1024 * 1024,
            role: MediaFileRole::Primary,
            quality_label: Some("1080p".to_string()),
            resolution: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert linked incumbent");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&incumbent_id, &link.id)
        .await
        .expect("link incumbent to series movie");

    // A decoy: the title-scope row the old `episode_ids.is_empty() => title`
    // fallback would have reopened. It is stored first, so a lookup that does
    // not key on the link id finds this one instead. It must stay `grabbed`.
    let title_scope_id = Id::new().0;
    app.services
        .workflow
        .acquisition_scope_states
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: title_scope_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_facet: Some(MediaFacet::Anime.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            media_type: "anime".to_string(),
            series_movie_link_id: None,
            status: AcquisitionScopeStatus::Grabbed,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            title_slug: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            last_search_at: None,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
        })
        .await
        .expect("seed decoy title scope row");

    // The scope row a link lives on: no episode id, a link id, and `grabbed`.
    let scope_id = Id::new().0;
    app.services
        .workflow
        .acquisition_scope_states
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: scope_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_facet: Some(MediaFacet::Anime.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            media_type: "series_movie".to_string(),
            series_movie_link_id: Some(link.id.clone()),
            status: AcquisitionScopeStatus::Grabbed,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            title_slug: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            last_search_at: None,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
        })
        .await
        .expect("seed grabbed link scope row");

    let release_title = "Refused.Link.Import.The.Movie.2026.1080p.BluRay.x264-Group";
    let item_id = "refused-link-import-1";
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some(release_title.to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::SeriesMovie {
                series_movie_link_id: link.id.clone(),
            },
        })
        .await
        .expect("record series movie submission");

    let download_dir = tempfile::tempdir().expect("download tempdir");
    let source_file = download_dir.path().join(format!("{release_title}.mkv"));
    std::fs::File::create(&source_file)
        .expect("create source video")
        .set_len(300 * 1024 * 1024)
        .expect("size source video");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        release_title,
        download_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed.clone()];

    let mut analysis = crate::post_download_gate::build_stream_pointer_media_file_analysis();
    analysis.video_codec = crate::release_parser::VideoCodec::parse("h264");
    analysis.video_width = Some(1280);
    analysis.video_height = Some(720);
    let _probe = crate::post_download_gate::probe_override::install(
        crate::post_download_gate::ImportedFileAcceptance {
            analysis: Some(analysis),
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        },
    );

    let result = crate::import_workflow::import_completed_download(&app, &user, &completed)
        .await
        .expect("the link import runs to a decision");
    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "{result:?}"
    );

    let entries = blocklist_repo.entries.lock().await.clone();
    let expected = crate::normalize_release_name(Some(release_title)).unwrap_or_default();
    entries
        .iter()
        .find(|entry| entry.normalized_release_name == expected)
        .expect("the lying release is blocklisted for the title");

    let reopened = app
        .services
        .workflow
        .acquisition_scope_states
        .get_acquisition_scope_state_by_id(&scope_id)
        .await
        .expect("scope lookup")
        .expect("the link scope row survives");
    assert_eq!(
        reopened.status,
        AcquisitionScopeStatus::Wanted,
        "a refused link import must reopen the *link* scope, not the title's"
    );
    let decoy = app
        .services
        .workflow
        .acquisition_scope_states
        .get_acquisition_scope_state_by_id(&title_scope_id)
        .await
        .expect("scope lookup")
        .expect("the decoy title scope row survives");
    assert_eq!(
        decoy.status,
        AcquisitionScopeStatus::Grabbed,
        "the title scope is not this rejection's scope and must be left alone"
    );
}

/// A submissions repo that answers only the durable download-history query and
/// delegates everything else.
///
/// The merge tests drive `list_download_history_page`, which reads nothing else
/// from this port; delegating the rest keeps the double from asserting anything
/// about calls it is not the subject of.
struct DurableHistorySubmissionRepo {
    inner: crate::NullDownloadSubmissionRepository,
    rows: Vec<crate::TerminalDownloadHistoryRow>,
}

impl DurableHistorySubmissionRepo {
    fn new(rows: Vec<crate::TerminalDownloadHistoryRow>) -> Self {
        Self {
            inner: crate::NullDownloadSubmissionRepository,
            rows,
        }
    }
}

#[async_trait]
impl crate::DownloadSubmissionRepository for DurableHistorySubmissionRepo {
    async fn list_terminal_download_history_rows(
        &self,
        _limit: usize,
    ) -> AppResult<Vec<crate::TerminalDownloadHistoryRow>> {
        Ok(self.rows.clone())
    }

    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        self.inner.record_submission(submission).await
    }

    async fn record_ambiguous_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        self.inner.record_ambiguous_submission(submission).await
    }

    async fn record_submission_with_identity(
        &self,
        submission: DownloadSubmission,
        submission_identity: crate::DownloadSubmissionIdentity,
        seed_goals: Option<crate::PersistedSeedGoals>,
    ) -> AppResult<crate::CanonicalDownloadIdentityDisposition> {
        self.inner
            .record_submission_with_identity(submission, submission_identity, seed_goals)
            .await
    }

    async fn find_by_client_item_id(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>> {
        self.inner.find_by_client_item_id(identity).await
    }

    async fn list_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<DownloadSubmission>> {
        self.inner.list_for_client_items(client_items).await
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        self.inner.list_for_title(title_id).await
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
        purpose: crate::DownloadSubmissionPurpose,
        scope: &crate::SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        self.inner
            .find_by_title_and_request_signature(title_id, request_signature, purpose, scope)
            .await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.inner.delete_for_title(title_id).await
    }

    async fn delete_by_client_item_id(&self, identity: &ClientJobLocator) -> AppResult<()> {
        self.inner.delete_by_client_item_id(identity).await
    }

    async fn update_tracked_state(
        &self,
        identity: &ClientJobLocator,
        tracked_state: &str,
    ) -> AppResult<()> {
        self.inner
            .update_tracked_state(identity, tracked_state)
            .await
    }

    async fn get_tracked_state(&self, identity: &ClientJobLocator) -> AppResult<Option<String>> {
        self.inner.get_tracked_state(identity).await
    }
}

fn terminal_history_row(
    download_id: scryer_domain::download_identity::DownloadId,
    item_id: &str,
    title_id: Option<&str>,
    source_title: &str,
) -> crate::TerminalDownloadHistoryRow {
    crate::TerminalDownloadHistoryRow {
        download_id,
        origin: crate::DownloadOrigin::ScryerSubmission,
        tracked_state: TrackedDownloadState::Imported.as_str().to_string(),
        tracked_reason: None,
        tracked_detail: None,
        title_id: title_id.map(str::to_string),
        episode_id: None,
        facet: Some("movie".to_string()),
        source_title: Some(source_title.to_string()),
        client_id: Some("primary".to_string()),
        client_type: Some("nzbget".to_string()),
        client_name: Some("NZBGet".to_string()),
        download_client_item_id: Some(item_id.to_string()),
        source_provider_name: None,
        size_bytes: Some(4_096),
        submitted_at: Some(Utc::now() - chrono::Duration::hours(2)),
        last_state_at: Some(Utc::now() - chrono::Duration::hours(1)),
    }
}

async fn history_app_with_durable_rows(
    rows: Vec<crate::TerminalDownloadHistoryRow>,
) -> (AppUseCase, User) {
    let download_client = Arc::new(StubDownloadClient::default());
    let (mut app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );
    app.services.workflow.download_submissions = Arc::new(DurableHistorySubmissionRepo::new(rows));
    (app, user)
}

/// rTorrent (among others) evicts finished jobs from its own list, which used
/// to erase the history entry with it: the projection read only the live
/// snapshot. The persisted terminal row has to stand in on its own.
#[tokio::test]
async fn download_history_keeps_a_terminal_row_the_client_has_evicted() {
    let evicted_id = scryer_domain::download_identity::DownloadId::new();
    let (app, user) = history_app_with_durable_rows(vec![terminal_history_row(
        evicted_id,
        "evicted-1",
        None,
        "Quiet Meridian",
    )])
    .await;
    publish_test_download_queue_snapshot(&app, Vec::new()).await;

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("history page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items[0].download_client_item_id, "evicted-1");
    assert_eq!(page.items[0].state, DownloadQueueState::Completed);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::Imported)
    );
    assert_eq!(page.items[0].title_name, "Quiet Meridian");
    assert_eq!(
        page.items[0].download_id.as_deref(),
        Some(evicted_id.to_wire().as_str())
    );
}

/// When both sources describe the same download the live row wins: it carries
/// the client's own progress, delete state and import overlay, none of which
/// the durable row can reconstruct.
#[tokio::test]
async fn download_history_prefers_the_live_row_for_a_download_both_sources_carry() {
    let shared_id = scryer_domain::download_identity::DownloadId::new();
    let (app, user) = history_app_with_durable_rows(vec![terminal_history_row(
        shared_id,
        "shared-1",
        None,
        "Salt and Signal (durable)",
    )])
    .await;

    let mut live = queue_history_fixture_item("shared-1", DownloadQueueState::Completed, 9_000);
    live.title_id = None;
    live.title_name = "Salt and Signal (live)".to_string();
    live.download_id = Some(shared_id.to_wire());
    publish_test_download_queue_snapshot(&app, vec![live]).await;

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("history page should load");

    assert_eq!(page.total_count, 1, "the durable row must not duplicate");
    assert_eq!(page.items[0].title_name, "Salt and Signal (live)");
}

/// Permission filtering is applied to the merged set, not just the live half:
/// a durable row for a title the actor cannot see, and an untitled operational
/// row, both stay hidden.
#[tokio::test]
async fn download_history_applies_permission_filtering_to_durable_rows() {
    let visible_id = scryer_domain::download_identity::DownloadId::new();
    let operational_id = scryer_domain::download_identity::DownloadId::new();
    let hidden_id = scryer_domain::download_identity::DownloadId::new();
    let (app, admin) = history_app_with_durable_rows(Vec::new()).await;

    let visible_title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Quiet Meridian".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: Some(2011),
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("movie title should be added");
    let hidden_title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Salt and Signal".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: Some(2013),
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("series title should be added");

    let mut app = app;
    app.services.workflow.download_submissions = Arc::new(DurableHistorySubmissionRepo::new(vec![
        terminal_history_row(
            visible_id,
            "visible-1",
            Some(&visible_title.id),
            "Quiet Meridian",
        ),
        terminal_history_row(operational_id, "operational-1", None, "Unattributed Grab"),
        terminal_history_row(
            hidden_id,
            "hidden-1",
            Some(&hidden_title.id),
            "Salt and Signal",
        ),
    ]));
    publish_test_download_queue_snapshot(&app, Vec::new()).await;

    let movie_viewer = library_permission_user(
        "movie-viewer",
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        &[scryer_domain::LibraryPermission::View],
    );
    let page = app
        .list_download_history_page(
            &movie_viewer,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("history page should load for the restricted viewer");

    assert_eq!(
        page.items
            .iter()
            .map(|item| item.download_client_item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["visible-1"],
        "only the durable row whose title is in a granted library survives"
    );

    let admin_page = app
        .list_download_history_page(
            &admin,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("history page should load for the admin");
    assert_eq!(
        admin_page.total_count, 3,
        "an operator with operational history sees every durable row"
    );
}
