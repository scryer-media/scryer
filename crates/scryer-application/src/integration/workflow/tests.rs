#[cfg(test)]
mod tests {
    use super::{
        DownloadQueueBucket, TrackedDownloadBackgroundWorkKind, TrackedDownloadWorkDrain,
        apply_manual_import_record_to_queue_item, apply_tracked_download_queue_metadata,
        canonicalize_download_queue_item_clients, classify_download_queue_item,
        collect_download_client_filter_options, dedupe_download_queue_items,
        derive_download_queue_display_state, download_queue_client_filter_key,
        prepare_next_tracked_download_background_work_dispatch,
        prepare_tracked_download_background_work_dispatch,
        reconcile_duplicate_terminal_source_states, synthetic_tracked_snapshot_queue_item,
        source_provider_label, tracked_download_queue_snapshot,
    };
    use crate::DownloadDisplayState;
    use chrono::{Duration, Utc};
    use scryer_domain::{
        DownloadClientConfig, DownloadClientStatus, DownloadQueueItem, DownloadQueueState,
        ImportRecord, ImportStatus, ImportTransferPhase, ImportType, TitleMatchType,
        TrackedDownloadState, TrackedDownloadStatus,
    };

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
        }
    }

    fn tracked_for_dispatch(id: &str) -> crate::tracked_downloads::TrackedDownload {
        let client_item = item("job-1", DownloadQueueState::Completed);
        crate::tracked_downloads::TrackedDownload {
            id: id.to_string(),
            client_id: client_item.client_id.clone(),
            client_type: client_item.client_type.clone(),
            client_item,
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
            foreign_import_classification: None,
            skip_reacquire_on_failure: false,
            snapshot_missing_since: None,
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
        tracker.insert_for_tests(tracked);

        assert!(prepare_tracked_download_background_work_dispatch(&mut tracker, id).is_none());
        assert_eq!(
            tracker.find(id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );

        tracker
            .find_mut(id)
            .expect("tracked download should remain cached")
            .no_video_import_retry = Some(no_video_retry_state(Utc::now() - Duration::seconds(1)));

        let dispatched = prepare_tracked_download_background_work_dispatch(&mut tracker, id);

        assert!(dispatched.is_some());
        assert_eq!(
            tracker.find(id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::Importing)
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
        tracker.insert_for_tests(delayed);
        tracker.insert_for_tests(tracked_for_dispatch(ready_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![delayed_id.to_string(), ready_id.to_string()],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let in_flight = std::collections::HashSet::new();

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("ready item should dispatch");

        assert_eq!(id, ready_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert_eq!(
            tracker.find(delayed_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::ImportPending)
        );
        assert_eq!(
            tracker.find(ready_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::Importing)
        );
    }

    #[test]
    fn tracked_work_drain_dispatches_next_after_first_remains_pending() {
        let first_id = "nzbget:first";
        let second_id = "nzbget:second";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        tracker.insert_for_tests(tracked_for_dispatch(first_id));
        tracker.insert_for_tests(tracked_for_dispatch(second_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![first_id.to_string(), second_id.to_string()],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let in_flight = std::collections::HashSet::new();

        let (id, _, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("first item should dispatch");
        assert_eq!(id, first_id);
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

        assert_eq!(id, second_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert_eq!(
            tracker.find(second_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::Importing)
        );
    }

    #[test]
    fn tracked_download_poller_drain_dispatches_next_after_worker_result_without_interval_tick() {
        let first_id = "nzbget:first";
        let second_id = "nzbget:second";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        tracker.insert_for_tests(tracked_for_dispatch(first_id));
        tracker.insert_for_tests(tracked_for_dispatch(second_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![first_id.to_string(), second_id.to_string()],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let mut in_flight = std::collections::HashSet::new();

        let (id, _, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("first item should dispatch");
        assert_eq!(id, first_id);
        in_flight.insert(id);

        assert!(in_flight.remove(first_id));
        tracker
            .find_mut(first_id)
            .expect("first tracked download")
            .state = TrackedDownloadState::ImportPending;

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("second item should dispatch after the worker result");

        assert_eq!(id, second_id);
        assert_eq!(kind, TrackedDownloadBackgroundWorkKind::Import);
        assert_eq!(
            tracker.find(second_id).map(|tracked| tracked.state),
            Some(TrackedDownloadState::Importing)
        );
    }

    #[test]
    fn tracked_work_drain_skips_blocked_item_and_dispatches_next() {
        let blocked_id = "nzbget:blocked";
        let ready_id = "nzbget:ready";
        let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
        let mut blocked = tracked_for_dispatch(blocked_id);
        blocked.state = TrackedDownloadState::ImportBlocked;
        tracker.insert_for_tests(blocked);
        tracker.insert_for_tests(tracked_for_dispatch(ready_id));
        let mut drain = TrackedDownloadWorkDrain::new(
            vec![blocked_id.to_string(), ready_id.to_string()],
            crate::completed_download_handler::CompletedDownloadLookup::default(),
        );
        let in_flight = std::collections::HashSet::new();

        let (id, kind, _) = prepare_next_tracked_download_background_work_dispatch(
            &mut tracker,
            &in_flight,
            &mut drain,
        )
        .expect("ready item should dispatch after blocked item");

        assert_eq!(id, ready_id);
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
        tracker.insert_for_tests(duplicate);

        reconcile_duplicate_terminal_source_states(&mut tracker);

        let duplicate = tracker
            .find(duplicate_id)
            .expect("duplicate tracked download should remain cached");
        assert_eq!(duplicate.state, TrackedDownloadState::Imported);
        assert_eq!(duplicate.status, TrackedDownloadStatus::Ok);
        assert!(duplicate.status_messages.is_empty());
        assert!(
            prepare_tracked_download_background_work_dispatch(&mut tracker, duplicate_id).is_none()
        );
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

        apply_manual_import_record_to_queue_item(&mut queue_item, &record);

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
        }
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
            id: "Weaver:job-1".to_string(),
            client_id: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            client_item,
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
            foreign_import_classification: None,
            skip_reacquire_on_failure: false,
            snapshot_missing_since: None,
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
    fn apply_tracked_download_queue_metadata_backfills_missing_facet() {
        let mut queue_item = item("job-1", DownloadQueueState::Completed);
        let tracked = crate::tracked_downloads::TrackedDownload {
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
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
            foreign_import_classification: None,
            skip_reacquire_on_failure: false,
            snapshot_missing_since: None,
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
        queue_item.title_name = "Titanic".to_string();
        let tracked = crate::tracked_downloads::TrackedDownload {
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("movie".to_string()),
            source_title: Some("Titanic.1997.2160p.UHD.BluRay.x265-GRP".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            waiting_for_completed_history: false,
            path_missing_since: None,
            no_video_import_retry: None,
            foreign_import_classification: None,
            skip_reacquire_on_failure: false,
            snapshot_missing_since: None,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(
            queue_item.title_name,
            "Titanic.1997.2160p.UHD.BluRay.x265-GRP"
        );
    }
}
