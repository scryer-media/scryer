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
    season: Option<u32>,
    episode: Option<u32>,
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
        season: Option<u32>,
        episode: Option<u32>,
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
            season,
            episode,
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
    wait_for(
        "the interactive release search to reach a terminal state",
        || async {
            let snapshot = app
                .interactive_release_search(user, job_id)
                .await
                .expect("poll interactive release search")
                .expect("job present");
            (snapshot.state != InteractiveReleaseSearchState::Running).then_some(snapshot)
        },
    )
    .await
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
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")],
        )
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
async fn explicitly_requested_disabled_indexer_is_reported_without_dispatch() {
    let client = ScriptedIndexerClient::default();
    let mut disabled = synthetic_direct_nab_indexer_config("idx-disabled", "newznab");
    disabled.is_enabled = false;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![disabled],
    );
    let job = app
        .start_interactive_release_search(
            &user,
            InteractiveReleaseSearchRequest {
                indexer_ids: Some(vec!["idx-disabled".into()]),
                ..query_request("q", InteractiveSearchKind::Raw)
            },
        )
        .await
        .expect("start explicit search");
    let done = await_completion(&app, &user, &job.id).await;
    let view = indexer_view(&done, "idx-disabled");
    assert_eq!(view.status, InteractiveReleaseSearchIndexerStatus::Skipped);
    assert_eq!(view.failure_reason.as_deref(), Some("indexer is disabled"));
    assert!(client.calls().await.is_empty());

    let job = app
        .start_interactive_release_search(&user, query_request("all", InteractiveSearchKind::Raw))
        .await
        .expect("start unrestricted search");
    assert!(
        await_completion(&app, &user, &job.id)
            .await
            .indexers
            .is_empty()
    );
}

#[tokio::test]
async fn equivalent_indexer_sets_replace_the_running_query_search() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![
            synthetic_direct_nab_indexer_config("idx-a", "newznab"),
            synthetic_direct_nab_indexer_config("idx-b", "newznab"),
        ],
    );
    // Keep the first job running until its equivalent replacement is registered.
    let releases = client.releases.lock().await;
    let first = app
        .start_interactive_release_search(
            &user,
            InteractiveReleaseSearchRequest {
                indexer_ids: Some(vec!["idx-a".into(), "idx-b".into()]),
                ..query_request("q", InteractiveSearchKind::Raw)
            },
        )
        .await
        .expect("start first search");
    let second = app
        .start_interactive_release_search(
            &user,
            InteractiveReleaseSearchRequest {
                indexer_ids: Some(vec!["idx-b".into(), "idx-a".into(), "idx-b".into()]),
                ..query_request("q", InteractiveSearchKind::Raw)
            },
        )
        .await
        .expect("start replacement");
    let first = app
        .interactive_release_search(&user, &first.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.state, InteractiveReleaseSearchState::Cancelled);
    drop(releases);
    let second = await_completion(&app, &user, &second.id).await;
    assert_eq!(second.indexers.len(), 2);
}

#[tokio::test]
async fn requested_indexer_ids_restrict_a_title_subject_fan_out() {
    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "a1")],
        )
        .await
        .with_releases(
            "idx-b",
            vec![nzb_release("Paperman.2012.720p.WEB-DL", "b1")],
        )
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

// ── Title subject: a whole season ──────────────────────────────────────────

/// A series with a season-2 collection holding two episodes. Returns the
/// title and the season's collection id.
async fn series_with_second_season(app: &AppUseCase, user: &User) -> (Title, String) {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: "Glass Harbor".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    let collection = app
        .create_collection(
            user,
            title.id.clone(),
            "season".into(),
            "2".into(),
            Some("Season 2".into()),
            None,
            Some("1".into()),
            Some("2".into()),
        )
        .await
        .expect("create season collection");
    for number in ["1", "2"] {
        app.create_episode(
            user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some(number.into()),
            Some("2".into()),
            Some(format!("S02E0{number}")),
            None,
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create season episode");
    }
    (title, collection.id)
}

#[tokio::test]
async fn a_season_only_title_search_asks_indexers_for_the_season_and_no_episode() {
    // No download client is configured here, so an announced source kind
    // would be dropped as unroutable before scoring.
    let mut pack = nzb_release("Glass.Harbor.S02.1080p.WEB-DL-GRP", "pack");
    pack.source_kind = None;
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![pack])
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let (title, _) = series_with_second_season(&app, &user).await;

    let start = app
        .start_interactive_release_search(
            &user,
            InteractiveReleaseSearchRequest {
                season: Some("2".into()),
                ..title_request(&title.id)
            },
        )
        .await
        .expect("start season search");
    let done = await_completion(&app, &user, &start.id).await;

    let calls = client.calls().await;
    assert!(!calls.is_empty(), "the season search reached the indexer");
    for call in &calls {
        assert_eq!(call.season, Some(2), "{call:?}");
        assert_eq!(
            call.episode, None,
            "a season search names no episode: {call:?}"
        );
        assert_eq!(call.facet.as_deref(), Some("series"), "{call:?}");
    }
    assert!(
        calls.iter().any(|call| call.query == "Glass Harbor S02"),
        "the season-pack query form is sent: {calls:?}"
    );
    assert!(
        done.results
            .iter()
            .any(|result| result.title == "Glass.Harbor.S02.1080p.WEB-DL-GRP"),
        "the season pack is listed: {done:?}"
    );
}

// ── Query subject: context-free rejections ─────────────────────────────

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

    // Raw has no facet, therefore no default profile to judge against.
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

#[tokio::test]
async fn recoverable_scores_faceted_queries_include_all_rules_before_finalizing() {
    for rescued in [false, true] {
        let client = ScriptedIndexerClient::default()
            .with_releases(
                "idx-a",
                vec![
                    nzb_release("Movie.2024.1080p.WEB-DL-GRP", "score"),
                    nzb_release("Movie.2024.1080p.WEB-DL-OTHER", "other"),
                ],
            )
            .await;
        let (app, user) = bootstrap_search(
            Arc::new(StoredSettingsRepo::default()),
            client,
            vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
        );
        let mut policies = vec![scryer_rules::UserPolicy {
            id: "penalty".into(),
            name: "Penalty".into(),
            applied_facets: vec![],
            origin: scryer_rules::PolicyOrigin::System,
            rego_source: scryer_rules::rewrite_package_declaration(
                "score_entry[\"penalty\"] := scryer.block_score()",
                "penalty",
            ),
        }];
        if rescued {
            policies.push(scryer_rules::UserPolicy {
                id: "boost".into(),
                name: "Boost".into(),
                applied_facets: vec![],
                origin: scryer_rules::PolicyOrigin::User,
                rego_source: scryer_rules::rewrite_package_declaration(
                    "score_entry[\"boost\"] := 20000 if { input.release.release_group == \"GRP\" }",
                    "boost",
                ),
            });
        }
        *app.services.customization.user_rules.write().unwrap() =
            scryer_rules::UserRulesEngine::build(&policies).unwrap();
        let job = app
            .start_interactive_release_search(
                &user,
                query_request("movie", InteractiveSearchKind::Movie),
            )
            .await
            .unwrap();
        let result = await_completion(&app, &user, &job.id).await;
        assert_eq!(result.results.len(), 2);
        for release in &result.results {
            let decision = release.quality_profile_decision.as_ref().unwrap();
            assert_eq!(
                decision.allowed,
                rescued && release.title.ends_with("-GRP"),
                "{decision:?}"
            );
            assert!(
                decision
                    .scoring_log
                    .iter()
                    .any(|entry| entry.code == "penalty" && entry.delta == -10_000)
            );
        }
    }
}

// ── Health fields ─────────────────────────────────────────────────────

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
    assert_eq!(
        view.status,
        InteractiveReleaseSearchIndexerStatus::Completed
    );
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

// ── Candidate tokens at grab time ──────────────────────────────────────

#[tokio::test]
async fn a_season_only_token_binds_a_series_season_and_is_refused_for_a_movie() {
    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![
                nzb_release("Glass.Harbor.S02.1080p.WEB-DL-GRP", "pack"),
                nzb_release("Glass.Harbor.S02E01.1080p.WEB-DL-GRP", "episode"),
                // Carries no season or episode marker, so its coverage cannot
                // be read from the name and the binding falls back to the
                // subject the token was issued for.
                nzb_release("Glass.Harbor.Extras.1080p.WEB-DL-GRP", "extras"),
            ],
        )
        .await;
    let (app, admin) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let (_, operator) = create_authenticated_user(
        &app,
        &admin,
        "season_operator",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::ConfigManagement,
        ],
    )
    .await;
    let (series, collection_id) = series_with_second_season(&app, &admin).await;
    let movie = app
        .add_title(
            &admin,
            NewTitle {
                name: "Paper Lantern".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2019),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");

    let start = app
        .start_interactive_release_search(
            &operator,
            query_request("glass harbor", InteractiveSearchKind::Series),
        )
        .await
        .expect("start");
    let done = await_completion(&app, &operator, &start.id).await;
    let download_url_for = |guid: &str| {
        let wanted = format!("https://example.invalid/{guid}.nzb");
        done.results
            .iter()
            .find_map(|result| {
                result
                    .download_url
                    .as_deref()
                    .filter(|url| *url == wanted)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| panic!("{guid} release listed: {:?}", done.results))
    };
    let first_episode_id = app
        .list_episodes(&admin, &collection_id)
        .await
        .expect("list season episodes")
        .into_iter()
        .find(|episode| episode.episode_number.as_deref() == Some("1"))
        .expect("season 2 episode 1")
        .id;

    let season_scope = |download_url: String| {
        let app = &app;
        let operator = &operator;
        let start_id = &start.id;
        let series_id = &series.id;
        async move {
            let issued = app
                .issue_interactive_release_candidate_token(
                    operator,
                    start_id,
                    &download_url,
                    series_id,
                    Some("2".into()),
                    None,
                )
                .await
                .expect("season-only token for a series");
            assert!(issued.candidate_token.is_some(), "{issued:?}");
            issued.queue_scope
        }
    };

    assert_eq!(
        season_scope(download_url_for("pack")).await,
        Some(SubmissionScope::Collection {
            collection_id: collection_id.clone()
        }),
        "a season pack binds its season"
    );
    assert_eq!(
        season_scope(download_url_for("episode")).await,
        Some(SubmissionScope::Episode {
            episode_id: first_episode_id
        }),
        "a single-episode release binds that episode"
    );
    assert_eq!(
        season_scope(download_url_for("extras")).await,
        Some(SubmissionScope::Collection {
            collection_id: collection_id.clone()
        }),
        "a release with no readable coverage binds the searched season"
    );
    let download_url = download_url_for("pack");

    let error = app
        .issue_interactive_release_candidate_token(
            &operator,
            &start.id,
            &download_url,
            &movie.id,
            Some("1".into()),
            None,
        )
        .await
        .expect_err("season-only token for a movie");
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
}

#[tokio::test]
async fn a_token_is_issued_for_a_release_still_held_by_the_search() {
    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")],
        )
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

    struct RoutedClient {
        inner: Arc<dyn DownloadClient>,
        requests: Arc<Mutex<Vec<DownloadClientAddRequest>>>,
    }
    #[async_trait]
    impl DownloadClient for RoutedClient {
        async fn indexer_grab_clients(
            &self,
            _: &Title,
            _: Option<&str>,
            _: DownloadSourceKind,
        ) -> AppResult<Vec<crate::IndexerGrabClient>> {
            Ok(vec![crate::IndexerGrabClient {
                id: "fixture-client".into(),
                name: "Fixture".into(),
                category: Some("routed".into()),
                mapped: true,
            }])
        }
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.requests.lock().await.push(request.clone());
            self.inner.submit_download(request).await
        }
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let routed_client = Arc::new(RoutedClient {
        inner: app.services.integrations.download_client.clone(),
        requests: requests.clone(),
    });
    let app = app.with_test_overrides(|services| {
        services
            .with_download_client(routed_client)
            .with_download_submissions(submissions.clone())
    });
    let token = issued.candidate_token.as_deref().unwrap();
    let denied = app
        .queue_indexer_search_assignment(
            &operator,
            &title.id,
            token,
            issued.size_bytes,
            SubmissionConflictPolicy::from_replace_flag(false),
            false,
            crate::IndexerGrabSelection {
                client_id: "stale-client".into(),
                category: Some("custom".into()),
            },
        )
        .await
        .expect_err("stale selection must be rejected before submission");
    assert!(matches!(denied, AppError::Validation(_)));
    assert!(requests.lock().await.is_empty());
    let outcome = app
        .queue_indexer_search_assignment(
            &operator,
            &title.id,
            token,
            issued.size_bytes,
            SubmissionConflictPolicy::from_replace_flag(false),
            false,
            crate::IndexerGrabSelection {
                client_id: "fixture-client".into(),
                category: Some(String::new()),
            },
        )
        .await
        .expect("assigned grab");
    assert!(matches!(outcome, QueueDownloadOutcome::Queued(_)));
    let handed = requests.lock().await;
    assert_eq!(handed.len(), 1);
    assert_eq!(handed[0].title.id, title.id);
    assert_eq!(handed[0].category.as_deref(), Some(""));
    assert_eq!(
        handed[0].pinned_download_client_id.as_deref(),
        Some("fixture-client")
    );
    drop(handed);
    let recorded = submissions.store.lock().await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].title_id, title.id);
    drop(recorded);

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

// ── Unlinked grab ──────────────────────────────────────────────────────

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
    let stats = Arc::new(RecordingIndexerStatsTracker::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_download_submissions(submissions.clone())
            .with_indexer_stats(stats.clone())
    });
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

    *submissions.record_submission_error.lock().await = Some("temporary catalog outage".into());
    let error = app
        .queue_unlinked_release_with_category(
            &user,
            &start.id,
            &download_url,
            &download_client.id,
            Some("custom".into()),
        )
        .await
        .expect_err("client acceptance survives a catalog outage");
    assert!(matches!(error, AppError::DownloadSubmitAmbiguous(_)));
    *submissions.record_submission_error.lock().await = None;
    let outcome = app
        .queue_unlinked_release_with_category(
            &user,
            &start.id,
            &download_url,
            &download_client.id,
            Some("custom".into()),
        )
        .await
        .expect("queue unlinked release");
    assert_eq!(outcome.client_name, download_client.name);
    assert_eq!(outcome.source_title, release.title);

    // The indexer's grab is counted like every other trigger, under the
    // configured indexer name rather than the release's display source.
    let grabs = stats.grabs.lock().expect("grab log mutex").clone();
    assert_eq!(
        grabs,
        vec![("idx-a".to_string(), "Synthetic newznab".to_string())],
        "an accepted unlinked grab counts once against its indexer: {grabs:?}"
    );
    assert_ne!(
        release.source, "Synthetic newznab",
        "the fixture must exercise the source-to-configured-name resolution"
    );

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
            stream_id: None,
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

/// The download-client router refuses an indexer URL it has to fetch itself,
/// so an unlinked grab must resolve the indexer artifact first, exactly as
/// the canonical submission path does, and hold the artifact lease until the
/// client accepts.
#[tokio::test]
async fn an_unlinked_grab_resolves_the_indexer_artifact_before_client_routing() {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Lease {
        active: Arc<AtomicBool>,
        staged: crate::StagedNzbRef,
    }
    impl Drop for Lease {
        fn drop(&mut self) {
            self.active.store(false, Ordering::SeqCst);
        }
    }
    impl crate::IndexerArtifactLease for Lease {
        fn staged_nzb(&self) -> &crate::StagedNzbRef {
            &self.staged
        }
    }
    struct Resolver {
        active: Arc<AtomicBool>,
        requests: Arc<StdMutex<Vec<crate::IndexerArtifactResolutionRequest>>>,
    }
    #[async_trait]
    impl crate::IndexerArtifactResolver for Resolver {
        async fn resolve_artifact(
            &self,
            request: &crate::IndexerArtifactResolutionRequest,
        ) -> AppResult<crate::PreparedIndexerArtifact> {
            self.requests
                .lock()
                .expect("resolver request log")
                .push(request.clone());
            self.active.store(true, Ordering::SeqCst);
            Ok(crate::PreparedIndexerArtifact::StagedNzb(Box::new(Lease {
                active: self.active.clone(),
                staged: crate::StagedNzbRef {
                    id: "unlinked-lease".into(),
                    compressed_path: "fixture.nzb.gz".into(),
                    raw_size_bytes: 10,
                },
            })))
        }
    }
    /// What the download client saw at the moment it was handed the grab.
    #[derive(Clone, Debug)]
    struct Handed {
        lease_active: bool,
        staged_nzb_id: Option<String>,
        source_hint: Option<String>,
    }
    struct Client {
        inner: Arc<dyn DownloadClient>,
        active: Arc<AtomicBool>,
        handed: Arc<StdMutex<Vec<Handed>>>,
    }
    #[async_trait]
    impl DownloadClient for Client {
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.handed.lock().expect("handed log").push(Handed {
                lease_active: self.active.load(Ordering::SeqCst),
                staged_nzb_id: request.staged_nzb.as_ref().map(|value| value.id.clone()),
                source_hint: request.source_hint.clone(),
            });
            tokio::task::yield_now().await;
            self.inner.submit_download(request).await
        }
    }

    let indexer_client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")],
        )
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        indexer_client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let active = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let handed = Arc::new(StdMutex::new(Vec::new()));
    let resolver: Arc<dyn crate::IndexerArtifactResolver> = Arc::new(Resolver {
        active: active.clone(),
        requests: requests.clone(),
    });
    let client = Arc::new(Client {
        inner: app.services.integrations.download_client.clone(),
        active: active.clone(),
        handed: handed.clone(),
    });
    let app = app.with_test_overrides(|services| {
        services
            .with_download_submissions(submissions.clone())
            .with_indexer_artifact_resolver(Some(resolver))
            .with_download_client(client)
    });
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

    app.queue_unlinked_release(&user, &start.id, &download_url, &download_client.id)
        .await
        .expect("queue unlinked release");

    let requests = requests.lock().expect("resolver request log").clone();
    assert_eq!(
        requests.len(),
        1,
        "an unlinked grab resolves its indexer artifact exactly once"
    );
    assert_eq!(requests[0].indexer_id.as_deref(), Some("idx-a"));
    assert_eq!(requests[0].source_url, download_url);
    assert_eq!(requests[0].source_kind, Some(DownloadSourceKind::NzbUrl));
    assert_eq!(requests[0].title_id, None, "an unlinked grab has no title");
    assert_eq!(
        requests[0].search_facet,
        Some(MediaFacet::Movie),
        "the search kind stands in for the owner facet"
    );

    let handed = handed.lock().expect("handed log").clone();
    assert_eq!(handed.len(), 1, "{handed:?}");
    assert!(
        handed[0].lease_active,
        "artifact lease ended before submission"
    );
    assert_eq!(handed[0].staged_nzb_id.as_deref(), Some("unlinked-lease"));
    assert!(
        handed[0].source_hint.is_none(),
        "download client must receive the artifact, not the indexer URL: {handed:?}"
    );
    assert!(
        !active.load(Ordering::SeqCst),
        "an accepted unlinked grab should release the artifact"
    );

    let rows = submissions.store.lock().await.clone();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].source_hint.as_deref(),
        Some(download_url.as_str()),
        "history keeps the release URL the operator grabbed"
    );
    assert_eq!(rows[0].source_kind, Some(DownloadSourceKind::NzbUrl));
}

/// An unlinked grab the client actually reports — first downloading, then
/// completed — must still reach an import-pending offer.
///
/// This is the shape the e2e run shows: the download is observed continuously
/// for seven minutes, the client has it completed the whole time, and the
/// submission's tracked state, its identity state and the import row all stay
/// empty. Every other accepted grab carries an identity persisted alongside the
/// submission by `submit_canonical_download` (`acquisition/submission.rs`); the
/// unlinked grab is the one that records the submission alone, so it is the one
/// an observation has nothing to bind back to. The release finishing between
/// two polls is not the story: the client reports it in both lanes here.
#[tokio::test]
async fn an_unlinked_grab_reaches_an_import_offer_once_the_client_completes_it() {
    let indexer_client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")],
        )
        .await;
    let (app, user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        indexer_client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let download_client = Arc::new(StubDownloadClient::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_download_submissions(submissions.clone())
            // The search bootstrap does not wire one, and the tracker writes
            // every observation through it; the download-tracking fixtures
            // (`bootstrap_with_cleanup_tracking`) wire exactly this.
            .with_download_registry(Arc::new(FixtureDownloadRegistry::default()))
            .with_download_client(download_client.clone())
    });
    let client_config =
        create_enabled_download_client_config(&app, &user, "Primary", "nzbget").await;

    let start = app
        .start_interactive_release_search(
            &user,
            query_request("paperman", InteractiveSearchKind::Movie),
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

    let outcome = app
        .queue_unlinked_release(&user, &start.id, &download_url, &client_config.id)
        .await
        .expect("queue unlinked release");

    // The durable half of an accepted grab: without the identity row there is
    // nothing for an observation to bind to, whatever the client later reports.
    let identities = submissions.identities.lock().await.clone();
    assert_eq!(
        identities.len(),
        1,
        "an accepted unlinked grab must persist its identity like every other accepted grab: {identities:?}"
    );

    // The client reports the job the way nzbget did in the failing run: first
    // in the queue, downloading, then completed in history.
    let source_title = done.results.first().expect("one result").title.clone();
    let mut client_item =
        queue_history_fixture_item(&outcome.download_id, DownloadQueueState::Downloading, 40);
    client_item.client_id = client_config.id.clone();
    client_item.client_name = client_config.name.clone();
    client_item.client_type = "nzbget".to_string();
    // An unlinked grab owns no title, which is the whole shape under test.
    client_item.title_id = None;
    client_item.title_name = source_title.clone();
    client_item.facet = Some("series".to_string());
    client_item.progress_percent = 40;
    // The stub calls one client authoritative by default ("primary"); this
    // flow's client is the one the grab was pinned to, so say so.
    download_client
        .set_snapshot_authoritative_client_ids([client_config.id.clone()])
        .await;
    *download_client.queue_items.lock().await = vec![client_item.clone()];

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                ..Default::default()
            },
        ),
    );

    // Let the poller see it downloading, so the failure under test is not "the
    // client finished it between two polls" but "it was watched the whole way".
    wait_until("the poller to read the client's queue twice", || async {
        *download_client.queue_calls.lock().await >= 2
    })
    .await;

    let payload_dir = tempfile::tempdir().expect("payload dir");
    // A real payload, so what the import check decides is about the missing
    // title and not about an empty directory.
    std::fs::write(
        payload_dir.path().join(format!("{source_title}.mkv")),
        vec![0u8; 1024],
    )
    .expect("write payload file");
    let mut completed = completed_download_fixture_item(
        &outcome.download_id,
        "",
        source_title.as_str(),
        payload_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = client_config.id.clone();
    completed.client_type = "nzbget".to_string();
    // No title parameters: the client was handed no title, because there is none.
    completed.parameters = Vec::new();

    let mut history_item = client_item.clone();
    history_item.state = DownloadQueueState::Completed;
    history_item.progress_percent = 100;
    *download_client.queue_items.lock().await = Vec::new();
    *download_client.history_items.lock().await = vec![history_item];
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed.clone()]);
    *download_client.completed_downloads.lock().await = vec![completed];

    // Bounded rather than panicking so a miss reports the tracked states below.
    let offered = timeout(TEST_WAIT_DEADLINE, async {
        loop {
            // Either state puts the download on the import activity as a row
            // the operator can assign by hand (`list_download_history_items`
            // ranks Pending and Blocked alike); a title-less grab has nothing
            // to import into, so Blocked is the honest one. The failure this
            // reproduces is neither: no tracked state at all.
            if submissions
                .tracked_states
                .lock()
                .await
                .values()
                .any(|state| {
                    state == TrackedDownloadState::ImportPending.as_str()
                        || state == TrackedDownloadState::ImportBlocked.as_str()
                })
            {
                return true;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or(false);

    let tracked_states = submissions.tracked_states.lock().await.clone();
    let identity_states = submissions.identity_states.lock().await.clone();

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");

    assert!(
        *download_client.history_calls.lock().await > 0
            || !download_client
                .recent_activity_calls
                .lock()
                .await
                .is_empty(),
        "the fixture must actually be observed for this assertion to mean anything"
    );
    assert!(
        offered,
        "a completed unlinked grab must reach an import-pending offer: tracked_states={tracked_states:?} identity_states={identity_states:?}"
    );
}

#[tokio::test]
async fn grab_indexer_name_prefers_the_configured_name_and_falls_back_to_the_source() {
    let (app, _user) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        ScriptedIndexerClient::default(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    // A live release: `source` is the adapter's "<name> (<type>)" display
    // string, but the grab is counted under the configured name.
    assert_eq!(
        app.grab_indexer_name(Some("idx-a"), Some("Synthetic newznab (newznab)"))
            .await
            .as_deref(),
        Some("Synthetic newznab")
    );
    // The indexer was deleted since the release was found (a parked pending
    // release, say): the recorded source is the best remaining name.
    assert_eq!(
        app.grab_indexer_name(Some("idx-gone"), Some("Old Indexer (newznab)"))
            .await
            .as_deref(),
        Some("Old Indexer (newznab)")
    );
    // No indexer identity at all.
    assert_eq!(
        app.grab_indexer_name(None, Some(" pushed "))
            .await
            .as_deref(),
        Some("pushed")
    );
    assert_eq!(app.grab_indexer_name(None, Some("  ")).await, None);
    assert_eq!(app.grab_indexer_name(Some("  "), None).await, None);
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

/// An assigned grab is gated like any other grab for the title: a title
/// manager may list clients for a release assigned to a title they manage and
/// submit it, while the title-less (unlinked) listing still needs system
/// settings.
#[tokio::test]
async fn an_assigned_grab_needs_only_title_management() {
    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Paperman.2012.1080p.WEB-DL", "c1")],
        )
        .await;
    let (app, admin) = bootstrap_search(
        Arc::new(StoredSettingsRepo::default()),
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let (_, manager) = create_authenticated_user(
        &app,
        &admin,
        "title_manager",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
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
    // Title searches keep only releases some enabled client can take.
    create_enabled_download_client_config(&app, &admin, "Primary", "nzbget").await;
    let start = app
        .start_interactive_release_search(&manager, title_request(&title.id))
        .await
        .expect("a title manager may search their title");
    let done = await_completion(&app, &manager, &start.id).await;
    let release = done
        .results
        .first()
        .unwrap_or_else(|| panic!("one result: {done:?}"))
        .clone();
    let download_url = release.download_url.clone().expect("release download url");

    struct ListedClient {
        inner: Arc<dyn DownloadClient>,
        requests: Arc<Mutex<Vec<DownloadClientAddRequest>>>,
    }
    #[async_trait]
    impl DownloadClient for ListedClient {
        async fn indexer_grab_clients(
            &self,
            _: &Title,
            _: Option<&str>,
            _: DownloadSourceKind,
        ) -> AppResult<Vec<crate::IndexerGrabClient>> {
            Ok(vec![crate::IndexerGrabClient {
                id: "fixture-client".into(),
                name: "Fixture".into(),
                category: Some("routed".into()),
                mapped: true,
            }])
        }
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.requests.lock().await.push(request.clone());
            self.inner.submit_download(request).await
        }
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listed = Arc::new(ListedClient {
        inner: app.services.integrations.download_client.clone(),
        requests: requests.clone(),
    });
    let app = app.with_test_overrides(|services| {
        services
            .with_download_client(listed)
            .with_download_submissions(Arc::new(TrackingDownloadSubmissionRepo::default()))
    });

    let clients = app
        .indexer_grab_clients(&manager, &start.id, &download_url, Some(&title.id))
        .await
        .expect("assigned grab clients need only ManageTitles");
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].id, "fixture-client");
    let denied = app
        .indexer_grab_clients(&manager, &start.id, &download_url, None)
        .await
        .expect_err("title-less grab clients need ManageSystemSettings");
    assert!(matches!(denied, AppError::Unauthorized(_)), "{denied:?}");

    // The grab the listing led to completes under the same gate.
    let outcome = app
        .queue_indexer_search_assignment(
            &manager,
            &title.id,
            release
                .candidate_token
                .as_deref()
                .expect("a title search result carries a candidate token"),
            release.size_bytes,
            SubmissionConflictPolicy::from_replace_flag(false),
            false,
            crate::IndexerGrabSelection {
                client_id: "fixture-client".into(),
                category: None,
            },
        )
        .await
        .expect("an assigned grab needs only ManageTitles");
    assert!(matches!(outcome, QueueDownloadOutcome::Queued(_)));
    assert_eq!(requests.lock().await.len(), 1);
}

// ── Download to browser ───────────────────────────────────────

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
            stream_id: None,
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
    assert_eq!(
        urls.len(),
        expected,
        "every seeded release should survive dedupe"
    );
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
    assert_eq!(
        artifact_file_name("  Some\t Release \n", ".nzb"),
        "Some Release.nzb"
    );
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
async fn browser_downloads_count_against_the_indexer_even_when_a_later_member_fails() {
    let (app, user, search_id, urls) = browser_download_fixture(vec![
        nzb_release("Paperman.2012.1080p.WEB-DL", "g1"),
        nzb_release("Paperman.2012.2160p.WEB-DL", "g2"),
    ])
    .await;
    let stats = Arc::new(RecordingIndexerStatsTracker::default());
    // Only the first release resolves; the second makes the batch fail.
    let artifacts = HashMap::from([(urls[0].clone(), nzb_artifact("first"))]);
    let app = app.with_test_overrides(|services| {
        services
            .with_download_client(Arc::new(ArtifactDownloadClient { artifacts }))
            .with_indexer_stats(stats.clone())
    });

    let error = app
        .download_interactive_search_artifacts(&user, &targets(&search_id, &urls))
        .await
        .expect_err("the second fetch fails the batch");
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");

    // The indexer served the first file, so that grab happened and is counted
    // under the configured indexer name, exactly as the unlinked-queue path
    // counts one.
    let grabs = stats.grabs.lock().expect("grab log mutex").clone();
    assert_eq!(
        grabs,
        vec![("idx-a".to_string(), "Synthetic newznab".to_string())],
        "each served artifact counts once: {grabs:?}"
    );
}

#[tokio::test]
async fn browser_download_rejects_an_oversized_aggregate_without_grab_history() {
    let (app, user, search_id, urls) = browser_download_fixture(vec![
        nzb_release("First.Release", "large-1"),
        nzb_release("Second.Release", "large-2"),
        nzb_release("Third.Release", "large-3"),
    ])
    .await;
    let artifacts = urls
        .iter()
        .map(|url| {
            (
                url.clone(),
                ResolvedDownloadArtifact::Nzb {
                    bytes: vec![b'x'; 24 * 1024 * 1024],
                    file_name: None,
                    content_type: None,
                },
            )
        })
        .collect();
    let app = with_artifacts(&app, artifacts);
    let error = app
        .download_interactive_search_artifacts(&user, &targets(&search_id, &urls))
        .await
        .expect_err("aggregate payload is bounded");
    assert!(matches!(error, AppError::Validation(message) if message.contains("64 MiB")));
    assert!(grabbed_release_titles(&app).await.is_empty());
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
async fn browser_download_history_failure_is_atomic_and_retryable() {
    let (app, user, search_id, urls) = browser_download_fixture(vec![
        nzb_release("First.2012.1080p.WEB-DL", "g1"),
        nzb_release("Second.2012.1080p.WEB-DL", "g2"),
    ])
    .await;
    let events = Arc::new(MockDomainEventRepo::default());
    events.fail_append_many.store(true, Ordering::SeqCst);
    let app = app.with_test_overrides(|builder| builder.with_domain_events(events.clone()));
    let app = with_artifacts(
        &app,
        HashMap::from([
            (urls[0].clone(), nzb_artifact("first")),
            (urls[1].clone(), nzb_artifact("second")),
        ]),
    );
    app.download_interactive_search_artifacts(&user, &targets(&search_id, &urls))
        .await
        .expect_err("history failure must withhold the bundle");
    assert!(grabbed_release_titles(&app).await.is_empty());
    events.fail_append_many.store(false, Ordering::SeqCst);
    app.download_interactive_search_artifacts(&user, &targets(&search_id, &urls))
        .await
        .expect("retry bundle");
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
    assert!(error.to_string().contains("magnet link"), "{error}");
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
    assert!(
        matches!(oversized, AppError::Validation(_)),
        "{oversized:?}"
    );

    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::default());
    let denied = app
        .download_interactive_search_artifacts(&viewer, &targets(&search_id, &urls[..1]))
        .await
        .expect_err("permission gate");
    assert!(matches!(denied, AppError::Unauthorized(_)), "{denied:?}");
}
