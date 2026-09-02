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
