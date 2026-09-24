//! Import-target and release-evidence threading through the tracked-download
//! import stage (`import_with_lookup` → `prepare_completed_import_request` →
//! `run_import`) and through `retry_failed_import`.
//!
//! The runs deliberately end at `resolve_completed_import_target` (an empty
//! completed directory yields a `NoVideoFiles` skip that carries the resolved
//! `title_id`), which is exactly the decision under test: which title the
//! import lands in and which release name it parses.

use super::*;
use crate::ImportRepository;
use scryer_domain::{ImportDecision, ImportSkipReason};

fn import_actor() -> User {
    let mut actor = User::new_admin("admin");
    actor.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageSystemSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    actor
}

const PAPER_LANTERN_RELEASE: &str = "Paper.Lantern.2012.1080p.WEB-DL";
const OBFUSCATED_RELEASE: &str = "a1b2c3d4e5f6a7b8c9d0";

fn paper_lantern_titles() -> Vec<Title> {
    let mut paper_lantern = build_title("title-a", "Paper Lantern", MediaFacet::Movie);
    paper_lantern.year = Some(2012);
    vec![
        paper_lantern,
        build_title("title-b", "Harbor Lights", MediaFacet::Movie),
    ]
}

/// A completed download whose destination exists but holds no video: import
/// resolves its target and stops with `NoVideoFiles` carrying that title.
fn completed_without_video(release_name: Option<&str>) -> (tempfile::TempDir, CompletedDownload) {
    let dir = tempfile::tempdir().expect("completed dir");
    let mut completed = build_completed_download(
        "downloader display label",
        dir.path().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.release_name = release_name.map(str::to_string);
    (dir, completed)
}

fn submission_row(
    title_id: &str,
    scope: crate::SubmissionScope,
    source_title: Option<&str>,
) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: title_id.to_string(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some("client-1".to_string()),
        download_client_type: "nzbget".to_string(),
        download_client_item_id: "dl-1".to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: source_title.map(str::to_string),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope,
    }
}

fn app_for_import(
    submission_repo: Arc<TestDownloadSubmissionRepo>,
    import_repo: Arc<TestImportRepo>,
) -> AppUseCase {
    build_app_with_download_client_configs_and_submissions(
        paper_lantern_titles(),
        vec![],
        vec![],
        vec![],
        Arc::new(TestDownloadClient::default()),
        Arc::new(NullDownloadClientConfigRepository),
        submission_repo,
    )
    .with_test_overrides(|services| services.with_imports(import_repo))
}

fn import_pending_observation(title_id: &str, match_type: TitleMatchType) -> TrackedDownload {
    let mut td = build_tracked_download(title_id, "movie", PAPER_LANTERN_RELEASE);
    td.state = TrackedDownloadState::ImportPending;
    td.match_type = match_type;
    td.client_item.is_scryer_origin = false;
    td.client_item.category = Some("movie".to_string());
    td
}

fn source_identity() -> ClientJobLocator {
    ClientJobLocator::new(Some("client-1"), "nzbget", "dl-1")
}

fn completed_request_payload(
    completed: &CompletedDownload,
    release_evidence: serde_json::Value,
    target_title_id: Option<&str>,
) -> String {
    serde_json::json!({
        "completed": completed,
        "release_evidence": release_evidence,
        "target_title_id": target_title_id,
    })
    .to_string()
}

fn observation_evidence_json(release_name: &str) -> serde_json::Value {
    serde_json::json!({ "DownloaderObservation": { "release_name": release_name } })
}

fn scryer_submission_evidence_json(title_id: &str, source_title: &str) -> serde_json::Value {
    serde_json::json!({
        "ScryerSubmission": {
            "title_id": title_id,
            "facet": "movie",
            "source_title": source_title,
            "purpose": serde_json::to_value(crate::DownloadSubmissionPurpose::Standard).unwrap(),
            "scope": serde_json::to_value(crate::SubmissionScope::Title).unwrap(),
        }
    })
}

async fn assert_lands_in(import_repo: &TestImportRepo, expected_title_id: &str) -> ImportResult {
    let result = import_repo
        .last_import_result()
        .await
        .expect("import must record a result");
    assert_eq!(
        result.decision,
        ImportDecision::Skipped,
        "unexpected result: {result:?}"
    );
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::NoVideoFiles),
        "unexpected result: {result:?}"
    );
    assert_eq!(
        result.title_id.as_deref(),
        Some(expected_title_id),
        "import landed in the wrong title: {result:?}"
    );
    result
}

// ── A2: the tracked download's validated title is the import target ──

#[tokio::test]
async fn operator_assigned_observation_imports_into_the_assigned_title() {
    // The release name parses to "Paper Lantern" (title-a), but the operator
    // assigned the download to title-b: a titled row for title-b that carries
    // no Scryer origin, tracked as a Submission match. (Assignments are
    // recorded like grabs; this shape guards the observation path.)
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-b",
            crate::SubmissionScope::Orphan,
            None,
        ))
        .await
        .expect("record assignment row");
    let import_repo = Arc::new(TestImportRepo::default());
    let app = app_for_import(submission_repo, import_repo.clone());
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = import_pending_observation("title-b", TitleMatchType::Submission);

    import_with_lookup(&app, &import_actor(), &mut td, &lookup).await;

    assert_lands_in(&import_repo, "title-b").await;
    assert_eq!(
        import_repo.last_queued_target_title_id().await.as_deref(),
        Some("title-b"),
        "the target must be persisted with the request so retries honor it"
    );
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn title_parse_observation_imports_into_the_checked_title() {
    // The completed-check proved title-a for this download; the release name
    // the client reports at completion is obfuscated and matches nothing on a
    // context-free parse. The import must still land in title-a instead of
    // failing with "could not match".
    let import_repo = Arc::new(TestImportRepo::default());
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        import_repo.clone(),
    );
    let (_dir, completed) = completed_without_video(Some(OBFUSCATED_RELEASE));
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = import_pending_observation("title-a", TitleMatchType::TitleParse);

    import_with_lookup(&app, &import_actor(), &mut td, &lookup).await;

    let result = assert_lands_in(&import_repo, "title-a").await;
    assert_eq!(result.source_title.as_deref(), Some(OBFUSCATED_RELEASE));
}

#[tokio::test]
async fn migrated_unverified_import_record_does_not_suppress_blocked_import_retry() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let import_repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "status-only-import",
        &source_identity(),
        ImportStatus::Completed,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            Some("title-a"),
        ),
    )]));
    let submissions = Arc::new(TestDownloadSubmissionRepo::default());
    let app = app_for_import(submissions.clone(), import_repo.clone());
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    // Restart reconstruction restores this durable state, but not the
    // in-memory `import_attempted` flag. The migrated durable reason must
    // reopen the pre-import block before the submission guard runs.
    let mut td = import_pending_observation("title-a", TitleMatchType::Submission);
    td.client_item.is_scryer_origin = true;
    td.state = TrackedDownloadState::ImportBlocked;
    td.status = TrackedDownloadStatus::Warning;
    td.status_messages = vec!["Awaiting import verification.".to_string()];
    submissions
        .canonical_identity_tracked_state_reasons
        .lock()
        .await
        .push((
            td.download_id.to_string(),
            crate::tracked_downloads::ImportBlockedReason::UnverifiedAlreadyImported
                .as_str()
                .to_string(),
        ));

    check_with_lookup(&app, &mut td, Some(&lookup)).await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    import_with_lookup(&app, &import_actor(), &mut td, &lookup).await;

    let result = assert_lands_in(&import_repo, "title-a").await;
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
    assert!(
        td.status_messages
            .iter()
            .all(|message| message != "Awaiting import verification.")
    );
}

#[tokio::test]
async fn attempted_import_does_not_reopen_when_durable_reason_is_stale() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let import_repo = Arc::new(TestImportRepo::default());
    let submissions = Arc::new(TestDownloadSubmissionRepo::default());
    let app = app_for_import(submissions.clone(), import_repo);
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = import_pending_observation("title-a", TitleMatchType::Submission);
    td.state = TrackedDownloadState::ImportBlocked;
    td.import_attempted = true;
    td.status = TrackedDownloadStatus::Error;
    td.status_messages = vec!["Import failed after admission.".to_string()];
    submissions
        .canonical_identity_tracked_state_reasons
        .lock()
        .await
        .push((
            td.download_id.to_string(),
            crate::tracked_downloads::ImportBlockedReason::UnverifiedAlreadyImported
                .as_str()
                .to_string(),
        ));

    check_with_lookup(&app, &mut td, Some(&lookup)).await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert!(td.import_attempted);
    assert_eq!(
        crate::tracked_downloads::import_blocked_reason_for_tracked(&app, &td).await,
        Some(crate::tracked_downloads::ImportBlockedReason::AfterImport)
    );
}

#[tokio::test]
async fn retry_skipped_import_reevaluates_instead_of_replaying_rejection() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let mut record = test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Skipped,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            Some("title-a"),
        ),
    );
    record.result_json =
        Some(r#"{"decision":"rejected","error_message":"old quality mismatch"}"#.into());
    let repo = Arc::new(TestImportRepo::with_records(vec![record]));
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        repo.clone(),
    );
    let result =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .expect("skipped imports are retryable");
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
    assert_eq!(result.title_id.as_deref(), Some("title-a"));
    assert!(!result.release_burned);
}

#[tokio::test]
async fn retry_reconciliation_replaces_durable_burned_failure_and_survives_restart() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Skipped,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            Some("title-a"),
        ),
    )]));
    let app = app_for_import(Arc::new(TestDownloadSubmissionRepo::default()), repo);
    let mut td = import_pending_observation("title-a", TitleMatchType::Submission);
    td.state = TrackedDownloadState::Failed;
    td.burned_by_import_gate = true;
    td.status_messages = vec!["old quality mismatch".into()];
    assert!(
        crate::tracked_downloads::persist_tracked_download_state_marker(
            &app,
            &td,
            td.state,
            Some("import_gate_rejected"),
            Some("old quality mismatch"),
        )
        .await
    );
    let result =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .unwrap();
    let evidence =
        serde_json::from_value(observation_evidence_json(PAPER_LANTERN_RELEASE)).unwrap();
    crate::import_workflow::reconcile_history_retry_result(
        &app, &mut td, &completed, &evidence, &result,
    )
    .await
    .unwrap();
    assert!(!td.burned_by_import_gate);
    assert_ne!(td.state, TrackedDownloadState::Failed);
    // A policy hold is terminal for this attempt, without burning the release.
    let mut hold = result.clone();
    hold.decision = ImportDecision::Rejected;
    hold.skip_reason = Some(ImportSkipReason::PolicyMismatch);
    hold.error_message = Some("release advertised 2160P but the file is 1440P".into());
    crate::import_workflow::reconcile_history_retry_result(
        &app, &mut td, &completed, &evidence, &hold,
    )
    .await
    .unwrap();
    let restored = crate::tracked_downloads::TrackedDownloadService::build_new_tracked_download(
        &app,
        td.download_id,
        td.id.clone(),
        td.client_item.clone(),
    )
    .await;
    assert_eq!(restored.state, TrackedDownloadState::ImportBlocked);
    assert!(!restored.burned_by_import_gate);
    assert!(
        restored
            .status_messages
            .iter()
            .any(|message| message.contains("1440P"))
    );
}

#[tokio::test]
async fn verified_retry_clears_failed_state_durably() {
    let dir = tempfile::tempdir().unwrap();
    let completed = build_completed_download(
        "Show.S01E01.1080p.WEB-DL",
        dir.path().to_str().unwrap(),
        Some("series"),
    );
    let submissions = Arc::new(TestDownloadSubmissionRepo::default());
    let app = build_app_with_download_client_configs_and_submissions(
        vec![build_title("title-1", "Show", MediaFacet::Series)],
        vec![build_collection("season-1", "title-1", "1")],
        vec![build_episode("ep-1", "title-1", "season-1", "1", "1", None)],
        vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )],
        Arc::new(TestDownloadClient::default()),
        Arc::new(NullDownloadClientConfigRepository),
        submissions.clone(),
    );
    let mut td = build_tracked_download("title-1", "series", "Show.S01E01.1080p.WEB-DL");
    td.state = TrackedDownloadState::Failed;
    td.burned_by_import_gate = true;
    assert!(
        crate::tracked_downloads::persist_tracked_download_state_marker(
            &app,
            &td,
            td.state,
            Some("import_gate_rejected"),
            Some("old quality mismatch"),
        )
        .await
    );
    let evidence =
        serde_json::from_value(observation_evidence_json("Show.S01E01.1080p.WEB-DL")).unwrap();
    let result = scryer_domain::ImportResult {
        import_id: "import-1".into(),
        decision: ImportDecision::Skipped,
        skip_reason: Some(ImportSkipReason::AlreadyImported),
        title_id: Some("title-1".into()),
        source_system: Some("nzbget".into()),
        source_ref: Some("dl-1".into()),
        source_title: Some("Show.S01E01.1080p.WEB-DL".into()),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: Some("1080p".into()),
        episode_ids: vec!["ep-1".into()],
        file_size_bytes: None,
        link_type: None,
        error_message: None,
        release_burned: false,
        started_at: Utc::now(),
        completed_at: Utc::now(),
    };
    crate::import_workflow::reconcile_history_retry_result(
        &app, &mut td, &completed, &evidence, &result,
    )
    .await
    .unwrap();
    assert_eq!(td.state, TrackedDownloadState::Imported);
    assert!(!td.burned_by_import_gate);
    assert!(
        submissions
            .canonical_identity_tracked_state_reasons
            .lock()
            .await
            .is_empty()
    );
    let restored = crate::tracked_downloads::TrackedDownloadService::build_new_tracked_download(
        &app,
        td.download_id,
        td.id.clone(),
        td.client_item.clone(),
    )
    .await;
    assert_eq!(restored.state, TrackedDownloadState::Imported);
    assert!(!restored.burned_by_import_gate);
}

#[tokio::test]
async fn retry_rejects_active_and_successful_imports() {
    for status in [
        ImportStatus::Pending,
        ImportStatus::Processing,
        ImportStatus::Completed,
    ] {
        let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
        let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
            "import-1",
            &source_identity(),
            status,
            completed_request_payload(
                &completed,
                observation_evidence_json(PAPER_LANTERN_RELEASE),
                Some("title-a"),
            ),
        )]));
        let app = app_for_import(Arc::new(TestDownloadSubmissionRepo::default()), repo);
        assert!(
            crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn retry_missing_source_preserves_the_previous_decision() {
    let (dir, mut completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    completed.dest_dir = dir.path().join("missing").to_string_lossy().into_owned();
    let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Skipped,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            Some("title-a"),
        ),
    )]));
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        repo.clone(),
    );
    let error =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .expect_err("missing source must not start an import");
    assert!(error.to_string().contains("source is no longer available"));
    assert_eq!(
        repo.get_import_by_id("import-1")
            .await
            .unwrap()
            .unwrap()
            .status,
        ImportStatus::Skipped
    );
}

#[tokio::test]
async fn retry_cannot_overlap_an_import_of_the_same_source() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Skipped,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            Some("title-a"),
        ),
    )]));
    let app = app_for_import(Arc::new(TestDownloadSubmissionRepo::default()), repo);
    let permit = app
        .runtime
        .imports
        .execution_coordinator
        .try_acquire_source(&completed)
        .await
        .unwrap();
    let error =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .expect_err("active source must reject a second attempt");
    assert!(error.to_string().contains("already being imported"));
    drop(permit);
    assert!(
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn retry_after_tracked_download_is_gone_lands_in_the_persisted_target() {
    // No tracked download and no submission row remain; only the failed
    // import's persisted request knows the download was validated for title-b.
    let (_dir, completed) = completed_without_video(Some(OBFUSCATED_RELEASE));
    let import_repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Failed,
        completed_request_payload(
            &completed,
            observation_evidence_json(OBFUSCATED_RELEASE),
            Some("title-b"),
        ),
    )]));
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        import_repo.clone(),
    );

    let result =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .expect("retry must run");

    assert_eq!(result.title_id.as_deref(), Some("title-b"), "{result:?}");
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
    assert_lands_in(&import_repo, "title-b").await;
}

/// An actor who may resolve imports only in `library_id`.
fn resolve_imports_actor_for(library_id: &str) -> User {
    let mut actor = User::new_admin("resolver");
    actor.authorization = scryer_domain::UserAuthorization {
        libraries: std::collections::HashMap::from([(
            library_id.to_string(),
            scryer_domain::LibraryPermissionMask::from_permissions([
                scryer_domain::LibraryPermission::View,
                scryer_domain::LibraryPermission::ResolveImports,
            ]),
        )]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    actor
}

fn titleless_failed_import_app() -> (tempfile::TempDir, Arc<TestImportRepo>, AppUseCase) {
    // No persisted target and no Scryer submission: the retry learns the
    // title (title-a, a movie) only by parsing the release name.
    let (dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let import_repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Failed,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            None,
        ),
    )]));
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        import_repo.clone(),
    );
    (dir, import_repo, app)
}

#[tokio::test]
async fn titleless_retry_is_refused_when_the_resolved_title_is_in_another_library() {
    let (_dir, import_repo, app) = titleless_failed_import_app();
    let actor = resolve_imports_actor_for(&scryer_domain::default_library_id_for_facet(
        &MediaFacet::Series,
    ));

    let error = crate::import_workflow::retry_failed_import(&app, &actor, "import-1", None)
        .await
        .expect_err("the resolved movie title is outside the actor's library");

    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
    let recorded = import_repo
        .last_import_result()
        .await
        .expect("the refused attempt is recorded");
    assert_eq!(recorded.decision, ImportDecision::Failed, "{recorded:?}");
    assert_ne!(
        recorded.skip_reason,
        Some(ImportSkipReason::NoVideoFiles),
        "the import must stop before scanning the matched title's files: {recorded:?}"
    );
    assert_eq!(
        import_repo
            .get_import_by_id("import-1")
            .await
            .unwrap()
            .unwrap()
            .status,
        ImportStatus::Failed
    );
}

#[tokio::test]
async fn titleless_retry_runs_when_the_resolved_title_is_in_the_actors_library() {
    let (_dir, import_repo, app) = titleless_failed_import_app();
    let actor = resolve_imports_actor_for(&scryer_domain::default_library_id_for_facet(
        &MediaFacet::Movie,
    ));

    let result = crate::import_workflow::retry_failed_import(&app, &actor, "import-1", None)
        .await
        .expect("the actor may resolve imports for the matched title");

    assert_eq!(result.title_id.as_deref(), Some("title-a"), "{result:?}");
    assert_lands_in(&import_repo, "title-a").await;
}

// ── A3: a live submission row is authoritative over persisted evidence ──

#[tokio::test]
async fn retry_prefers_live_reassignment_row_over_persisted_scryer_submission() {
    // Grabbed for title-a, import failed, then the operator reassigned the
    // download to title-b (the row is now an orphan naming title-b). The retry
    // must not replay the persisted ScryerSubmission{title-a}.
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let import_repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Failed,
        completed_request_payload(
            &completed,
            scryer_submission_evidence_json("title-a", PAPER_LANTERN_RELEASE),
            None,
        ),
    )]));
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-b",
            crate::SubmissionScope::Orphan,
            None,
        ))
        .await
        .expect("record reassignment row");
    let app = app_for_import(submission_repo, import_repo.clone());

    let result =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .expect("retry must run");

    assert_eq!(result.title_id.as_deref(), Some("title-b"), "{result:?}");
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
}

#[tokio::test]
async fn retry_uses_persisted_scryer_submission_when_the_row_is_lost() {
    let (_dir, completed) = completed_without_video(Some(OBFUSCATED_RELEASE));
    let import_repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Failed,
        completed_request_payload(
            &completed,
            scryer_submission_evidence_json("title-a", PAPER_LANTERN_RELEASE),
            None,
        ),
    )]));
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        import_repo.clone(),
    );

    let result =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .expect("retry must run");

    assert_eq!(result.title_id.as_deref(), Some("title-a"), "{result:?}");
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
    assert_eq!(
        result.source_title.as_deref(),
        Some(PAPER_LANTERN_RELEASE),
        "the persisted grab-time release title is still THE name for the lost row"
    );
}

#[tokio::test]
async fn automatic_reimport_prefers_live_reassignment_row_over_persisted_evidence() {
    // Same reassignment, but through the tracked download's automatic
    // re-import (prepare_completed_import_request used to replay the newest
    // persisted evidence for the identity regardless of the live row).
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let import_repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Failed,
        completed_request_payload(
            &completed,
            scryer_submission_evidence_json("title-a", PAPER_LANTERN_RELEASE),
            None,
        ),
    )]));
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-b",
            crate::SubmissionScope::Orphan,
            None,
        ))
        .await
        .expect("record reassignment row");
    let app = app_for_import(submission_repo, import_repo.clone());
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = import_pending_observation("title-b", TitleMatchType::Submission);

    import_with_lookup(&app, &import_actor(), &mut td, &lookup).await;

    assert_lands_in(&import_repo, "title-b").await;
    assert_eq!(
        import_repo.last_queued_target_title_id().await.as_deref(),
        Some("title-b")
    );
}

// ── A1: a Scryer submission without a persisted release title stays importable ──

#[tokio::test]
async fn scryer_submission_without_source_title_imports_with_the_client_release_name() {
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-a",
            crate::SubmissionScope::Title,
            None,
        ))
        .await
        .expect("record submission without a release title");
    let import_repo = Arc::new(TestImportRepo::default());
    let app = app_for_import(submission_repo, import_repo.clone());
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download("title-a", "movie", PAPER_LANTERN_RELEASE);
    td.state = TrackedDownloadState::ImportPending;

    import_with_lookup(&app, &import_actor(), &mut td, &lookup).await;

    // Not "Import failed: ... missing its durable source title": the import ran
    // against the Scryer identity with the client-reported release name.
    assert_ne!(
        td.state,
        TrackedDownloadState::ImportBlocked,
        "{:?}",
        td.status_messages
    );
    let result = assert_lands_in(&import_repo, "title-a").await;
    assert_eq!(result.source_title.as_deref(), Some(PAPER_LANTERN_RELEASE));
}

#[tokio::test]
async fn scryer_submission_without_any_release_name_still_imports() {
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-a",
            crate::SubmissionScope::Title,
            None,
        ))
        .await
        .expect("record submission without a release title");
    let import_repo = Arc::new(TestImportRepo::default());
    let app = app_for_import(submission_repo, import_repo.clone());
    let (_dir, completed) = completed_without_video(None);
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download("title-a", "movie", PAPER_LANTERN_RELEASE);
    td.state = TrackedDownloadState::ImportPending;

    import_with_lookup(&app, &import_actor(), &mut td, &lookup).await;

    assert_ne!(
        td.state,
        TrackedDownloadState::ImportBlocked,
        "{:?}",
        td.status_messages
    );
    let result = assert_lands_in(&import_repo, "title-a").await;
    // With no video on disk there is no file stem to fall back to yet.
    assert_eq!(result.source_title, None);
}

#[tokio::test]
async fn retry_of_scryer_submission_without_source_title_does_not_error() {
    // A legacy (completion-only) payload forces the retry to resolve evidence
    // from the live row, which has no persisted release title.
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let import_repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Failed,
        serde_json::to_string(&completed).expect("legacy payload"),
    )]));
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-a",
            crate::SubmissionScope::Title,
            None,
        ))
        .await
        .expect("record submission without a release title");
    let app = app_for_import(submission_repo, import_repo.clone());

    let result =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .expect("retry must not error on a missing persisted release title");

    assert_eq!(result.title_id.as_deref(), Some("title-a"), "{result:?}");
    assert_eq!(result.source_title.as_deref(), Some(PAPER_LANTERN_RELEASE));
}

#[tokio::test]
async fn manual_selection_evidence_for_scryer_submission_without_source_title_does_not_error() {
    // begin_manual_import_selection resolves its evidence through this exact
    // call; it must yield the Scryer identity with the client-reported name.
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-a",
            crate::SubmissionScope::Title,
            None,
        ))
        .await
        .expect("record submission without a release title");
    let app = app_for_import(submission_repo, Arc::new(TestImportRepo::default()));
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));

    let evidence = crate::import_workflow::resolve_release_evidence_for_completed_download(
        &app, &completed, None,
    )
    .await
    .expect("evidence resolution must not error");

    assert_eq!(evidence.title_id(), Some("title-a"));
    assert_eq!(evidence.scope(), Some(&crate::SubmissionScope::Title));
    assert_eq!(
        evidence.release_title(None).as_deref(),
        Some(PAPER_LANTERN_RELEASE)
    );
}

// ── Execution failure before a result exists: automatic retry, never a sticky block ──

#[tokio::test]
async fn pipeline_error_before_result_schedules_automatic_retry_instead_of_blocking() {
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-a",
            crate::SubmissionScope::Title,
            Some(PAPER_LANTERN_RELEASE),
        ))
        .await
        .expect("record submission");
    let import_repo = Arc::new(TestImportRepo::default());
    // The attempt cannot even be recorded: the shape of a DB hiccup mid-refresh.
    import_repo.fail_queueing();
    let app = app_for_import(submission_repo, import_repo.clone());
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download("title-a", "movie", PAPER_LANTERN_RELEASE);
    td.state = TrackedDownloadState::ImportPending;

    let before = Utc::now();
    assert!(!import_with_lookup(&app, &import_actor(), &mut td, &lookup).await);

    // Sonarr leaves the item in place and re-processes it on the next refresh;
    // Scryer does the same behind the capped execution backoff.
    assert_eq!(
        td.state,
        TrackedDownloadState::ImportPending,
        "{:?}",
        td.status_messages
    );
    assert_eq!(td.status, TrackedDownloadStatus::Warning);
    let retry = td
        .import_execution_retry
        .as_ref()
        .expect("pipeline error must schedule an execution retry");
    assert_eq!(retry.attempts, 1);
    assert!(retry.next_retry_at >= before + chrono::Duration::seconds(30));
    assert!(td.import_retry_deferred(Utc::now()));
    assert!(
        td.status_messages[0].starts_with("Import failed: ")
            && td.status_messages[0].contains("simulated import queue failure")
            && td.status_messages[0].contains("Retrying automatically (attempt 1)"),
        "{:?}",
        td.status_messages
    );
    assert!(
        import_repo.last_import_result().await.is_none(),
        "no attempt row exists, so nothing is recorded as failed"
    );
}

#[tokio::test]
async fn execution_error_after_the_attempt_row_exists_never_writes_failed() {
    // A pipeline error once the import row exists (`finalize_completed_import_error`)
    // must decide retryability BEFORE the status write: a `Failed` write emits
    // an `ImportRejected` domain event on every automatic re-attempt.
    let submission_repo = Arc::new(TestDownloadSubmissionRepo::default());
    submission_repo
        .record_submission(submission_row(
            "title-a",
            crate::SubmissionScope::Title,
            Some(PAPER_LANTERN_RELEASE),
        ))
        .await
        .expect("record submission");
    let import_repo = Arc::new(TestImportRepo::default());
    let app = app_for_import(submission_repo, import_repo.clone());
    let dir = tempfile::tempdir().expect("completed dir");
    // A real video makes the import execute; the fixture app has no root
    // folder configured for the title, so execution errs after the row exists.
    std::fs::write(
        dir.path().join("Paper.Lantern.2012.1080p.WEB-DL.mkv"),
        vec![0u8; 4096],
    )
    .expect("write video");
    let mut completed = build_completed_download(
        "downloader display label",
        dir.path().to_string_lossy().as_ref(),
        Some("movie"),
    );
    completed.release_name = Some(PAPER_LANTERN_RELEASE.to_string());
    let lookup =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);
    let mut td = build_tracked_download("title-a", "movie", PAPER_LANTERN_RELEASE);
    td.state = TrackedDownloadState::ImportPending;

    assert!(!import_with_lookup(&app, &import_actor(), &mut td, &lookup).await);

    assert_eq!(
        td.state,
        TrackedDownloadState::ImportPending,
        "{:?}",
        td.status_messages
    );
    assert!(
        td.import_execution_retry.is_some(),
        "{:?}",
        td.status_messages
    );
    let updates = import_repo.status_updates.lock().await.clone();
    let statuses = updates
        .iter()
        .map(|(_, status, _)| *status)
        .collect::<Vec<_>>();
    assert!(
        statuses.contains(&ImportStatus::Processing) && statuses.contains(&ImportStatus::Pending),
        "{statuses:?}"
    );
    assert!(
        !statuses.contains(&ImportStatus::Failed),
        "an automatically retried attempt must not be recorded as failed: {statuses:?}"
    );
    let result = import_repo
        .last_import_result()
        .await
        .expect("the attempt records its result");
    assert_eq!(result.decision, ImportDecision::Failed);
    assert!(
        result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("root folder")),
        "{result:?}"
    );
}

struct RetryObservationClient {
    state: Option<DownloadQueueState>,
    unavailable: bool,
    unknown: bool,
    conflicting_identity: bool,
}

#[async_trait]
impl DownloadClient for RetryObservationClient {
    async fn submit_download(&self, _: &DownloadClientAddRequest) -> AppResult<DownloadGrabResult> {
        panic!("Retry import must never submit a download")
    }
    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        Ok(vec![])
    }
    async fn observe_download(
        &self,
        _: &ClientJobLocator,
        _: usize,
    ) -> AppResult<crate::DownloadClientObservation> {
        if self.unavailable {
            return Err(AppError::Repository("client unavailable".into()));
        }
        if self.unknown {
            return Ok(crate::DownloadClientObservation::Unknown {
                reason: "history unavailable".into(),
                next_history_offset: 0,
            });
        }
        Ok(match self.state {
            Some(state) => {
                let mut item =
                    build_tracked_download("title-a", "movie", PAPER_LANTERN_RELEASE).client_item;
                item.state = state;
                if self.conflicting_identity {
                    item.download_client_item_id = "different-job".into();
                }
                crate::DownloadClientObservation::Present(Box::new(item))
            }
            None => crate::DownloadClientObservation::Absent,
        })
    }
}

fn retry_client_config() -> DownloadClientConfig {
    DownloadClientConfig {
        id: "client-1".into(),
        name: "Fixture client".into(),
        client_type: "nzbget".into(),
        config_json: "{}".into(),
        is_enabled: true,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        client_priority: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        proxy_config_id: None,
    }
}

#[tokio::test]
async fn retry_eligibility_requires_completed_or_retained_source_and_rejects_client_errors() {
    for (state, unavailable, unknown, conflicting_identity, eligible) in [
        (
            Some(DownloadQueueState::Completed),
            false,
            false,
            false,
            true,
        ),
        (
            Some(DownloadQueueState::ImportPending),
            false,
            false,
            false,
            true,
        ),
        (None, false, false, false, true),
        (Some(DownloadQueueState::Failed), false, false, false, false),
        (
            Some(DownloadQueueState::Downloading),
            false,
            false,
            false,
            false,
        ),
        (Some(DownloadQueueState::Queued), false, false, false, false),
        (
            Some(DownloadQueueState::Verifying),
            false,
            false,
            false,
            false,
        ),
        (
            Some(DownloadQueueState::Repairing),
            false,
            false,
            false,
            false,
        ),
        (
            Some(DownloadQueueState::Extracting),
            false,
            false,
            false,
            false,
        ),
        (
            Some(DownloadQueueState::Completed),
            false,
            false,
            true,
            false,
        ),
        (None, true, false, false, false),
        (None, false, true, false, false),
    ] {
        let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
        let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
            "import-1",
            &source_identity(),
            ImportStatus::Skipped,
            completed_request_payload(
                &completed,
                observation_evidence_json(PAPER_LANTERN_RELEASE),
                Some("title-a"),
            ),
        )]));
        let app = build_app_with_download_client_configs_and_submissions(
            paper_lantern_titles(),
            vec![],
            vec![],
            vec![],
            Arc::new(RetryObservationClient {
                state,
                unavailable,
                unknown,
                conflicting_identity,
            }),
            Arc::new(TestDownloadClientConfigRepo {
                configs: vec![retry_client_config()],
            }),
            Arc::new(TestDownloadSubmissionRepo::default()),
        )
        .with_test_overrides(|services| services.with_imports(repo.clone()));
        let result =
            crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
                .await;
        assert_eq!(
            result.is_ok(),
            eligible,
            "{state:?}, unavailable={unavailable}, unknown={unknown}: {result:?}"
        );
        if !eligible {
            assert_eq!(
                repo.get_import_by_id("import-1")
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                ImportStatus::Skipped
            );
            assert!(repo.retry_claims.lock().await.is_empty());
        }
    }
}

#[tokio::test]
async fn retry_recovery_reconciles_recorded_result_without_executing_again() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Skipped,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            Some("title-a"),
        ),
    )]));
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        repo.clone(),
    );
    repo.retry_finish_fail.store(true, Ordering::SeqCst);
    let error =
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("reconciliation failure"));
    let before = repo.get_import_by_id("import-1").await.unwrap().unwrap();
    let claim = repo
        .list_import_retry_recovery(None, 25)
        .await
        .unwrap()
        .remove(0);
    repo.retry_finish_fail.store(false, Ordering::SeqCst);
    // A new runtime has no in-memory reservation or client history.
    let restarted = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        repo.clone(),
    );
    crate::import_workflow::recover_import_retry(&restarted, &claim)
        .await
        .unwrap();
    let after = repo.get_import_by_id("import-1").await.unwrap().unwrap();
    assert_eq!(
        before.result_json, after.result_json,
        "recovery must not execute import again"
    );
    assert!(repo.retry_claims.lock().await.is_empty());
    assert_eq!(
        *repo.retry_finished_states.lock().await,
        vec![TrackedDownloadState::ImportBlocked]
    );
    // A stale page of recovery work must not replace the finished result.
    crate::import_workflow::recover_import_retry(&restarted, &claim)
        .await
        .unwrap();
    assert_eq!(repo.retry_finished_states.lock().await.len(), 1);
}

#[tokio::test]
async fn retry_recovery_interrupted_execution_becomes_explicitly_retryable_hold() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let payload = completed_request_payload(
        &completed,
        observation_evidence_json(PAPER_LANTERN_RELEASE),
        Some("title-a"),
    );
    let record = test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Skipped,
        payload.clone(),
    );
    let expected = chrono::DateTime::parse_from_rfc3339(&record.updated_at)
        .unwrap()
        .with_timezone(&Utc);
    let repo = Arc::new(TestImportRepo::with_records(vec![record]));
    let claim = crate::ImportRetryClaim {
        download_id: repo
            .canonical_download_id_for_import("import-1")
            .await
            .unwrap()
            .unwrap(),
        import_id: "import-1".into(),
        attempt_id: "interrupted-attempt".into(),
        started_at: Utc::now(),
        source: source_identity(),
        previous_result_json: None,
    };
    assert!(
        repo.claim_import_retry(&claim, expected, &payload)
            .await
            .unwrap()
            .is_claimed()
    );
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        repo.clone(),
    );
    crate::import_workflow::recover_import_retry(&app, &claim)
        .await
        .unwrap();
    assert_eq!(
        *repo.retry_finished_states.lock().await,
        vec![TrackedDownloadState::ImportBlocked]
    );
    assert!(
        repo.last_import_result()
            .await
            .unwrap()
            .error_message
            .unwrap()
            .contains("interrupted")
    );
    assert!(Path::new(&completed.dest_dir).exists());
    assert!(
        crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn retry_recovery_verifies_partial_episode_artifacts_without_client_history() {
    for complete in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let completed = build_completed_download(
            "Show.S01E01E02.1080p.WEB-DL",
            dir.path().to_str().unwrap(),
            Some("series"),
        );
        let payload = completed_request_payload(
            &completed,
            observation_evidence_json("Show.S01E01E02.1080p.WEB-DL"),
            Some("title-1"),
        );
        let record = test_import_record(
            "import-1",
            &source_identity(),
            ImportStatus::Skipped,
            payload.clone(),
        );
        let expected = chrono::DateTime::parse_from_rfc3339(&record.updated_at)
            .unwrap()
            .with_timezone(&Utc);
        let repo = Arc::new(TestImportRepo::with_records(vec![record]));
        let claim = crate::ImportRetryClaim {
            download_id: repo
                .canonical_download_id_for_import("import-1")
                .await
                .unwrap()
                .unwrap(),
            import_id: "import-1".into(),
            attempt_id: "interrupted-series".into(),
            started_at: Utc::now(),
            source: source_identity(),
            previous_result_json: None,
        };
        assert!(
            repo.claim_import_retry(&claim, expected, &payload)
                .await
                .unwrap()
                .is_claimed()
        );
        let mut artifacts = vec![build_artifact_with_result(
            "dl-1",
            Some("ep-1"),
            "Show.S01E01.mkv",
            "already_present",
        )];
        if complete {
            artifacts.push(build_artifact_with_result(
                "dl-1",
                Some("ep-2"),
                "Show.S01E02.mkv",
                "already_present",
            ));
        }
        let app = build_app_with_download_client_configs_and_submissions(
            vec![build_title("title-1", "Show", MediaFacet::Series)],
            vec![build_collection("season-1", "title-1", "1")],
            vec![
                build_episode("ep-1", "title-1", "season-1", "1", "1", None),
                build_episode("ep-2", "title-1", "season-1", "1", "2", None),
            ],
            artifacts,
            Arc::new(TestDownloadClient::default()),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(TestDownloadSubmissionRepo::default()),
        )
        .with_test_overrides(|services| services.with_imports(repo.clone()));
        crate::import_workflow::recover_import_retry(&app, &claim)
            .await
            .unwrap();
        assert_eq!(
            *repo.retry_finished_states.lock().await,
            vec![if complete {
                TrackedDownloadState::Imported
            } else {
                TrackedDownloadState::ImportBlocked
            }]
        );
        assert!(repo.retry_claims.lock().await.is_empty());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "recovery must not create or copy files"
        );
    }
}

#[tokio::test]
async fn retry_legacy_gate_failure_is_recoverable_but_download_failure_is_not() {
    for gate_failure in [false, true] {
        let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
        let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
            "import-1",
            &source_identity(),
            ImportStatus::Failed,
            completed_request_payload(
                &completed,
                observation_evidence_json(PAPER_LANTERN_RELEASE),
                Some("title-a"),
            ),
        )]));
        let submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let app = app_for_import(submissions, repo.clone());
        let mut tracked = import_pending_observation("title-a", TitleMatchType::Submission);
        tracked.download_id = repo
            .canonical_download_id_for_import("import-1")
            .await
            .unwrap()
            .unwrap();
        assert!(
            crate::tracked_downloads::persist_tracked_download_state_marker(
                &app,
                &tracked,
                TrackedDownloadState::Failed,
                gate_failure.then_some("import_gate_rejected"),
                None
            )
            .await
        );
        let result =
            crate::import_workflow::retry_failed_import(&app, &import_actor(), "import-1", None)
                .await;
        assert_eq!(result.is_ok(), gate_failure, "{result:?}");
    }
}

#[tokio::test]
async fn history_and_tracked_retry_share_the_same_source_reservation() {
    let (_dir, completed) = completed_without_video(Some(PAPER_LANTERN_RELEASE));
    let repo = Arc::new(TestImportRepo::with_records(vec![test_import_record(
        "import-1",
        &source_identity(),
        ImportStatus::Skipped,
        completed_request_payload(
            &completed,
            observation_evidence_json(PAPER_LANTERN_RELEASE),
            Some("title-a"),
        ),
    )]));
    let app = app_for_import(
        Arc::new(TestDownloadSubmissionRepo::default()),
        repo.clone(),
    );
    let mut tracked = import_pending_observation("title-a", TitleMatchType::Submission);
    tracked.download_id = repo
        .canonical_download_id_for_import("import-1")
        .await
        .unwrap()
        .unwrap();
    let _permit = app
        .runtime
        .imports
        .execution_coordinator
        .try_acquire_source(&completed)
        .await
        .unwrap();
    let actor = import_actor();
    let (history, activity) = tokio::join!(
        crate::import_workflow::retry_failed_import(&app, &actor, "import-1", None),
        crate::import_workflow::retry_tracked_import(&app, &actor, &tracked)
    );
    assert!(
        history
            .unwrap_err()
            .to_string()
            .contains("already being imported")
    );
    assert!(
        activity
            .unwrap_err()
            .to_string()
            .contains("already being imported")
    );
}

#[tokio::test]
async fn retry_verification_does_not_accept_artifacts_from_a_previous_title_assignment() {
    let dir = tempfile::tempdir().unwrap();
    let completed = build_completed_download(
        "Current.Movie.1080p",
        dir.path().to_str().unwrap(),
        Some("movie"),
    );
    let evidence =
        serde_json::from_value(observation_evidence_json("Current.Movie.1080p")).unwrap();
    for artifact_title in ["title-1", "previous-title"] {
        let mut artifact =
            build_artifact_with_result("dl-1", None, "Current.Movie.mkv", "imported");
        artifact.title_id = Some(artifact_title.into());
        let app = build_app_with_download_client_configs_and_submissions(
            vec![build_title("title-1", "Current Movie", MediaFacet::Movie)],
            vec![],
            vec![],
            vec![artifact],
            Arc::new(TestDownloadClient::default()),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(TestDownloadSubmissionRepo::default()),
        );
        let tracked = build_tracked_download("title-1", "movie", "Current.Movie.1080p");
        let verified =
            crate::completed_download_handler::verify_retry_import_with_release_evidence(
                &app, &tracked, 0, &completed, &evidence,
            )
            .await
            .unwrap();
        assert_eq!(verified, artifact_title == "title-1");
    }
}
