use super::*;

async fn rejection_fixture() -> (
    AppUseCase,
    Title,
    AcquisitionScopeState,
    Arc<TrackingDownloadSubmissionRepo>,
    Arc<TrackingAcquisitionScopeStateRepo>,
    Arc<RecordingScopeIndexerCoverageRepo>,
    scryer_domain::CompletedDownload,
) {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::default());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Rejected Import Coverage".into(),
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
    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: Some("movie".to_string()),
        library_id: Some(title.library_id.clone()),
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some("Rejected.Import.Coverage.2024.1080p.WEB-DL".to_string()),
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted scope");
    let scope_key = format!("title:{}", title.id);
    for indexer_id in ["indexer-a", "indexer-b"] {
        coverage
            .record_coverage(&scope_key, "movie", indexer_id, "fp")
            .await
            .expect("seed coverage");
    }
    let completed = completed_download_fixture_item(
        "rejected-import-job",
        &title.id,
        "Rejected.Import.Coverage.2024.1080p.WEB-DL",
        "/downloads/rejected-import",
    );

    (
        app,
        title,
        wanted,
        download_submissions,
        wanted_items,
        coverage,
        completed,
    )
}

async fn reject_import(
    app: &AppUseCase,
    title: &Title,
    completed: &scryer_domain::CompletedDownload,
) {
    crate::post_download_gate::reject_source_file_before_import(
        app,
        crate::domain_events::DomainEventActor::system(),
        title,
        &completed.name,
        std::path::Path::new("/downloads/rejected-import/video.mkv"),
        crate::post_download_gate::BlocklistAttribution::default(),
        None,
        &crate::post_download_gate::ImportedFileRejection {
            message: "required audio language is missing".to_string(),
            recycle_reason: "language_mismatch",
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            blocking_rule_codes: vec!["language_mismatch".to_string()],
        },
    )
    .await;
}

#[tokio::test]
async fn rejected_import_keeps_coverage_and_reopens_for_the_saved_results_walk() {
    let (app, title, wanted, download_submissions, wanted_items, coverage, completed) =
        rejection_fixture().await;
    let collection_id = "import-rejection-pack";
    let pack_scope_key = format!("collection:{collection_id}");
    for indexer_id in ["indexer-a", "indexer-b"] {
        coverage
            .record_coverage(&pack_scope_key, "series", indexer_id, "fp")
            .await
            .expect("seed season-pack coverage");
    }
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(completed.client_id.clone()),
            download_client_type: completed.client_type.clone(),
            download_client_item_id: completed.download_client_item_id.clone(),
            source_hint: None,
            source_provider_id: Some("indexer-a".to_string()),
            source_provider_name: None,
            source_kind: None,
            source_title: Some(completed.name.clone()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Collection {
                collection_id: collection_id.to_string(),
            },
        })
        .await
        .expect("record attributed submission");

    reject_import(&app, &title, &completed).await;

    // A burned import never touches coverage: the scope is re-opened under its
    // existing coverage and the cursor walks the saved search results instead
    // of re-querying the indexer.
    let scope_key = format!("title:{}", title.id);
    for key in [&scope_key, &pack_scope_key] {
        let mut covered = coverage.indexers_for_scope(key).await;
        covered.sort();
        assert_eq!(
            covered,
            vec!["indexer-a".to_string(), "indexer-b".to_string()],
            "coverage for {key} must survive the rejection"
        );
    }
    assert_eq!(
        wanted_items
            .get_acquisition_scope_state_by_id(&wanted.id)
            .await
            .expect("load wanted scope")
            .expect("wanted scope exists")
            .status,
        AcquisitionScopeStatus::Wanted
    );
    assert_eq!(
        app.services
            .workflow
            .blocklist_repo
            .list_for_title(&title.id, 10)
            .await
            .expect("list blocklist")
            .len(),
        1
    );
}

#[tokio::test]
async fn rejected_import_without_submission_keeps_scope_coverage() {
    let (app, title, wanted, _download_submissions, wanted_items, coverage, completed) =
        rejection_fixture().await;

    reject_import(&app, &title, &completed).await;

    let scope_key = format!("title:{}", title.id);
    // Even without an attributable submission the coverage stays: a failure
    // is never a reason to re-query an indexer.
    assert!(
        !coverage.indexers_for_scope(&scope_key).await.is_empty(),
        "coverage survives an unattributed rejection"
    );
    assert_eq!(
        wanted_items
            .get_acquisition_scope_state_by_id(&wanted.id)
            .await
            .expect("load wanted scope")
            .expect("wanted scope exists")
            .status,
        AcquisitionScopeStatus::Wanted
    );
    assert_eq!(
        app.services
            .workflow
            .blocklist_repo
            .list_for_title(&title.id, 10)
            .await
            .expect("list blocklist")
            .len(),
        1
    );
}
