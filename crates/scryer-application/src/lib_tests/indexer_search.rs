//! Unit coverage for the title-less indexer-search job (spec 0002, WP1).

use super::*;

use crate::catalog::indexer_search::{IndexerSearchIndexerStatus, IndexerSearchState};
use crate::{IndexerSearchKind, IndexerSearchRequest, IndexerSearchSnapshot};

/// One recorded dispatch to the search port.
#[derive(Clone, Debug)]
struct RecordedIndexerSearchCall {
    query: String,
    facet: Option<String>,
    newznab_categories: Option<Vec<String>>,
    /// Indexers the restriction plan left enabled — exactly one per task.
    enabled_indexers: Vec<String>,
}

/// Test double that answers per indexer, can be told to fail some of them, and
/// records the envelope each call carried.
#[derive(Clone, Default)]
struct ScriptedIndexerClient {
    calls: Arc<Mutex<Vec<RecordedIndexerSearchCall>>>,
    releases: Arc<Mutex<HashMap<String, Vec<IndexerSearchResult>>>>,
    failing: Arc<Mutex<HashSet<String>>>,
}

impl ScriptedIndexerClient {
    async fn with_releases(self, indexer_id: &str, releases: Vec<IndexerSearchResult>) -> Self {
        self.releases
            .lock()
            .await
            .insert(indexer_id.to_string(), releases);
        self
    }

    async fn failing_indexer(self, indexer_id: &str) -> Self {
        self.failing.lock().await.insert(indexer_id.to_string());
        self
    }

    async fn heal(&self, indexer_id: &str) {
        self.failing.lock().await.remove(indexer_id);
    }

    async fn calls(&self) -> Vec<RecordedIndexerSearchCall> {
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
        self.calls.lock().await.push(RecordedIndexerSearchCall {
            query,
            facet,
            newznab_categories,
            enabled_indexers: enabled_indexers.clone(),
        });

        let Some(indexer_id) = enabled_indexers.first().cloned() else {
            return Err(AppError::Repository("no indexer routed".to_string()));
        };
        if self.failing.lock().await.contains(&indexer_id) {
            return Err(AppError::Repository(
                "indexer returned http 503".to_string(),
            ));
        }
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
        response_attributes: crate::IndexerResponseAttributes {
            categories: vec!["2040".into(), "Movies/HD".into()],
            ..Default::default()
        },
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

fn request(query: &str, kind: IndexerSearchKind) -> IndexerSearchRequest {
    IndexerSearchRequest {
        query: query.to_string(),
        kind,
        ..IndexerSearchRequest::default()
    }
}

fn bootstrap_indexer_search(
    client: ScriptedIndexerClient,
    configs: Vec<IndexerConfig>,
) -> (AppUseCase, User) {
    bootstrap_with_search_settings_indexer_and_configs(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(client),
        configs,
    )
}

/// Poll until the job leaves `Running`, then return the final snapshot.
async fn await_completion(app: &AppUseCase, user: &User, job_id: &str) -> IndexerSearchSnapshot {
    for _ in 0..200 {
        let snapshot = app
            .indexer_search(user, job_id)
            .await
            .expect("poll indexer search")
            .expect("job present");
        if snapshot.state != IndexerSearchState::Running {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("indexer search never reached a terminal state");
}

fn indexer_view<'a>(
    snapshot: &'a IndexerSearchSnapshot,
    indexer_id: &str,
) -> &'a crate::IndexerSearchIndexerView {
    snapshot
        .indexers
        .iter()
        .find(|view| view.indexer_id == indexer_id)
        .unwrap_or_else(|| panic!("indexer {indexer_id} missing from snapshot: {snapshot:?}"))
}

// ── T101: kind mapping and validation ───────────────────────────────────────

#[tokio::test]
async fn movie_kind_sends_the_movie_facet_and_its_default_categories() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("Paperman.2012.1080p.WEB-DL", "g1")])
        .await;
    let (app, user) = bootstrap_indexer_search(
        client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_indexer_search(&user, request("paperman", IndexerSearchKind::Movie))
        .await
        .expect("start");
    await_completion(&app, &user, &start.id).await;

    let calls = client.calls().await;
    assert_eq!(calls.len(), 1, "one call per indexer: {calls:?}");
    assert_eq!(calls[0].facet.as_deref(), Some("movie"));
    assert_eq!(calls[0].query, "paperman");
    let categories = calls[0]
        .newznab_categories
        .clone()
        .expect("movie kind carries default categories");
    assert!(
        !categories.is_empty(),
        "movie defaults should be non-empty: {categories:?}"
    );
    assert_eq!(
        start.request.categories.as_deref(),
        Some(categories.as_slice()),
        "the snapshot echoes the effective categories"
    );
}

#[tokio::test]
async fn raw_kind_sends_a_text_query_with_no_facet_and_no_categories() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("Some.Odd.Pack.2024", "g1")])
        .await;
    let (app, user) = bootstrap_indexer_search(
        client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_indexer_search(&user, request("  some odd pack  ", IndexerSearchKind::Raw))
        .await
        .expect("start");
    await_completion(&app, &user, &start.id).await;

    let calls = client.calls().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].facet, None, "raw kind must not send a facet");
    assert_eq!(calls[0].newznab_categories, None);
    assert_eq!(calls[0].query, "some odd pack", "query is trimmed");
}

#[tokio::test]
async fn invalid_requests_are_refused_before_dispatch() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_indexer_search(
        client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    for (label, request) in [
        ("empty query", request("   ", IndexerSearchKind::Raw)),
        (
            "limit above cap",
            IndexerSearchRequest {
                per_indexer_limit: Some(251),
                ..request("x", IndexerSearchKind::Raw)
            },
        ),
        (
            "inverted size window",
            IndexerSearchRequest {
                min_size_bytes: Some(10),
                max_size_bytes: Some(5),
                ..request("x", IndexerSearchKind::Raw)
            },
        ),
        (
            "zero age",
            IndexerSearchRequest {
                max_age_days: Some(0),
                ..request("x", IndexerSearchKind::Raw)
            },
        ),
        (
            "unknown indexer",
            IndexerSearchRequest {
                indexer_ids: Some(vec!["idx-nope".into()]),
                ..request("x", IndexerSearchKind::Raw)
            },
        ),
    ] {
        let error = app
            .start_indexer_search(&user, request)
            .await
            .expect_err(label);
        assert!(
            matches!(error, AppError::Validation(_)),
            "{label} should be a validation error, got {error:?}"
        );
    }
    assert!(client.calls().await.is_empty());
}

// ── T103: dispatch resolution ───────────────────────────────────────────────

#[tokio::test]
async fn dispatch_honours_requested_ids_and_skips_ineligible_indexers() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![nzb_release("A.2024.1080p.WEB-DL", "a1")])
        .await
        .with_releases("idx-b", vec![nzb_release("B.2024.1080p.WEB-DL", "b1")])
        .await;

    let mut backoff = synthetic_direct_nab_indexer_config("idx-backoff", "newznab");
    backoff.disabled_until = Some(Utc::now() + chrono::Duration::hours(1));
    let mut no_interactive = synthetic_direct_nab_indexer_config("idx-auto-only", "newznab");
    no_interactive.enable_interactive_search = false;
    let mut disabled = synthetic_direct_nab_indexer_config("idx-off", "newznab");
    disabled.is_enabled = false;

    let (app, user) = bootstrap_indexer_search(
        client.clone(),
        vec![
            synthetic_direct_nab_indexer_config("idx-a", "newznab"),
            synthetic_direct_nab_indexer_config("idx-b", "newznab"),
            backoff,
            no_interactive,
            disabled,
        ],
    );

    // Every eligible indexer when none are named.
    let all = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect("start");
    let all = await_completion(&app, &user, &all.id).await;
    let listed = all
        .indexers
        .iter()
        .map(|view| view.indexer_id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        listed,
        HashSet::from(["idx-a", "idx-b", "idx-backoff"]),
        "disabled and auto-only indexers are omitted entirely: {listed:?}"
    );
    assert_eq!(
        indexer_view(&all, "idx-backoff").status,
        IndexerSearchIndexerStatus::Skipped
    );
    assert_eq!(
        indexer_view(&all, "idx-backoff").failure_reason.as_deref(),
        Some("temporarily disabled")
    );
    assert_eq!(all.totals.indexers_queried, 2);

    // A named subset restricts the fan-out.
    let restricted = app
        .start_indexer_search(
            &user,
            IndexerSearchRequest {
                indexer_ids: Some(vec!["idx-b".into()]),
                ..request("q", IndexerSearchKind::Raw)
            },
        )
        .await
        .expect("start restricted");
    let restricted = await_completion(&app, &user, &restricted.id).await;
    assert_eq!(restricted.indexers.len(), 1);
    assert_eq!(restricted.indexers[0].indexer_id, "idx-b");
    assert!(
        restricted
            .releases
            .iter()
            .all(|release| release.indexer_id == "idx-b"),
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

// ── T105: merge, facets, stable ids ─────────────────────────────────────────

#[tokio::test]
async fn merge_derives_facets_and_stable_release_ids() {
    let mut torrent = nzb_release("Show.S01.2160p.BluRay.REMUX.Atmos.DV-GRP", "t1");
    torrent.source_kind = Some(DownloadSourceKind::TorrentFile);
    torrent
        .extra
        .insert("seeders".into(), serde_json::json!(12));
    torrent
        .extra
        .insert("leechers".into(), serde_json::json!(3));
    torrent
        .extra
        .insert("indexer_flags".into(), serde_json::json!(["freeleech"]));

    let client = ScriptedIndexerClient::default()
        .with_releases(
            "idx-a",
            vec![nzb_release("Movie.2024.1080p.WEB-DL-GRP", "m1"), torrent],
        )
        .await;
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;
    assert_eq!(done.state, IndexerSearchState::Completed);
    assert_eq!(done.releases.len(), 2, "{done:?}");
    assert_eq!(done.totals.matched, 2);
    assert!(!done.totals.truncated);

    let movie = done
        .releases
        .iter()
        .find(|release| release.title.starts_with("Movie"))
        .expect("movie release");
    assert_eq!(movie.protocol, "usenet");
    assert_eq!(movie.facet_values.resolution, "1080p");
    assert_eq!(movie.facet_values.source, "WEB-DL");
    assert_eq!(movie.file_summary, "1 NZB");
    assert_eq!(movie.category_label.as_deref(), Some("Movies/HD (2040)"));
    assert_eq!(movie.grabs, Some(42));
    assert_eq!(movie.release_group.as_deref(), Some("GRP"));

    let pack = done
        .releases
        .iter()
        .find(|release| release.title.starts_with("Show"))
        .expect("torrent release");
    assert_eq!(pack.protocol, "torrent");
    assert_eq!(pack.facet_values.resolution, "2160p");
    assert_eq!(pack.facet_values.source, "REMUX");
    assert_eq!(pack.file_summary, "1 torrent");
    assert_eq!(pack.seeders, Some(12));
    assert_eq!(pack.leechers, Some(3));
    assert!(pack.facet_values.flags.contains(&"Freeleech".to_string()));
    assert!(pack.facet_values.audio_hdr.contains(&"Atmos".to_string()));
    assert!(pack.is_season_pack, "S01 with no episode is a season pack");
    assert!(
        pack.flags.starts_with(&["2160p".to_string(), "REMUX".to_string()]),
        "badge list leads with resolution and source: {:?}",
        pack.flags
    );

    // Ids are stable across two runs of the same query and unique within one.
    let second = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect("second start");
    let second = await_completion(&app, &user, &second.id).await;
    let first_ids = done
        .releases
        .iter()
        .map(|release| release.id.clone())
        .collect::<HashSet<_>>();
    let second_ids = second
        .releases
        .iter()
        .map(|release| release.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(first_ids.len(), 2, "release ids are unique");
    assert_eq!(first_ids, second_ids, "release ids are stable");

    let protocol_facet = done
        .facets
        .iter()
        .find(|facet| facet.key == "protocol")
        .expect("protocol facet");
    assert_eq!(protocol_facet.items.len(), 2, "{protocol_facet:?}");
    assert!(
        protocol_facet
            .items
            .iter()
            .all(|item| item.count == 1),
        "{protocol_facet:?}"
    );
}

// ── T105: advanced limits ───────────────────────────────────────────────────

#[tokio::test]
async fn advanced_limits_filter_the_merged_set() {
    let mut small = nzb_release("Small.2024.1080p.WEB-DL", "s1");
    small.size_bytes = Some(100);
    let mut large = nzb_release("Large.2024.1080p.WEB-DL", "l1");
    large.size_bytes = Some(10_000_000_000);
    let mut old = nzb_release("Old.2024.1080p.WEB-DL", "o1");
    old.size_bytes = Some(1_000);
    old.published_at = Some((Utc::now() - chrono::Duration::days(400)).to_rfc3339());
    // *nab plugins pass `pubDate` through as RFC 2822; the age filter must read
    // both encodings or it silently never fires for them.
    let mut old_rfc2822 = nzb_release("Ancient.2024.1080p.WEB-DL", "o2");
    old_rfc2822.size_bytes = Some(1_000);
    old_rfc2822.published_at = Some((Utc::now() - chrono::Duration::days(400)).to_rfc2822());
    let mut lonely = nzb_release("Lonely.2024.1080p.WEB-DL", "p1");
    lonely.size_bytes = Some(1_000);
    lonely.extra.insert("seeders".into(), serde_json::json!(1));

    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![small, large, old, old_rfc2822, lonely])
        .await;
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_indexer_search(
            &user,
            IndexerSearchRequest {
                min_size_bytes: Some(500),
                max_size_bytes: Some(5_000),
                max_age_days: Some(30),
                min_seeders: Some(5),
                ..request("q", IndexerSearchKind::Raw)
            },
        )
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;

    assert!(
        done.releases.is_empty(),
        "every release trips one of the limits: {:?}",
        done.releases
            .iter()
            .map(|release| release.title.as_str())
            .collect::<Vec<_>>()
    );
    // The indexer still reported every row before filtering.
    assert_eq!(indexer_view(&done, "idx-a").result_count, 5);
    assert_eq!(done.totals.matched, 0);
}

#[tokio::test]
async fn per_indexer_limit_truncates_the_batch() {
    let releases = (0..5)
        .map(|index| nzb_release(&format!("Title.{index}.2024.1080p.WEB-DL"), &format!("g{index}")))
        .collect::<Vec<_>>();
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", releases)
        .await;
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_indexer_search(
            &user,
            IndexerSearchRequest {
                per_indexer_limit: Some(2),
                ..request("q", IndexerSearchKind::Raw)
            },
        )
        .await
        .expect("start");
    let done = await_completion(&app, &user, &start.id).await;
    assert_eq!(done.releases.len(), 2, "{done:?}");
    assert_eq!(indexer_view(&done, "idx-a").result_count, 2);
}

// ── T106: context-free rejections ───────────────────────────────────────────

#[tokio::test]
async fn faceted_kinds_reject_a_banned_source_while_raw_does_not() {
    let telesync = nzb_release("Movie.2024.TELESYNC.1080p-GRP", "ts1");
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-a", vec![telesync])
        .await;
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let movie = app
        .start_indexer_search(&user, request("movie", IndexerSearchKind::Movie))
        .await
        .expect("start movie");
    let movie = await_completion(&app, &user, &movie.id).await;
    let rejected = movie.releases.first().expect("one release");
    assert!(
        rejected
            .rejections
            .iter()
            .any(|reason| reason.starts_with("banned source")),
        "profile block should surface as a rejection: {:?}",
        rejected.rejections
    );

    // Raw has no facet, therefore no default profile to judge against (D6).
    let raw = app
        .start_indexer_search(&user, request("movie", IndexerSearchKind::Raw))
        .await
        .expect("start raw");
    let raw = await_completion(&app, &user, &raw.id).await;
    assert!(
        raw.releases
            .first()
            .expect("one release")
            .rejections
            .is_empty(),
        "raw kind carries no profile rejections: {:?}",
        raw.releases
    );
}

// ── T107: retry, cancel, actor scoping, TTL, cap ────────────────────────────

#[tokio::test]
async fn retry_reruns_only_failed_indexers_and_does_not_duplicate_results() {
    let client = ScriptedIndexerClient::default()
        .with_releases("idx-ok", vec![nzb_release("Ok.2024.1080p.WEB-DL", "ok1")])
        .await
        .with_releases("idx-bad", vec![nzb_release("Bad.2024.1080p.WEB-DL", "bad1")])
        .await
        .failing_indexer("idx-bad")
        .await;
    let (app, user) = bootstrap_indexer_search(
        client.clone(),
        vec![
            synthetic_direct_nab_indexer_config("idx-ok", "newznab"),
            synthetic_direct_nab_indexer_config("idx-bad", "newznab"),
        ],
    );

    let start = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect("start");
    let failed = await_completion(&app, &user, &start.id).await;
    assert_eq!(
        indexer_view(&failed, "idx-ok").status,
        IndexerSearchIndexerStatus::Ok
    );
    let bad = indexer_view(&failed, "idx-bad");
    assert_eq!(bad.status, IndexerSearchIndexerStatus::Failed);
    assert_eq!(
        bad.failure_reason.as_deref(),
        Some("http 503"),
        "failure reason is a short, stable word"
    );
    assert_eq!(failed.releases.len(), 1);

    client.heal("idx-bad").await;
    let calls_before = client.calls().await.len();
    app.retry_indexer_search(&user, &start.id)
        .await
        .expect("retry");
    let healed = await_completion(&app, &user, &start.id).await;

    let calls_after = client.calls().await;
    assert_eq!(
        calls_after.len() - calls_before,
        1,
        "retry re-dispatches only the failed indexer: {calls_after:?}"
    );
    assert_eq!(
        calls_after.last().expect("call").enabled_indexers,
        vec!["idx-bad".to_string()]
    );
    assert_eq!(
        indexer_view(&healed, "idx-bad").status,
        IndexerSearchIndexerStatus::Ok
    );
    assert_eq!(healed.releases.len(), 2, "{healed:?}");
    assert_eq!(
        (healed.totals.indexers_queried, healed.totals.indexers_responded),
        (2, 2),
        "totals cover the whole job, not just the retried indexer: {healed:?}"
    );
    assert_eq!(
        healed
            .releases
            .iter()
            .map(|release| release.id.clone())
            .collect::<HashSet<_>>()
            .len(),
        2,
        "merge dedupes on release id"
    );
}

#[tokio::test]
async fn retry_refuses_a_running_job_and_an_unknown_id() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let missing = app
        .retry_indexer_search(&user, "no-such-job")
        .await
        .expect_err("unknown id");
    assert!(matches!(missing, AppError::NotFound(_)), "{missing:?}");

    // Seeded rather than started, so the "still running" guard is exercised
    // without racing the runner to a terminal state.
    {
        let mut registry = app.runtime.acquisition.indexer_searches.lock().await;
        registry.insert(
            "still-running".to_string(),
            crate::catalog::indexer_search::IndexerSearchJobEntry {
                snapshot: IndexerSearchSnapshot {
                    id: "still-running".to_string(),
                    state: IndexerSearchState::Running,
                    request: request("q", IndexerSearchKind::Raw),
                    totals: Default::default(),
                    indexers: Vec::new(),
                    facets: Vec::new(),
                    releases: Vec::new(),
                    started_at: Utc::now(),
                    completed_at: None,
                },
                actor_id: user.id.clone(),
                cancel: tokio_util::sync::CancellationToken::new(),
            },
        );
    }
    let running = app
        .retry_indexer_search(&user, "still-running")
        .await
        .expect_err("running job");
    assert!(
        matches!(&running, AppError::Validation(message) if message.contains("still running")),
        "{running:?}"
    );
}

#[tokio::test]
async fn cancel_marks_the_job_cancelled_once() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect("start");
    let accepted = app
        .cancel_indexer_search(&user, &start.id)
        .await
        .expect("cancel");
    let snapshot = app
        .indexer_search(&user, &start.id)
        .await
        .expect("poll")
        .expect("present");
    if accepted {
        assert_eq!(snapshot.state, IndexerSearchState::Cancelled);
        assert!(snapshot.completed_at.is_some());
    }
    assert!(
        !app.cancel_indexer_search(&user, &start.id)
            .await
            .expect("second cancel"),
        "a terminal job never accepts a second cancel"
    );
}

#[tokio::test]
async fn jobs_are_scoped_to_their_actor() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let mut other = test_admin_user();
    other.id = "other-actor".to_string();

    let start = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect("start");
    assert!(
        app.indexer_search(&other, &start.id)
            .await
            .expect("poll")
            .is_none(),
        "another actor must not see the job"
    );
    assert!(
        !app.cancel_indexer_search(&other, &start.id)
            .await
            .expect("cancel"),
        "another actor must not cancel the job"
    );
    assert!(
        matches!(
            app.retry_indexer_search(&other, &start.id).await,
            Err(AppError::NotFound(_))
        ),
        "another actor must not retry the job"
    );
}

#[tokio::test]
async fn a_completed_job_is_evicted_once_its_ttl_expires() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    let start = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect("start");
    await_completion(&app, &user, &start.id).await;

    // Age the completion past the 30-minute terminal TTL.
    {
        let mut registry = app.runtime.acquisition.indexer_searches.lock().await;
        let entry = registry.get_mut(&start.id).expect("entry");
        entry.snapshot.completed_at = Some(Utc::now() - chrono::Duration::minutes(31));
    }
    assert!(
        app.indexer_search(&user, &start.id)
            .await
            .expect("poll")
            .is_none(),
        "a terminal job past its TTL is evicted on access"
    );
}

#[tokio::test]
async fn the_per_actor_running_cap_is_enforced() {
    let client = ScriptedIndexerClient::default();
    let (app, user) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );

    // Fill the registry with running jobs the runner cannot finish.
    {
        let mut registry = app.runtime.acquisition.indexer_searches.lock().await;
        for index in 0..8 {
            let id = format!("running-{index}");
            registry.insert(
                id.clone(),
                crate::catalog::indexer_search::IndexerSearchJobEntry {
                    snapshot: IndexerSearchSnapshot {
                        id,
                        state: IndexerSearchState::Running,
                        request: request("q", IndexerSearchKind::Raw),
                        totals: Default::default(),
                        indexers: Vec::new(),
                        facets: Vec::new(),
                        releases: Vec::new(),
                        started_at: Utc::now(),
                        completed_at: None,
                    },
                    actor_id: user.id.clone(),
                    cancel: tokio_util::sync::CancellationToken::new(),
                },
            );
        }
    }

    let error = app
        .start_indexer_search(&user, request("q", IndexerSearchKind::Raw))
        .await
        .expect_err("cap");
    assert!(
        matches!(&error, AppError::Validation(message) if message.contains("too many concurrent")),
        "{error:?}"
    );
}

#[tokio::test]
async fn starting_a_search_requires_manage_system_settings() {
    let client = ScriptedIndexerClient::default();
    let (app, _) = bootstrap_indexer_search(
        client,
        vec![synthetic_direct_nab_indexer_config("idx-a", "newznab")],
    );
    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::default());

    let error = app
        .start_indexer_search(&viewer, request("q", IndexerSearchKind::Raw))
        .await
        .expect_err("permission gate");
    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
}
