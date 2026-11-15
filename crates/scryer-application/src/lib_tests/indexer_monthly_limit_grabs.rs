//! A grab refused by an indexer's monthly limit is not a verdict on the
//! release. Each automatic path keeps the refused release eligible and still
//! grabs a release from an indexer that is not at its limit.

use super::*;

const CAPPED_INDEXER: &str = "indexer-capped";
const OPEN_INDEXER: &str = "indexer-open";

/// Refuses monthly grab admission for the listed indexers and counts every
/// admission asked of each indexer. Like the real tracker, it remembers a
/// reached limit once admission has refused it, and not a failed check.
struct PerIndexerGrabLimit {
    /// Indexer id to the reason code its admission is refused with.
    refused: HashMap<String, &'static str>,
    admissions: std::sync::Mutex<HashMap<String, usize>>,
    remembered: std::sync::Mutex<HashSet<String>>,
    /// Set once the limit lifts, as a raised limit or a new month does.
    lifted: std::sync::atomic::AtomicBool,
}

impl PerIndexerGrabLimit {
    fn refusing(indexer_id: &str) -> Self {
        Self::refusing_with(
            indexer_id,
            crate::error_reasons::indexer_monthly_quota::GRAB_LIMIT_REACHED,
        )
    }

    fn refusing_with(indexer_id: &str, reason: &'static str) -> Self {
        Self {
            refused: HashMap::from([(indexer_id.to_string(), reason)]),
            admissions: std::sync::Mutex::new(HashMap::new()),
            remembered: std::sync::Mutex::new(HashSet::new()),
            lifted: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn lift(&self) {
        self.lifted.store(true, std::sync::atomic::Ordering::SeqCst);
        self.remembered.lock().expect("remembered refusals").clear();
    }

    fn is_lifted(&self) -> bool {
        self.lifted.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn admissions(&self, indexer_id: &str) -> usize {
        self.admissions
            .lock()
            .expect("admission counts")
            .get(indexer_id)
            .copied()
            .unwrap_or(0)
    }
}

impl crate::IndexerStatsTracker for PerIndexerGrabLimit {
    fn record_query(&self, _indexer_id: &str, _indexer_name: &str, _success: bool) {}

    fn record_grab(&self, _indexer_id: &str, _indexer_name: &str) {}

    fn record_api_limits(
        &self,
        _indexer_id: &str,
        _api_current: Option<u32>,
        _api_max: Option<u32>,
        _grab_current: Option<u32>,
        _grab_max: Option<u32>,
    ) {
    }

    fn admit_monthly_grab<'a>(
        &'a self,
        indexer_id: &'a str,
        _at: chrono::DateTime<Utc>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            *self
                .admissions
                .lock()
                .expect("admission counts")
                .entry(indexer_id.to_string())
                .or_default() += 1;
            if self.is_lifted() {
                return Ok(());
            }
            match self.refused.get(indexer_id).copied() {
                Some(crate::error_reasons::indexer_monthly_quota::ACCOUNTING_UNAVAILABLE) => {
                    Err(AppError::temporary_unavailable_reason(
                        crate::error_reasons::indexer_monthly_quota::ACCOUNTING_UNAVAILABLE,
                        "monthly usage could not be checked",
                        None,
                    ))
                }
                Some(reason) => {
                    self.remembered
                        .lock()
                        .expect("remembered refusals")
                        .insert(indexer_id.to_string());
                    Err(AppError::validation_refused(
                        reason,
                        "monthly limit reached",
                    ))
                }
                None => Ok(()),
            }
        })
    }

    fn monthly_grab_refusal_remembered(
        &self,
        indexer_id: &str,
        _at: chrono::DateTime<Utc>,
    ) -> bool {
        !self.is_lifted()
            && self
                .remembered
                .lock()
                .expect("remembered refusals")
                .contains(indexer_id)
    }

    fn all_stats(&self) -> Vec<crate::IndexerQueryStats> {
        Vec::new()
    }
}

/// Answers every search with the same fixed listing.
struct ListingIndexerClient {
    releases: Vec<IndexerSearchResult>,
}

#[async_trait]
impl IndexerClient for ListingIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            next_cursor: None,
            completion: crate::IndexerSearchCompletion::Complete,
            indexer_outcomes: Vec::new(),
            results: self.releases.clone(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

fn listed_release(indexer_id: &str, release_title: &str) -> IndexerSearchResult {
    let mut release = FixedReleaseIndexerClient::new(release_title).release();
    let slug = release_title.to_ascii_lowercase();
    release.indexer_id = Some(indexer_id.to_string());
    release.source = indexer_id.to_string();
    release.download_url = Some(format!("https://example.invalid/{indexer_id}/{slug}.nzb"));
    release.link = Some(format!("https://example.invalid/{indexer_id}/{slug}"));
    release.info_url = release.link.clone();
    release.guid = Some(format!("{indexer_id}-{slug}"));
    release
}

struct Fixture {
    app: AppUseCase,
    user: User,
    limit: Arc<PerIndexerGrabLimit>,
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    wanted_items: Arc<TrackingAcquisitionScopeStateRepo>,
    release_attempts: Arc<MockReleaseAttemptRepo>,
}

fn fixture(releases: Vec<IndexerSearchResult>, limit: PerIndexerGrabLimit) -> Fixture {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(ListingIndexerClient { releases }),
        );
    let limit = Arc::new(limit);
    let app = app.with_test_overrides(|services| services.with_indexer_stats(limit.clone()));
    Fixture {
        app,
        user,
        limit,
        download_client,
        download_submissions,
        pending_releases,
        wanted_items,
        release_attempts,
    }
}

impl Fixture {
    /// The refused release is as eligible afterwards as it was before, and
    /// the release from the open indexer was the one grabbed.
    async fn assert_only_the_open_indexer_was_grabbed(
        &self,
        title: &Title,
        open_release_title: &str,
    ) {
        assert_eq!(
            self.download_client
                .submitted_release_titles
                .lock()
                .await
                .len(),
            1,
            "exactly one release reaches the download client"
        );
        let submissions = self.download_submissions.store.lock().await.clone();
        assert_eq!(submissions.len(), 1, "{submissions:?}");
        assert_eq!(
            submissions[0].source_title.as_deref(),
            Some(open_release_title),
            "the release from the indexer that is not at its limit is grabbed"
        );
        assert!(
            title_blocklist_entries(&self.app, &title.id)
                .await
                .is_empty(),
            "a monthly limit refusal must not blocklist a release"
        );
        let attempts = self.release_attempts.attempts.lock().await.clone();
        assert!(
            attempts
                .iter()
                .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed),
            "a monthly limit refusal is not a failed attempt: {:?}",
            attempts
                .iter()
                .map(|attempt| (&attempt.source_title, &attempt.outcome))
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn auto_search_skips_an_indexer_at_its_monthly_limit_and_grabs_from_another() {
    let open_release = "Limit.Search.Subject.2024.1080p.WEB-DL-OPEN";
    let fixture = fixture(
        vec![
            listed_release(CAPPED_INDEXER, "Limit.Search.Subject.2024.1080p.WEB-DL-ONE"),
            listed_release(CAPPED_INDEXER, "Limit.Search.Subject.2024.1080p.WEB-DL-TWO"),
            listed_release(OPEN_INDEXER, open_release),
        ],
        PerIndexerGrabLimit::refusing(CAPPED_INDEXER),
    );
    let (title, _) = seed_movie_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit Search Subject",
        2024,
    )
    .await;

    fixture.app.run_background_acquisition_cycle_once().await;

    assert_eq!(
        fixture.limit.admissions(CAPPED_INDEXER),
        1,
        "the indexer's other candidates are passed over after the first refusal"
    );
    fixture
        .assert_only_the_open_indexer_was_grabbed(&title, open_release)
        .await;
}

#[tokio::test]
async fn auto_search_writes_nothing_for_an_indexer_already_known_to_be_at_its_limit() {
    let first_capped = "Limit.First.Subject.2024.1080p.WEB-DL-ONE";
    let second_capped = "Limit.Second.Subject.2023.1080p.WEB-DL-ONE";
    let second_open = "Limit.Second.Subject.2023.1080p.WEB-DL-OPEN";
    let fixture = fixture(
        vec![
            listed_release(CAPPED_INDEXER, first_capped),
            listed_release(OPEN_INDEXER, "Limit.First.Subject.2024.1080p.WEB-DL-OPEN"),
            listed_release(CAPPED_INDEXER, second_capped),
            listed_release(OPEN_INDEXER, second_open),
        ],
        PerIndexerGrabLimit::refusing(CAPPED_INDEXER),
    );
    seed_movie_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit First Subject",
        2024,
    )
    .await;

    // The first pass meets the limit through real admission.
    fixture.app.run_background_acquisition_cycle_once().await;
    assert_eq!(fixture.limit.admissions(CAPPED_INDEXER), 1);
    let attempts_after_first_pass = fixture.release_attempts.attempts.lock().await.len();
    let pending_after_first_pass = fixture.pending_releases.store.lock().await.clone();

    let (second_title, _) = seed_movie_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit Second Subject",
        2023,
    )
    .await;
    fixture.app.run_background_acquisition_cycle_once().await;

    assert_eq!(
        fixture.limit.admissions(CAPPED_INDEXER),
        1,
        "an indexer known to be at its limit is not put through admission again"
    );
    let attempts = fixture.release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.source_title.as_deref() != Some(second_capped)),
        "no attempt row is written for the capped indexer's release: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    assert!(
        attempts.len() > attempts_after_first_pass,
        "the open indexer's grab is still recorded"
    );
    let pending = fixture.pending_releases.store.lock().await.clone();
    assert_eq!(
        format!("{pending:?}"),
        format!("{pending_after_first_pass:?}"),
        "the second pass writes no pending release"
    );
    let submissions = fixture.download_submissions.store.lock().await.clone();
    assert!(
        submissions
            .iter()
            .any(|submission| submission.source_title.as_deref() == Some(second_open)),
        "the open indexer is still grabbed: {submissions:?}"
    );
    assert!(
        submissions
            .iter()
            .all(|submission| submission.source_title.as_deref() != Some(second_capped)),
        "{submissions:?}"
    );
    assert!(
        title_blocklist_entries(&fixture.app, &second_title.id)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn auto_search_keeps_a_release_eligible_when_monthly_usage_cannot_be_checked() {
    let open_release = "Limit.Unchecked.Subject.2024.1080p.WEB-DL-OPEN";
    let fixture = fixture(
        vec![
            listed_release(
                CAPPED_INDEXER,
                "Limit.Unchecked.Subject.2024.1080p.WEB-DL-ONE",
            ),
            listed_release(OPEN_INDEXER, open_release),
        ],
        PerIndexerGrabLimit::refusing_with(
            CAPPED_INDEXER,
            crate::error_reasons::indexer_monthly_quota::ACCOUNTING_UNAVAILABLE,
        ),
    );
    let (title, _) = seed_movie_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit Unchecked Subject",
        2024,
    )
    .await;

    fixture.app.run_background_acquisition_cycle_once().await;

    assert_eq!(fixture.limit.admissions(CAPPED_INDEXER), 1);
    fixture
        .assert_only_the_open_indexer_was_grabbed(&title, open_release)
        .await;
}

#[tokio::test]
async fn season_pack_grab_skips_an_indexer_at_its_monthly_limit_and_grabs_from_another() {
    let open_release = "Limit.Pack.Subject.S01.1080p.WEB-DL-OPEN";
    let fixture = fixture(
        vec![
            listed_release(CAPPED_INDEXER, "Limit.Pack.Subject.S01.1080p.WEB-DL-ONE"),
            listed_release(CAPPED_INDEXER, "Limit.Pack.Subject.S01.1080p.WEB-DL-TWO"),
            listed_release(OPEN_INDEXER, open_release),
        ],
        PerIndexerGrabLimit::refusing(CAPPED_INDEXER),
    );
    let (title, _) = seed_anime_season_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit Pack Subject",
        1,
    )
    .await;

    fixture.app.run_background_acquisition_cycle_once().await;

    assert_eq!(
        fixture.limit.admissions(CAPPED_INDEXER),
        1,
        "the indexer's other packs are passed over after the first refusal"
    );
    fixture
        .assert_only_the_open_indexer_was_grabbed(&title, open_release)
        .await;
}

#[tokio::test]
async fn rss_sync_skips_an_indexer_at_its_monthly_limit_and_grabs_from_another() {
    let open_release = "Limit.Feed.Subject.2024.1080p.WEB-DL-OPEN";
    let fixture = fixture(
        vec![
            listed_release(CAPPED_INDEXER, "Limit.Feed.Subject.2024.1080p.WEB-DL-ONE"),
            listed_release(CAPPED_INDEXER, "Limit.Feed.Subject.2024.1080p.WEB-DL-TWO"),
            listed_release(OPEN_INDEXER, open_release),
        ],
        PerIndexerGrabLimit::refusing(CAPPED_INDEXER),
    );
    let title = fixture
        .app
        .add_title(
            &fixture.user,
            NewTitle {
                name: "Limit Feed Subject".into(),
                sort_title: Some("Limit Feed Subject".into()),
                slug: Some("limit-feed-subject".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2024),
                content_status: Some("Released".into()),
                min_availability: Some("announced".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie");
    fixture
        .wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    let report = fixture
        .app
        .run_scheduled_rss_sync()
        .await
        .expect("run RSS sync");

    assert_eq!(report.releases_grabbed, 1);
    assert_eq!(
        fixture.limit.admissions(CAPPED_INDEXER),
        1,
        "the indexer's other feed items are passed over after the first refusal"
    );
    fixture
        .assert_only_the_open_indexer_was_grabbed(&title, open_release)
        .await;
}

#[tokio::test]
async fn rss_sync_decides_a_held_release_once_when_a_monthly_limit_refuses_the_next_one() {
    const HELD_INDEXER: &str = "indexer-held";
    let held_release = "Limit.Held.Subject.2024.1080p.WEB-DL-HELD";
    let capped_release = "Limit.Held.Subject.2024.1080p.WEB-DL-ONE";
    let open_release = "Limit.Held.Subject.2024.1080p.WEB-DL-OPEN";
    // Listed first and too new for the delay; the other two are long past it.
    let mut held = listed_release(HELD_INDEXER, held_release);
    held.published_at = Some(Utc::now().to_rfc3339());
    let fixture = fixture(
        vec![
            held,
            listed_release(CAPPED_INDEXER, capped_release),
            listed_release(OPEN_INDEXER, open_release),
        ],
        PerIndexerGrabLimit::refusing(CAPPED_INDEXER),
    );
    let title = fixture
        .app
        .add_title(
            &fixture.user,
            NewTitle {
                name: "Limit Held Subject".into(),
                sort_title: Some("Limit Held Subject".into()),
                slug: Some("limit-held-subject".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2024),
                content_status: Some("Released".into()),
                min_availability: Some("announced".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie");
    fixture
        .wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;
    let delay_profiles = &fixture.app.services.config.delay_profiles;
    for profile in delay_profiles.list().await.expect("list delay profiles") {
        delay_profiles
            .delete(&profile.id)
            .await
            .expect("delete delay profile");
    }
    let profiles = crate::parse_delay_profile_catalog(
        &serde_json::json!([{
            "id": "delay-new-releases",
            "name": "Delay new releases",
            "usenet_delay_minutes": 120,
        }])
        .to_string(),
    )
    .expect("parse delay profile catalog")
    .into_iter()
    .map(crate::normalize_delay_profile)
    .collect();
    delay_profiles
        .save_many(profiles)
        .await
        .expect("seed delay profile catalog");

    let report = fixture
        .app
        .run_scheduled_rss_sync()
        .await
        .expect("run RSS sync");

    assert_eq!(fixture.limit.admissions(CAPPED_INDEXER), 1);
    assert_eq!(
        report.releases_held, 1,
        "the held release is counted once, not again after the refusal"
    );
    assert_eq!(
        fixture
            .pending_releases
            .observation_writes_for(held_release),
        1,
        "the held release is parked once, not again after the refusal"
    );
    assert_eq!(
        fixture
            .pending_releases
            .observation_writes_for(capped_release),
        0,
        "a release refused by the monthly limit is not parked"
    );
    assert_eq!(report.releases_grabbed, 1);
    fixture
        .assert_only_the_open_indexer_was_grabbed(&title, open_release)
        .await;
}

#[tokio::test]
async fn expired_pending_release_at_a_monthly_limit_is_passed_over_and_another_is_grabbed() {
    let fixture = fixture(Vec::new(), PerIndexerGrabLimit::refusing(CAPPED_INDEXER));
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit Pending Subject",
        2024,
    )
    .await;
    // The same release offered by two indexers ranks equally, so the id
    // decides the order and the capped indexer's row is tried first.
    let release_title = "Limit.Pending.Subject.2024.1080p.WEB-DL-GRP";
    let mut capped = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    capped.id = "a-capped-row".into();
    capped.indexer_id = Some(CAPPED_INDEXER.into());
    capped.release_url = Some("https://example.invalid/capped/pending.nzb".into());
    capped.release_guid = Some("capped-pending-guid".into());
    capped.release_identity = "capped-pending-guid".into();
    let mut open = capped.clone();
    open.id = "b-open-row".into();
    open.indexer_id = Some(OPEN_INDEXER.into());
    open.release_url = Some("https://example.invalid/open/pending.nzb".into());
    open.release_guid = Some("open-pending-guid".into());
    open.release_identity = "open-pending-guid".into();
    for release in [&capped, &open] {
        fixture
            .pending_releases
            .insert_pending_release(release)
            .await
            .expect("insert pending release");
    }

    assert_eq!(
        fixture
            .app
            .process_expired_pending_releases()
            .await
            .expect("process expired pending releases"),
        1
    );

    assert_eq!(fixture.limit.admissions(CAPPED_INDEXER), 1);
    assert_eq!(
        fixture
            .pending_releases
            .get_pending_release(&capped.id)
            .await
            .expect("load the capped row")
            .expect("the capped row still exists")
            .status,
        PendingReleaseStatus::Superseded,
        "the grab from the other indexer retires the refused row like any other sibling"
    );
    assert_eq!(
        fixture
            .pending_releases
            .get_pending_release(&open.id)
            .await
            .expect("load the open row")
            .expect("the open row still exists")
            .status,
        PendingReleaseStatus::Grabbed
    );
    let submissions = fixture.download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1, "{submissions:?}");
    assert!(
        title_blocklist_entries(&fixture.app, &title.id)
            .await
            .is_empty(),
        "a monthly limit refusal must not blocklist a release"
    );
    assert!(
        fixture
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 10)
            .await
            .expect("list failed signatures")
            .is_empty(),
        "a monthly limit refusal is not a failed attempt"
    );
}

#[tokio::test]
async fn expired_pending_release_at_a_monthly_limit_stays_waiting() {
    let fixture = fixture(Vec::new(), PerIndexerGrabLimit::refusing(CAPPED_INDEXER));
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit Waiting Subject",
        2024,
    )
    .await;
    let mut capped = pending_movie_release(
        &wanted_id,
        &title,
        "Limit.Waiting.Subject.2024.1080p.WEB-DL-GRP",
        PendingReleaseStatus::Waiting,
    );
    capped.indexer_id = Some(CAPPED_INDEXER.into());
    fixture
        .pending_releases
        .insert_pending_release(&capped)
        .await
        .expect("insert pending release");

    assert_eq!(
        fixture
            .app
            .process_expired_pending_releases()
            .await
            .expect("process expired pending releases"),
        0
    );

    assert_eq!(
        fixture
            .pending_releases
            .get_pending_release(&capped.id)
            .await
            .expect("load the capped row")
            .expect("the capped row still exists")
            .status,
        PendingReleaseStatus::Waiting,
        "the refused release is kept exactly as it was"
    );
    assert!(fixture.download_submissions.store.lock().await.is_empty());
    assert!(
        title_blocklist_entries(&fixture.app, &title.id)
            .await
            .is_empty(),
        "a monthly limit refusal must not blocklist a release"
    );
    assert!(
        fixture
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 10)
            .await
            .expect("list failed signatures")
            .is_empty(),
        "a monthly limit refusal is not a failed attempt"
    );
}

#[tokio::test]
async fn operator_grab_of_a_pending_release_returns_the_monthly_limit_refusal() {
    let fixture = fixture(Vec::new(), PerIndexerGrabLimit::refusing(CAPPED_INDEXER));
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &fixture.app,
        &fixture.user,
        &fixture.wanted_items,
        "Limit Operator Subject",
        2024,
    )
    .await;
    let mut capped = pending_movie_release(
        &wanted_id,
        &title,
        "Limit.Operator.Subject.2024.1080p.WEB-DL-GRP",
        PendingReleaseStatus::Waiting,
    );
    capped.indexer_id = Some(CAPPED_INDEXER.into());
    fixture
        .pending_releases
        .insert_pending_release(&capped)
        .await
        .expect("insert pending release");

    let error = fixture
        .app
        .force_grab_pending_release(&fixture.user, &capped.id)
        .await
        .expect_err("the monthly grab limit refuses the operator's grab");

    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::indexer_monthly_quota::GRAB_LIMIT_REACHED)
    );
    assert!(
        title_blocklist_entries(&fixture.app, &title.id)
            .await
            .is_empty()
    );
    assert!(
        fixture
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 10)
            .await
            .expect("list failed signatures")
            .is_empty()
    );
}

#[tokio::test]
async fn rss_sync_grabs_a_capped_release_once_the_limit_lifts() {
    let capped_release = "Limit.Lifted.Subject.2024.1080p.WEB-DL-ONE";
    let fixture = fixture(
        vec![listed_release(CAPPED_INDEXER, capped_release)],
        PerIndexerGrabLimit::refusing(CAPPED_INDEXER),
    );
    let title = fixture
        .app
        .add_title(
            &fixture.user,
            NewTitle {
                name: "Limit Lifted Subject".into(),
                sort_title: Some("Limit Lifted Subject".into()),
                slug: Some("limit-lifted-subject".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2024),
                content_status: Some("Released".into()),
                min_availability: Some("announced".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie");
    fixture
        .wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    let refused = fixture
        .app
        .run_scheduled_rss_sync()
        .await
        .expect("run RSS sync at the limit");
    assert_eq!(refused.releases_grabbed, 0);
    assert_eq!(fixture.limit.admissions(CAPPED_INDEXER), 1);

    // While the refusal is remembered, the feed item is passed over without
    // being matched or remembered as seen.
    let still_refused = fixture
        .app
        .run_scheduled_rss_sync()
        .await
        .expect("run RSS sync while the limit is remembered");
    assert_eq!(still_refused.releases_grabbed, 0);
    assert_eq!(fixture.limit.admissions(CAPPED_INDEXER), 1);

    fixture.limit.lift();
    let lifted = fixture
        .app
        .run_scheduled_rss_sync()
        .await
        .expect("run RSS sync after the limit lifts");

    assert_eq!(
        lifted.releases_grabbed, 1,
        "a release passed over for a monthly limit is grabbed once the limit lifts"
    );
    let submissions = fixture.download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1, "{submissions:?}");
    assert_eq!(submissions[0].source_title.as_deref(), Some(capped_release));
}

#[tokio::test]
async fn replace_early_refused_by_a_monthly_limit_keeps_the_existing_download() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let limit = Arc::new(PerIndexerGrabLimit::refusing(CAPPED_INDEXER));
    let app = app.with_test_overrides(|services| services.with_indexer_stats(limit.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Limit Replace Subject".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "existing-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Limit.Replace.Subject.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
            release_listing_json: None,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "existing-job",
        DownloadQueueState::Queued,
        0,
    )];

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: Some(CAPPED_INDEXER.to_string()),
                source_hint: Some("https://example.invalid/limit-replace.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Limit.Replace.Subject.2026.2160p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::ReplaceEarly,
        )
        .await
        .expect_err("the replacement's indexer is at its monthly limit");

    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::indexer_monthly_quota::GRAB_LIMIT_REACHED)
    );
    assert!(
        download_client.deleted_items.lock().await.is_empty(),
        "the existing download is not cancelled for a refused replacement"
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1, "{submissions:?}");
    assert_eq!(submissions[0].download_client_item_id, "existing-job");
}

#[tokio::test]
async fn retry_refused_by_a_monthly_limit_keeps_the_failed_submission_it_would_replace() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let limit = Arc::new(PerIndexerGrabLimit::refusing(CAPPED_INDEXER));
    let app = app.with_test_overrides(|services| services.with_indexer_stats(limit.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Limit Retry Subject".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let source_hint = "https://example.invalid/limit-retry.nzb";
    let source_title = "Limit.Retry.Subject.2026.1080p.WEB-DL";
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: Some(source_hint.to_string()),
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some(source_title.to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: crate::helpers::normalize_release_selection_signature(
                Some(source_hint),
                Some(source_title),
                Some(DownloadSourceKind::NzbUrl),
            ),
            scope: SubmissionScope::Title,
            release_listing_json: None,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "failed-job",
        DownloadQueueState::Failed,
        0,
    )];
    let selection = QueuedReleaseSelection {
        indexer_id: Some(CAPPED_INDEXER.to_string()),
        source_hint: Some(source_hint.to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some(source_title.to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            selection.clone(),
            SubmissionScope::Title,
            SubmissionConflictPolicy::ReplaceEarly,
        )
        .await
        .expect_err("the retry's indexer is at its monthly limit");

    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::indexer_monthly_quota::GRAB_LIMIT_REACHED)
    );
    assert_eq!(limit.admissions(CAPPED_INDEXER), 1);
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(
        submissions.len(),
        1,
        "a refused retry keeps the submission it would replace: {submissions:?}"
    );
    assert_eq!(submissions[0].download_client_item_id, "failed-job");

    // Once the client shows the same job running, it is accepted as it is:
    // nothing is submitted and no grab is admitted for it.
    app.runtime
        .acquisition
        .download_submission_guards
        .invalidate_client_snapshots();
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "failed-job",
        DownloadQueueState::Queued,
        0,
    )];
    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            selection,
            SubmissionScope::Title,
            SubmissionConflictPolicy::ReplaceEarly,
        )
        .await
        .expect("the running job is accepted as it is");
    let QueueDownloadOutcome::Queued(queued) = outcome else {
        panic!("the running job is not a conflict");
    };
    assert!(queued.reused_existing);
    assert_eq!(
        limit.admissions(CAPPED_INDEXER),
        1,
        "accepting the existing download admits no grab"
    );
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "nothing reaches the download client"
    );
}
