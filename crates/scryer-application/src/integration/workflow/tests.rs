#[cfg(test)]
mod tests {
    use super::{
        DownloadImportActions, DownloadQueueBucket, TrackedDownloadBackgroundWorkKind,
        TrackedDownloadWorkDrain, apply_import_record_to_queue_item,
        apply_submission_to_queue_item, apply_tracked_download_activity_projection,
        apply_tracked_download_queue_metadata, build_download_queue_status_detail,
        canonicalize_download_queue_item_clients, classify_download_queue_item,
        collect_download_client_filter_options, dedupe_download_queue_items,
        derive_download_queue_display_state, derive_download_queue_import_actions,
        derive_indexer_base_url_from_config_fields, download_queue_client_filter_key,
        import_record_result_overlay, normalize_indexer_config_json,
        prepare_next_tracked_download_background_work_dispatch,
        prepare_tracked_download_background_work_dispatch,
        reconcile_duplicate_terminal_source_states, source_provider_label,
        synthetic_tracked_snapshot_queue_item, tracked_download_queue_snapshot,
    };
    use crate::DownloadDisplayState;
    use crate::{DownloadSubmission, DownloadSubmissionPurpose};
    use chrono::{Duration, Utc};
    use scryer_domain::{
        ConfigFieldDef, ConfigFieldOption, ConfigFieldRole, ConfigFieldType,
        ConfigFieldValueSource, DownloadClientConfig, DownloadClientStatus, DownloadQueueItem,
        DownloadQueueState, ImportRecord, ImportStatus, ImportTransferPhase, ImportType,
        TitleMatchType, TrackedDownloadState, TrackedDownloadStatus,
    };
    use std::collections::BTreeMap;

    #[test]
    fn fixed_endpoint_indexer_without_config_fields_has_no_derived_base_url() {
        assert_eq!(
            derive_indexer_base_url_from_config_fields(&[], Some("{}"))
                .expect("configless indexer is valid"),
            ""
        );
    }

    #[test]
    fn selected_indexer_option_fills_only_missing_config_values() {
        let fields = vec![
            ConfigFieldDef {
                key: "profile_id".to_string(),
                label: "Known Provider".to_string(),
                field_type: ConfigFieldType::Select,
                required: true,
                default_value: Some("custom".to_string()),
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![ConfigFieldOption {
                    value: "known".to_string(),
                    label: "Known".to_string(),
                    config_overrides: BTreeMap::from([
                        (
                            "base_url".to_string(),
                            "https://api.example.test".to_string(),
                        ),
                        ("api_path".to_string(), "/api".to_string()),
                        ("request_interval_ms".to_string(), "2000".to_string()),
                    ]),
                }],
                help_text: None,
                ..Default::default()
            },
            ConfigFieldDef {
                key: "base_url".to_string(),
                label: "Base URL".to_string(),
                field_type: ConfigFieldType::String,
                required: false,
                default_value: None,
                value_source: ConfigFieldValueSource::User,
                role: Some(ConfigFieldRole::ConnectionUrl),
                host_binding: None,
                options: vec![],
                help_text: None,
                ..Default::default()
            },
            ConfigFieldDef {
                key: "api_path".to_string(),
                label: "API Path".to_string(),
                field_type: ConfigFieldType::String,
                required: false,
                default_value: Some("/default".to_string()),
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
                ..Default::default()
            },
            ConfigFieldDef {
                key: "request_interval_ms".to_string(),
                label: "Request Interval".to_string(),
                field_type: ConfigFieldType::Number,
                required: false,
                default_value: Some("1000".to_string()),
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
                ..Default::default()
            },
        ];

        let normalized = normalize_indexer_config_json(
            &fields,
            Some(r#"{"profile_id":"known","request_interval_ms":"750"}"#),
            Some(r#"{"api_path":"/persisted"}"#),
        )
        .expect("selected option should normalize");
        let config: serde_json::Value = serde_json::from_str(&normalized).expect("normalized JSON");

        assert_eq!(config["base_url"], "https://api.example.test");
        assert_eq!(config["api_path"], "/persisted");
        assert_eq!(config["request_interval_ms"], "750");

        let error =
            normalize_indexer_config_json(&fields, Some(r#"{"profile_id":"custom"}"#), None)
                .expect_err("custom selection requires an explicit connection URL");
        assert!(error.to_string().contains("Base URL is required"));
    }

    fn item(id: &str, state: DownloadQueueState) -> DownloadQueueItem {
        DownloadQueueItem {
            id: id.to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Example".to_string(),
            facet: None,
            category: None,
            client_id: "client-1".to_string(),
            client_name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            state,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: Some(100),
            remaining_seconds: None,
            queued_at: Some(Utc::now().timestamp_millis().to_string()),
            last_updated_at: Some(Utc::now().timestamp_millis().to_string()),
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            download_id: None,
            import_status: None,
            import_type: None,
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
        }
    }

    fn tracked_for_dispatch(id: &str) -> crate::tracked_downloads::TrackedDownload {
        let client_item = item("job-1", DownloadQueueState::Completed);
        crate::tracked_downloads::TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: id.to_string(),
            client_id: client_item.client_id.clone(),
            client_type: client_item.client_type.clone(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::ImportPending,
            status: TrackedDownloadStatus::Warning,
            status_messages: vec!["retry later".to_string()],
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: Some("Example.Release".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
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
            retained_in_client_after_cleanup: false,
        }
    }

    fn no_video_retry_state(
        next_retry_at: chrono::DateTime<Utc>,
    ) -> crate::tracked_downloads::NoVideoImportRetryState {
        crate::tracked_downloads::NoVideoImportRetryState {
            signature: crate::tracked_downloads::NoVideoImportSourceSignature {
                source_path: "/tmp/download".to_string(),
                file_count: 1,
                total_bytes: 5,
                latest_mtime: None,
            },
            attempts: 1,
            next_retry_at,
        }
    }

    #[test]
    fn import_dispatch_respects_no_video_retry_gate() {
        let id = "nzbget:job-1";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let mut tracked = tracked_for_dispatch(id);
        tracked.no_video_import_retry =
            Some(no_video_retry_state(Utc::now() + Duration::seconds(30)));
        let download_id = tracker.insert_for_tests(tracked);

        assert!(
            prepare_tracked_download_background_work_dispatch(&mut tracker, download_id).is_none()
        );
        assert_eq!(
            tracker.find(id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );

        tracker
            .find_mut(id)
            .expect("tracked download should remain cached")
            .no_video_import_retry = Some(no_video_retry_state(Utc::now() - Duration::seconds(1)));

        let dispatched =
            prepare_tracked_download_background_work_dispatch(&mut tracker, download_id);

        assert!(dispatched.is_some());
        assert_eq!(
            tracker.find(id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );
    }

    #[test]
    fn import_dispatch_respects_execution_retry_gate() {
        let id = "nzbget:job-2";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let mut tracked = tracked_for_dispatch(id);
        tracked.import_execution_retry =
            Some(crate::tracked_downloads::ImportExecutionRetryState {
                attempts: 1,
                next_retry_at: Utc::now() + Duration::seconds(30),
            });
        let download_id = tracker.insert_for_tests(tracked);

        assert!(
            prepare_tracked_download_background_work_dispatch(&mut tracker, download_id).is_none()
        );
        assert_eq!(
            tracker.find(id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );

        tracker
            .find_mut(id)
            .expect("tracked download should remain cached")
            .import_execution_retry = Some(crate::tracked_downloads::ImportExecutionRetryState {
            attempts: 1,
            next_retry_at: Utc::now() - Duration::seconds(1),
        });

        let dispatched =
            prepare_tracked_download_background_work_dispatch(&mut tracker, download_id);

        assert!(dispatched.is_some());
        assert_eq!(
            tracker.find(id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );
    }

    #[test]
    fn tracked_work_drain_skips_retry_delayed_item_and_dispatches_next() {
        let delayed_id = "nzbget:delayed";
        let ready_id = "nzbget:ready";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let mut delayed = tracked_for_dispatch(delayed_id);
        delayed.no_video_import_retry =
            Some(no_video_retry_state(Utc::now() + Duration::seconds(30)));
        let delayed_download_id = tracker.insert_for_tests(delayed);
        let ready_download_id = tracker.insert_for_tests(tracked_for_dispatch(ready_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![delayed_download_id, ready_download_id],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let in_flight = std::collections::HashSet::new();

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("ready item should dispatch");

        assert_eq!(id, ready_download_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert_eq!(
            tracker.find(delayed_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );
        assert_eq!(
            tracker.find(ready_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );
    }

    #[test]
    fn tracked_work_drain_dispatches_next_after_first_remains_pending() {
        let first_id = "nzbget:first";
        let second_id = "nzbget:second";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let first_download_id = tracker.insert_for_tests(tracked_for_dispatch(first_id));
        let second_download_id = tracker.insert_for_tests(tracked_for_dispatch(second_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![first_download_id, second_download_id],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let in_flight = std::collections::HashSet::new();

        let (id, _, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("first item should dispatch");
        assert_eq!(id, first_download_id);
        tracker
            .find_mut(first_id)
            .expect("first tracked download")
            .state = TrackedDownloadState::ImportPending;

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("second item should dispatch in the same drain");

        assert_eq!(id, second_download_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert_eq!(
            tracker.find(second_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );
    }

    #[test]
    fn tracked_download_drain_dispatches_multiple_imports_without_waiting_for_worker_result() {
        let first_id = "nzbget:first";
        let second_id = "nzbget:second";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let first_download_id = tracker.insert_for_tests(tracked_for_dispatch(first_id));
        let second_download_id = tracker.insert_for_tests(tracked_for_dispatch(second_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![first_download_id, second_download_id],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let mut in_flight = std::collections::HashSet::new();

        let (id, _, tracked) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("first item should dispatch");
        assert_eq!(id, first_download_id);
        in_flight.insert(tracked.id);

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("second import should dispatch while the first remains in flight");

        assert_eq!(id, second_download_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert_eq!(
            tracker.find(second_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );
    }

    #[test]
    fn failed_work_is_capped_at_four_without_blocking_import_dispatch() {
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let failed_ids = (0..5)
            .map(|index| format!("nzbget:failed-{index}"))
            .collect::<Vec<_>>();
        let failed_download_ids = failed_ids
            .iter()
            .map(|id| {
                let mut tracked = tracked_for_dispatch(id);
                tracked.state = TrackedDownloadState::FailedPending;
                tracker.insert_for_tests(tracked)
            })
            .collect::<Vec<_>>();
        let import_id = "nzbget:import";
        let import_download_id = tracker.insert_for_tests(tracked_for_dispatch(import_id));
        let in_flight = failed_ids[..4].iter().cloned().collect();
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![failed_download_ids[4], import_download_id],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("import should pass the saturated failed-work lane");

        assert_eq!(id, import_download_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert!(
            drain.has_pending(),
            "fifth failed item should remain queued"
        );
    }

    #[test]
    fn tracked_work_drain_skips_blocked_item_and_dispatches_next() {
        let blocked_id = "nzbget:blocked";
        let ready_id = "nzbget:ready";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let mut blocked = tracked_for_dispatch(blocked_id);
        blocked.state = TrackedDownloadState::ImportBlocked;
        let blocked_download_id = tracker.insert_for_tests(blocked);
        let ready_download_id = tracker.insert_for_tests(tracked_for_dispatch(ready_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![blocked_download_id, ready_download_id],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let in_flight = std::collections::HashSet::new();

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("ready item should dispatch after blocked item");

        assert_eq!(id, ready_download_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert_eq!(
            tracker.find(blocked_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportBlocked)
        );
    }

    #[test]
    fn duplicate_terminal_source_state_prevents_import_redispatch() {
        let terminal_id = "download:submission:nzbget:scryer-download-1";
        let duplicate_id = "download:client-parameter:nzbget:scryer-download-1";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let mut terminal = tracked_for_dispatch(terminal_id);
        terminal.state = TrackedDownloadState::Imported;
        terminal.status = TrackedDownloadStatus::Ok;
        terminal.status_messages.clear();
        tracker.insert_for_tests(terminal);

        let mut duplicate = tracked_for_dispatch(duplicate_id);
        duplicate.state = TrackedDownloadState::ImportPending;
        duplicate.status = TrackedDownloadStatus::Warning;
        duplicate.status_messages = vec!["waiting for import".to_string()];
        let duplicate_download_id = tracker.insert_for_tests(duplicate);

        reconcile_duplicate_terminal_source_states(&mut tracker);

        let duplicate = tracker
            .find(duplicate_id)
            .expect("duplicate tracked download should remain cached");
        assert_eq!(duplicate.state, TrackedDownloadState::Imported);
        assert_eq!(duplicate.status, TrackedDownloadStatus::Ok);
        assert!(duplicate.status_messages.is_empty());
        assert!(
            prepare_tracked_download_background_work_dispatch(&mut tracker, duplicate_download_id)
                .is_none()
        );
    }

    /// Rows of one torrent share an info-hash id string. The reconcile is keyed
    /// by `DownloadId` and so is deterministic; the loops in these tests guard
    /// against a regression to string lookups, whose outcome depended on the
    /// layout of a freshly seeded `HashMap` and so varied from run to run.
    const SHARED_TORRENT_ID: &str = "download:client-a:qbittorrent:0123456789abcdef";

    fn reconcile_row(
        title_id: Option<&str>,
        state: TrackedDownloadState,
    ) -> crate::tracked_downloads::TrackedDownload {
        let mut tracked = tracked_for_dispatch(SHARED_TORRENT_ID);
        tracked.title_id = title_id.map(str::to_owned);
        tracked.state = state;
        tracked.status = TrackedDownloadStatus::Ok;
        tracked.status_messages.clear();
        tracked
    }

    fn reconciled_state(
        tracker: &crate::tracked_downloads::TrackedDownloadService,
        download_id: scryer_domain::download_identity::DownloadId,
    ) -> TrackedDownloadState {
        tracker
            .get_by_download_id(download_id)
            .expect("tracked download should remain cached")
            .state
    }

    #[test]
    fn reconcile_settles_the_collected_row_when_two_rows_share_an_id_string() {
        let shared_id = SHARED_TORRENT_ID;

        for _ in 0..64 {
            let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
            let mut imported = tracked_for_dispatch(shared_id);
            imported.state = TrackedDownloadState::Imported;
            imported.status = TrackedDownloadStatus::Ok;
            imported.status_messages.clear();
            tracker.insert_for_tests(imported);

            let mut live = tracked_for_dispatch(shared_id);
            live.state = TrackedDownloadState::Downloading;
            live.status = TrackedDownloadStatus::Ok;
            live.status_messages.clear();
            let live_download_id = tracker.insert_for_tests(live);

            let applied = reconcile_duplicate_terminal_source_states(&mut tracker);

            let live = tracker
                .get_all()
                .into_iter()
                .find(|tracked| tracked.download_id == live_download_id)
                .expect("live tracked download should remain cached");
            assert_eq!(live.state, TrackedDownloadState::Imported);
            assert_eq!(
                applied,
                vec![(live_download_id, TrackedDownloadState::Imported)]
            );
        }
    }

    #[test]
    fn reconcile_never_settles_a_live_row_for_a_different_title() {
        let shared_id = SHARED_TORRENT_ID;

        for (terminal_title_id, live_title_id) in [
            (Some("title-2"), Some("title-1")),
            (None, Some("title-1")),
            (Some("title-1"), None),
        ] {
            for _ in 0..64 {
                let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
                let mut imported = tracked_for_dispatch(shared_id);
                imported.title_id = terminal_title_id.map(str::to_owned);
                imported.state = TrackedDownloadState::Imported;
                imported.status = TrackedDownloadStatus::Ok;
                imported.status_messages.clear();
                tracker.insert_for_tests(imported);

                let mut live = tracked_for_dispatch(shared_id);
                live.title_id = live_title_id.map(str::to_owned);
                live.state = TrackedDownloadState::Downloading;
                live.status = TrackedDownloadStatus::Ok;
                live.status_messages.clear();
                let live_download_id = tracker.insert_for_tests(live);

                let applied = reconcile_duplicate_terminal_source_states(&mut tracker);

                let live = tracker
                    .get_all()
                    .into_iter()
                    .find(|tracked| tracked.download_id == live_download_id)
                    .expect("live tracked download should remain cached");
                assert_eq!(live.state, TrackedDownloadState::Downloading);
                assert!(applied.is_empty());
            }
        }
    }

    #[test]
    fn tracked_work_drain_dispatches_the_queued_row_when_two_rows_share_an_id_string() {
        for _ in 0..64 {
            let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
            let mut sibling = tracked_for_dispatch(SHARED_TORRENT_ID);
            sibling.title_id = Some("title-2".to_string());
            sibling.state = TrackedDownloadState::FailedPending;
            tracker.insert_for_tests(sibling);
            let live = tracker.insert_for_tests(tracked_for_dispatch(SHARED_TORRENT_ID));
            let mut drain = TrackedDownloadWorkDrain::new(
                vec![live],
                crate::completed_download_handler::CompletedDownloadLookup::default(),
            );

            let (download_id, kind, tracked) =
                prepare_next_tracked_download_background_work_dispatch(
                    &mut tracker,
                    &std::collections::HashSet::new(),
                    &mut drain,
                )
                .expect("the queued row should dispatch");

            assert_eq!(download_id, live);
            assert_eq!(tracked.download_id, live);
            assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        }
    }

    #[test]
    fn reconcile_takes_the_strongest_terminal_state_of_the_same_title() {
        for _ in 0..64 {
            let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
            tracker.insert_for_tests(reconcile_row(Some("title-1"), TrackedDownloadState::Failed));
            let imported = tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::Imported,
            ));
            let failed = tracker
                .insert_for_tests(reconcile_row(Some("title-1"), TrackedDownloadState::Failed));
            let live = tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::Downloading,
            ));

            let applied = reconcile_duplicate_terminal_source_states(&mut tracker);

            assert_eq!(applied, vec![(live, TrackedDownloadState::Imported)]);
            assert_eq!(
                reconciled_state(&tracker, live),
                TrackedDownloadState::Imported
            );
            assert_eq!(
                reconciled_state(&tracker, imported),
                TrackedDownloadState::Imported
            );
            assert_eq!(
                reconciled_state(&tracker, failed),
                TrackedDownloadState::Failed
            );
        }
    }

    #[test]
    fn reconcile_leaves_an_imported_seeding_row_parked() {
        for _ in 0..64 {
            let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
            tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::Imported,
            ));
            let seeding = tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::ImportedSeeding,
            ));

            let applied = reconcile_duplicate_terminal_source_states(&mut tracker);

            assert!(applied.is_empty());
            assert_eq!(
                reconciled_state(&tracker, seeding),
                TrackedDownloadState::ImportedSeeding
            );
        }
    }

    /// Pins today's behaviour: `Importing` is not import-settled, so an
    /// importing row of the same title is settled by its imported sibling.
    #[test]
    fn reconcile_settles_an_importing_row_next_to_an_imported_sibling_of_the_same_title() {
        for _ in 0..64 {
            let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
            tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::Imported,
            ));
            let importing = tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::Importing,
            ));

            let applied = reconcile_duplicate_terminal_source_states(&mut tracker);

            assert_eq!(applied, vec![(importing, TrackedDownloadState::Imported)]);
            assert_eq!(
                reconciled_state(&tracker, importing),
                TrackedDownloadState::Imported
            );
        }
    }

    #[test]
    fn reconcile_never_crosses_download_clients_for_the_same_item_id() {
        for _ in 0..64 {
            let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
            let mut imported = reconcile_row(Some("title-1"), TrackedDownloadState::Imported);
            imported.client_id = "client-b".to_string();
            imported.client_item.client_id = "client-b".to_string();
            tracker.insert_for_tests(imported);
            let mut live = reconcile_row(Some("title-1"), TrackedDownloadState::Downloading);
            live.client_id = "client-a".to_string();
            live.client_item.client_id = "client-a".to_string();
            let live = tracker.insert_for_tests(live);

            let applied = reconcile_duplicate_terminal_source_states(&mut tracker);

            assert!(applied.is_empty());
            assert_eq!(
                reconciled_state(&tracker, live),
                TrackedDownloadState::Downloading
            );
        }
    }

    #[test]
    fn reconcile_applies_nothing_on_a_second_pass() {
        for _ in 0..64 {
            let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
            tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::Imported,
            ));
            let live = tracker.insert_for_tests(reconcile_row(
                Some("title-1"),
                TrackedDownloadState::Downloading,
            ));

            assert_eq!(
                reconcile_duplicate_terminal_source_states(&mut tracker),
                vec![(live, TrackedDownloadState::Imported)]
            );
            assert!(reconcile_duplicate_terminal_source_states(&mut tracker).is_empty());
        }
    }

    #[test]
    fn manual_import_record_overlay_includes_transfer_progress() {
        let mut queue_item = item("job-1", DownloadQueueState::Completed);
        let record = ImportRecord {
            id: "import-1".to_string(),
            source_client_id: Some("client-1".to_string()),
            source_system: "weaver".to_string(),
            source_ref: "job-1".to_string(),
            import_type: ImportType::ManualImport,
            status: ImportStatus::Processing,
            payload_json: "{}".to_string(),
            result_json: None,
            download_id: None,
            import_transfer_phase: Some(ImportTransferPhase::Copying),
            import_transfer_bytes: Some(524_288),
            import_transfer_total_bytes: Some(1_048_576),
            import_transfer_started_at: Some("2026-06-17T12:00:00Z".to_string()),
            import_transfer_updated_at: Some("2026-06-17T12:00:01Z".to_string()),
            started_at: None,
            finished_at: None,
            created_at: "2026-06-17T12:00:00Z".to_string(),
            updated_at: "2026-06-17T12:00:01Z".to_string(),
        };

        apply_import_record_to_queue_item(&mut queue_item, &record);

        assert_eq!(queue_item.import_status, Some(ImportStatus::Processing));
        assert_eq!(
            queue_item.import_transfer_phase,
            Some(ImportTransferPhase::Copying)
        );
        assert_eq!(queue_item.import_transfer_bytes, Some(524_288));
        assert_eq!(queue_item.import_transfer_total_bytes, Some(1_048_576));
        assert_eq!(
            queue_item.import_transfer_started_at.as_deref(),
            Some("2026-06-17T12:00:00Z")
        );
        assert_eq!(
            queue_item.import_transfer_updated_at.as_deref(),
            Some("2026-06-17T12:00:01Z")
        );
        assert_eq!(
            queue_item.imported_at.as_deref(),
            Some("2026-06-17T12:00:01Z")
        );
    }

    fn client_config(
        id: &str,
        name: &str,
        client_type: &str,
        priority: i64,
    ) -> DownloadClientConfig {
        DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            is_enabled: true,
            status: DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            client_priority: priority,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            proxy_config_id: None,
        }
    }

    fn submission_for_client(client_id: &str, client_type: &str) -> DownloadSubmission {
        DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title-1".to_string(),
            facet: "anime".to_string(),
            download_client_id: Some(client_id.to_string()),
            download_client_type: client_type.to_string(),
            download_client_item_id: "hash-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: None,
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            purpose: DownloadSubmissionPurpose::Standard,
            scope: crate::SubmissionScope::Title,
        }
    }

    /// A client-observed row often knows only its provider type — the plugin
    /// pollers report `client_id == client_type` — and with two configured
    /// clients of the same type the unique-type canonicalization cannot pick
    /// one, so the row rendered "rtorrent" where the operator expects the
    /// configured name. The submission remembers the routed config id;
    /// enrichment restores it and the follow-up canonicalize resolves the name.
    #[test]
    fn submission_enrichment_restores_the_configured_client_identity() {
        let configs = vec![
            client_config("cfg-identity", "E2E rTorrent Identity", "rtorrent", 1),
            client_config("cfg-cleanup", "E2E rTorrent Seed Cleanup", "rtorrent", 2),
        ];

        let mut degraded = item("hash-1", DownloadQueueState::Completed);
        degraded.client_id = "rtorrent".to_string();
        degraded.client_name = "rtorrent".to_string();
        degraded.client_type = "rtorrent".to_string();

        apply_submission_to_queue_item(
            &mut degraded,
            &submission_for_client("cfg-cleanup", "rtorrent"),
        );
        assert_eq!(degraded.client_id, "cfg-cleanup");
        assert!(
            degraded.client_name.is_empty(),
            "a type-derived stand-in name must not survive identity recovery"
        );

        let mut items = vec![degraded];
        canonicalize_download_queue_item_clients(&mut items, &configs);
        assert_eq!(items[0].client_name, "E2E rTorrent Seed Cleanup");
        assert_eq!(items[0].client_type, "rtorrent");

        // An item that already names a real config keeps it: the submission is
        // a fallback, never an override.
        let mut exact = item("hash-1", DownloadQueueState::Completed);
        exact.client_id = "cfg-identity".to_string();
        exact.client_name = "E2E rTorrent Identity".to_string();
        exact.client_type = "rtorrent".to_string();
        apply_submission_to_queue_item(
            &mut exact,
            &submission_for_client("cfg-cleanup", "rtorrent"),
        );
        assert_eq!(exact.client_id, "cfg-identity");
        assert_eq!(exact.client_name, "E2E rTorrent Identity");
    }

    #[test]
    fn dedupe_download_queue_items_merges_duplicate_client_job_ids() {
        let mut first = item("job-1", DownloadQueueState::Completed);
        first.import_error_message = Some("failed to import".to_string());
        let mut second = item("job-1", DownloadQueueState::Completed);
        second.title_id = Some("title-1".to_string());

        let deduped = dedupe_download_queue_items(vec![
            first,
            second,
            item("job-2", DownloadQueueState::Queued),
        ]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].download_client_item_id, "job-1");
        assert_eq!(deduped[0].title_id.as_deref(), Some("title-1"));
        assert_eq!(
            deduped[0].import_error_message.as_deref(),
            Some("failed to import")
        );
    }

    #[test]
    fn dedupe_download_queue_items_keeps_same_native_id_from_different_clients() {
        let mut first = item("job-1", DownloadQueueState::Queued);
        first.client_id = "client-1".to_string();
        let mut second = item("job-1", DownloadQueueState::Queued);
        second.client_id = "client-2".to_string();

        let deduped = dedupe_download_queue_items(vec![first, second]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].client_id, "client-1");
        assert_eq!(deduped[1].client_id, "client-2");
    }

    #[test]
    fn canonicalize_download_queue_items_maps_legacy_type_id_to_unique_config() {
        let configs = vec![client_config("Weaver", "Weaver", "weaver", 1)];
        let mut queue_item = item("job-1", DownloadQueueState::Downloading);
        queue_item.client_id = "weaver".to_string();
        queue_item.client_name = "weaver".to_string();
        queue_item.client_type = "weaver".to_string();
        let mut items = vec![queue_item];

        canonicalize_download_queue_item_clients(&mut items, &configs);

        assert_eq!(items[0].client_id, "Weaver");
        assert_eq!(items[0].client_name, "Weaver");
        assert_eq!(items[0].client_type, "weaver");
        assert_eq!(download_queue_client_filter_key(&items[0]), "Weaver");

        let options = collect_download_client_filter_options(&items);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].client_id, "Weaver");
        assert_eq!(options[0].client_name, "Weaver");
    }

    #[test]
    fn canonicalize_download_queue_items_does_not_guess_for_multiple_same_type_clients() {
        let configs = vec![
            client_config("weaver-primary", "Weaver Primary", "weaver", 1),
            client_config("weaver-secondary", "Weaver Secondary", "weaver", 2),
        ];
        let mut queue_item = item("job-1", DownloadQueueState::Downloading);
        queue_item.client_id = "weaver".to_string();
        queue_item.client_name = "weaver".to_string();
        queue_item.client_type = "weaver".to_string();
        let mut items = vec![queue_item];

        canonicalize_download_queue_item_clients(&mut items, &configs);

        assert_eq!(items[0].client_id, "weaver");
        assert_eq!(items[0].client_name, "weaver");
        assert_eq!(download_queue_client_filter_key(&items[0]), "weaver");
    }

    #[test]
    fn synthetic_tracked_snapshot_queue_item_uses_tracked_client_identity_hint() {
        let config = client_config("Weaver", "Weaver", "weaver", 1);
        let mut client_item = item("job-1", DownloadQueueState::Completed);
        client_item.client_id.clear();
        client_item.client_name.clear();
        client_item.client_type.clear();
        let tracked = crate::tracked_downloads::TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: "Weaver:job-1".to_string(),
            client_id: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::Imported,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: false,
            import_attempted: true,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            import_execution_retry: None,
            import_hold: None,
            skip_reacquire_on_failure: false,
            burned_by_import_gate: false,
            snapshot_missing_since: None,
            retained_in_client_after_cleanup: false,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        let mut items = vec![
            synthetic_tracked_snapshot_queue_item(&metadata, Some(&config))
                .expect("synthetic tracked snapshot item"),
        ];
        canonicalize_download_queue_item_clients(&mut items, &[config]);

        assert_eq!(items[0].client_id, "Weaver");
        assert_eq!(items[0].client_name, "Weaver");
        assert_eq!(items[0].client_type, "weaver");
        assert_eq!(download_queue_client_filter_key(&items[0]), "Weaver");
    }

    #[test]
    fn imported_seeding_tracked_rows_stay_visible_in_the_download_queue() {
        // Regression rail: a tracked state that the queue projection filters
        // out is a download that has silently vanished from the operator's
        // view. `ImportedSeeding` rows are held back from removal precisely so
        // they can be seen.
        let config = client_config("qBittorrent", "qBittorrent", "qbittorrent", 1);
        let client_item = item("torrent-1", DownloadQueueState::Completed);
        let tracked = crate::tracked_downloads::TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: "qbittorrent:torrent-1".to_string(),
            client_id: "qBittorrent".to_string(),
            client_type: "qbittorrent".to_string(),
            client_item,
            completed_source: None,
            state: TrackedDownloadState::ImportedSeeding,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("movie".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
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
            retained_in_client_after_cleanup: false,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        let projected = synthetic_tracked_snapshot_queue_item(&metadata, Some(&config))
            .expect("an imported-but-still-seeding row must be projected into the queue");

        assert_eq!(
            projected.tracked_state,
            Some(TrackedDownloadState::ImportedSeeding)
        );
        assert_eq!(projected.state, DownloadQueueState::Completed);
        assert_eq!(projected.import_status, Some(ImportStatus::Completed));
        assert_eq!(projected.progress_percent, 100);
        assert_eq!(
            derive_download_queue_display_state(&projected),
            DownloadDisplayState::ImportedSeeding
        );
    }

    #[test]
    fn apply_tracked_download_queue_metadata_backfills_missing_facet() {
        let mut queue_item = item("job-1", DownloadQueueState::Completed);
        let tracked = crate::tracked_downloads::TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
            completed_source: None,
            state: TrackedDownloadState::ImportBlocked,
            status: TrackedDownloadStatus::Warning,
            status_messages: vec!["needs manual import".to_string()],
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::TitleParse,
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
            retained_in_client_after_cleanup: false,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(queue_item.title_id.as_deref(), Some("title-1"));
        assert_eq!(queue_item.facet.as_deref(), Some("series"));
        assert_eq!(
            queue_item.tracked_state,
            Some(TrackedDownloadState::ImportBlocked)
        );
        assert_eq!(
            queue_item.tracked_status,
            Some(TrackedDownloadStatus::Warning)
        );
        assert_eq!(
            queue_item.tracked_match_type,
            Some(TitleMatchType::TitleParse)
        );
    }

    #[test]
    fn import_blocked_projection_keeps_a_live_manual_import_and_drops_a_finished_one() {
        // A queued/running manual import on a blocked row is live state the
        // row must render (Pending/Importing, actions greyed); a finished record is
        // not — the block stays authoritative over stale Failed/Skipped/
        // Completed statuses.
        fn blocked_tracked(
            queue_item: &DownloadQueueItem,
        ) -> crate::tracked_downloads::TrackedDownload {
            crate::tracked_downloads::TrackedDownload {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                id: "weaver:job-1".to_string(),
                client_id: "client-1".to_string(),
                client_type: "weaver".to_string(),
                client_item: queue_item.clone(),
                completed_source: None,
                state: TrackedDownloadState::ImportBlocked,
                status: TrackedDownloadStatus::Warning,
                status_messages: vec!["needs manual import".to_string()],
                title_id: Some("title-1".to_string()),
                facet: Some("series".to_string()),
                source_title: None,
                indexer: None,
                added_at: None,
                notified_manual_interaction: false,
                match_type: TitleMatchType::TitleParse,
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
                retained_in_client_after_cleanup: false,
            }
        }

        for live in [
            ImportStatus::Pending,
            ImportStatus::Running,
            ImportStatus::Processing,
        ] {
            let mut queue_item = item("job-1", DownloadQueueState::Completed);
            queue_item.import_status = Some(live);
            let metadata = tracked_download_queue_snapshot(&blocked_tracked(&queue_item));
            apply_tracked_download_activity_projection(&mut queue_item, &metadata);
            assert_eq!(
                queue_item.import_status,
                Some(live),
                "{live:?} must survive the block"
            );
            assert_eq!(
                derive_download_queue_display_state(&queue_item),
                if live == ImportStatus::Pending {
                    DownloadDisplayState::ImportPending
                } else {
                    DownloadDisplayState::Importing
                },
                "{live:?} renders with its active lifecycle state"
            );
            assert!(
                build_download_queue_status_detail(&queue_item).is_empty(),
                "{live:?} must not repeat the superseded manual-import warning"
            );
        }

        for finished in [
            ImportStatus::Failed,
            ImportStatus::Skipped,
            ImportStatus::Completed,
        ] {
            let mut queue_item = item("job-1", DownloadQueueState::Completed);
            queue_item.import_status = Some(finished);
            let metadata = tracked_download_queue_snapshot(&blocked_tracked(&queue_item));
            apply_tracked_download_activity_projection(&mut queue_item, &metadata);
            assert_eq!(
                queue_item.import_status, None,
                "{finished:?} is cleared by the block"
            );
            assert_eq!(
                derive_download_queue_display_state(&queue_item),
                DownloadDisplayState::ImportBlocked
            );
        }
    }

    #[test]
    fn failed_source_state_stays_out_of_import_bucket() {
        let mut queue_item = item("job-failed", DownloadQueueState::Failed);
        queue_item.import_status = Some(ImportStatus::Failed);
        queue_item.tracked_state = Some(TrackedDownloadState::ImportBlocked);
        queue_item.import_error_message = Some("manual import failed".to_string());

        let classified = classify_download_queue_item(&queue_item);

        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::Failed
        );
        assert_eq!(classified.bucket, DownloadQueueBucket::HistoryFailed);
    }

    #[test]
    fn warning_state_stays_in_the_activity_bucket_with_its_message() {
        let mut queue_item = item("job-warning", DownloadQueueState::Warning);
        queue_item.attention_required = true;
        queue_item.attention_reason = Some("files are missing from the save path".to_string());

        let classified = classify_download_queue_item(&queue_item);

        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::Warning
        );
        assert_eq!(classified.bucket, DownloadQueueBucket::Activity);
        assert_eq!(
            queue_item.attention_reason.as_deref(),
            Some("files are missing from the save path"),
            "the client's message is what makes the warning actionable"
        );

        // The queue page asks for an explicit list of activity filters, so the
        // row is only reachable if it answers to one of them.
        assert!(crate::matches_download_activity_filter(
            &queue_item,
            crate::DownloadActivityFilter::Warning
        ));
        assert!(crate::matches_download_activity_filter(
            &queue_item,
            crate::DownloadActivityFilter::All
        ));
        assert!(!crate::matches_download_activity_filter(
            &queue_item,
            crate::DownloadActivityFilter::Downloading
        ));
    }

    fn warned_client_row(id: &str) -> DownloadQueueItem {
        let mut queue_item = item(id, DownloadQueueState::Warning);
        queue_item.attention_required = true;
        queue_item.attention_reason = Some("files are missing from the save path".to_string());
        queue_item
    }

    fn tracked_in_state(
        queue_item: &DownloadQueueItem,
        state: TrackedDownloadState,
    ) -> crate::tracked_downloads::TrackedDownload {
        crate::tracked_downloads::TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: format!("qbittorrent:{}", queue_item.id),
            client_id: "client-1".to_string(),
            client_type: "qbittorrent".to_string(),
            client_item: queue_item.clone(),
            completed_source: None,
            state,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("movie".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
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
            retained_in_client_after_cleanup: false,
        }
    }

    #[test]
    fn warning_state_never_preempts_a_live_import_overlay() {
        // Run the overlay itself, not a hand-built row: an import that is
        // actually moving files is the more specific answer and has to keep the
        // row, exactly as it did before `Warning` existed.
        for (tracked_state, expected) in [
            (
                TrackedDownloadState::ImportPending,
                DownloadDisplayState::ImportPending,
            ),
            (
                TrackedDownloadState::Importing,
                DownloadDisplayState::Importing,
            ),
            (
                TrackedDownloadState::ImportBlocked,
                DownloadDisplayState::ImportBlocked,
            ),
        ] {
            let mut queue_item = warned_client_row("job-warning-importing");
            let metadata =
                tracked_download_queue_snapshot(&tracked_in_state(&queue_item, tracked_state));
            apply_tracked_download_activity_projection(&mut queue_item, &metadata);

            assert_eq!(
                derive_download_queue_display_state(&queue_item),
                expected,
                "{tracked_state:?} overlays the client warning"
            );
        }
    }

    #[test]
    fn a_settled_import_still_surfaces_a_live_client_warning() {
        // The scenario this workstream exists for: a torrent that hits
        // `error` / `missingFiles` while it is seeding out its goal after the
        // import finished. The overlay used to repaint it `Completed`, so it
        // read as perfectly healthy while the client was stuck.
        for tracked_state in [
            TrackedDownloadState::Imported,
            TrackedDownloadState::ImportedSeeding,
        ] {
            let mut queue_item = warned_client_row("job-warning-seeding");
            let metadata =
                tracked_download_queue_snapshot(&tracked_in_state(&queue_item, tracked_state));
            apply_tracked_download_activity_projection(&mut queue_item, &metadata);

            assert_eq!(
                queue_item.state,
                DownloadQueueState::Warning,
                "{tracked_state:?} must not repaint a live client warning"
            );
            assert_eq!(
                derive_download_queue_display_state(&queue_item),
                DownloadDisplayState::Warning,
                "{tracked_state:?}"
            );
            assert_eq!(
                queue_item.attention_reason.as_deref(),
                Some("files are missing from the save path"),
                "{tracked_state:?}"
            );
            // The import is still settled: nothing may re-import it, and the
            // seeding gate keeps whatever hold it has.
            assert_eq!(
                queue_item.import_status,
                Some(ImportStatus::Completed),
                "{tracked_state:?}"
            );
            assert_eq!(queue_item.progress_percent, 100, "{tracked_state:?}");
            assert!(queue_item.imported_at.is_some(), "{tracked_state:?}");
        }
    }

    #[test]
    fn a_settled_import_without_a_warning_still_reads_as_completed() {
        let mut queue_item = item("job-imported", DownloadQueueState::Downloading);
        let metadata = tracked_download_queue_snapshot(&tracked_in_state(
            &queue_item,
            TrackedDownloadState::Imported,
        ));
        apply_tracked_download_activity_projection(&mut queue_item, &metadata);

        assert_eq!(queue_item.state, DownloadQueueState::Completed);
        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::Completed
        );
    }

    #[test]
    fn an_imported_seeding_row_remains_visible_in_activity() {
        let mut queue_item = item("job-imported-seeding", DownloadQueueState::Downloading);
        let metadata = tracked_download_queue_snapshot(&tracked_in_state(
            &queue_item,
            TrackedDownloadState::ImportedSeeding,
        ));
        apply_tracked_download_activity_projection(&mut queue_item, &metadata);

        assert_eq!(queue_item.state, DownloadQueueState::Completed);
        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::ImportedSeeding
        );
        assert!(crate::matches_download_activity_filter(
            &queue_item,
            crate::DownloadActivityFilter::Seeding
        ));
        assert!(crate::matches_download_activity_filter(
            &queue_item,
            crate::DownloadActivityFilter::All
        ));
    }

    fn manual_import_record(id: &str, status: ImportStatus, result_json: &str) -> ImportRecord {
        let now = Utc::now().to_rfc3339();
        ImportRecord {
            id: id.to_string(),
            source_client_id: Some("client-1".to_string()),
            source_system: "weaver".to_string(),
            source_ref: id.to_string(),
            import_type: ImportType::ManualImport,
            status,
            payload_json: "{}".to_string(),
            result_json: Some(result_json.to_string()),
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
        }
    }

    /// The user-reported failure: a download blocked with `no_video_files`, an
    /// operator-queued manual import that failed against it, and a row that
    /// went straight back to the original block with `importStatus: null`.
    /// The block is still the tracker's state, but the manual failure happened
    /// after it and is the only record of what went wrong.
    #[test]
    fn a_failed_manual_import_survives_the_import_blocked_projection() {
        let mut queue_item = item("job-manual-failed", DownloadQueueState::Completed);
        queue_item.title_id = Some("title-1".to_string());
        queue_item.facet = Some("series".to_string());
        let record = manual_import_record(
            "job-manual-failed",
            ImportStatus::Failed,
            r#"{"import_id":"import-1","client_type":"weaver","download_client_item_id":"job-manual-failed","title_id":"title-1","status":"failed","error_code":"permission_denied","error_message":"permission denied writing to the library root","file_results":[],"completed_at":"2026-09-17T00:00:00Z"}"#,
        );
        apply_import_record_to_queue_item(&mut queue_item, &record);
        assert_eq!(queue_item.import_type, Some(ImportType::ManualImport));

        let mut tracked = tracked_in_state(&queue_item, TrackedDownloadState::ImportBlocked);
        tracked.status = TrackedDownloadStatus::Warning;
        tracked.status_messages =
            vec!["no_video_files: no importable video found after 3 unchanged checks".to_string()];
        let metadata = tracked_download_queue_snapshot(&tracked);
        apply_tracked_download_activity_projection(&mut queue_item, &metadata);

        assert_eq!(queue_item.import_status, Some(ImportStatus::Failed));
        assert_eq!(
            queue_item.import_error_message.as_deref(),
            Some("permission denied writing to the library root")
        );
        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::ImportFailed
        );
        assert_eq!(
            classify_download_queue_item(&queue_item).import_filter,
            Some(crate::DownloadImportFilter::Failed)
        );
    }

    /// The reason the projection clears a finished import at all: an automatic
    /// import that skipped is *why* the tracker blocked the download, so
    /// letting it through would delete the Import Blocked state entirely.
    #[test]
    fn a_stale_automatic_import_result_still_loses_to_the_block() {
        let mut queue_item = item("job-auto-skipped", DownloadQueueState::Completed);
        queue_item.import_status = Some(ImportStatus::Skipped);
        queue_item.import_type = Some(ImportType::SeriesDownload);

        let metadata = tracked_download_queue_snapshot(&tracked_in_state(
            &queue_item,
            TrackedDownloadState::ImportBlocked,
        ));
        apply_tracked_download_activity_projection(&mut queue_item, &metadata);

        assert_eq!(queue_item.import_status, None);
        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::ImportBlocked
        );
    }

    /// A manual import result carries its reason on the failed mapping when the
    /// run itself recorded no summary; history and the queue row both read it
    /// through the same overlay.
    #[test]
    fn a_manual_import_overlay_falls_back_to_the_failed_file_result() {
        let record = manual_import_record(
            "job-file-failure",
            ImportStatus::Failed,
            r#"{"import_id":"import-2","client_type":"weaver","download_client_item_id":"job-file-failure","title_id":"title-1","status":"failed","error_code":null,"error_message":null,"file_results":[{"file_path":"/downloads/Harbor.Pals.S01E02.mkv","success":false,"skipped":false,"dest_path":null,"error_code":"io_failed","error_message":"failed to move the file into the library"}],"completed_at":"2026-09-17T00:00:00Z"}"#,
        );

        let overlay = import_record_result_overlay(&record);

        assert_eq!(
            overlay.error_message.as_deref(),
            Some("failed to move the file into the library")
        );
        assert_eq!(
            overlay.error_code,
            Some(scryer_domain::ImportErrorCode::IoFailed)
        );
        assert_eq!(
            overlay.source_path.as_deref(),
            Some("/downloads/Harbor.Pals.S01E02.mkv")
        );
        assert_eq!(overlay.title_id.as_deref(), Some("title-1"));
    }

    #[test]
    fn import_actions_follow_the_state_the_download_is_actually_in() {
        // A blocked series download offers the mapping dialog, plus the
        // tracked-state actions the activity row has always shown.
        let mut blocked = item("job-actions-blocked", DownloadQueueState::Completed);
        blocked.title_id = Some("title-1".to_string());
        blocked.facet = Some("series".to_string());
        blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
        let actions = derive_download_queue_import_actions(&blocked);
        assert!(actions.manual_import_interactive);
        assert!(!actions.manual_import_direct);
        assert!(actions.assign_title);
        assert!(actions.ignore);
        assert!(actions.mark_failed);

        // A movie imports without the dialog.
        let mut movie = blocked.clone();
        movie.facet = Some("MOVIE".to_string());
        let actions = derive_download_queue_import_actions(&movie);
        assert!(actions.manual_import_direct);
        assert!(!actions.manual_import_interactive);

        // Without a title there is nothing to import into; assigning one is
        // still offered.
        let mut unassigned = blocked.clone();
        unassigned.title_id = None;
        let actions = derive_download_queue_import_actions(&unassigned);
        assert!(!actions.manual_import_interactive);
        assert!(!actions.manual_import_direct);
        assert!(actions.assign_title);

        // A manual import that is already running owns the row.
        let mut importing = blocked.clone();
        importing.import_status = Some(ImportStatus::Running);
        importing.tracked_state = Some(TrackedDownloadState::Importing);
        let actions = derive_download_queue_import_actions(&importing);
        assert_eq!(actions, DownloadImportActions::default());

        // A pending import is not something to start another import against.
        let mut pending = blocked.clone();
        pending.import_status = Some(ImportStatus::Pending);
        let actions = derive_download_queue_import_actions(&pending);
        assert!(!actions.manual_import_interactive);
        assert!(!actions.manual_import_direct);

        // A finished download nothing ever imported is what a title overview
        // offers its Manual Import button for.
        let mut never_imported = item("job-actions-completed", DownloadQueueState::Completed);
        never_imported.title_id = Some("title-1".to_string());
        never_imported.facet = Some("movie".to_string());
        assert!(derive_download_queue_import_actions(&never_imported).manual_import_direct);

        // One that imported successfully is not.
        let mut imported = never_imported.clone();
        imported.import_status = Some(ImportStatus::Completed);
        assert!(!derive_download_queue_import_actions(&imported).manual_import_direct);

        // A download still running offers nothing.
        let mut downloading = never_imported.clone();
        downloading.state = DownloadQueueState::Downloading;
        assert_eq!(
            derive_download_queue_import_actions(&downloading),
            DownloadImportActions::default()
        );
    }

    #[test]
    fn an_import_failure_outranks_the_imported_seeding_projection() {
        let mut queue_item = item("job-seeding-import-failed", DownloadQueueState::Completed);
        queue_item.tracked_state = Some(TrackedDownloadState::ImportedSeeding);
        queue_item.import_status = Some(ImportStatus::Failed);

        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::ImportFailed
        );
    }

    #[test]
    fn a_terminal_failure_outranks_a_warning_when_observations_disagree() {
        let warned = item("job-1", DownloadQueueState::Warning);
        let mut failed = item("job-1", DownloadQueueState::Failed);
        failed.progress_percent = 0;

        let merged = dedupe_download_queue_items(vec![warned, failed]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].state, DownloadQueueState::Failed);
    }

    #[test]
    fn ignored_state_overrides_a_stale_failed_import_overlay() {
        let mut queue_item = item("job-ignored", DownloadQueueState::Failed);
        queue_item.import_status = Some(ImportStatus::Failed);
        queue_item.tracked_state = Some(TrackedDownloadState::Ignored);

        let classified = classify_download_queue_item(&queue_item);

        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::Ignored
        );
        assert_eq!(classified.bucket, DownloadQueueBucket::HistorySuccess);
    }

    #[test]
    fn source_provider_label_prefers_a_name_and_never_returns_a_url() {
        assert_eq!(
            source_provider_label(
                Some("Fixture Indexer"),
                Some("https://indexer.example/api?t=get&apikey=secret"),
            )
            .as_deref(),
            Some("Fixture Indexer")
        );
        assert_eq!(
            source_provider_label(
                None,
                Some("https://indexer.example/api?t=get&apikey=secret"),
            )
            .as_deref(),
            Some("indexer.example")
        );
        assert_eq!(
            source_provider_label(None, Some("Historic RSS Indexer")).as_deref(),
            Some("Historic RSS Indexer")
        );
    }

    #[test]
    fn apply_tracked_download_queue_metadata_prefers_source_release_title() {
        let mut queue_item = item("job-1", DownloadQueueState::Downloading);
        queue_item.title_name = "Ironclad".to_string();
        let tracked = crate::tracked_downloads::TrackedDownload {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
            completed_source: None,
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("movie".to_string()),
            source_title: Some("Ironclad.1997.2160p.UHD.BluRay.x265-GRP".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
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
            retained_in_client_after_cleanup: false,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(
            queue_item.title_name,
            "Ironclad.1997.2160p.UHD.BluRay.x265-GRP"
        );
    }

    // ── queue seeding progress ─────────────────────────────────────────────

    fn seeding_item(snapshot: scryer_domain::DownloadSeedingSnapshot) -> DownloadQueueItem {
        let mut item = item("torrent-1", DownloadQueueState::Completed);
        item.client_type = "qbittorrent".to_string();
        item.seeding = Some(snapshot);
        item
    }

    #[test]
    fn a_usenet_row_has_no_seeding_state() {
        // `can_remove` is reported by usenet clients too, so it alone must not
        // make a row look like a torrent.
        let item = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(true),
            can_move_files: Some(true),
            ..Default::default()
        });
        assert_eq!(crate::derive_download_seeding_state(&item), None);

        let mut bare = item.clone();
        bare.seeding = None;
        assert_eq!(crate::derive_download_seeding_state(&bare), None);
    }

    #[test]
    fn a_torrent_still_downloading_is_not_reported_as_seeding() {
        let mut item = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            // Every audited plugin reports `Some(false)` for an incomplete
            // payload; that means "not finished", not "still seeding".
            can_remove: Some(false),
            can_move_files: Some(false),
            seed_ratio: Some(0.0),
            ..Default::default()
        });
        item.state = DownloadQueueState::Downloading;
        item.progress_percent = 42;
        assert_eq!(
            crate::derive_download_seeding_state(&item),
            Some(crate::DownloadSeedingState::None)
        );
    }

    #[test]
    fn seeding_progress_reads_the_goal_beside_the_observation() {
        let unmet = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(0.8),
            seed_goal_ratio: Some(2.0),
            ..Default::default()
        });
        assert_eq!(
            crate::derive_download_seeding_state(&unmet),
            Some(crate::DownloadSeedingState::Seeding)
        );

        let met = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(2.1),
            seed_goal_ratio: Some(2.0),
            ..Default::default()
        });
        assert_eq!(
            crate::derive_download_seeding_state(&met),
            Some(crate::DownloadSeedingState::GoalMet)
        );
    }

    #[test]
    fn a_client_verdict_alone_still_drives_the_badge_when_no_profile_applied() {
        let done = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(true),
            seed_ratio: Some(1.1),
            ..Default::default()
        });
        assert_eq!(
            crate::derive_download_seeding_state(&done),
            Some(crate::DownloadSeedingState::GoalMet)
        );

        let unknown = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            can_remove: None,
            seed_ratio: Some(1.1),
            ..Default::default()
        });
        assert_eq!(
            crate::derive_download_seeding_state(&unknown),
            Some(crate::DownloadSeedingState::Seeding)
        );
    }

    #[test]
    fn the_private_rail_and_seed_forever_are_visible_in_the_queue() {
        let private = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(true),
            is_private: Some(true),
            seed_ratio: Some(4.0),
            ..Default::default()
        });
        assert_eq!(
            crate::derive_download_seeding_state(&private),
            Some(crate::DownloadSeedingState::HeldPrivate)
        );

        let forever = seeding_item(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(true),
            seed_ratio: Some(4.0),
            seed_goal_ratio: Some(1.0),
            never_remove: true,
            ..Default::default()
        });
        assert_eq!(
            crate::derive_download_seeding_state(&forever),
            Some(crate::DownloadSeedingState::NeverRemove)
        );
    }

    #[test]
    fn a_live_row_without_an_observation_inherits_the_tracked_one() {
        let mut queue_item = item("job-1", DownloadQueueState::Completed);
        assert!(queue_item.seeding.is_none());
        let mut tracked = tracked_for_dispatch("qbittorrent:job-1");
        tracked.client_item.seeding = Some(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(false),
            seed_ratio: Some(1.1),
            seed_goal_ratio: Some(2.0),
            ..Default::default()
        });
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(
            queue_item
                .seeding
                .as_ref()
                .and_then(|seeding| seeding.seed_ratio),
            Some(1.1)
        );

        // A live observation is never replaced by the tracked copy.
        let mut fresher = item("job-1", DownloadQueueState::Completed);
        fresher.seeding = Some(scryer_domain::DownloadSeedingSnapshot {
            seed_ratio: Some(2.6),
            ..Default::default()
        });
        apply_tracked_download_queue_metadata(&mut fresher, &metadata);
        assert_eq!(
            fresher
                .seeding
                .as_ref()
                .and_then(|seeding| seeding.seed_ratio),
            Some(2.6)
        );
    }

    #[test]
    fn a_row_parked_by_the_gate_reports_seeding_even_with_a_silent_client() {
        // `ImportedSeeding` only exists because the gate held this torrent, so
        // the row is a torrent regardless of what the client will admit to.
        let mut item = seeding_item(scryer_domain::DownloadSeedingSnapshot::default());
        item.tracked_state = Some(TrackedDownloadState::ImportedSeeding);
        assert_eq!(
            crate::derive_download_seeding_state(&item),
            Some(crate::DownloadSeedingState::Seeding)
        );
    }

    /// A deleted title's settled row and the live row of a re-grab of the same
    /// torrent, sharing one id string. Returns the tracker and both cache keys.
    fn settled_and_live_siblings() -> (
        crate::tracked_downloads::TrackedDownloadService,
        scryer_domain::download_identity::DownloadId,
        scryer_domain::download_identity::DownloadId,
    ) {
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let settled = tracker.insert_for_tests(reconcile_row(
            Some("title-1"),
            TrackedDownloadState::Imported,
        ));
        let live = tracker.insert_for_tests(reconcile_row(
            Some("title-2"),
            TrackedDownloadState::Downloading,
        ));
        (tracker, settled, live)
    }

    fn row_debug(
        tracker: &crate::tracked_downloads::TrackedDownloadService,
        download_id: scryer_domain::download_identity::DownloadId,
    ) -> String {
        format!(
            "{:?}",
            tracker
                .get_by_download_id(download_id)
                .expect("tracked download should remain cached")
        )
    }

    async fn run_tracked_command(
        app: &crate::AppUseCase,
        actor: &scryer_domain::User,
        tracker: &mut crate::tracked_downloads::TrackedDownloadService,
        command: crate::tracked_downloads::TrackedDownloadCommand,
    ) {
        let mut in_flight = std::collections::HashSet::new();
        let (work_tx, _work_rx) = tokio::sync::mpsc::unbounded_channel();
        super::handle_tracked_download_command(
            app,
            actor,
            tracker,
            &mut in_flight,
            &work_tx,
            command,
        )
        .await;
    }

    #[test]
    fn resolve_tracked_command_target_never_falls_back_past_a_canonical_id() {
        for _ in 0..16 {
            let (tracker, settled, live) = settled_and_live_siblings();

            // A cached canonical id names its own row, whichever shares the string.
            assert_eq!(
                super::resolve_tracked_command_target_for_download(
                    &tracker,
                    Some(&settled),
                    SHARED_TORRENT_ID,
                ),
                (SHARED_TORRENT_ID.to_string(), Some(settled))
            );
            assert_eq!(
                super::resolve_tracked_command_target_for_download(
                    &tracker,
                    Some(&live),
                    SHARED_TORRENT_ID,
                ),
                (SHARED_TORRENT_ID.to_string(), Some(live))
            );

            // An uncached canonical id has no target, even though the id string
            // it came with matches two cached rows.
            assert_eq!(
                super::resolve_tracked_command_target_for_download(
                    &tracker,
                    Some(&scryer_domain::download_identity::DownloadId::new()),
                    SHARED_TORRENT_ID,
                ),
                (SHARED_TORRENT_ID.to_string(), None)
            );

            // Without a canonical id the string path runs, and prefers the row
            // that is not import-settled.
            assert_eq!(
                super::resolve_tracked_command_target_for_download(
                    &tracker,
                    None,
                    SHARED_TORRENT_ID,
                ),
                (SHARED_TORRENT_ID.to_string(), Some(live))
            );
            assert_eq!(
                super::resolve_tracked_command_target_for_download(
                    &tracker,
                    None,
                    "download:client-a:qbittorrent:unknown",
                ),
                ("download:client-a:qbittorrent:unknown".to_string(), None)
            );
        }
    }

    #[test]
    fn resolve_cached_download_id_prefers_the_row_that_is_not_import_settled() {
        for settled_state in [
            TrackedDownloadState::Imported,
            TrackedDownloadState::ImportedSeeding,
            TrackedDownloadState::Failed,
            TrackedDownloadState::Ignored,
        ] {
            for _ in 0..16 {
                let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
                tracker.insert_for_tests(reconcile_row(Some("title-1"), settled_state));
                let live = tracker.insert_for_tests(reconcile_row(
                    Some("title-2"),
                    TrackedDownloadState::Downloading,
                ));
                assert_eq!(
                    tracker.resolve_cached_download_id(SHARED_TORRENT_ID),
                    Some(live)
                );
                assert_eq!(
                    tracker.resolve_cached_id(SHARED_TORRENT_ID).as_deref(),
                    Some(SHARED_TORRENT_ID)
                );
            }
        }
    }

    #[tokio::test]
    async fn background_work_result_merges_into_its_own_row_and_leaves_the_sibling_untouched() {
        let (app, _) = crate::lib_tests::bootstrap();
        for _ in 0..16 {
            let (mut tracker, settled, live) = settled_and_live_siblings();
            let settled_before = row_debug(&tracker, settled);

            let mut finished = tracker.get_by_download_id(live).expect("live row").clone();
            finished.state = TrackedDownloadState::ImportPending;
            finished.status = TrackedDownloadStatus::Warning;
            finished.status_messages = vec!["worker finished".to_string()];

            let mut in_flight = std::collections::HashSet::from([SHARED_TORRENT_ID.to_string()]);
            super::handle_tracked_download_background_work_result(
                &app,
                &mut tracker,
                &mut in_flight,
                super::TrackedDownloadBackgroundWorkResult {
                    download_id: live,
                    id: SHARED_TORRENT_ID.to_string(),
                    kind: TrackedDownloadBackgroundWorkKind::Import,
                    outcome: Ok(finished),
                    elapsed: std::time::Duration::ZERO,
                },
            )
            .await;

            assert!(in_flight.is_empty());
            let merged = tracker.get_by_download_id(live).expect("live row");
            assert_eq!(merged.state, TrackedDownloadState::ImportPending);
            assert_eq!(merged.status_messages, vec!["worker finished".to_string()]);
            assert_eq!(merged.title_id.as_deref(), Some("title-2"));
            assert_eq!(row_debug(&tracker, settled), settled_before);
        }
    }

    #[tokio::test]
    async fn history_retry_addresses_the_row_of_the_claimed_download_id() {
        use crate::tracked_downloads::TrackedDownloadCommand;
        let (app, actor) = crate::lib_tests::bootstrap();
        for _ in 0..8 {
            // The retry claimed the settled row's download, while the id
            // string alone would prefer its unsettled sibling.
            let (mut tracker, settled, live) = settled_and_live_siblings();
            let live_before = row_debug(&tracker, live);
            let mut in_flight = std::collections::HashSet::new();
            let (work_tx, _work_rx) = tokio::sync::mpsc::unbounded_channel();

            let (reply, response) = tokio::sync::oneshot::channel();
            super::handle_tracked_download_command(
                &app,
                &actor,
                &mut tracker,
                &mut in_flight,
                &work_tx,
                TrackedDownloadCommand::BeginHistoryRetry {
                    id: SHARED_TORRENT_ID.to_string(),
                    download_id: settled,
                    reply,
                },
            )
            .await;
            let snapshot = response
                .await
                .expect("reply")
                .expect("begin")
                .expect("snapshot of the claimed row");
            assert_eq!(snapshot.download_id, settled);
            assert!(in_flight.contains(SHARED_TORRENT_ID));

            let (reply, response) = tokio::sync::oneshot::channel();
            super::handle_tracked_download_command(
                &app,
                &actor,
                &mut tracker,
                &mut in_flight,
                &work_tx,
                TrackedDownloadCommand::PublishHistoryRetry {
                    id: SHARED_TORRENT_ID.to_string(),
                    download_id: settled,
                    reply,
                },
            )
            .await;
            response.await.expect("reply").expect("publish");
            assert_eq!(
                reconciled_state(&tracker, settled),
                TrackedDownloadState::Importing
            );
            assert_eq!(row_debug(&tracker, live), live_before);

            let mut finished = snapshot;
            finished.state = TrackedDownloadState::Imported;
            finished.status_messages = vec!["retry finished".to_string()];
            let (reply, response) = tokio::sync::oneshot::channel();
            super::handle_tracked_download_command(
                &app,
                &actor,
                &mut tracker,
                &mut in_flight,
                &work_tx,
                TrackedDownloadCommand::FinishHistoryRetry {
                    id: SHARED_TORRENT_ID.to_string(),
                    download_id: settled,
                    finished: Some(Box::new(finished)),
                    reply,
                },
            )
            .await;
            response.await.expect("reply").expect("finish");
            assert!(in_flight.is_empty());
            assert_eq!(
                tracker
                    .get_by_download_id(settled)
                    .expect("claimed row")
                    .status_messages,
                vec!["retry finished".to_string()]
            );
            assert_eq!(row_debug(&tracker, live), live_before);
        }
    }

    #[tokio::test]
    async fn operator_commands_act_on_the_live_row_when_a_settled_sibling_shares_its_id_string() {
        use crate::tracked_downloads::TrackedDownloadCommand;
        let (app, actor) = crate::lib_tests::bootstrap();
        for _ in 0..8 {
            // Mark failed. Nothing grabbed this synthetic torrent, so the arm
            // warns the row it resolved instead of failing it.
            let (mut tracker, settled, live) = settled_and_live_siblings();
            let settled_before = row_debug(&tracker, settled);
            let (reply, response) = tokio::sync::oneshot::channel();
            run_tracked_command(
                &app,
                &actor,
                &mut tracker,
                TrackedDownloadCommand::MarkFailed {
                    id: SHARED_TORRENT_ID.to_string(),
                    skip_reacquire: false,
                    reply,
                },
            )
            .await;
            response.await.expect("reply").expect("mark failed");
            let warned = tracker.get_by_download_id(live).expect("live row");
            assert_eq!(warned.status, TrackedDownloadStatus::Warning);
            assert!(
                warned
                    .status_messages
                    .iter()
                    .any(|message| message.contains("wasn't grabbed"))
            );
            assert_eq!(row_debug(&tracker, settled), settled_before);

            // Ignore.
            let (mut tracker, settled, live) = settled_and_live_siblings();
            let settled_before = row_debug(&tracker, settled);
            let (reply, response) = tokio::sync::oneshot::channel();
            run_tracked_command(
                &app,
                &actor,
                &mut tracker,
                TrackedDownloadCommand::Ignore {
                    id: SHARED_TORRENT_ID.to_string(),
                    reply,
                },
            )
            .await;
            response.await.expect("reply").expect("ignore");
            // Finalize either keeps an ignored row cached as Ignored or stops
            // tracking it; with this fixture it stops tracking it.
            assert!(
                tracker.get_by_download_id(live).is_none(),
                "the live row must be the one ignored"
            );
            assert_eq!(row_debug(&tracker, settled), settled_before);

            // Forget.
            let (mut tracker, settled, live) = settled_and_live_siblings();
            let settled_before = row_debug(&tracker, settled);
            let (reply, response) = tokio::sync::oneshot::channel();
            run_tracked_command(
                &app,
                &actor,
                &mut tracker,
                TrackedDownloadCommand::Forget {
                    id: SHARED_TORRENT_ID.to_string(),
                    reply,
                },
            )
            .await;
            response.await.expect("reply").expect("forget");
            assert!(tracker.get_by_download_id(live).is_none());
            assert_eq!(row_debug(&tracker, settled), settled_before);

            // Snapshot reports the live row.
            let (mut tracker, _, _) = settled_and_live_siblings();
            let (reply, response) = tokio::sync::oneshot::channel();
            run_tracked_command(
                &app,
                &actor,
                &mut tracker,
                TrackedDownloadCommand::Snapshot {
                    ids: vec![SHARED_TORRENT_ID.to_string()],
                    reply,
                },
            )
            .await;
            let snapshot = response.await.expect("reply");
            let reported = snapshot.get(SHARED_TORRENT_ID).expect("snapshot row");
            assert_eq!(reported.state, TrackedDownloadState::Downloading);
            assert_eq!(reported.title_id.as_deref(), Some("title-2"));
        }
    }
}
