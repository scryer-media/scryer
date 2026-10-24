//! Grab intents no download client ever confirmed: a process that died
//! between recording a grab's intent and the client's answer, or a client
//! that answered ambiguously and never listed the job, leaves an active
//! binding with no client item. Released once they are provably absent, they
//! stop blocking the title's moves and grabs.

use super::*;

const RELEASE_TITLE: &str = "D.Gray-man.S01.1080p.WEB-DL";
/// The one download client the acquisition bootstrap configures.
const CLIENT_ID: &str = "background-search-default-client";

struct UnconfirmedGrabFixture {
    app: AppUseCase,
    title: Title,
    wanted: Option<AcquisitionScopeState>,
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    wanted_items: Arc<TrackingAcquisitionScopeStateRepo>,
}

async fn unconfirmed_grab_fixture(seed_wanted: bool) -> UnconfirmedGrabFixture {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );
    // Observations go through the registry so a listed intent can be bound.
    let app = app.with_test_overrides(|services| {
        services.with_download_registry(Arc::new(FixtureDownloadRegistry::default()))
    });
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "D.Gray-man".into(),
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
    // The configured client answers the snapshot unless a test says otherwise.
    download_client
        .set_snapshot_authoritative_client_ids([CLIENT_ID.to_string()])
        .await;
    let wanted = if seed_wanted {
        let wanted = AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(30)).to_rfc3339()),
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        wanted_items
            .upsert_acquisition_scope_state(&wanted)
            .await
            .expect("seed wanted item");
        Some(wanted)
    } else {
        None
    };
    UnconfirmedGrabFixture {
        app,
        title,
        wanted,
        download_client,
        download_submissions,
        wanted_items,
    }
}

impl UnconfirmedGrabFixture {
    /// Record a grab intent the way the submission path does before it
    /// submits (no client item yet), submitted at `submitted_at`.
    async fn record_intent(
        &self,
        download_client_id: Option<&str>,
        submitted_at: chrono::DateTime<Utc>,
    ) -> scryer_domain::download_identity::DownloadId {
        let download_id = scryer_domain::download_identity::DownloadId::new();
        self.download_submissions
            .record_submission(DownloadSubmission {
                download_id,
                title_id: self.title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: download_client_id.map(str::to_string),
                download_client_type: String::new(),
                download_client_item_id: String::new(),
                source_hint: None,
                source_provider_id: Some("indexer-a".to_string()),
                source_provider_name: None,
                source_kind: None,
                source_title: Some(RELEASE_TITLE.to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
                release_listing_json: None,
            })
            .await
            .expect("record the grab intent");
        self.download_submissions
            .unbound_submitted_at
            .lock()
            .await
            .insert(download_id, submitted_at);
        download_id
    }

    async fn release(&self) {
        crate::acquisition_workflow::release_unconfirmed_grab_intents(&self.app).await;
    }

    async fn released(&self) -> Vec<scryer_domain::download_identity::DownloadId> {
        self.download_submissions
            .released_unconfirmed
            .lock()
            .await
            .clone()
    }

    async fn unbound(&self) -> usize {
        self.download_submissions
            .list_active_unbound_for_title(&self.title.id)
            .await
            .expect("list unbound intents")
            .len()
    }

    async fn download_failures(&self) -> Vec<scryer_domain::DownloadFailedEventData> {
        self.app
            .services
            .events
            .domain_events
            .list(&DomainEventFilter {
                title_id: Some(self.title.id.clone()),
                ..Default::default()
            })
            .await
            .expect("list title events")
            .into_iter()
            .filter_map(|event| match event.payload {
                DomainEventPayload::DownloadFailed(data) => Some(data),
                _ => None,
            })
            .collect()
    }

    fn twenty_minutes_ago() -> chrono::DateTime<Utc> {
        Utc::now() - chrono::Duration::minutes(20)
    }
}

#[tokio::test]
async fn a_stale_unconfirmed_grab_is_released_its_history_written_and_its_title_freed() {
    let fixture = unconfirmed_grab_fixture(true).await;
    let download_id = fixture
        .record_intent(
            Some(CLIENT_ID),
            UnconfirmedGrabFixture::twenty_minutes_ago(),
        )
        .await;

    let blocked = fixture
        .app
        .active_work_blocking_a_move(&fixture.title)
        .await
        .expect("move block check")
        .expect("an unconfirmed grab blocks the move");
    assert_eq!(
        blocked,
        "\"D.Gray-man\" has 1 grab(s) that the download client has not confirmed; Scryer releases unconfirmed grabs after 15 minutes, then the move can start"
    );

    fixture.release().await;

    assert_eq!(fixture.released().await, vec![download_id]);
    assert_eq!(fixture.unbound().await, 0);

    let failures = fixture.download_failures().await;
    assert_eq!(failures.len(), 1, "the release is in the title's history");
    let reason = failures[0].reason.as_deref().unwrap_or_default();
    assert!(
        reason.contains("the download client never confirmed this grab")
            && reason.contains("released after 15 minutes"),
        "{reason}"
    );
    assert!(
        failures[0]
            .source_title
            .as_deref()
            .is_some_and(|source_title| source_title.eq_ignore_ascii_case(RELEASE_TITLE))
    );
    assert!(
        fixture
            .app
            .services
            .workflow
            .blocklist_repo
            .list_for_title(&fixture.title.id, 10)
            .await
            .expect("list blocklist")
            .is_empty(),
        "a release never seen to fail stays eligible"
    );

    let wanted = fixture.wanted.as_ref().expect("seeded wanted item");
    let reopened = fixture
        .wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted exists");
    assert_eq!(reopened.status, AcquisitionScopeStatus::Wanted);
    assert!(
        fixture
            .app
            .runtime
            .acquisition
            .scope_reopened_since_walk
            .load(std::sync::atomic::Ordering::SeqCst),
        "the re-opened scope is walked on this pass"
    );

    assert_eq!(
        fixture
            .app
            .active_work_blocking_a_move(&fixture.title)
            .await
            .expect("move block check"),
        None,
        "the catalog-only reassignment is no longer blocked"
    );

    fixture.release().await;
    assert_eq!(
        fixture.download_failures().await.len(),
        1,
        "a second pass settles nothing twice"
    );
}

#[tokio::test]
async fn a_fresh_unconfirmed_grab_is_left_alone() {
    let fixture = unconfirmed_grab_fixture(true).await;
    fixture.record_intent(Some(CLIENT_ID), Utc::now()).await;

    fixture.release().await;

    assert!(fixture.released().await.is_empty());
    assert_eq!(fixture.unbound().await, 1);
    assert!(fixture.download_failures().await.is_empty());
}

#[tokio::test]
async fn an_unconfirmed_grab_its_client_lists_is_bound_not_dropped() {
    let fixture = unconfirmed_grab_fixture(true).await;
    let download_id = fixture
        .record_intent(
            Some(CLIENT_ID),
            UnconfirmedGrabFixture::twenty_minutes_ago(),
        )
        .await;
    let mut listed = failed_history_item("job-1", RELEASE_TITLE);
    listed.state = DownloadQueueState::Downloading;
    listed.attention_required = false;
    listed.attention_reason = None;
    listed.client_id = CLIENT_ID.to_string();
    listed.download_id = Some(download_id.to_wire());
    *fixture.download_client.queue_items.lock().await = vec![listed];

    fixture.release().await;

    assert!(fixture.released().await.is_empty());
    assert_eq!(fixture.unbound().await, 1);
    assert!(fixture.download_failures().await.is_empty());
    let binding = fixture
        .app
        .services
        .workflow
        .download_registry
        .find_active_binding_by_locator(&ClientJobLocator::new(Some(CLIENT_ID), "nzbget", "job-1"))
        .await
        .expect("load binding")
        .expect("the listed job is resolved through the observer's binding path");
    assert_eq!(binding.download_id, download_id);
}

#[tokio::test]
async fn an_unconfirmed_grab_is_kept_while_a_client_that_could_hold_it_has_not_answered() {
    let fixture = unconfirmed_grab_fixture(false).await;
    let pinned = fixture
        .record_intent(
            Some(CLIENT_ID),
            UnconfirmedGrabFixture::twenty_minutes_ago(),
        )
        .await;
    fixture
        .download_client
        .set_snapshot_authoritative_client_ids(["other".to_string()])
        .await;

    fixture.release().await;
    assert!(
        fixture.released().await.is_empty(),
        "the client it was sent to did not answer"
    );

    fixture
        .download_client
        .set_snapshot_authoritative_client_ids([CLIENT_ID.to_string()])
        .await;
    fixture.release().await;
    assert_eq!(fixture.released().await, vec![pinned]);

    // An intent that never learned its client could be on any enabled one.
    let unpinned = fixture
        .record_intent(None, UnconfirmedGrabFixture::twenty_minutes_ago())
        .await;
    fixture
        .download_client
        .set_snapshot_authoritative_client_ids(["other".to_string()])
        .await;
    fixture.release().await;
    assert_eq!(
        fixture.released().await,
        vec![pinned],
        "the enabled client did not answer"
    );

    fixture
        .download_client
        .set_snapshot_authoritative_client_ids([CLIENT_ID.to_string()])
        .await;
    fixture.release().await;
    assert_eq!(fixture.released().await, vec![pinned, unpinned]);
}

#[tokio::test]
async fn an_unconfirmed_grab_pinned_to_a_removed_client_is_released() {
    let fixture = unconfirmed_grab_fixture(false).await;
    let download_id = fixture
        .record_intent(
            Some("removed-client"),
            UnconfirmedGrabFixture::twenty_minutes_ago(),
        )
        .await;
    fixture
        .download_client
        .set_snapshot_authoritative_client_ids(["other".to_string()])
        .await;

    fixture.release().await;

    assert_eq!(fixture.released().await, vec![download_id]);
}

#[tokio::test]
async fn an_unconfirmed_grab_from_before_a_restart_is_released_without_waiting_out_the_grace() {
    let fixture = unconfirmed_grab_fixture(false).await;
    let started_at = fixture.app.runtime.acquisition.process_started_at;
    let download_id = fixture
        .record_intent(Some(CLIENT_ID), started_at - chrono::Duration::seconds(30))
        .await;
    let after_start = fixture.record_intent(None, Utc::now()).await;

    fixture.release().await;

    assert_eq!(fixture.released().await, vec![download_id]);
    assert_ne!(download_id, after_start);
    let failures = fixture.download_failures().await;
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0]
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("before Scryer restarted")
    );
}

#[tokio::test]
async fn the_failure_check_pass_releases_unconfirmed_grabs() {
    let fixture = unconfirmed_grab_fixture(false).await;
    let download_id = fixture
        .record_intent(
            Some(CLIENT_ID),
            UnconfirmedGrabFixture::twenty_minutes_ago(),
        )
        .await;

    fixture
        .app
        .run_background_acquisition_pass_once(
            crate::acquisition_workflow::BackgroundAcquisitionPass::FailureCheckOnly,
        )
        .await;

    assert_eq!(fixture.released().await, vec![download_id]);
    assert_eq!(fixture.unbound().await, 0);
}

#[tokio::test]
async fn an_unconfirmed_grab_is_kept_while_its_clients_history_window_is_full() {
    let fixture = unconfirmed_grab_fixture(false).await;
    fixture
        .record_intent(
            Some(CLIENT_ID),
            UnconfirmedGrabFixture::twenty_minutes_ago(),
        )
        .await;
    // The client returned as many history rows as were asked for, with no
    // telling how far back they reach: the job may sit just past the window.
    fixture
        .download_client
        .set_snapshot_saturated_activity_window(CLIENT_ID, None)
        .await;

    fixture.release().await;

    assert!(
        fixture.released().await.is_empty(),
        "a filled history window does not prove the job absent"
    );
    assert_eq!(fixture.unbound().await, 1);
    assert!(fixture.download_failures().await.is_empty());

    // An intent that never learned its client is kept for the same reason.
    fixture
        .record_intent(None, UnconfirmedGrabFixture::twenty_minutes_ago())
        .await;
    fixture.release().await;
    assert!(fixture.released().await.is_empty());
    assert_eq!(fixture.unbound().await, 2);
}

#[tokio::test]
async fn an_unconfirmed_grab_is_released_when_its_clients_history_window_is_partial() {
    let fixture = unconfirmed_grab_fixture(false).await;
    let download_id = fixture
        .record_intent(
            Some(CLIENT_ID),
            UnconfirmedGrabFixture::twenty_minutes_ago(),
        )
        .await;
    // Fewer rows than the limit came back, so that history is the client's
    // whole history; another client's filled window is no concern of it.
    fixture
        .download_client
        .set_snapshot_saturated_activity_window("other", None)
        .await;

    fixture.release().await;

    assert_eq!(fixture.released().await, vec![download_id]);
    assert_eq!(fixture.unbound().await, 0);
}

#[tokio::test]
async fn an_unconfirmed_grab_is_kept_while_a_full_window_is_newer_than_its_submission() {
    let fixture = unconfirmed_grab_fixture(false).await;
    let submitted_at = Utc::now() - chrono::Duration::hours(3);
    let download_id = fixture.record_intent(Some(CLIENT_ID), submitted_at).await;
    // The full window only reaches back to an hour after the submission, so
    // the job could have finished and aged out of it.
    fixture
        .download_client
        .set_snapshot_saturated_activity_window(
            CLIENT_ID,
            Some(submitted_at + chrono::Duration::hours(1)),
        )
        .await;

    fixture.release().await;
    assert!(
        fixture.released().await.is_empty(),
        "a full window newer than the submission does not cover it"
    );

    // Within the clock-skew margin is still not coverage.
    fixture
        .download_client
        .set_snapshot_saturated_activity_window(
            CLIENT_ID,
            Some(submitted_at - chrono::Duration::minutes(30)),
        )
        .await;
    fixture.release().await;
    assert!(fixture.released().await.is_empty());

    // A busy client whose full window reaches well before the submission
    // does cover it, so the intent is released rather than held forever.
    fixture
        .download_client
        .set_snapshot_saturated_activity_window(
            CLIENT_ID,
            Some(submitted_at - chrono::Duration::hours(2)),
        )
        .await;
    fixture.release().await;
    assert_eq!(fixture.released().await, vec![download_id]);
}
