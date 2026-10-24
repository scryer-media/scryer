//! A Scryer grab whose download finishes before the grab is recorded.
//!
//! A fast client can complete a job, and the tracker can see it complete,
//! before `submit_download` has even returned to the submission path. These
//! tests hold the fake client's answer until the tracker has observed the
//! finished job, so that order is forced rather than raced.

use super::*;

use scryer_application::DownloadRegistryRepository;
use scryer_application::{
    DownloadClient, DownloadClientAddRequest, DownloadGrabResult, DownloadQueuePollerOptions,
    QueuedReleaseSelection, SubmissionConflictPolicy, start_download_queue_poller_with_options,
    tracked_downloads::{
        TrackedDownloadSnapshotIngestHandle, TrackedDownloadSnapshotScope,
        TrackedDownloadSnapshotUpdate,
    },
};
use scryer_domain::download_identity::DownloadId;
use scryer_domain::{DownloadQueueItem, DownloadQueueState};
use scryer_infrastructure_workflow::workflow::stores::DownloadRegistryStore;
use tokio::sync::{Semaphore, mpsc};

const RACE_CLIENT_ID: &str = "race-fixture-client";
const RACE_CLIENT_TYPE: &str = "weaver";
const RACE_JOB_ID: &str = "race-fixture-job-1";

/// How the fake client answers once the test lets it.
#[derive(Clone, Copy)]
enum RaceAnswer {
    Accept,
    /// A definitive refusal: the client holds nothing.
    Refuse,
    /// The mutation may have reached the client, which never said.
    Ambiguous,
}

/// A download client that reports each submission to the test and then waits
/// for the test to let it answer.
struct GatedClient {
    reached: mpsc::UnboundedSender<DownloadClientAddRequest>,
    release: Semaphore,
    answer: std::sync::Mutex<RaceAnswer>,
}

#[async_trait::async_trait]
impl DownloadClient for GatedClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> scryer_application::AppResult<DownloadGrabResult> {
        self.reached
            .send(request.clone())
            .expect("the test listens for submissions");
        self.release
            .acquire()
            .await
            .expect("the gate stays open")
            .forget();
        let answer = *self.answer.lock().expect("answer lock");
        match answer {
            RaceAnswer::Accept => Ok(DownloadGrabResult {
                job_id: RACE_JOB_ID.to_string(),
                client_id: Some(RACE_CLIENT_ID.to_string()),
                client_type: RACE_CLIENT_TYPE.to_string(),
                info_hash: None,
                download_id: request.download_id,
                seed_goals: None,
                grabbed_file_names: Vec::new(),
            }),
            RaceAnswer::Refuse => Err(scryer_application::AppError::Validation(
                "the fixture client refuses this release".to_string(),
            )),
            RaceAnswer::Ambiguous => Err(scryer_application::AppError::DownloadSubmitAmbiguous(
                "the fixture client never answered".to_string(),
            )
            .with_ambiguous_download_submission_client(
                Some(RACE_CLIENT_ID.to_string()),
                RACE_CLIENT_TYPE.to_string(),
            )),
        }
    }
}

struct RaceHarness {
    ctx: TestContext,
    app: scryer_application::AppUseCase,
    client: Arc<GatedClient>,
    reached: mpsc::UnboundedReceiver<DownloadClientAddRequest>,
    ingest: TrackedDownloadSnapshotIngestHandle,
    poller_token: tokio_util::sync::CancellationToken,
    poller: tokio::task::JoinHandle<()>,
    title: Title,
    _media_root: tempfile::TempDir,
}

impl RaceHarness {
    async fn new(answer: RaceAnswer) -> Self {
        let ctx = TestContext::new().await;
        let media_root = tempfile::tempdir().expect("media root");
        let title = add_movie_title(
            &ctx,
            "race-fixture-title",
            "Race Fixture Movie",
            &media_root.path().to_string_lossy(),
        )
        .await;
        let (reached_tx, reached) = mpsc::unbounded_channel();
        let client = Arc::new(GatedClient {
            reached: reached_tx,
            release: Semaphore::new(0),
            answer: std::sync::Mutex::new(answer),
        });
        let base = app_with_real_imports(&ctx).await;
        let fake: Arc<dyn DownloadClient> = client.clone();
        let app = base.with_test_overrides(|builder| builder.with_download_client(fake));
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (snapshot_tx, snapshot_rx) = mpsc::channel(8);
        let poller_token = tokio_util::sync::CancellationToken::new();
        let poller = tokio::spawn(start_download_queue_poller_with_options(
            app.clone(),
            poller_token.child_token(),
            command_rx,
            snapshot_rx,
            DownloadQueuePollerOptions {
                interval: Duration::from_secs(3600),
                // The fixture client has no listing; only published deltas
                // reach the tracker.
                excluded_client_types: vec![RACE_CLIENT_TYPE.to_string()],
                ..Default::default()
            },
        ));
        Self {
            ctx,
            app,
            client,
            reached,
            ingest: TrackedDownloadSnapshotIngestHandle::new(snapshot_tx),
            poller_token,
            poller,
            title,
            _media_root: media_root,
        }
    }

    /// Start an additional-file grab for the fixture movie.
    fn spawn_additional_file_grab(
        &self,
    ) -> tokio::task::JoinHandle<
        scryer_application::AppResult<scryer_application::QueueDownloadOutcome>,
    > {
        let app = self.app.clone();
        let title_id = self.title.id.clone();
        tokio::spawn(async move {
            let actor = app
                .find_or_create_default_user()
                .await
                .expect("default user");
            app.queue_existing_title_download_with_purpose(
                &actor,
                &title_id,
                QueuedReleaseSelection {
                    indexer_id: None,
                    source_hint: None,
                    source_kind: None,
                    source_title: Some("Race.Fixture.Movie.2024.Extras.1080p.WEB-DL".to_string()),
                    source_password: None,
                    info_hash_hint: None,
                    size_bytes: Some(1_000),
                    seeders: None,
                },
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
                DownloadSubmissionPurpose::AdditionalFile,
            )
            .await
        })
    }

    /// Publish the finished job the client reports for `request`, carrying
    /// the parameters a Scryer submission stamps on it.
    async fn publish_completed(
        &self,
        request: &DownloadClientAddRequest,
        dest_dir: &Path,
        completed_at: chrono::DateTime<chrono::Utc>,
    ) {
        let download_id = request.download_id.expect("a grab carries its id");
        let wire = download_id.to_wire();
        let parameters = vec![
            ("*scryer_title_id".to_string(), self.title.id.clone()),
            ("*scryer_facet".to_string(), "movie".to_string()),
            (
                "*scryer_import_purpose".to_string(),
                request.purpose.as_str().to_string(),
            ),
            ("*scryer_download_id".to_string(), wire.clone()),
        ];
        let name = "Race.Fixture.Movie.2024.Extras.1080p.WEB-DL".to_string();
        let item = DownloadQueueItem {
            id: RACE_JOB_ID.to_string(),
            title_id: Some(self.title.id.clone()),
            episode_id: None,
            title_name: name.clone(),
            facet: Some("movie".to_string()),
            category: request.category.clone(),
            client_id: RACE_CLIENT_ID.to_string(),
            client_name: "Race Fixture".to_string(),
            client_type: RACE_CLIENT_TYPE.to_string(),
            state: DownloadQueueState::Completed,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: Some(1_000),
            remaining_seconds: Some(0),
            queued_at: Some(chrono::Utc::now().to_rfc3339()),
            last_updated_at: Some(chrono::Utc::now().to_rfc3339()),
            attention_required: false,
            attention_reason: None,
            download_client_item_id: RACE_JOB_ID.to_string(),
            download_id: Some(wire.clone()),
            import_status: None,
            import_type: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            source_provider: None,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        };
        let completed = CompletedDownload {
            subtitle_paths: Vec::new(),
            client_type: RACE_CLIENT_TYPE.to_string(),
            client_id: RACE_CLIENT_ID.to_string(),
            download_client_item_id: RACE_JOB_ID.to_string(),
            download_id: Some(wire),
            name,
            release_name: None,
            dest_dir: dest_dir.to_string_lossy().into_owned(),
            category: request.category.clone(),
            size_bytes: Some(1_000),
            completed_at: Some(completed_at),
            parameters,
            posted_file_names: Vec::new(),
        };
        self.ingest
            .publish(TrackedDownloadSnapshotUpdate {
                scope: TrackedDownloadSnapshotScope::Delta,
                items: vec![item],
                completed_downloads: vec![completed],
                actor_id: None,
            })
            .await
            .expect("publish the finished job");
    }

    fn submissions(&self) -> DownloadSubmissionStore {
        DownloadSubmissionStore::new(self.ctx.db.datastore())
    }

    /// The title's recorded grabs that no client job has bound yet.
    async fn unbound_intents(&self) -> Vec<scryer_application::DownloadSubmission> {
        self.submissions()
            .list_active_unbound_for_title(&self.title.id)
            .await
            .expect("list unbound submissions")
    }

    /// Let the client answer the grab it is holding.
    fn answer(&self, answer: RaceAnswer) {
        *self.client.answer.lock().expect("answer lock") = answer;
        self.client.release.add_permits(1);
    }

    async fn stop(self) {
        self.poller_token.cancel();
        self.poller.await.expect("poller stops cleanly");
    }
}

async fn within_bound<T>(what: &str, future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(common::WAIT_UNTIL_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out after {:?} waiting for {what}",
                common::WAIT_UNTIL_TIMEOUT
            )
        })
}

/// The intent a grab records before its mutation reaches the client.
fn assert_is_the_grabs_intent(
    intents: &[scryer_application::DownloadSubmission],
    download_id: DownloadId,
) {
    assert_eq!(intents.len(), 1, "one recorded intent: {intents:?}");
    let intent = &intents[0];
    assert_eq!(intent.download_id, download_id);
    assert_eq!(intent.purpose, DownloadSubmissionPurpose::AdditionalFile);
    assert_eq!(intent.scope, SubmissionScope::Title);
    assert_eq!(
        intent.source_title.as_deref(),
        Some("Race.Fixture.Movie.2024.Extras.1080p.WEB-DL")
    );
}

#[tokio::test]
async fn a_grab_whose_job_finishes_before_the_client_answers_still_returns_and_records_its_purpose()
{
    let mut harness = RaceHarness::new(RaceAnswer::Accept).await;
    let grab = harness.spawn_additional_file_grab();
    let request = within_bound("the grab to reach the client", harness.reached.recv())
        .await
        .expect("the grab reaches the client");
    let download_id: DownloadId = request.download_id.expect("a grab carries its id");
    // The intent was durable before the client saw the mutation.
    assert_is_the_grabs_intent(&harness.unbound_intents().await, download_id);

    let source_dir = tempfile::tempdir().expect("source dir");
    harness
        .publish_completed(&request, source_dir.path(), chrono::Utc::now())
        .await;
    let submissions = harness.submissions();
    common::wait_until("the tracker to bind the finished job to the intent", || {
        let submissions = &submissions;
        async move {
            submissions
                .find_by_canonical_download_id(&download_id)
                .await
                .expect("read the canonical submission")
                .is_some_and(|submission| submission.download_client_item_id == RACE_JOB_ID)
        }
    })
    .await;

    harness.answer(RaceAnswer::Accept);
    let outcome = within_bound("the grab to return", grab)
        .await
        .expect("the grab task completes");
    assert!(
        matches!(
            outcome,
            Ok(scryer_application::QueueDownloadOutcome::Queued(_))
        ),
        "the grab is queued: {outcome:?}"
    );

    let recorded = submissions
        .find_by_canonical_download_id(&download_id)
        .await
        .expect("read the canonical submission")
        .expect("the grab is recorded under its own id");
    assert_eq!(recorded.title_id, harness.title.id);
    assert_eq!(recorded.purpose, DownloadSubmissionPurpose::AdditionalFile);
    assert_eq!(recorded.download_client_item_id, RACE_JOB_ID);
    assert_eq!(recorded.download_client_id.as_deref(), Some(RACE_CLIENT_ID));
    assert!(harness.unbound_intents().await.is_empty());
    let binding = DownloadRegistryStore::new(harness.ctx.db.datastore())
        .load_binding(&download_id)
        .await
        .expect("read the binding")
        .expect("the grab keeps one binding");
    assert_eq!(binding.native_item_id.as_deref(), Some(RACE_JOB_ID));
    assert!(binding.ended_at.is_none());

    harness.stop().await;
}

#[tokio::test]
async fn a_job_imported_before_the_client_answered_imports_with_its_grabs_purpose() {
    let mut harness = RaceHarness::new(RaceAnswer::Accept).await;
    let grab = harness.spawn_additional_file_grab();
    let request = within_bound("the grab to reach the client", harness.reached.recv())
        .await
        .expect("the grab reaches the client");
    let download_id: DownloadId = request.download_id.expect("a grab carries its id");

    let source_dir = tempfile::tempdir().expect("source dir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Race.Fixture.Movie.2024.Extras.1080p.WEB-DL.mkv",
    );
    // Completed long enough ago that the import would no longer wait for a
    // missing grab record.
    harness
        .publish_completed(
            &request,
            source_dir.path(),
            chrono::Utc::now() - chrono::Duration::minutes(5),
        )
        .await;
    let imports = ImportStore::new(harness.ctx.db.datastore());
    common::wait_until("the import to settle the finished job", || {
        let imports = &imports;
        async move {
            imports
                .list_imports(100)
                .await
                .expect("list imports")
                .iter()
                .any(|record| record.finished_at.is_some())
        }
    })
    .await;

    let records = imports.list_imports(100).await.expect("list imports");
    assert_eq!(records.len(), 1, "one import: {records:?}");
    let payload: serde_json::Value =
        serde_json::from_str(&records[0].payload_json).expect("import payload is JSON");
    let evidence = &payload["release_evidence"]["ScryerSubmission"];
    assert_eq!(
        evidence["purpose"], "AdditionalFile",
        "the import resolved the grab's intent: {payload}"
    );
    assert_eq!(evidence["title_id"], harness.title.id.as_str());
    let result: serde_json::Value = serde_json::from_str(
        records[0]
            .result_json
            .as_deref()
            .expect("a settled import has a result"),
    )
    .expect("import result is JSON");
    assert_eq!(result["decision"], "imported", "import result: {result}");

    harness.answer(RaceAnswer::Accept);
    let outcome = within_bound("the grab to return", grab)
        .await
        .expect("the grab task completes");
    assert!(
        matches!(
            outcome,
            Ok(scryer_application::QueueDownloadOutcome::Queued(_))
        ),
        "the grab is queued: {outcome:?}"
    );
    let recorded = harness
        .submissions()
        .find_by_canonical_download_id(&download_id)
        .await
        .expect("read the canonical submission")
        .expect("the grab is recorded under its own id");
    assert_eq!(recorded.purpose, DownloadSubmissionPurpose::AdditionalFile);
    assert_eq!(recorded.download_client_item_id, RACE_JOB_ID);
    harness.stop().await;
}

#[tokio::test]
async fn a_refused_grab_leaves_no_intent_behind_and_does_not_hold_a_retry() {
    let mut harness = RaceHarness::new(RaceAnswer::Refuse).await;
    let grab = harness.spawn_additional_file_grab();
    let request = within_bound("the grab to reach the client", harness.reached.recv())
        .await
        .expect("the grab reaches the client");
    let refused_id: DownloadId = request.download_id.expect("a grab carries its id");
    assert_is_the_grabs_intent(&harness.unbound_intents().await, refused_id);

    harness.answer(RaceAnswer::Refuse);
    let outcome = within_bound("the grab to return", grab)
        .await
        .expect("the grab task completes");
    assert!(outcome.is_err(), "the refused grab fails: {outcome:?}");

    assert!(harness.unbound_intents().await.is_empty());
    assert!(
        harness
            .submissions()
            .find_by_canonical_download_id(&refused_id)
            .await
            .expect("read the canonical submission")
            .is_none()
    );
    let registry = DownloadRegistryStore::new(harness.ctx.db.datastore());
    assert!(
        registry
            .load_binding(&refused_id)
            .await
            .expect("read the binding")
            .is_none()
    );
    assert!(
        registry
            .load_download(&refused_id)
            .await
            .expect("read the download")
            .is_none()
    );

    // A retry reaches the client and is recorded as usual.
    let retry = harness.spawn_additional_file_grab();
    let retried = within_bound("the retry to reach the client", harness.reached.recv())
        .await
        .expect("the retry reaches the client");
    harness.answer(RaceAnswer::Accept);
    let outcome = within_bound("the retry to return", retry)
        .await
        .expect("the retry task completes");
    assert!(
        matches!(
            outcome,
            Ok(scryer_application::QueueDownloadOutcome::Queued(_))
        ),
        "the retry is queued: {outcome:?}"
    );
    let retried_id = retried.download_id.expect("a grab carries its id");
    assert_ne!(retried_id, refused_id);
    let recorded = harness
        .submissions()
        .find_by_canonical_download_id(&retried_id)
        .await
        .expect("read the canonical submission")
        .expect("the retry is recorded");
    assert_eq!(recorded.download_client_item_id, RACE_JOB_ID);
    harness.stop().await;
}

#[tokio::test]
async fn an_ambiguous_submit_keeps_its_intent_unbound_and_names_the_client() {
    let mut harness = RaceHarness::new(RaceAnswer::Ambiguous).await;
    let grab = harness.spawn_additional_file_grab();
    let request = within_bound("the grab to reach the client", harness.reached.recv())
        .await
        .expect("the grab reaches the client");
    let download_id: DownloadId = request.download_id.expect("a grab carries its id");

    harness.answer(RaceAnswer::Ambiguous);
    let outcome = within_bound("the grab to return", grab)
        .await
        .expect("the grab task completes");
    assert!(
        matches!(
            outcome,
            Err(ref error) if error.is_download_submit_ambiguous()
        ),
        "the grab stays ambiguous: {outcome:?}"
    );

    let intents = harness.unbound_intents().await;
    assert_is_the_grabs_intent(&intents, download_id);
    assert_eq!(
        intents[0].download_client_id.as_deref(),
        Some(RACE_CLIENT_ID)
    );
    assert_eq!(intents[0].download_client_type, RACE_CLIENT_TYPE);
    let binding = DownloadRegistryStore::new(harness.ctx.db.datastore())
        .load_binding(&download_id)
        .await
        .expect("read the binding")
        .expect("the ambiguous grab keeps its binding");
    assert!(binding.native_item_id.is_none());
    assert!(binding.ended_at.is_none());
    assert_eq!(binding.client_config_id.as_deref(), Some(RACE_CLIENT_ID));
    harness.stop().await;
}
