//! Unit coverage for the interactive release-search job's query subject, its
//! per-indexer health fields and grab-time candidate tokens (spec 0002).

use super::*;

use crate::catalog::interactive_release_search::InteractiveReleaseSearchState;
use crate::{
    INDEXER_ROUTING_SETTINGS_KEY, InteractiveReleaseSearchIndexerStatus,
    InteractiveReleaseSearchRequest, InteractiveReleaseSearchSnapshot, InteractiveSearchKind,
    SETTINGS_SCOPE_SYSTEM,
};

/// One recorded dispatch to the search port.
#[derive(Clone, Debug)]
struct RecordedSearchCall {
    query: String,
    facet: Option<String>,
    newznab_categories: Option<Vec<String>>,
    /// Indexers the restriction plan left enabled — exactly one per task.
    enabled_indexers: Vec<String>,
}

/// Test double that answers per indexer and records the envelope each call
/// carried.
#[derive(Clone, Default)]
struct ScriptedIndexerClient {
    calls: Arc<Mutex<Vec<RecordedSearchCall>>>,
    releases: Arc<Mutex<HashMap<String, Vec<IndexerSearchResult>>>>,
}

impl ScriptedIndexerClient {
    async fn with_releases(self, indexer_id: &str, releases: Vec<IndexerSearchResult>) -> Self {
        self.releases
            .lock()
            .await
            .insert(indexer_id.to_string(), releases);
        self
    }

    async fn calls(&self) -> Vec<RecordedSearchCall> {
        self.calls.lock().await.clone()
    }
}

#[async_trait]
impl IndexerClient for ScriptedIndexerClient {
    #[allow(clippy::too_many_arguments)]
    async fn search(
        &self,
        query: String,
        _ids: HashMap<String, String>,
        _category: Option<String>,
        facet: Option<String>,
        _id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
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
        let mut enabled_indexers = indexer_routing
            .iter()
            .flat_map(|plan| plan.entries.iter())
            .filter(|(_, entry)| entry.enabled)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        enabled_indexers.sort();
        self.calls.lock().await.push(RecordedSearchCall {
            query,
            facet,
            newznab_categories,
            enabled_indexers: enabled_indexers.clone(),
        });

        let Some(indexer_id) = enabled_indexers.first().cloned() else {
            return Err(AppError::Repository("no indexer routed".to_string()));
        };
        let results = self
            .releases
            .lock()
            .await
            .get(&indexer_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut result| {
                result.indexer_id = Some(indexer_id.clone());
                result
            })
            .collect();
        Ok(IndexerSearchResponse {
            results,
            completion: crate::IndexerSearchCompletion::Complete,
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
            indexer_outcomes: Vec::new(),
        })
    }
}

fn nzb_release(title: &str, guid: &str) -> IndexerSearchResult {
    IndexerSearchResult {
        indexer_id: None,
        source: "test".into(),
        title: title.to_string(),
        link: None,
        download_url: Some(format!("https://example.invalid/{guid}.nzb")),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        size_bytes: Some(1_073_741_824),
        published_at: Some(Utc::now().to_rfc3339()),
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: Some(42),
        password_hint: None,
        parsed_release_metadata: None,
        quality_profile_decision: None,
        extra: HashMap::new(),
        response_attributes: crate::IndexerResponseAttributes::default(),
        guid: Some(guid.to_string()),
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        coverage_scope: None,
        auto_eligible: None,
        auto_decision_code: None,
        auto_decision_summary: None,
    }
}

fn query_request(query: &str, kind: InteractiveSearchKind) -> InteractiveReleaseSearchRequest {
    InteractiveReleaseSearchRequest {
        query: Some(query.to_string()),
        kind: Some(kind),
        ..InteractiveReleaseSearchRequest::default()
    }
}

fn title_request(title_id: &str) -> InteractiveReleaseSearchRequest {
    InteractiveReleaseSearchRequest {
        title_id: Some(title_id.to_string()),
        ..InteractiveReleaseSearchRequest::default()
    }
}

fn bootstrap_search(
    settings: Arc<StoredSettingsRepo>,
    client: ScriptedIndexerClient,
    configs: Vec<IndexerConfig>,
) -> (AppUseCase, User) {
    bootstrap_with_search_settings_indexer_and_configs(settings, Arc::new(client), configs)
}

/// Poll until the job leaves `Running`, then return the final snapshot.
async fn await_completion(
    app: &AppUseCase,
    user: &User,
    job_id: &str,
) -> InteractiveReleaseSearchSnapshot {
    for _ in 0..200 {
        let snapshot = app
            .interactive_release_search(user, job_id)
            .await
            .expect("poll interactive release search")
            .expect("job present");
        if snapshot.state != InteractiveReleaseSearchState::Running {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("interactive release search never reached a terminal state");
}

fn indexer_view<'a>(
    snapshot: &'a InteractiveReleaseSearchSnapshot,
    indexer_id: &str,
) -> &'a crate::InteractiveReleaseSearchIndexerView {
    snapshot
        .indexers
        .iter()
        .find(|view| view.indexer_id == indexer_id)
        .unwrap_or_else(|| panic!("indexer {indexer_id} missing from snapshot: {snapshot:?}"))
}

// ── Query subject: kind mapping ─────────────────────────────────────────────

#[tokio::test]
async fn a_movie_kind_query_sends_the_movie_facet_and_its_default_categories() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")])
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_interactive_release_search(
            &user,
            query_request("paperman", InteractiveSearchKind::Movie),
        )
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;
    assert_eq!(done.results.len(), 1, "{done:?}");

    let calls = client.calls().await;
    assert_eq!(calls.len(), 1, "one call per indexer: {calls:?}");
    assert_eq!(calls[0].facet.as_deref(), Some("movie"));
    assert_eq!(calls[0].query, "paperman");
    assert!(
        calls[0]
            .newznab_categories
            .as_ref()
            .is_some_and(|categories| !categories.is_empty()),
        "movie kind carries the facet's default categories: {calls:?}"
    );
}

#[tokio::test]
async fn a_raw_kind_query_sends_a_text_search_with_no_facet_and_no_categories() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("Some.Odd.Pack.2024", "g1")])
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_interactive_release_search(
            &user,
            query_request("  some odd pack  ", InteractiveSearchKind::Raw),
        )
        .await
        .expect("start");
    await_completion(&app, &user, &start.id).await;

    let calls = client.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].facet, None, "raw kind must not send a facet");
    assert_eq!(calls[0].newznab_categories, None);
    assert_eq!(calls[0].query, "some odd pack", "query is trimmed");
}

// ── Both subjects: indexer restriction ──────────────────────────────────────

#[tokio::test]
async fn requested_indexer_ids_restrict_a_query_subject_fan_out() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("A.2024.1080p.WEB-DL", "a1")])
        .await
        .with_releases("idx-b", vec![nzb_release("B.2024.1080p.WEB-DL", "b1")])
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![
            synthetic_direct_nab_indexer_config("idx-a", "newznab"),
            synthetic_direct_nab_indexer_config("idx-b", "newznab"),
        ],
    );

    let all = app
        .start_interactive_release_search(&user, query_request("q", InteractiveSearchKind::Raw))
        .await
        .expect("start");
    let all = await_completion(&app, &user, &all.id).await;
    assert_eq!(all.indexers.len(), 2, "{all:?}");

    let restricted = app
        .start_interactive_release_search(
            &user,
            InteractiveReleaseSearchRequest {
                indexer_ids: Some(vec!["idx-b".into()]),
                ..query_request("q", InteractiveSearchKind::Raw)
            },
        )
        .await
        .expect("start restricted");
    let restricted = await_completion(&app, &user, &restricted.id).await;
    assert_eq!(restricted.indexers.len(), 1);
    assert_eq!(restricted.indexers[0].indexer_id, "idx-b");
    assert!(
        restricted
            .results
            .iter()
            .all(|result| result.indexer_id.as_deref() == Some("idx-b")),
        "{restricted:?}"
    );

    // Every dispatched call routes exactly one indexer.
    for call in client.calls().await {
        assert_eq!(
            call.enabled_indexers.len(),
            1,
            "each task restricts to a single indexer: {call:?}"
        );
    }
}

#[tokio::test]
async fn requested_indexer_ids_restrict_a_title_subject_fan_out() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("Paperman.2012.1080p.WEB-DL", "a1")])
        .await
        .with_releases("idx-b", vec![nzb_release("Paperman.2012.720p.WEB-DL", "b1")])
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![
            synthetic_direct_nab_indexer_config("idx-a", "newznab"),
            synthetic_direct_nab_indexer_config("idx-b", "newznab"),
        ],
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Paperman".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2012),
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let start = app
        .start_interactive_release_search(
            &user,
            InteractiveReleaseSearchRequest {
                indexer_ids: Some(vec!["idx-b".into()]),
                ..title_request(&title.id)
            },
        )
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;

    assert_eq!(done.indexers.len(), 1, "{done:?}");
    assert_eq!(done.indexers[0].indexer_id, "idx-b");
    let calls = client.calls().await;
    assert!(
        calls
            .iter()
            .all(|call| call.enabled_indexers == vec!["idx-b".to_string()]),
        "the title subject dispatches only the requested indexer: {calls:?}"
    );
}

// ── Query subject: context-free rejections (D6) ─────────────────────────────

#[tokio::test]
async fn a_faceted_query_judges_releases_while_raw_leaves_them_unjudged() {
    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Movie.2024.TELESYNC.1080p-GRP", "ts1")],
        )
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let movie = app
        .start_interactive_release_search(
            &user,
            query_request("movie", InteractiveSearchKind::Movie),
        )
        .await
        .expect("start movie");
    let movie = await_completion(&app, &user, &movie.id).await;
    let judged = movie.results.first().expect("one result");
    assert!(
        judged.parsed_release_metadata.is_some(),
        "query results are parsed server-side: {judged:?}"
    );
    let decision = judged
        .quality_profile_decision
        .as_ref()
        .expect("faceted kinds carry a profile decision");
    assert!(
        !decision.block_codes.is_empty(),
        "a telesync is blocked by the facet's default profile: {decision:?}"
    );

    // Raw has no facet, therefore no default profile to judge against (D6).
    let raw = app
        .start_interactive_release_search(&user, query_request("movie", InteractiveSearchKind::Raw))
        .await
        .expect("start raw");
    let raw = await_completion(&app, &user, &raw.id).await;
    assert!(
        raw.results
            .first()
            .expect("one result")
            .quality_profile_decision
            .is_none(),
        "raw kind carries no profile decision: {raw:?}"
    );
}

// ── Health fields (D15) ─────────────────────────────────────────────────────

#[tokio::test]
async fn indexer_views_carry_routing_priority_and_call_timing() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "idx-a": { "enabled": true, "categories": ["2000"], "priority": 7 }
            })
            .to_string(),
        )
        .await;
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("Movie.2024.1080p.WEB-DL", "m1")])
        .await;
    let (app, user) = bootstrap_search(
        settings,
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_interactive_release_search(&user, query_request("q", InteractiveSearchKind::Movie))
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;

    let view = indexer_view(&done, "idx-a");
    assert_eq!(view.status, InteractiveReleaseSearchIndexerStatus::Completed);
    assert_eq!(view.priority, 7, "{view:?}");
    assert!(
        view.elapsed_ms.is_some(),
        "an answered indexer records its call timing: {view:?}"
    );
}

// ── Subject validation ──────────────────────────────────────────────────────

#[tokio::test]
async fn exactly_one_subject_is_required() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    for (label, request) in [
        ("neither", InteractiveReleaseSearchRequest::default()),
        (
            "both",
            InteractiveReleaseSearchRequest {
                title_id: Some("title-1".into()),
                ..query_request("q", InteractiveSearchKind::Raw)
            },
        ),
        (
            "blank query",
            query_request("   ", InteractiveSearchKind::Raw),
        ),
        (
            "query without a kind",
            InteractiveReleaseSearchRequest {
                query: Some("q".into()),
                ..InteractiveReleaseSearchRequest::default()
            },
        ),
        (
            "unknown indexer",
            InteractiveReleaseSearchRequest {
                indexer_ids: Some(vec!["idx-nope".into()]),
                ..query_request("q", InteractiveSearchKind::Raw)
            },
        ),
    ] {
        let error = app
            .start_interactive_release_search(&user, request)
            .await
            .expect_err(label);
        assert!(
            matches!(error, AppError::Validation(_)),
            "{label} should be a validation error, got {error:?}"
        );
    }
    assert!(client.calls().await.is_empty());
}

#[tokio::test]
async fn a_query_subject_search_requires_manage_system_settings() {
    let client = ScriptedIndexerClient::default();
    let (app, _) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::default());

    let error = app
        .start_interactive_release_search(&viewer, query_request("q", InteractiveSearchKind::Raw))
        .await
        .expect_err("permission gate");
    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
}

// ── Candidate tokens at grab time (D4) ──────────────────────────────────────

#[tokio::test]
async fn a_token_is_issued_for_a_release_still_held_by_the_search() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")])
        .await;
    let (app, admin) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    // A real, stored user: the candidate-token signing key is derived per actor.
    let (_, operator) = create_authenticated_user(
        &app,
        &admin,
        "grab_operator",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::ConfigManagement,
        ],
    )
    .await;

    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Paperman".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2012),
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let start = app
        .start_interactive_release_search(
            &operator,
            query_request("paperman", InteractiveSearchKind::Raw),
        )
        .await
        .expect("start");
    let done = await_completion(&app, &operator, &start.id).await;
    let download_url = done
        .results
        .first()
        .expect("one result")
        .download_url
        .clone()
        .expect("release download url");
    assert!(
        done.results[0].candidate_token.is_none(),
        "a query-subject result carries no token until a title is chosen"
    );

    let issued = app
        .issue_interactive_release_candidate_token(
            &operator,
            &start.id,
            &download_url,
            &title.id,
            None,
            None,
        )
        .await
        .expect("issue candidate token");
    assert!(issued.candidate_token.is_some(), "{issued:?}");
    assert!(issued.queue_scope.is_some(), "{issued:?}");

    let missing = app
        .issue_interactive_release_candidate_token(
            &operator,
            &start.id,
            "https://example.invalid/not-in-this-search.nzb",
            &title.id,
            None,
            None,
        )
        .await
        .expect_err("unknown release");
    assert!(matches!(missing, AppError::NotFound(_)), "{missing:?}");
}

// ── Unlinked grab (D8) ──────────────────────────────────────────────────────

#[tokio::test]
async fn an_unlinked_grab_records_an_orphan_scoped_submission_and_history() {
    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")],
        )
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let app =
        app.with_test_overrides(|services| services.with_download_submissions(submissions.clone()));
    let download_client =
        create_enabled_download_client_config(&app, &user, "Primary", "nzbget").await;

    let start = app
        .start_interactive_release_search(
            &user,
            query_request("paperman", InteractiveSearchKind::Movie),
        )
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;
    let release = done.results.first().expect("one result").clone();
    let download_url = release.download_url.clone().expect("release download url");

    let outcome = app
        .queue_unlinked_release(&user, &start.id, &download_url, &download_client.id)
        .await
        .expect("queue unlinked release");
    assert_eq!(outcome.client_name, download_client.name);
    assert_eq!(outcome.source_title, release.title);

    let rows = submissions.store.lock().await.clone();
    assert_eq!(rows.len(), 1, "one submission per grab: {rows:?}");
    let row = &rows[0];
    assert!(
        row.title_id.is_empty(),
        "an unlinked grab claims no title: {row:?}"
    );
    assert_eq!(row.scope, SubmissionScope::Orphan);
    assert_eq!(row.purpose, DownloadSubmissionPurpose::OperatorQueued);
    assert_eq!(row.source_title.as_deref(), Some(release.title.as_str()));
    assert_eq!(
        row.source_provider_name.as_deref(),
        Some(release.source.as_str())
    );
    assert_eq!(row.source_provider_id.as_deref(), Some("idx-a"));
    assert_eq!(row.release_size_bytes, release.size_bytes);
    assert_eq!(row.download_client_item_id, outcome.download_id);
    assert_eq!(
        row.facet, "movie",
        "the search kind stands in for the owner facet"
    );
    assert!(
        !crate::import::parameters::submission_has_scryer_origin(row),
        "an unlinked grab must stay unowned so the import waits for a manual assignment: {row:?}"
    );

    let events = app
        .services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::ReleaseGrabbed]),
            title_id: None,
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 10,
        })
        .await
        .expect("release grabbed events should load");
    let grabbed = events
        .iter()
        .find_map(|event| match &event.payload {
            DomainEventPayload::ReleaseGrabbed(data) => Some(data),
            _ => None,
        })
        .expect("release grabbed event");
    assert_eq!(
        grabbed.source_title.as_deref(),
        Some(release.title.as_str())
    );
    assert_eq!(
        grabbed.download_id.as_deref(),
        Some(outcome.download_id.as_str())
    );
    assert_eq!(
        grabbed.source_provider.as_deref(),
        Some(release.source.as_str())
    );
    assert_eq!(
        grabbed.title.title_name, release.title,
        "with no catalog title the release name stands in"
    );
}

#[tokio::test]
async fn an_unlinked_grab_refuses_unknown_releases_unusable_clients_and_unprivileged_actors() {
    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")],
        )
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let enabled = create_enabled_download_client_config(&app, &user, "Primary", "nzbget").await;
    let disabled = app
        .create_download_client_config(
            &user,
            NewDownloadClientConfig {
                proxy_config_id: None,
                name: "Retired".to_string(),
                client_type: "nzbget".to_string(),
                config_json: "{}".to_string(),
                client_priority: 2,
                is_enabled: false,
            },
        )
        .await
        .expect("create disabled download client config");

    let start = app
        .start_interactive_release_search(
            &user,
            query_request("paperman", InteractiveSearchKind::Raw),
        )
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;
    let download_url = done
        .results
        .first()
        .expect("one result")
        .download_url
        .clone()
        .expect("release download url");

    let missing = app
        .queue_unlinked_release(
            &user,
            &start.id,
            "https://example.invalid/not-in-this-search.nzb",
            &enabled.id,
        )
        .await
        .expect_err("unknown release");
    assert!(matches!(missing, AppError::NotFound(_)), "{missing:?}");

    for (label, client_id) in [("disabled", disabled.id.as_str()), ("unknown", "dc-nope")] {
        let error = app
            .queue_unlinked_release(&user, &start.id, &download_url, client_id)
            .await
            .expect_err(label);
        assert!(
            matches!(error, AppError::Validation(_)),
            "{label} client should be a validation error, got {error:?}"
        );
    }

    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::default());
    let denied = app
        .queue_unlinked_release(&viewer, &start.id, &download_url, &enabled.id)
        .await
        .expect_err("permission gate");
    assert!(matches!(denied, AppError::Unauthorized(_)), "{denied:?}");
}

// ── Download to browser (D17, FR-028) ───────────────────────────────────────

/// Answers `fetch_release_artifact` from a scripted table keyed by download
/// URL; a URL with no entry fails the way an unreachable indexer would.
#[derive(Default)]
struct ArtifactDownloadClient {
    artifacts: HashMap<String, ResolvedDownloadArtifact>,
}

#[async_trait]
impl DownloadClient for ArtifactDownloadClient {
    async fn submit_download(
        &self,
        _request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        Err(AppError::Repository(
            "a browser download never submits".to_string(),
        ))
    }

    async fn fetch_release_artifact(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let url = request.source_hint.clone().unwrap_or_default();
        self.artifacts
            .get(&url)
            .cloned()
            .ok_or_else(|| AppError::Validation(format!("indexer refused {url}")))
    }
}

fn nzb_artifact(marker: &str) -> ResolvedDownloadArtifact {
    ResolvedDownloadArtifact::Nzb {
        bytes: format!("<nzb>{marker}</nzb>").into_bytes(),
        file_name: None,
        content_type: None,
    }
}

async fn grabbed_release_titles(app: &AppUseCase) -> Vec<String> {
    app.services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::ReleaseGrabbed]),
            title_id: None,
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 50,
        })
        .await
        .expect("release grabbed events should load")
        .iter()
        .filter_map(|event| match &event.payload {
            DomainEventPayload::ReleaseGrabbed(data) => data.source_title.clone(),
            _ => None,
        })
        .collect()
}

/// Swap in a download client that answers with exactly these artifacts.
fn with_artifacts(
    app: &AppUseCase,
    artifacts: HashMap<String, ResolvedDownloadArtifact>,
) -> AppUseCase {
    app.with_test_overrides(|services| {
        services.with_download_client(Arc::new(ArtifactDownloadClient { artifacts }))
    })
}

/// Bootstrap a completed search over `releases` and return its download URLs.
async fn browser_download_fixture(
    releases: Vec<IndexerSearchResult>,
) -> (AppUseCase, User, String, Vec<String>) {
    let expected = releases.len();
    let indexer_client = ScriptedIndexerClient::default()
        .with_releases("idx-a", releases)
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        indexer_client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let start = app
        .start_interactive_release_search(
            &user,
            query_request("paperman", InteractiveSearchKind::Movie),
        )
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;
    let urls = done
        .results
        .iter()
        .map(|result| result.download_url.clone().expect("release download url"))
        .collect::<Vec<_>>();
    assert_eq!(urls.len(), expected, "every seeded release should survive dedupe");
    (app, user, start.id, urls)
}

fn targets(search_id: &str, urls: &[String]) -> Vec<InteractiveSearchArtifactTarget> {
    urls.iter()
        .map(|url| InteractiveSearchArtifactTarget {
            search_id: search_id.to_string(),
            download_url: url.clone(),
        })
        .collect()
}

#[test]
fn artifact_file_names_are_sanitised_and_deduped() {
    use crate::catalog::interactive_release_search::{
        artifact_file_name, dedupe_archive_file_name,
    };

    assert_eq!(
        artifact_file_name("Paperman.2012.1080p.WEB-DL", ".nzb"),
        "Paperman.2012.1080p.WEB-DL.nzb"
    );
    // Path separators and control characters cannot escape the archive root.
    assert_eq!(
        artifact_file_name("../etc/pass\u{7}wd", ".torrent"),
        "etcpasswd.torrent"
    );
    assert_eq!(artifact_file_name("  Some\t Release \n", ".nzb"), "Some Release.nzb");
    assert_eq!(artifact_file_name("   ", ".nzb"), "release.nzb");
    assert_eq!(artifact_file_name(&"a".repeat(400), ".nzb").len(), 184);

    let mut taken = std::collections::HashSet::new();
    assert_eq!(dedupe_archive_file_name(&mut taken, "Same.nzb"), "Same.nzb");
    assert_eq!(
        dedupe_archive_file_name(&mut taken, "Same.nzb"),
        "Same (2).nzb"
    );
    assert_eq!(
        dedupe_archive_file_name(&mut taken, "Same.nzb"),
        "Same (3).nzb"
    );
    assert_eq!(
        dedupe_archive_file_name(&mut taken, "no-extension"),
        "no-extension"
    );
}

#[tokio::test]
async fn one_release_downloads_its_own_file_and_records_a_grab() {
    let (app, user, search_id, urls) =
        browser_download_fixture(vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")]).await;
    let app = with_artifacts(
        &app,
        HashMap::from([(urls[0].clone(), nzb_artifact("one"))]),
    );

    let bundle = app
        .download_interactive_search_artifacts(&user, &targets(&search_id, &urls))
        .await
        .expect("single release download");
    assert_eq!(bundle.file_name, "Paperman.2012.1080p.WEB-DL.nzb");
    assert_eq!(bundle.content_type, "application/x-nzb");
    assert_eq!(bundle.bytes, b"<nzb>one</nzb>");

    assert_eq!(
        grabbed_release_titles(&app).await,
        vec!["Paperman.2012.1080p.WEB-DL".to_string()],
        "a browser download is a grab from the indexer's perspective"
    );
}

#[tokio::test]
async fn several_releases_download_as_one_tar_gz_and_record_a_grab_each() {
    use std::io::Read as _;

    let (app, user, search_id, urls) = browser_download_fixture(vec![
        nzb_release("Paperman.2012.1080p.WEB-DL", "g1"),
        nzb_release("Paperman.2012.1080p.WEB-DL", "g2"),
    ])
    .await;
    let app = with_artifacts(
        &app,
        HashMap::from([
            (urls[0].clone(), nzb_artifact("first")),
            (urls[1].clone(), nzb_artifact("second")),
        ]),
    );

    let bundle = app
        .download_interactive_search_artifacts(&user, &targets(&search_id, &urls))
        .await
        .expect("bundled download");
    assert!(
        bundle.file_name.starts_with("scryer-releases-") && bundle.file_name.ends_with(".tar.gz"),
        "{}",
        bundle.file_name
    );
    assert_eq!(bundle.content_type, "application/gzip");

    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bundle.bytes.as_slice()));
    let mut members = Vec::new();
    for entry in archive.entries().expect("archive entries") {
        let mut entry = entry.expect("archive entry");
        let path = entry.path().expect("entry path").display().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("entry bytes");
        members.push((path, bytes));
    }
    // Both indexer rows carry the same release name, so the second is deduped
    // rather than overwriting the first.
    assert_eq!(
        members,
        vec![
            (
                "Paperman.2012.1080p.WEB-DL.nzb".to_string(),
                b"<nzb>first</nzb>".to_vec()
            ),
            (
                "Paperman.2012.1080p.WEB-DL (2).nzb".to_string(),
                b"<nzb>second</nzb>".to_vec()
            ),
        ]
    );

    assert_eq!(grabbed_release_titles(&app).await.len(), 2);
}

#[tokio::test]
async fn a_failed_or_magnet_only_release_fails_the_bundle_without_recording_a_grab() {
    let (app, user, search_id, urls) = browser_download_fixture(vec![
        nzb_release("Paperman.2012.1080p.WEB-DL", "g1"),
        nzb_release("Bluey.S01E01.1080p.WEB-DL", "g2"),
    ])
    .await;

    // Only the first release resolves: the second fetch fails the request.
    let partial = with_artifacts(
        &app,
        HashMap::from([(urls[0].clone(), nzb_artifact("first"))]),
    );
    let error = partial
        .download_interactive_search_artifacts(&user, &targets(&search_id, &urls))
        .await
        .expect_err("a failed fetch fails the whole bundle");
    assert_eq!(
        error.to_string(),
        "validation: Bluey.S01E01.1080p.WEB-DL: indexer refused https://example.invalid/g2.nzb",
        "the message must name the release that failed"
    );
    assert!(
        grabbed_release_titles(&partial).await.is_empty(),
        "a failed bundle records no grab"
    );

    let magnet_only = with_artifacts(
        &app,
        HashMap::from([(
            urls[0].clone(),
            ResolvedDownloadArtifact::Magnet {
                uri: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_string(),
                info_hash_hint: None,
            },
        )]),
    );
    let error = magnet_only
        .download_interactive_search_artifacts(&user, &targets(&search_id, &urls[..1]))
        .await
        .expect_err("a magnet has no file");
    assert!(
        error.to_string().contains("magnet link"),
        "{error}"
    );
    assert!(grabbed_release_titles(&magnet_only).await.is_empty());
}

#[tokio::test]
async fn browser_downloads_refuse_empty_oversized_and_unprivileged_requests() {
    let (app, user, search_id, urls) =
        browser_download_fixture(vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")]).await;

    let empty = app
        .download_interactive_search_artifacts(&user, &[])
        .await
        .expect_err("nothing selected");
    assert!(matches!(empty, AppError::Validation(_)), "{empty:?}");

    let too_many = vec![urls[0].clone(); 51];
    let oversized = app
        .download_interactive_search_artifacts(&user, &targets(&search_id, &too_many))
        .await
        .expect_err("over the per-request cap");
    assert!(matches!(oversized, AppError::Validation(_)), "{oversized:?}");

    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::default());
    let denied = app
        .download_interactive_search_artifacts(&viewer, &targets(&search_id, &urls[..1]))
        .await
        .expect_err("permission gate");
    assert!(matches!(denied, AppError::Unauthorized(_)), "{denied:?}");
}
