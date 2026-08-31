use super::*;

async fn verify_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
) -> bool {
    super::verify_import(app, td, files_imported_this_pass)
        .await
        .expect("verification")
}

async fn verify_manual_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    expected_mapping_count: Option<usize>,
) -> bool {
    super::verify_manual_import(app, td, files_imported_this_pass, expected_mapping_count)
        .await
        .expect("verification")
}

async fn verify_import_inner(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    completed: Option<&CompletedDownload>,
) -> bool {
    super::verify_import_inner(app, td, files_imported_this_pass, completed)
        .await
        .expect("verification")
}

async fn verify_import_with_release_evidence(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
    release_evidence: &crate::import_workflow::ReleaseEvidence,
) -> bool {
    super::verify_import_inner_with_release_evidence(
        app,
        td,
        files_imported_this_pass,
        None,
        Some(release_evidence),
    )
    .await
    .expect("verification")
}

#[tokio::test]
async fn visible_file_count_reuses_resolved_completed_download() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        temp_dir.path().join("Paper.Lantern.2012.1080p.mkv"),
        b"video",
    )
    .expect("write video");
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("movie"),
    );
    let download_client = test_download_client_with_completed(completed.clone());
    let app =
        build_app_with_download_client(vec![], vec![], vec![], vec![], download_client.clone());
    let td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");

    let count = current_visible_video_file_count(&app, &td, Some(&completed)).await;

    assert_eq!(count, 1);
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
        0
    );
}

#[tokio::test]
async fn verify_import_terminalizes_movie_after_one_successful_file() {
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let artifacts = vec![build_artifact(
        "dl-1",
        "movie-file",
        "Paper.Lantern.2012.1080p.mkv",
    )];
    let app = build_app(vec![title], vec![], vec![], artifacts);
    let td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");

    assert!(verify_import(&app, &td, 1).await);
}

#[tokio::test]
async fn verify_import_uses_completed_identity_alias_with_normalized_client_fields() {
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let mut artifact = build_artifact(
        " history-item ",
        "movie-file",
        "Paper.Lantern.2012.1080p.mkv",
    );
    artifact.source_client_id = Some(" Client-A ".to_string());
    artifact.source_system = " NZBGET ".to_string();
    let app = build_app(vec![title], vec![], vec![], vec![artifact]);

    let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p");
    td.client_id = "client-a".to_string();
    td.client_type = "nzbget".to_string();
    td.client_item.download_client_item_id = "queue-item".to_string();
    let mut completed = build_completed_download(
        "Paper.Lantern.2012.1080p",
        "/downloads/Paper.Lantern.2012.1080p",
        Some("movie"),
    );
    completed.client_id = "CLIENT-A".to_string();
    completed.download_client_item_id = "history-item".to_string();

    assert!(verify_import_inner(&app, &td, 1, Some(&completed)).await);
}

#[tokio::test]
async fn verify_manual_import_keeps_partial_unresolved_series_blocked() {
    let title = build_title("title-1", "Unparsed Series", MediaFacet::Series);
    let artifacts = vec![build_artifact("dl-1", "file-1", "Part.One.mkv")];
    let app = build_app(vec![title], vec![], vec![], artifacts);
    let td = build_tracked_download("title-1", "series", "Unparsed.Release");

    assert!(!verify_manual_import(&app, &td, 1, Some(2)).await);
}

#[tokio::test]
async fn verify_manual_import_terminalizes_after_successful_retry_for_any_client_type() {
    for client_type in ["nzbget", "qbittorrent"] {
        let title = build_title("title-1", "Upgrade Show", MediaFacet::Series);
        let mut rejected =
            build_artifact_with_result("dl-1", Some("ep-1"), "Upgrade.Show.S01E01.mkv", "rejected");
        rejected.source_system = client_type.to_string();
        let mut td = build_tracked_download("title-1", "series", "Upgrade.Show.S01E01");
        td.client_type = client_type.to_string();
        let rejected_only = build_app(vec![title.clone()], vec![], vec![], vec![rejected.clone()]);
        assert!(!verify_import(&rejected_only, &td, 1).await);

        let mut imported = build_artifact("dl-1", "ep-1", "Upgrade.Show.S01E01.mkv");
        imported.source_system = client_type.to_string();
        let app = build_app(vec![title], vec![], vec![], vec![rejected, imported]);

        assert!(verify_manual_import(&app, &td, 1, Some(1)).await);
    }
}

#[tokio::test]
async fn verify_manual_import_terminalizes_complete_series_movie_source() {
    let title = build_title("title-1", "HarborTales", MediaFacet::Series);
    let artifacts = vec![build_artifact(
        "dl-1",
        "movie-file",
        "HarborTales.The.Movie.mkv",
    )];
    let app = build_app(vec![title], vec![], vec![], artifacts);
    let td = build_tracked_download("title-1", "series", "HarborTales.The.Movie.1990");

    assert!(verify_manual_import(&app, &td, 1, Some(1)).await);
}

#[tokio::test]
async fn verify_import_terminalizes_when_every_artifact_source_is_intentionally_ignored() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let mut episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    episode.monitored = false;
    let artifacts = vec![build_artifact_with_result(
        "dl-1",
        Some("ep-1"),
        "Show.S01E01.mkv",
        "ignored",
    )];
    let app = build_app(vec![title], vec![collection], vec![episode], artifacts);
    let td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_does_not_terminalize_an_ignored_but_monitored_episode() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let artifacts = vec![build_artifact_with_result(
        "dl-1",
        Some("ep-1"),
        "Show.S01E01.mkv",
        "ignored",
    )];
    let app = build_app(vec![title], vec![collection], vec![episode], artifacts);
    let td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");

    assert!(!verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_manual_import_does_not_terminalize_all_ignored_artifacts() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let artifacts = vec![build_artifact_with_result(
        "dl-1",
        Some("ep-1"),
        "Show.S01E01.mkv",
        "ignored",
    )];
    let app = build_app(vec![title], vec![collection], vec![episode], artifacts);
    let td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");

    assert!(!verify_manual_import(&app, &td, 0, Some(1)).await);
}

#[tokio::test]
async fn verify_import_does_not_terminalize_ignored_artifacts_for_a_stale_title() {
    let title = build_title("title-2", "Replacement Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-2", "1");
    let episode = build_episode("ep-2", "title-2", "season-1", "1", "1", None);
    let artifacts = vec![build_artifact_with_result(
        "dl-1",
        Some("ep-1"),
        "Show.S01E01.mkv",
        "ignored",
    )];
    let app = build_app(vec![title], vec![collection], vec![episode], artifacts);
    let td = build_tracked_download("title-2", "series", "Replacement.Show.S01E01.1080p.WEB-DL");

    assert!(!verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_manual_import_does_not_credit_an_ignored_source_as_successful_coverage() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episodes = vec![
        build_episode("ep-1", "title-1", "season-1", "1", "1", None),
        build_episode("ep-2", "title-1", "season-1", "1", "2", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-1", "Show.S01E01.mkv"),
        build_artifact_with_result("dl-1", Some("ep-2"), "Show.S01E02.mkv", "ignored"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download("title-1", "series", "Show.S01.Complete.1080p.WEB-DL");

    assert!(!verify_manual_import(&app, &td, 1, Some(2)).await);
}

#[tokio::test]
async fn verify_import_excludes_only_currently_unmonitored_ignored_episodes_from_wanted_coverage() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let imported_episode = build_episode("ep-1", "title-1", "season-1", "1", "1", None);
    let mut ignored_episode = build_episode("ep-2", "title-1", "season-1", "1", "2", None);
    ignored_episode.monitored = false;
    let artifacts = vec![
        build_artifact("dl-1", "ep-1", "Show.S01E01.mkv"),
        build_artifact_with_result("dl-1", Some("ep-2"), "Show.S01E02.mkv", "ignored"),
    ];
    let app = build_app(
        vec![title],
        vec![collection],
        vec![imported_episode, ignored_episode],
        artifacts,
    );
    let td = build_tracked_download("title-1", "series", "Show.S01.Complete.1080p.WEB-DL");
    let evidence = crate::import_workflow::ReleaseEvidence::ScryerSubmission {
        title_id: "title-1".to_string(),
        facet: "series".to_string(),
        source_title: Some("Show.S01.Complete.1080p.WEB-DL".to_string()),
        observed_release_name: None,
        release_size_bytes: None,
        purpose: crate::DownloadSubmissionPurpose::Standard,
        scope: crate::SubmissionScope::EpisodeSet {
            episode_ids: vec!["ep-1".to_string(), "ep-2".to_string()],
        },
    };

    assert!(verify_import_with_release_evidence(&app, &td, 1, &evidence).await);
}

#[tokio::test]
async fn verify_import_keeps_monitored_ignored_episode_in_wanted_coverage() {
    let title = build_title("title-1", "Show", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episodes = vec![
        build_episode("ep-1", "title-1", "season-1", "1", "1", None),
        build_episode("ep-2", "title-1", "season-1", "1", "2", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-1", "Show.S01E01.mkv"),
        build_artifact_with_result("dl-1", Some("ep-2"), "Show.S01E02.mkv", "ignored"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download("title-1", "series", "Show.S01.Complete.1080p.WEB-DL");
    let evidence = crate::import_workflow::ReleaseEvidence::ScryerSubmission {
        title_id: "title-1".to_string(),
        facet: "series".to_string(),
        source_title: Some("Show.S01.Complete.1080p.WEB-DL".to_string()),
        observed_release_name: None,
        release_size_bytes: None,
        purpose: crate::DownloadSubmissionPurpose::Standard,
        scope: crate::SubmissionScope::EpisodeSet {
            episode_ids: vec!["ep-1".to_string(), "ep-2".to_string()],
        },
    };

    assert!(!verify_import_with_release_evidence(&app, &td, 1, &evidence).await);
}

#[tokio::test]
async fn verify_import_requires_full_season_pack_coverage() {
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-201", "Lantern.Watch.Legacy.S02E01.mkv"),
        build_artifact("dl-1", "ep-202", "Lantern.Watch.Legacy.S02E02.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );

    let parsed = crate::parse_release_metadata(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    assert_eq!(
        parsed.episode.as_ref().and_then(|episode| episode.season),
        Some(2)
    );

    match expected_episode_units(&app, &td).await {
        ExpectedEpisodeResolution::Resolved(expected) => assert_eq!(expected.len(), 3),
        _ => panic!("expected a resolved season-pack episode set"),
    }

    assert!(!verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_accepts_resolved_season_pack_when_visible_source_units_are_imported() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let first_video =
        std::fs::File::create(temp_dir.path().join("Lantern.Watch.Legacy.S02E01.mkv"))
            .expect("create first video");
    first_video
        .set_len(60 * 1024 * 1024)
        .expect("size first video");
    let second_video =
        std::fs::File::create(temp_dir.path().join("Lantern.Watch.Legacy.S02E02.mkv"))
            .expect("create second video");
    second_video
        .set_len(60 * 1024 * 1024)
        .expect("size second video");
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-201", "Lantern.Watch.Legacy.S02E01.mkv"),
        build_artifact("dl-1", "ep-202", "Lantern.Watch.Legacy.S02E02.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    let completed = build_completed_download(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );

    assert!(!verify_import(&app, &td, 0).await);
    let artifacts = import_artifacts_for_completed_download(&app, &td, Some(&completed))
        .await
        .expect("load import artifacts");
    assert_eq!(
        visible_source_files_have_terminal_dispositions(&app, &td, &artifacts, Some(&completed),)
            .await,
        Some(true)
    );
    assert!(matches!(
        visible_source_episode_units(&app, &td, &artifacts, Some(&completed)).await,
        SourceVideoEpisodeResolution::Resolved(ref units) if units.len() == 2
    ));
    assert!(verify_import_inner(&app, &td, 2, Some(&completed)).await);
}

#[tokio::test]
async fn verify_import_accepts_resolved_pack_when_move_removed_source_but_artifacts_cover_source_units()
 {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let mut first = build_artifact("dl-1", "ep-201", "Lantern.Watch.Legacy.S02E01.mkv");
    first.relative_path = Some("Season 02/Lantern.Watch.Legacy.S02E01.mkv".to_string());
    let mut second = build_artifact("dl-1", "ep-202", "Lantern.Watch.Legacy.S02E02.mkv");
    second.relative_path = Some("Season 02/Lantern.Watch.Legacy.S02E02.mkv".to_string());
    let app = build_app(vec![title], vec![collection], episodes, vec![first, second]);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    let completed = build_completed_download(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );

    assert!(verify_import_inner(&app, &td, 2, Some(&completed)).await);
}

#[tokio::test]
async fn verify_import_rejects_artifact_manifest_when_source_episode_lacks_successful_coverage() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let mut first = build_artifact("dl-1", "ep-201", "Lantern.Watch.Legacy.S02E01.mkv");
    first.relative_path = Some("Season 02/Lantern.Watch.Legacy.S02E01.mkv".to_string());
    let mut second = build_artifact("dl-1", "ep-202", "Lantern.Watch.Legacy.S02E02.mkv");
    second.relative_path = Some("Season 02/Lantern.Watch.Legacy.S02E02.mkv".to_string());
    let mut third = build_artifact_with_result(
        "dl-1",
        Some("ep-203"),
        "Lantern.Watch.Legacy.S02E03.mkv",
        "rejected",
    );
    third.relative_path = Some("Season 02/Lantern.Watch.Legacy.S02E03.mkv".to_string());
    let app = build_app(
        vec![title],
        vec![collection],
        episodes,
        vec![first, second, third],
    );
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    let completed = build_completed_download(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );

    assert!(!verify_import_inner(&app, &td, 2, Some(&completed)).await);
}

#[tokio::test]
async fn verify_import_rejects_artifact_manifest_with_unmapped_source_group() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let mut first = build_artifact("dl-1", "ep-201", "Lantern.Watch.Legacy.S02E01.mkv");
    first.relative_path = Some("Season 02/Lantern.Watch.Legacy.S02E01.mkv".to_string());
    let mut second = build_artifact("dl-1", "ep-202", "Lantern.Watch.Legacy.S02E02.mkv");
    second.relative_path = Some("Season 02/Lantern.Watch.Legacy.S02E02.mkv".to_string());
    let mut unmapped = build_artifact_with_result(
        "dl-1",
        None,
        "Lantern.Watch.Legacy.Special.Featurette.mkv",
        "rejected",
    );
    unmapped.relative_path =
        Some("Season 02/Lantern.Watch.Legacy.Special.Featurette.mkv".to_string());
    let app = build_app(
        vec![title],
        vec![collection],
        episodes,
        vec![first, second, unmapped],
    );
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    let completed = build_completed_download(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );

    assert!(!verify_import_inner(&app, &td, 2, Some(&completed)).await);
}

#[tokio::test]
async fn verify_import_does_not_over_credit_duplicate_visible_basenames_from_filename_artifacts() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(temp_dir.path().join("disc-a")).expect("create first dir");
    std::fs::create_dir_all(temp_dir.path().join("disc-b")).expect("create second dir");
    for path in [
        temp_dir.path().join("disc-a").join("video.mkv"),
        temp_dir.path().join("disc-b").join("video.mkv"),
    ] {
        let video = std::fs::File::create(path).expect("create source video");
        video.set_len(60 * 1024 * 1024).expect("size source video");
    }
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
    ];
    let artifacts = vec![build_artifact("dl-1", "ep-201", "video.mkv")];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    let completed = build_completed_download(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );

    assert!(!verify_import_inner(&app, &td, 1, Some(&completed)).await);
}

#[tokio::test]
async fn verify_import_rejects_resolved_pack_when_visible_source_episode_is_not_imported() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    for file_name in [
        "Lantern.Watch.Legacy.S02E01.mkv",
        "Lantern.Watch.Legacy.S02E02.mkv",
        "Lantern.Watch.Legacy.S02E03.mkv",
    ] {
        let video =
            std::fs::File::create(temp_dir.path().join(file_name)).expect("create source video");
        video.set_len(60 * 1024 * 1024).expect("size source video");
    }
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-201", "S02E01.mkv"),
        build_artifact("dl-1", "ep-202", "S02E02.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    let completed = build_completed_download(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );

    assert!(!verify_import_inner(&app, &td, 2, Some(&completed)).await);
}

#[tokio::test]
async fn verify_import_rejects_resolved_pack_with_unmapped_visible_source_video() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    for file_name in [
        "Lantern.Watch.Legacy.S02E01.mkv",
        "Lantern.Watch.Legacy.S02E02.mkv",
        "Behind.The.Scenes.mkv",
    ] {
        let video =
            std::fs::File::create(temp_dir.path().join(file_name)).expect("create source video");
        video.set_len(60 * 1024 * 1024).expect("size source video");
    }
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-201", "S02E01.mkv"),
        build_artifact("dl-1", "ep-202", "S02E02.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );
    let completed = build_completed_download(
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        temp_dir.path().to_string_lossy().as_ref(),
        Some("series"),
    );

    assert!(!verify_import_inner(&app, &td, 2, Some(&completed)).await);
}

#[tokio::test]
async fn verify_import_accepts_full_season_pack_coverage() {
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-201", "S02E01.mkv"),
        build_artifact("dl-1", "ep-202", "S02E02.mkv"),
        build_artifact("dl-1", "ep-203", "S02E03.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_ignores_rejected_extras_when_expected_units_are_satisfied() {
    let title = build_title("title-1", "Lantern Watch Legacy", MediaFacet::Series);
    let collection = build_collection("season-2", "title-1", "2");
    let episodes = vec![
        build_episode("ep-201", "title-1", "season-2", "2", "1", None),
        build_episode("ep-202", "title-1", "season-2", "2", "2", None),
        build_episode("ep-203", "title-1", "season-2", "2", "3", None),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-201", "S02E01.mkv"),
        build_artifact("dl-1", "ep-202", "S02E02.mkv"),
        build_artifact("dl-1", "ep-203", "S02E03.mkv"),
        build_artifact_with_result("dl-1", None, "sample.mkv", "rejected"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Lantern.Watch.Legacy.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
    );

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_resolves_absolute_episode_ranges() {
    let title = build_title("title-1", "Tidebreaker", MediaFacet::Anime);
    let collection = build_collection("season-22", "title-1", "22");
    let episodes = vec![
        build_episode("ep-1122", "title-1", "season-22", "22", "1", Some("1122")),
        build_episode("ep-1123", "title-1", "season-22", "22", "2", Some("1123")),
        build_episode("ep-1124", "title-1", "season-22", "22", "3", Some("1124")),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-1122", "1122.mkv"),
        build_artifact("dl-1", "ep-1123", "1123.mkv"),
        build_artifact("dl-1", "ep-1124", "1124.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "anime",
        "[HatSubs] Tidebreaker 1122-1124 (WEB 1080p)",
    );

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_absolute_range_requires_all_monitored_episodes_only() {
    let title = build_title("title-1", "Tidebreaker", MediaFacet::Anime);
    let collection = build_collection("season-22", "title-1", "22");
    let mut unmonitored = build_episode("ep-1123", "title-1", "season-22", "22", "2", Some("1123"));
    unmonitored.monitored = false;
    let episodes = vec![
        build_episode("ep-1122", "title-1", "season-22", "22", "1", Some("1122")),
        unmonitored,
        build_episode("ep-1124", "title-1", "season-22", "22", "3", Some("1124")),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-1122", "1122.mkv"),
        build_artifact("dl-1", "ep-1124", "1124.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "anime",
        "[HatSubs] Tidebreaker 1122-1124 (WEB 1080p)",
    );

    match expected_episode_units(&app, &td).await {
        ExpectedEpisodeResolution::Resolved(expected) => {
            assert_eq!(
                expected,
                HashSet::from(["ep-1122".to_string(), "ep-1124".to_string()])
            );
        }
        _ => panic!("expected monitored range episode set"),
    }

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_accepts_release_when_title_has_no_monitored_episodes() {
    let title = build_title("title-1", "Harbor Pals", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let mut first_episode = build_episode("ep-101", "title-1", "season-1", "1", "1", None);
    first_episode.monitored = false;
    let episodes = vec![first_episode];
    let artifacts = vec![build_artifact("dl-1", "ep-101", "S01E01.mkv")];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download("title-1", "series", "Harbor.Pals.S01E01.720p.WEB-DL-NTb");

    match expected_episode_units(&app, &td).await {
        ExpectedEpisodeResolution::Resolved(expected) => {
            assert_eq!(expected, HashSet::from(["ep-101".to_string()]));
        }
        _ => panic!("expected resolved episodes for an explicitly queued release"),
    }

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_absolute_range_blocks_when_monitored_episode_missing() {
    let title = build_title("title-1", "Tidebreaker", MediaFacet::Anime);
    let collection = build_collection("season-22", "title-1", "22");
    let episodes = vec![
        build_episode("ep-1122", "title-1", "season-22", "22", "1", Some("1122")),
        build_episode("ep-1123", "title-1", "season-22", "22", "2", Some("1123")),
        build_episode("ep-1124", "title-1", "season-22", "22", "3", Some("1124")),
    ];
    let artifacts = vec![
        build_artifact("dl-1", "ep-1122", "1122.mkv"),
        build_artifact("dl-1", "ep-1123", "1123.mkv"),
    ];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "anime",
        "[HatSubs] Tidebreaker 1122-1124 (WEB 1080p)",
    );

    assert!(!verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_partial_pack_accepts_one_monitored_episode() {
    let title = build_title(
        "title-1",
        "Nightfall!! Heavy Chorus, Dark Lantern",
        MediaFacet::Anime,
    );
    let collection = build_collection("season-1", "title-1", "1");
    let episodes = vec![
        build_episode("ep-14", "title-1", "season-1", "1", "14", Some("14")),
        build_episode("ep-15", "title-1", "season-1", "1", "15", Some("15")),
    ];
    let artifacts = vec![build_artifact("dl-1", "ep-14", "S01E14.mkv")];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "anime",
        "[EMBER] NIGHTFALL‼ Heavy Chorus, Dark Lantern (2022) (Season 1 | Part 02) [1080p] [Dual Audio HEVC 10 bits WEBRip AAC] (Batch)",
    );
    match expected_episode_units(&app, &td).await {
        ExpectedEpisodeResolution::AtLeastOne(expected) => {
            assert!(expected.contains("ep-14"));
            assert!(expected.contains("ep-15"));
        }
        _ => panic!("expected partial pack monitored episode set"),
    }

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_resolves_daily_episode_by_air_date() {
    let title = build_title("title-1", "Series Title", MediaFacet::Series);
    let collection = build_collection("season-1", "title-1", "1");
    let episodes = vec![
        build_episode_with_details(
            "ep-101",
            "title-1",
            "season-1",
            EpisodeType::Standard,
            "1",
            "1",
            Some("2015-09-07"),
            None,
        ),
        build_episode_with_details(
            "ep-102",
            "title-1",
            "season-1",
            EpisodeType::Standard,
            "1",
            "2",
            Some("2015-09-08"),
            None,
        ),
    ];
    let artifacts = vec![build_artifact(
        "dl-1",
        "ep-101",
        "Series.Title.2015.09.07.mkv",
    )];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "series",
        "Series.Title.2015.09.07.Part.1.720p.HULU.WEBRip.AAC2.0.H.264-Sonarr",
    );

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_resolves_special_by_season_zero_number() {
    let title = build_title("title-1", "Another Anime Show", MediaFacet::Anime);
    let collection = build_collection("season-0", "title-1", "0");
    let episodes = vec![build_episode_with_details(
        "ep-special-1",
        "title-1",
        "season-0",
        EpisodeType::Ova,
        "0",
        "1",
        None,
        None,
    )];
    let artifacts = vec![build_artifact(
        "dl-1",
        "ep-special-1",
        "Another.Anime.Show.S00E01.ova.mkv",
    )];
    let app = build_app(vec![title], vec![collection], episodes, artifacts);
    let td = build_tracked_download(
        "title-1",
        "anime",
        "[DeadFish] Another Anime Show - 01 - OVA [BD][720p][AAC]",
    );

    assert!(verify_import(&app, &td, 0).await);
}

#[tokio::test]
async fn verify_import_unresolved_episode_resolution_falls_back_to_successful_pass() {
    let title = build_title("title-1", "Mystery Show", MediaFacet::Series);
    let artifacts = vec![build_artifact_with_result(
        "dl-1",
        None,
        "Mystery.Show.S01E01.mkv",
        "imported",
    )];
    let app = build_app(vec![title], vec![], vec![], artifacts);
    let td = build_tracked_download("title-1", "series", "Mystery.Show.S01E01.1080p.WEB-DL");

    match expected_episode_units(&app, &td).await {
        ExpectedEpisodeResolution::Unresolved => {}
        _ => panic!("expected unresolved episodic resolution"),
    }

    assert!(verify_import(&app, &td, 1).await);
}
