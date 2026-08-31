//! Import-target and release-evidence threading through the tracked-download
//! import stage (`import_with_lookup` → `prepare_completed_import_request` →
//! `run_import`) and through `retry_failed_import`.
//!
//! The runs deliberately end at `resolve_completed_import_target` (an empty
//! completed directory yields a `NoVideoFiles` skip that carries the resolved
//! `title_id`), which is exactly the decision under test: which title the
//! import lands in and which release name it parses.

use super::*;
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
