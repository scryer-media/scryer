use super::*;

async fn run_category_admission_check(
    category: Option<&str>,
    match_type: TitleMatchType,
    is_scryer_origin: bool,
    external_manager: bool,
) -> TrackedDownload {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        temp_dir.path().join("Paper.Lantern.2012.1080p.WEB-DL.mkv"),
        b"video",
    )
    .expect("write fixture video");
    let mut completed = build_completed_download(
        "downloader display label",
        temp_dir.path().to_string_lossy().as_ref(),
        category,
    );
    completed.release_name = Some("Paper.Lantern.2012.1080p.WEB-DL".to_string());
    if external_manager {
        completed
            .parameters
            .push(("DrOnE".to_string(), "true".to_string()));
    }
    let app = build_app_with_download_client(
        vec![build_title("title-1", "Paper Lantern", MediaFacet::Movie)],
        vec![],
        vec![],
        vec![],
        test_download_client_with_completed(completed),
    );
    let mut tracked = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p.WEB-DL");
    tracked.client_item.category = category.map(str::to_string);
    tracked.client_item.is_scryer_origin = is_scryer_origin;
    tracked.match_type = match_type;

    check(&app, &mut tracked).await;
    tracked
}

#[tokio::test]
async fn tracked_submission_bypasses_unknown_category() {
    let tracked =
        run_category_admission_check(Some("unrelated"), TitleMatchType::Submission, true, false)
            .await;
    assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
    assert_eq!(tracked.import_hold, None);
}

#[tokio::test]
async fn observation_requires_a_known_nonblank_category() {
    for category in [None, Some(" "), Some("unrelated")] {
        let tracked =
            run_category_admission_check(category, TitleMatchType::TitleParse, false, false).await;
        assert_eq!(tracked.state, TrackedDownloadState::Downloading);
    }

    let tracked =
        run_category_admission_check(Some("  MoViE "), TitleMatchType::TitleParse, false, false)
            .await;
    assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn globally_known_route_mismatch_requires_manual_import() {
    let tracked =
        run_category_admission_check(Some("series"), TitleMatchType::TitleParse, false, false)
            .await;
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
    assert!(tracked.status_messages.iter().any(|message| {
        message
            .text
            .contains("does not match this title's active route")
    }));
}

#[tokio::test]
async fn explicit_external_manager_marker_is_excluded() {
    let tracked =
        run_category_admission_check(Some("movie"), TitleMatchType::TitleParse, false, true).await;
    assert_eq!(tracked.state, TrackedDownloadState::Downloading);
    assert_eq!(
        tracked.import_hold,
        Some(crate::tracked_downloads::ImportHold::ExternalManager)
    );
}
