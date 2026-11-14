use super::*;
use crate::acquisition_release_search::ResolvedReleaseSearchSubject;
use crate::ports::IndexerSearchLearningContext;
use crate::settings::keys::default_indexer_routing_categories_for_scope;
use scryer_domain::{MediaFacet, TaggedAlias};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Preparation belongs to one walk; scope-specific inputs remain on each call.
#[derive(Default)]
pub(crate) struct WalkSearchContext {
    prepared: Option<PreparedReleaseScoringInputs>,
    search_identity: Option<(String, MediaFacet)>,
    pub(crate) reads: Option<Arc<crate::acquisition::title_reads::TitleCatalogReads>>,
    pub(crate) rss_yield: Option<CancellationToken>,
    pub(crate) interrupted: bool,
}

impl WalkSearchContext {
    fn should_yield(&mut self) -> bool {
        let yielded = self
            .rss_yield
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled);
        self.interrupted |= yielded;
        yielded
    }
}

pub(crate) struct PreparedReleaseScoringInputs {
    quality_profile: QualityProfile,
    routing_priorities: Vec<(String, i64)>,
    blocklist: TitleReleaseBlocklistSignatures,
    has_usenet_client: bool,
    has_torrent_client: bool,
    has_http_client: bool,
    has_unavailable_client_plugin: bool,
    preferred_source_kind: String,
    enabled_protocols: Option<(bool, bool)>,
    title: Title,
    canonical_context: crate::quality::canonical_context::ResolvedScoringContext,
    catalog_episodes: Arc<Vec<Episode>>,
    catalog_collections: Arc<Vec<Collection>>,
    /// The title's community numbering layout, loaded once per scoring batch.
    /// `None` for every non-anime title and for anime SMG has no bridge for,
    /// which is what keeps their numbering handling byte-for-byte as it was.
    anime_numbering_bridge: Option<scryer_domain::AnimeNumberingBridge>,
    primary_episode_ids: Option<Option<HashSet<String>>>,
    indexer_priority_by_name: HashMap<String, i64>,
    now: chrono::DateTime<chrono::Utc>,
    /// One mutable evaluator bound to this prepared scoring snapshot. Search
    /// page processing is sequential, so it can safely reuse Regorus's input
    /// buffers for every candidate without crossing task boundaries.
    rule_evaluation_batch: crate::canonical_scoring::RuleEvaluationBatch,
}

/// Releases one indexer returned that scoring removed because no enabled
/// download client accepts their source kind, counted by the reason code for
/// the missing client family.
pub type UnroutableReleaseCounts = std::collections::BTreeMap<&'static str, usize>;

/// What one scoring pass produced, plus the releases it had to set aside
/// because nothing could download them.
pub(crate) struct ScoredReleaseBatch {
    pub results: Vec<IndexerSearchResult>,
    /// Keyed by indexer id; empty when every release had an accepting client.
    pub unroutable_by_indexer: HashMap<String, UnroutableReleaseCounts>,
    /// Releases an automatic search set aside because their indexer has used
    /// its whole grab allowance, keyed by indexer id.
    pub quota_exhausted_by_indexer: HashMap<String, usize>,
}

/// Results one blocking task parses or scores before the walk yields the
/// runtime back. Small enough that one chunk stays well under a millisecond
/// of work on a large title, large enough that the hand-off cost is noise.
const SCORING_CHUNK_RESULTS: usize = 32;

/// Which search a scoring pass serves, and so how many results it scores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScoringLane {
    /// An operator's search, RSS, or anything else that is not the background
    /// walk: every result is scored.
    Full,
    /// A background walk search. `cap` bounds the results it scores; `None`
    /// for a hot scope, which is scored in full.
    Walk { cap: Option<usize> },
}

impl ScoringLane {
    fn evaluation_cap(self) -> Option<usize> {
        match self {
            Self::Full => None,
            Self::Walk { cap } => cap,
        }
    }

    fn record_evaluated(self, duplicates: usize, blocklisted: usize, over_cap: usize, kept: usize) {
        if self == Self::Full {
            return;
        }
        for (outcome, count) in [
            ("duplicate", duplicates),
            ("blocklisted", blocklisted),
            ("over_cap", over_cap),
            ("evaluated", kept),
        ] {
            if count > 0 {
                metrics::counter!(
                    crate::acquisition::walk_metrics::EVALUATED_RESULTS_TOTAL,
                    "outcome" => outcome
                )
                .increment(count as u64);
            }
        }
    }
}

/// A stored setting value as the string readers return it: `None` for empty,
/// `null` or the inherit marker, the inner text of a JSON string, and the
/// trimmed raw JSON for anything else.
pub(crate) fn normalize_setting_string_value(raw_value: &str) -> Option<String> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() || trimmed == INHERIT_QUALITY_PROFILE_VALUE {
        return None;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
        return Some(trimmed.to_string());
    };
    match parsed {
        Value::Null => None,
        Value::String(value) => {
            let normalized = value.trim();
            if normalized.is_empty() || normalized == INHERIT_QUALITY_PROFILE_VALUE {
                None
            } else {
                Some(normalized.to_string())
            }
        }
        _ => Some(trimmed.to_string()),
    }
}

/// Keeps at most `cap` results, taken a round at a time across indexers in
/// the order each indexer returned them, so one indexer's long feed cannot
/// crowd out another's first page. Kept results stay in their original order.
/// Returns the kept results and how many were dropped.
fn cap_evaluated_results(
    results: Vec<IndexerSearchResult>,
    cap: Option<usize>,
) -> (Vec<IndexerSearchResult>, usize) {
    let Some(cap) = cap.filter(|cap| results.len() > *cap) else {
        return (results, 0);
    };
    let mut keep = vec![false; results.len()];
    {
        let mut slots: HashMap<Option<&str>, usize> = HashMap::new();
        let mut positions: Vec<Vec<usize>> = Vec::new();
        for (index, result) in results.iter().enumerate() {
            let slot = *slots
                .entry(result.indexer_id.as_deref())
                .or_insert_with(|| {
                    positions.push(Vec::new());
                    positions.len() - 1
                });
            positions[slot].push(index);
        }
        let mut kept = 0usize;
        let mut round = 0usize;
        'rounds: while kept < cap {
            let mut any = false;
            for indexer in &positions {
                if let Some(&index) = indexer.get(round) {
                    keep[index] = true;
                    kept += 1;
                    any = true;
                    if kept == cap {
                        break 'rounds;
                    }
                }
            }
            if !any {
                break;
            }
            round += 1;
        }
    }
    let total = results.len();
    let kept = results
        .into_iter()
        .zip(keep)
        .filter_map(|(result, keep)| keep.then_some(result))
        .collect::<Vec<_>>();
    let dropped = total - kept.len();
    (kept, dropped)
}

/// Runs `step` over `items` on the blocking pool, [`SCORING_CHUNK_RESULTS`]
/// at a time, yielding to the runtime between chunks. The prepared scoring
/// snapshot moves into each chunk and back, so every item sees the same
/// snapshot and its one rule evaluator, exactly as a sequential loop would.
/// If the caller is dropped mid-chunk the snapshot is gone and the next
/// scoring pass prepares a fresh one.
async fn score_in_blocking_chunks<I, O, F>(
    prepared: &mut Option<PreparedReleaseScoringInputs>,
    items: Vec<I>,
    step: F,
) -> AppResult<Vec<O>>
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(&mut PreparedReleaseScoringInputs, I) -> Option<O> + Send + Sync + 'static,
{
    let step = Arc::new(step);
    let mut output = Vec::with_capacity(items.len());
    let mut items = items.into_iter().peekable();
    while items.peek().is_some() {
        let chunk = items
            .by_ref()
            .take(SCORING_CHUNK_RESULTS)
            .collect::<Vec<_>>();
        let mut inputs = prepared.take().expect("release scoring inputs initialized");
        let step = Arc::clone(&step);
        let joined = tokio::task::spawn_blocking(move || {
            let produced = chunk
                .into_iter()
                .filter_map(|item| step(&mut inputs, item))
                .collect::<Vec<_>>();
            (inputs, produced)
        })
        .await;
        match joined {
            Ok((inputs, produced)) => {
                *prepared = Some(inputs);
                output.extend(produced);
            }
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(_) => return Err(AppError::canceled("release scoring was canceled")),
        }
        tokio::task::yield_now().await;
    }
    Ok(output)
}

/// A search result after parsing and numbering translation, before scoring.
struct ParsedCandidate {
    result: IndexerSearchResult,
    metadata: ParsedReleaseMetadata,
    numbering_ambiguous: bool,
    is_series_pack: bool,
}

/// A scored result with the rank it sorts by.
struct EvaluatedCandidate {
    key: String,
    rank: crate::acquisition::scoring::SearchRank,
    result: IndexerSearchResult,
}

/// What one scoring pass is looking for, shared by every candidate in it.
#[derive(Clone, Copy)]
struct CandidateRequest {
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    multi_episode_season: Option<u32>,
    runtime_minutes: Option<i32>,
    /// Index into the prepared catalog episodes of the episode asked for.
    requested_episode_index: Option<usize>,
}

fn parse_candidate(
    inputs: &PreparedReleaseScoringInputs,
    parse_context: &ReleaseParseContext,
    title_id: &str,
    result: IndexerSearchResult,
) -> ParsedCandidate {
    let parsed_release_metadata = parse_release_metadata_for_target(&result.title, parse_context);
    let mut scored_release_metadata = parsed_release_metadata;

    // Anime released in community (per-cour) numbering is rewritten
    // into the catalog's TVDB numbering before anything reads it, so
    // coverage, the numbering veto and the rank all see one numbering.
    // A release nobody can place is refused outright rather than
    // grabbed on a coin toss.
    let numbering = crate::anime_numbering::translate_release_numbering(
        inputs.anime_numbering_bridge.as_ref(),
        &inputs.title,
        &inputs.catalog_episodes,
        &mut scored_release_metadata,
        result
            .published_at
            .as_deref()
            .and_then(crate::quality_profile::parse_published_at)
            .map(|published| published.date_naive()),
    );
    let numbering_ambiguous = numbering.is_ambiguous();
    if let Some(summary) = numbering.ambiguity_summary() {
        tracing::info!(
            title_id,
            release = result.title.as_str(),
            readings = summary.as_str(),
            "release numbering has several equally-good readings; not auto-grabbing"
        );
    }

    let is_series_pack = scored_release_metadata
        .episode
        .as_ref()
        .is_some_and(|episode| episode.is_series_pack);
    ParsedCandidate {
        result,
        metadata: scored_release_metadata,
        numbering_ambiguous,
        is_series_pack,
    }
}

fn evaluate_candidate(
    inputs: &mut PreparedReleaseScoringInputs,
    request: &CandidateRequest,
    candidate: ParsedCandidate,
) -> Option<EvaluatedCandidate> {
    let CandidateRequest {
        season,
        episode,
        absolute_episode,
        multi_episode_season,
        runtime_minutes,
        requested_episode_index,
    } = *request;
    let ParsedCandidate {
        result,
        metadata: scored_release_metadata,
        numbering_ambiguous,
        is_series_pack,
    } = candidate;
    let key = release_search_key(&result);
    let catalog_episodes = Arc::clone(&inputs.catalog_episodes);
    let requested_episode = requested_episode_index.map(|index| &catalog_episodes[index]);
    let pack_below_missing_threshold = is_series_pack
        && !inputs
            .primary_episode_ids
            .as_ref()
            .and_then(Option::as_ref)
            .is_some_and(|primary_episode_ids| {
                crate::acquisition_coverage::series_pack_missing_ratio_qualifies(
                    &scored_release_metadata,
                    &inputs.catalog_episodes,
                    primary_episode_ids,
                )
            });

    let release_coverage = crate::acquisition_coverage::resolve_release_coverage(
        &scored_release_metadata,
        &inputs.catalog_episodes,
        &inputs.catalog_collections,
        requested_episode,
    );
    if let Some(wanted_episode) = requested_episode
        && !release_coverage.covers_episode(wanted_episode)
    {
        return None;
    }
    let candidate_size_basis = crate::acquisition_coverage::coverage_size_basis(
        &release_coverage,
        &scored_release_metadata,
        &inputs.catalog_episodes,
        runtime_minutes,
    );

    if let Some(wanted_season) = multi_episode_season {
        // Replaces the single-season check below, which passes any
        // release that names no single season: a multi-season pack of
        // other seasons as well as one that holds this season.
        if !release_is_multi_episode_for_season(
            scored_release_metadata.episode.as_ref(),
            wanted_season,
        ) {
            return None;
        }
    } else if requested_episode.is_none()
        && let Some(ref ep_meta) = scored_release_metadata.episode
        && let Some(wanted_season) = season
        && let Some(parsed_season) = ep_meta.season
        && parsed_season != wanted_season
    {
        return None;
    }
    if requested_episode.is_none()
        && let Some(ref ep_meta) = scored_release_metadata.episode
        && let Some(wanted_episode) = episode
    {
        if !ep_meta.episode_numbers.is_empty() && !ep_meta.episode_numbers.contains(&wanted_episode)
        {
            return None;
        }
        if ep_meta.episode_numbers.is_empty()
            && ep_meta.absolute_episode_numbers.is_empty()
            && let (Some(parsed_abs), Some(wanted_abs)) =
                (ep_meta.absolute_episode, absolute_episode)
            && parsed_abs != wanted_abs
        {
            return None;
        }
    }

    // One canonical score, from the same function and the same resolved
    // context the import path uses. Everything that used to be added
    // here and nowhere else — the freshness bonus, the single-episode
    // pack penalty — is gone from the number; what survives of it
    // orders the results, in `acquisition::scoring`, and never crosses
    // a comparison. The listing facts rules read come from the frozen
    // snapshot, captured here at the search's `now` and carried on the
    // result, so the grab persists exactly what was scored.
    let listing =
        crate::quality::release_listing::ReleaseListingSnapshot::for_scoring(&result, inputs.now);
    let scored_listing_json = listing.to_json_string();
    let scoring_context = inputs
        .canonical_context
        .view(candidate_size_basis, false)
        .without_rules();
    let listing = crate::canonical_scoring::ListingFacts::candidate(listing, inputs.now);
    // Shown with the result; the scorer fills the same languages in itself.
    let displayed_release_metadata = crate::canonical_scoring::canonical_parse(
        &scored_release_metadata,
        Some(&listing),
        &scoring_context,
    );
    let scored_release = crate::canonical_scoring::score_release_in_batch(
        &crate::canonical_scoring::ReleaseEvidence::announced(
            scored_release_metadata.clone(),
            result.size_bytes,
        )
        .with_listing(Some(listing)),
        &scoring_context,
        &mut inputs.rule_evaluation_batch,
    );
    let decision = scored_release.announced_decision;

    // Rank is built here, where the listing and the coverage are still
    // in hand, and dropped when the search ends. It is keyed by release
    // rather than carried on the result so it cannot leak into anything
    // that gets stored or compared later.
    let protocol_enabled =
        inputs
            .enabled_protocols
            .is_none_or(|(usenet, torrent)| match result.source_kind {
                Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => usenet,
                Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => torrent,
                Some(DownloadSourceKind::HttpResource) | None => true,
            });
    let rank = crate::acquisition::scoring::SearchRank {
        head: crate::acquisition::scoring::RankHead {
            blocked: !decision.allowed
                || pack_below_missing_threshold
                || numbering_ambiguous
                || !protocol_enabled,
            side: crate::decision_pipeline::SideFacts::of_decision(Some(&decision)),
            negated_revision: 0,
            negated_score: decision.preference_score.saturating_neg(),
            size_fit_penalty: decision.size_fit_penalty,
            pipeline: decision.decision_pipeline,
        },
        non_preferred_protocol: !source_kind_matches_preference(
            &result,
            &inputs.preferred_source_kind,
        ),
        coverage_distance: release_coverage.coverage_distance(requested_episode),
        episode_number: scored_release_metadata
            .episode
            .as_ref()
            .and_then(|episode| episode.episode_numbers.iter().min().copied())
            .unwrap_or(0),
        indexer_priority: source_grab_sort_priority(
            &result,
            inputs
                .indexer_priority_by_name
                .get(&result.source)
                .copied()
                .unwrap_or(i64::MAX),
        ),
        negated_seeders: crate::acquisition::scoring::listing_negated_seeders(&result),
        usenet_age_hours: if matches!(
            result.source_kind,
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl)
        ) {
            crate::acquisition::scoring::listing_age_hours(
                result.published_at.as_deref(),
                inputs.now,
            )
        } else {
            0
        },
    };

    let mut scored_result = IndexerSearchResult {
        parsed_release_metadata: Some(displayed_release_metadata),
        quality_profile_decision: Some(decision),
        // Carry the coverage the scoring pass already resolved (D21);
        // the auto evaluator has no catalog of its own and cannot
        // recompute it.
        coverage_scope: match release_coverage {
            // `Title` and `Unknown` both map to `SubmissionScope::Title`,
            // which would read as "this release covers the whole title"
            // — an assertion neither of them makes. Absent is honest.
            crate::acquisition_coverage::ReleaseCoverage::Title
            | crate::acquisition_coverage::ReleaseCoverage::Unknown => None,
            resolved => Some(resolved.submission_scope()),
        },
        release_listing_json: Some(scored_listing_json),
        ..result
    };
    if numbering_ambiguous {
        crate::acquisition_release_search::annotate_auto_decision(
            &mut scored_result,
            crate::acquisition_release_search::ReleaseAutoDecisionCode::AnimeNumberingAmbiguous,
        );
    } else if pack_below_missing_threshold {
        crate::acquisition_release_search::annotate_auto_decision(
            &mut scored_result,
            crate::acquisition_release_search::ReleaseAutoDecisionCode::PackBelowMissingThreshold,
        );
    }
    Some(EvaluatedCandidate {
        key,
        rank,
        result: scored_result,
    })
}

/// Removes releases no enabled download client can take and counts them per
/// indexer and missing client family. A release with no source kind is kept.
/// When an enabled client's plugin is unavailable, the missing family is
/// reported under the plugin-unavailable code, so the operator is pointed at
/// the plugin rather than at client configuration.
fn retain_routable_releases(
    results: &mut Vec<IndexerSearchResult>,
    has_usenet_client: bool,
    has_torrent_client: bool,
    has_http_client: bool,
    has_unavailable_client_plugin: bool,
) -> HashMap<String, UnroutableReleaseCounts> {
    use crate::error_reasons::release_search as reasons;
    let mut unroutable: HashMap<String, UnroutableReleaseCounts> = HashMap::new();
    let pick = |no_client: &'static str, plugin_unavailable: &'static str| {
        if has_unavailable_client_plugin {
            plugin_unavailable
        } else {
            no_client
        }
    };
    results.retain(|result| {
        let missing = match result.source_kind {
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl)
                if !has_usenet_client =>
            {
                pick(
                    reasons::NO_USENET_CLIENT,
                    reasons::USENET_CLIENT_PLUGIN_UNAVAILABLE,
                )
            }
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
                if !has_torrent_client =>
            {
                pick(
                    reasons::NO_TORRENT_CLIENT,
                    reasons::TORRENT_CLIENT_PLUGIN_UNAVAILABLE,
                )
            }
            Some(DownloadSourceKind::HttpResource) if !has_http_client => pick(
                reasons::NO_HTTP_CLIENT,
                reasons::HTTP_CLIENT_PLUGIN_UNAVAILABLE,
            ),
            _ => return true,
        };
        *unroutable
            .entry(result.indexer_id.clone().unwrap_or_default())
            .or_default()
            .entry(missing)
            .or_default() += 1;
        false
    });
    unroutable
}

/// Reports releases a search set aside because no enabled download client
/// accepts them. An operator's own search warns: those releases were what
/// they asked for. Background searches repeat on a schedule, so they log at
/// debug to keep one missing client from flooding the log.
fn log_unroutable_releases(
    caller_label: &str,
    mode: SearchMode,
    unroutable_by_indexer: &HashMap<String, UnroutableReleaseCounts>,
) {
    for (indexer_id, counts) in unroutable_by_indexer {
        for (missing_client, count) in counts {
            if mode == SearchMode::Interactive {
                warn!(
                    caller = caller_label,
                    indexer_id = indexer_id.as_str(),
                    removed = *count,
                    reason = *missing_client,
                    "removed search results no available download client accepts"
                );
            } else {
                debug!(
                    caller = caller_label,
                    indexer_id = indexer_id.as_str(),
                    removed = *count,
                    reason = *missing_client,
                    "removed search results no available download client accepts"
                );
            }
        }
    }
}

/// Reports releases an automatic search set aside because their indexer is
/// at its grab limit: one line per indexer, with the count. They are dropped
/// before the search picks a candidate, so without this the search would
/// look as though the indexer had returned nothing usable.
fn log_quota_exhausted_releases(
    caller_label: &str,
    quota_exhausted_by_indexer: &HashMap<String, usize>,
) {
    for (indexer_id, count) in quota_exhausted_by_indexer {
        info!(
            caller = caller_label,
            indexer_id = indexer_id.as_str(),
            set_aside = *count,
            reason = crate::error_reasons::release_search::INDEXER_GRAB_QUOTA_EXHAUSTED,
            "set aside search results from an indexer at its grab limit"
        );
    }
}

pub(crate) struct ScoredSearchOutcome {
    pub results: Vec<IndexerSearchResult>,
    /// Releases removed before scoring because no enabled download client
    /// accepts their source kind, keyed by indexer id.
    pub unroutable_by_indexer: HashMap<String, UnroutableReleaseCounts>,
    /// Releases an automatic search set aside because their indexer is at its
    /// grab limit, keyed by indexer id. Always empty for an interactive search,
    /// which lists those releases.
    pub quota_exhausted_by_indexer: HashMap<String, usize>,
    pub complete_indexer_ids: Vec<String>,
    /// Indexers that could not answer any of the search's queries: no id they
    /// can search the facet by and no usable text query. Nothing was sent.
    pub unsupported_indexer_ids: Vec<String>,
    pub incomplete_indexer_reasons: HashMap<String, IncompleteIndexerReason>,
    pub search_session_id: String,
}

/// The release's cross-indexer content identity — THE fingerprint, singular.
///
/// This is both what the search client stages candidates under and what
/// `finalize_evaluated_search_session` retains by. The two sides must agree
/// byte-for-byte or every finalize silently discards its whole session's
/// corpus (no fingerprint ever matches), so there is exactly one
/// implementation and the infrastructure crate calls this one.
///
/// Identity tiers: a torrent is its infohash wherever it came from; an NZB is
/// its normalized title plus exact announced size (conservative — indexers
/// that disagree on size yield separate candidates rather than a false merge);
/// anything else is scoped to its indexer and guid.
pub fn release_candidate_fingerprint(candidate: &IndexerSearchResult) -> String {
    let info_hash = candidate
        .extra
        .get("info_hash")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let normalized_title = candidate
        .title
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let identity = if let Some(info_hash) = info_hash {
        format!("torrent\0{info_hash}")
    } else if !normalized_title.is_empty() && candidate.size_bytes.is_some_and(|size| size > 0) {
        format!(
            "nzb\0{normalized_title}\0{}",
            candidate.size_bytes.unwrap_or_default()
        )
    } else {
        format!(
            "scoped\0{}\0{}\0{}\0{}",
            candidate.source,
            candidate.guid.as_deref().unwrap_or_default(),
            normalized_title,
            candidate.size_bytes.unwrap_or_default()
        )
    };
    crate::blake3_identity_hex(crate::HashDomain::CandidateSessionIdentity, identity)
}

fn candidate_is_reusable(candidate: &IndexerSearchResult) -> bool {
    matches!(
        candidate.auto_decision_code.as_deref(),
        Some("eligible" | "pending_delay" | "minimum_age" | "download_client_unavailable")
    )
}

fn localized_search_names(title: &Title, languages: &[String]) -> Vec<String> {
    let mut names = title.custom_search_aliases();
    for language in languages {
        let Some(language) = crate::normalize_detected_subtitle_language_code(language) else {
            continue;
        };
        let mut localized = title
            .tagged_aliases
            .iter()
            .filter(|alias| alias_search_language(alias).as_ref() == Some(&language))
            .map(|alias| alias.name.trim().to_string())
            .collect::<Vec<_>>();
        localized.sort();
        names.extend(localized);
    }
    let mut seen = HashSet::from([scryer_domain::title_spelling::title_lookup_form(
        &title.name,
    )]);
    names
        .into_iter()
        .filter(|name| {
            let key = scryer_domain::title_spelling::title_lookup_form(name);
            !key.is_empty() && seen.insert(key)
        })
        .collect()
}

/// The search language an alias is searched under, normalized the way
/// configured search languages are.
///
/// A tagged alias keeps its tag. The Japanese romanization tags the matcher
/// already treats as Japanese (`x-jat`, through the romanization rule set) are
/// Japanese here too, so romaji names are searched; `ja-Latn`, `ko-Latn` and
/// `zh-Latn` already resolve through their primary subtag. An alias with no
/// tag (or `und`) is attributed by its script, with its country only as
/// corroboration: see [`scryer_domain::title_spelling::untagged_name_language`].
fn alias_search_language(alias: &TaggedAlias) -> Option<String> {
    let tag = alias.language.trim();
    if !tag.is_empty() && !tag.eq_ignore_ascii_case("und") {
        return crate::normalize_detected_subtitle_language_code(tag).or_else(|| {
            scryer_domain::title_normalization::rules_for(Some(tag))
                .filter(|rules| {
                    rules.equivalence_tag()
                        == scryer_domain::title_normalization::JAPANESE_ROMANIZATION_TAG
                })
                .and_then(|_| crate::normalize_detected_subtitle_language_code("jpn"))
        });
    }
    scryer_domain::title_spelling::untagged_name_language(&alias.name, alias.country.as_deref())
        .and_then(crate::normalize_detected_subtitle_language_code)
}

fn localized_search_queries(primary: &str, alias: &str, queries: &[String]) -> Vec<String> {
    queries
        .iter()
        .filter_map(|query| {
            query
                .strip_prefix(primary.trim())
                .filter(|suffix| suffix.is_empty() || suffix.starts_with(' '))
                .map(|suffix| format!("{alias}{suffix}"))
        })
        .collect()
}

fn release_search_tagged_aliases(title: &Title) -> Vec<TaggedAlias> {
    let title = title.with_custom_search_aliases();
    let mut aliases = title.tagged_aliases.clone();
    let mut seen: HashSet<String> = aliases
        .iter()
        .map(|alias| alias.name.trim().to_ascii_lowercase())
        .filter(|alias| !alias.is_empty())
        .collect();
    for alias in &title.aliases {
        let key = alias.trim().to_ascii_lowercase();
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        aliases.push(TaggedAlias {
            name: alias.clone(),
            language: "und".to_string(),
            country: None,
        });
    }
    aliases
}

fn merge_newznab_category_codes(
    base: impl IntoIterator<Item = String>,
    extras: &[String],
) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for category in base.into_iter().chain(extras.iter().cloned()) {
        let category = category.trim().to_string();
        if !category.is_empty() && seen.insert(category.clone()) {
            merged.push(category);
        }
    }
    merged
}

fn merge_series_movie_categories_into_routing(
    plan: &mut IndexerRoutingPlan,
    owner_facet: &MediaFacet,
    extra_categories: &[String],
) {
    if extra_categories.is_empty() {
        return;
    }
    for entry in plan.entries.values_mut().filter(|entry| entry.enabled) {
        let base_categories = if entry.categories.is_empty() {
            default_indexer_routing_categories_for_scope(owner_facet.as_str())
        } else {
            std::mem::take(&mut entry.categories)
        };
        entry.categories = merge_newznab_category_codes(base_categories, extra_categories);
    }
}

fn source_kind_matches_preference(result: &IndexerSearchResult, preferred: &str) -> bool {
    match result.source_kind {
        Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => preferred == "nzb",
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
            preferred == "torrent"
        }
        Some(DownloadSourceKind::HttpResource) => preferred == "http",
        None => false,
    }
}

/// Whether a release belongs in an operator's whole-season search for
/// `season`: a release of several episodes that includes that season. Season
/// packs, partial-season and multi-episode releases, multi-season packs that
/// name the season and complete-series packs qualify; single episodes, daily
/// episodes, season extras and packs that name only other seasons do not.
///
/// A multi-episode release that names no season at all (an absolute-numbered
/// anime batch) is kept: the season is not contradicted, and the title and
/// numbering checks that run beside this one have already placed it. A
/// release with no episode numbering parsed at all is kept for the same
/// reason the ordinary season check keeps it — nothing parsed says it is a
/// single episode or another season.
pub(crate) fn release_is_multi_episode_for_season(
    episode: Option<&crate::ParsedEpisodeMetadata>,
    season: u32,
) -> bool {
    let Some(episode) = episode else {
        return true;
    };
    if episode.is_season_extra {
        return false;
    }
    let multi_episode = match episode.release_type {
        crate::ParsedEpisodeReleaseType::SeasonPack
        | crate::ParsedEpisodeReleaseType::RangePack
        | crate::ParsedEpisodeReleaseType::MultiEpisode => true,
        crate::ParsedEpisodeReleaseType::SingleEpisode | crate::ParsedEpisodeReleaseType::Daily => {
            false
        }
        crate::ParsedEpisodeReleaseType::Unknown => {
            episode.full_season
                || episode.is_partial_season
                || episode.is_multi_season
                || episode.is_series_pack
                || episode.episode_numbers.len() > 1
                || episode.absolute_episode_numbers.len() > 1
        }
    };
    if !multi_episode {
        return false;
    }
    if !episode.season_numbers.is_empty() {
        return episode.season_numbers.contains(&season);
    }
    episode.season.is_none_or(|named| named == season)
}

#[cfg(test)]
pub(crate) fn extract_http_status_from_message(message: &str) -> Option<u16> {
    let marker = "status ";
    let lowered = message.to_ascii_lowercase();
    let marker_position = lowered.find(marker)?;
    let mut digits = String::new();

    for character in lowered[marker_position + marker.len()..].chars() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else if !digits.is_empty() {
            break;
        }
    }

    digits.parse::<u16>().ok()
}

#[cfg(test)]
pub(crate) fn is_4xx_or_5xx_status(status: u16) -> bool {
    (400..=599).contains(&status)
}

fn resolve_requested_episode(
    episodes: &[Episode],
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Option<&Episode> {
    if let (Some(season), Some(episode_number)) = (season, episode)
        && let Some(found) = episodes.iter().find(|candidate| {
            candidate
                .season_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                == Some(season)
                && candidate
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    == Some(episode_number)
        })
    {
        return Some(found);
    }

    // Matched on the title's one absolute scale; see `AbsoluteScale`.
    let scale = scryer_domain::AbsoluteScale::for_catalog(episodes);
    absolute_episode.and_then(|wanted_absolute| {
        episodes
            .iter()
            .find(|candidate| scale.episode_absolute(candidate) == Some(wanted_absolute))
    })
}

#[cfg(test)]
fn extract_indexer_http_status(error: &AppError) -> Option<u16> {
    match error {
        AppError::Repository(message)
        | AppError::RepositoryFailure { message, .. }
        | AppError::RepositoryFailed { message, .. } => extract_http_status_from_message(message),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn is_indexer_http_error(error: &AppError) -> bool {
    extract_indexer_http_status(error).is_some_and(is_4xx_or_5xx_status)
}

pub(crate) fn release_search_key(result: &IndexerSearchResult) -> String {
    if let Some(download_url) = result
        .download_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return download_url.to_string();
    }

    if let Some(link) = result
        .link
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return link.to_string();
    }

    result.title.clone()
}

fn looks_like_structured_query_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return false;
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper == "OVA" || upper == "SPECIAL" {
        return true;
    }

    if let Some(rest) = upper.strip_prefix('S') {
        if rest.chars().all(|ch| ch.is_ascii_digit()) {
            return true;
        }
        if let Some((season_part, episode_part)) = rest.split_once('E') {
            return !season_part.is_empty()
                && !episode_part.is_empty()
                && season_part.chars().all(|ch| ch.is_ascii_digit())
                && episode_part.chars().all(|ch| ch.is_ascii_digit());
        }
    }

    false
}

/// The season and episode a trailing `Sxx` / `SxxEyy` token asks for, if it is
/// one of those.
fn structured_query_token_numbers(token: &str) -> Option<(u32, Option<u32>)> {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    let rest = trimmed.to_ascii_uppercase();
    let rest = rest.strip_prefix('S')?;
    if let Some((season_part, episode_part)) = rest.split_once('E') {
        let season = season_part.parse::<u32>().ok()?;
        let episode = episode_part.parse::<u32>().ok()?;
        return Some((season, Some(episode)));
    }
    rest.parse::<u32>().ok().map(|season| (season, None))
}

/// Whether a trailing structured token asks for the same thing the dispatch
/// already carries in its season/episode parameters.
///
/// This is what makes the collapse safe. `{title} S01E05` alongside
/// `season=1&ep=5` is the same request written twice, so dropping one saves an
/// API call. A token that names a *different* season and episode is not a
/// duplicate at all — it is a second, genuinely different ask, which is how an
/// anime numbered per cour is searched for alongside its official numbering.
fn structured_query_token_agrees(token: &str, season: Option<u32>, episode: Option<u32>) -> bool {
    let Some((token_season, token_episode)) = structured_query_token_numbers(token) else {
        // OVA/SPECIAL markers name no season or episode to contradict.
        return true;
    };
    if season.is_some_and(|season| season != token_season) {
        return false;
    }
    match (token_episode, episode) {
        (Some(token_episode), Some(episode)) => token_episode == episode,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn normalize_structured_dispatch_query(
    query: &str,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> String {
    let mut tokens: Vec<&str> = query.split_whitespace().collect();
    while let Some(last) = tokens.last().copied() {
        let trimmed = last.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if trimmed.is_empty() {
            tokens.pop();
            continue;
        }

        let removable_numeric = absolute_episode.is_some_and(|value| {
            trimmed.chars().all(|ch| ch.is_ascii_digit())
                && trimmed.parse::<u32>().ok() == Some(value)
        });
        let removable_structured = removable_numeric
            || (looks_like_structured_query_token(trimmed)
                && structured_query_token_agrees(trimmed, season, episode));
        if removable_structured {
            tokens.pop();
            continue;
        }
        break;
    }

    tokens.join(" ").trim().to_string()
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum StructuredDispatchQueryShape {
    AbsoluteEpisode,
    SeasonEpisode,
    Season,
    Other,
}

fn structured_dispatch_query_shape(
    query: &str,
    absolute_episode: Option<u32>,
) -> StructuredDispatchQueryShape {
    let Some(last) = query.split_whitespace().last() else {
        return StructuredDispatchQueryShape::Other;
    };
    let trimmed = last.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return StructuredDispatchQueryShape::Other;
    }

    if absolute_episode.is_some_and(|value| {
        trimmed.chars().all(|ch| ch.is_ascii_digit()) && trimmed.parse::<u32>().ok() == Some(value)
    }) {
        return StructuredDispatchQueryShape::AbsoluteEpisode;
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with('S') && upper.contains('E') {
        return StructuredDispatchQueryShape::SeasonEpisode;
    }
    if upper.starts_with('S') || upper == "OVA" || upper == "SPECIAL" {
        return StructuredDispatchQueryShape::Season;
    }

    StructuredDispatchQueryShape::Other
}

fn dedupe_structured_dispatch_queries(
    queries: Vec<String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Vec<String> {
    if season.is_none() && episode.is_none() && absolute_episode.is_none() {
        return queries;
    }

    let mut deduped = Vec::with_capacity(queries.len());
    let mut seen = std::collections::HashSet::new();

    for query in queries {
        let normalized =
            normalize_structured_dispatch_query(&query, season, episode, absolute_episode);
        let key_source = if normalized.is_empty() {
            query.trim()
        } else {
            normalized.as_str()
        };
        if seen.insert(key_source.to_ascii_lowercase()) {
            deduped.push(query);
        }
    }

    deduped
}

#[derive(Clone, Copy, Debug)]
struct QueryCoverageAggregate {
    completed_queries: usize,
    all_complete: bool,
    all_unsupported: bool,
}

fn record_query_coverage_outcomes(
    aggregate: &mut HashMap<String, QueryCoverageAggregate>,
    outcomes: &[IndexerQueryOutcome],
) {
    // A provider may contribute more than one internal outcome for a query.
    // Collapse those first so duplicate rows cannot masquerade as completing
    // multiple effective title/alias queries.
    let mut per_query = HashMap::<String, (bool, bool)>::new();
    for outcome in outcomes {
        let complete = outcome.outcome.coverage_eligible();
        let unsupported = outcome.outcome.is_unsupported();
        per_query
            .entry(outcome.indexer_id.clone())
            .and_modify(|state| {
                state.0 &= complete;
                state.1 &= unsupported;
            })
            .or_insert((complete, unsupported));
    }
    for (indexer_id, (query_complete, query_unsupported)) in per_query {
        aggregate
            .entry(indexer_id)
            .and_modify(|state| {
                state.completed_queries = state.completed_queries.saturating_add(1);
                state.all_complete &= query_complete;
                state.all_unsupported &= query_unsupported;
            })
            .or_insert(QueryCoverageAggregate {
                completed_queries: 1,
                all_complete: query_complete,
                all_unsupported: query_unsupported,
            });
    }
}

/// The outcome that kept an indexer from covering the search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IncompleteIndexerKind {
    RateLimited,
    QueryBudget,
    Contained,
    Failed,
}

/// Human explanation beside the outcome used by the search state machine.
#[derive(Clone, Debug)]
pub(crate) struct IncompleteIndexerReason {
    pub kind: IncompleteIndexerKind,
    pub message: String,
    /// Stable code for a condition the operator must act on, when the
    /// outcome names one.
    pub code: Option<&'static str>,
}
impl IncompleteIndexerReason {
    pub fn failed(message: String) -> Self {
        Self {
            kind: IncompleteIndexerKind::Failed,
            message,
            code: None,
        }
    }
}
impl std::ops::Deref for IncompleteIndexerReason {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}

pub(crate) const RATE_LIMITED_INDEXER_REASON: &str = "indexer is cooling down after a rate limit";
pub(crate) const QUERY_BUDGET_INDEXER_REASON: &str = "indexer is over its query budget";
pub(crate) const CONTAINED_INDEXER_REASON: &str =
    "indexer search contained at the provider's result limit";

pub(crate) fn reason_is_rate_limit(reason: &IncompleteIndexerReason) -> bool {
    matches!(
        reason.kind,
        IncompleteIndexerKind::RateLimited | IncompleteIndexerKind::QueryBudget
    )
}

pub(crate) fn incomplete_indexer_reason(
    outcome: IndexerSearchOutcome,
) -> Option<IncompleteIndexerReason> {
    let kind = match outcome {
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::RateLimited),
            ..
        } => IncompleteIndexerKind::RateLimited,
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::QueryBudgetExhausted),
            ..
        } => IncompleteIndexerKind::QueryBudget,
        IndexerSearchOutcome::Partial {
            reason:
                Some(
                    IndexerSearchIncompleteReason::PageCeilingReached
                    | IndexerSearchIncompleteReason::SaturatedPartition,
                ),
            ..
        } => IncompleteIndexerKind::Contained,
        _ => IncompleteIndexerKind::Failed,
    };
    let (reason, retry_after) = match outcome {
        IndexerSearchOutcome::Complete { .. } => return None,
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::RateLimited),
            retry_after,
            ..
        } => (RATE_LIMITED_INDEXER_REASON, retry_after),
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::QueryBudgetExhausted),
            retry_after,
            ..
        } => (QUERY_BUDGET_INDEXER_REASON, retry_after),
        // Unattested legacy responses are operationally successful; they only
        // withhold convergence coverage until the plugin declares semantics.
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::Unattested),
            ..
        } => return None,
        IndexerSearchOutcome::Partial {
            reason:
                Some(
                    IndexerSearchIncompleteReason::PageCeilingReached
                    | IndexerSearchIncompleteReason::SaturatedPartition,
                ),
            retry_after,
            ..
        } => (CONTAINED_INDEXER_REASON, retry_after),
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::UpstreamFailure),
            retry_after,
            ..
        } => ("indexer upstream search failed", retry_after),
        IndexerSearchOutcome::Partial { retry_after, .. } => {
            ("indexer search returned partial results", retry_after)
        }
        IndexerSearchOutcome::Deferred { retry_after } => {
            ("indexer search was deferred", retry_after)
        }
        IndexerSearchOutcome::Skipped { retry_after } => {
            ("indexer search was skipped", retry_after)
        }
        // Never asked: an indexer that cannot serve the facet is not a failure.
        IndexerSearchOutcome::Unsupported | IndexerSearchOutcome::BackedOff { .. } => return None,
        IndexerSearchOutcome::ClientUnavailable { .. } => {
            ("indexer client could not be set up", None)
        }
        IndexerSearchOutcome::Errored => ("indexer search failed", None),
    };
    let code = match outcome {
        IndexerSearchOutcome::ClientUnavailable { reason } => Some(reason),
        _ => None,
    };
    Some(IncompleteIndexerReason {
        kind,
        message: match retry_after {
            Some(delay) => format!("{reason}; retry after {}s", delay.as_secs()),
            None => reason.to_string(),
        },
        code,
    })
}

fn completely_covered_indexers(
    aggregate: HashMap<String, QueryCoverageAggregate>,
    required_query_count: usize,
) -> Vec<String> {
    indexers_answering_every_query(&aggregate, required_query_count, |state| state.all_complete)
}

/// Indexers that reported `Unsupported` for every query of the search.
fn unsupported_indexers(
    aggregate: &HashMap<String, QueryCoverageAggregate>,
    required_query_count: usize,
) -> Vec<String> {
    indexers_answering_every_query(aggregate, required_query_count, |state| {
        state.all_unsupported
    })
}

fn indexers_answering_every_query(
    aggregate: &HashMap<String, QueryCoverageAggregate>,
    required_query_count: usize,
    answered: impl Fn(&QueryCoverageAggregate) -> bool,
) -> Vec<String> {
    if required_query_count == 0 {
        return Vec::new();
    }
    let mut indexers = aggregate
        .iter()
        .filter(|(_, state)| state.completed_queries == required_query_count && answered(state))
        .map(|(indexer_id, _)| indexer_id.clone())
        .collect::<Vec<_>>();
    indexers.sort();
    indexers
}

fn dedupe_text_safe_structured_dispatch_queries(
    queries: Vec<String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Vec<String> {
    if season.is_none() && episode.is_none() && absolute_episode.is_none() {
        return queries;
    }

    let mut deduped = Vec::with_capacity(queries.len());
    let mut seen = std::collections::HashSet::new();

    for query in queries {
        let normalized =
            normalize_structured_dispatch_query(&query, season, episode, absolute_episode);
        let key_source = if normalized.is_empty() {
            query.trim()
        } else {
            normalized.as_str()
        };
        let key = (
            key_source.to_ascii_lowercase(),
            structured_dispatch_query_shape(&query, absolute_episode),
        );
        if seen.insert(key) {
            deduped.push(query);
        }
    }

    deduped
}

fn should_collapse_structured_nab_queries(
    configs: &[IndexerConfig],
    routing: Option<&IndexerRoutingPlan>,
    search_restriction: Option<&HashSet<String>>,
    mode: SearchMode,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if configs.is_empty() {
        return false;
    }

    let mut saw_nab_transport = false;

    for config in configs {
        if !config.is_enabled {
            continue;
        }
        if config.disabled_until.is_some_and(|until| until > now) {
            continue;
        }

        let mode_ok = match mode {
            SearchMode::Interactive => config.enable_interactive_search,
            SearchMode::Auto => auto_mode_enabled_for_structured_collapse(config),
        };
        if !mode_ok {
            continue;
        }

        if crate::contracts::indexer_search_eligibility(routing, search_restriction, &config.id)
            != crate::contracts::IndexerSearchEligibility::Eligible
        {
            continue;
        }

        match config.nab_transport_kind() {
            Some(_) => saw_nab_transport = true,
            None => return false,
        }
    }

    saw_nab_transport
}

fn has_eligible_search_indexer(
    routing: Option<&IndexerRoutingPlan>,
    search_restriction: Option<&HashSet<String>>,
) -> bool {
    match search_restriction {
        Some(allowed) => allowed.iter().any(|indexer_id| {
            crate::contracts::indexer_search_eligibility(routing, Some(allowed), indexer_id)
                == crate::contracts::IndexerSearchEligibility::Eligible
        }),
        None => routing.is_none_or(|plan| plan.entries.values().any(|entry| entry.enabled)),
    }
}

fn dispatch_routing_for_search_restriction(
    configs: &[IndexerConfig],
    routing: Option<&IndexerRoutingPlan>,
    search_restriction: Option<&HashSet<String>>,
) -> Option<IndexerRoutingPlan> {
    let Some(allowed) = search_restriction else {
        return routing.cloned();
    };

    let mut entries = configs
        .iter()
        .filter(|config| config.is_enabled)
        .map(|config| {
            (
                config.id.clone(),
                crate::contracts::IndexerRoutingEntry {
                    enabled: true,
                    categories: Vec::new(),
                    priority: 0,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    if let Some(routing) = routing {
        entries.extend(routing.entries.clone());
    }
    for (indexer_id, entry) in &mut entries {
        entry.enabled &= allowed.contains(indexer_id);
    }

    Some(IndexerRoutingPlan { entries })
}

#[derive(Debug, Default, Deserialize)]
struct ManagedIndexerAutoModeMetadata {
    enable_automatic_search: Option<bool>,
}

fn auto_mode_enabled_for_structured_collapse(config: &IndexerConfig) -> bool {
    if !config.enable_auto_search {
        return false;
    }

    let Some(raw) = config.managed_metadata_json.as_deref() else {
        return true;
    };
    let Ok(metadata) = serde_json::from_str::<ManagedIndexerAutoModeMetadata>(raw) else {
        return true;
    };

    metadata.enable_automatic_search.unwrap_or(true)
}

/// Presentation order for scored release results: **allowed, then tier, then
/// revision, then score** — the head of the search rank, shared with it
/// (`RankHead`) so the two orderings cannot drift apart.
///
/// Used by the interactive job's incremental merge, and only there: it re-sorts
/// a partial snapshot as batches arrive, and the GraphQL payload truncates to
/// the requested limit, so a comparator that disagreed with the rank would cut
/// the wrong releases. It compared allowed → score only, and with the tier no
/// longer inside the score that listed a 720p release above a 2160p one (D11).
/// The one-shot path sorts with the full [`SearchRank`]
/// (`compare_ranked_results`), so the two surfaces agree on the head of the key
/// and differ below it — a deferred follow-on, not a claim this doc should make.
///
/// [`SearchRank`]: crate::acquisition::scoring::SearchRank
///
/// The listing steps (indexer priority, seeders, age, coverage) are deliberately
/// absent: a merge sees results from several indexers arriving at different
/// times, and nothing here may depend on when a batch landed.
pub(crate) fn compare_release_search_results(
    left: &IndexerSearchResult,
    right: &IndexerSearchResult,
) -> std::cmp::Ordering {
    crate::acquisition::scoring::RankHead::compare(left, right)
}

fn grab_quota_preference(candidate: &IndexerSearchResult) -> (u8, u64, u64) {
    let current = candidate.extra.get("grab_current").and_then(Value::as_u64);
    let maximum = candidate.extra.get("grab_max").and_then(Value::as_u64);
    match (current, maximum) {
        (_, None | Some(0)) => (0, 0, 1),
        (Some(current), Some(maximum)) if current < maximum => (1, current, maximum),
        (None, Some(maximum)) => (2, 0, maximum),
        (Some(current), Some(maximum)) => (3, current, maximum),
    }
}

fn source_is_preferred_for_grab(
    candidate: &IndexerSearchResult,
    existing: &IndexerSearchResult,
) -> bool {
    let candidate_quota = grab_quota_preference(candidate);
    let existing_quota = grab_quota_preference(existing);
    candidate_quota.0 < existing_quota.0
        || (candidate_quota.0 == existing_quota.0
            && candidate_quota.0 == 1
            && u128::from(candidate_quota.1) * u128::from(existing_quota.2)
                < u128::from(existing_quota.1) * u128::from(candidate_quota.2))
}

fn source_grab_sort_priority(candidate: &IndexerSearchResult, configured: i64) -> i64 {
    let (class, current, maximum) = grab_quota_preference(candidate);
    let usage = if class == 1 {
        (u128::from(current) * 1_000_000 / u128::from(maximum)) as i64
    } else {
        0
    };
    i64::from(class) * 2_000_000_000 + usage * 1_000 + configured.clamp(0, 999)
}

pub(crate) fn dedupe_cross_indexer_release_results(
    results: Vec<IndexerSearchResult>,
    indexer_priority_by_name: &HashMap<String, i64>,
    preferred_source_kind: &str,
) -> Vec<IndexerSearchResult> {
    let mut best_by_key: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut remove_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (idx, result) in results.iter().enumerate() {
        let key = result
            .parsed_release_metadata
            .as_ref()
            .map(crate::release_dedup::build_release_dedup_key)
            .unwrap_or_default();
        if key.is_empty() {
            continue;
        }

        if let Some(&existing_idx) = best_by_key.get(&key) {
            let existing = &results[existing_idx];

            let existing_prio = indexer_priority_by_name
                .get(&existing.source)
                .copied()
                .unwrap_or(i64::MAX);
            let new_prio = indexer_priority_by_name
                .get(&result.source)
                .copied()
                .unwrap_or(i64::MAX);

            let existing_preferred =
                source_kind_matches_preference(existing, preferred_source_kind);
            let new_preferred = source_kind_matches_preference(result, preferred_source_kind);
            let new_wins = if existing_preferred != new_preferred {
                new_preferred
            } else if grab_quota_preference(existing) != grab_quota_preference(result) {
                source_is_preferred_for_grab(result, existing)
            } else {
                new_prio < existing_prio
            };

            if new_wins {
                remove_indices.insert(existing_idx);
                best_by_key.insert(key, idx);
            } else {
                remove_indices.insert(idx);
            }
        } else {
            best_by_key.insert(key, idx);
        }
    }

    if remove_indices.is_empty() {
        return results;
    }

    let before = results.len();
    let mut idx = 0usize;
    let mut deduped = results;
    deduped.retain(|_| {
        let keep = !remove_indices.contains(&idx);
        idx += 1;
        keep
    });
    debug!(before, after = deduped.len(), "cross-indexer release dedup");
    deduped
}

/// Which protocol families the enabled download clients cover, and the
/// source kind the highest-priority client prefers.
pub(crate) struct DownloadSourceCapabilities {
    pub(crate) has_usenet: bool,
    pub(crate) has_torrent: bool,
    pub(crate) has_http: bool,
    /// An enabled client's type is known to no native client and no loaded
    /// plugin: its plugin is blocked or uninstalled.
    pub(crate) has_unavailable_client_plugin: bool,
    pub(crate) preferred: String,
}

impl AppUseCase {
    pub(crate) async fn download_source_capabilities(&self) -> DownloadSourceCapabilities {
        let clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await
            .unwrap_or_default();
        let enabled: Vec<_> = clients.iter().filter(|c| c.is_enabled).collect();
        let plugin_provider = self
            .services
            .integrations
            .download_client_plugin_provider
            .available();
        let client_accepts = |c: &&scryer_domain::DownloadClientConfig,
                              kind: DownloadSourceKind| {
            let inputs = crate::accepted_inputs_for_client(&c.client_type, plugin_provider);
            inputs.contains(&kind)
        };
        let has_usenet = enabled
            .iter()
            .any(|c| client_accepts(c, DownloadSourceKind::NzbFile));
        let has_torrent = enabled.iter().any(|c| {
            client_accepts(c, DownloadSourceKind::TorrentFile)
                || client_accepts(c, DownloadSourceKind::MagnetUri)
        });
        let has_http = enabled
            .iter()
            .any(|c| client_accepts(c, DownloadSourceKind::HttpResource));
        let unavailable_client_names = enabled
            .iter()
            .filter(|c| {
                !crate::helpers::download_client_type_is_known(&c.client_type, plugin_provider)
            })
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>();
        if !unavailable_client_names.is_empty() {
            debug!(
                clients = ?unavailable_client_names,
                "enabled download clients have no loaded plugin"
            );
        }
        let preferred = enabled
            .iter()
            .min_by_key(|c| c.client_priority)
            .map(|c| {
                if client_accepts(c, DownloadSourceKind::NzbFile) {
                    "nzb"
                } else {
                    "torrent"
                }
            })
            .unwrap_or("nzb")
            .to_string();

        DownloadSourceCapabilities {
            has_usenet,
            has_torrent,
            has_http,
            has_unavailable_client_plugin: !unavailable_client_names.is_empty(),
            preferred,
        }
    }

    pub(crate) async fn build_indexer_priority_by_name(
        &self,
        indexer_routing: Option<&IndexerRoutingPlan>,
    ) -> HashMap<String, i64> {
        let Some(plan) = indexer_routing else {
            return HashMap::new();
        };

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_default();
        let id_to_name: std::collections::HashMap<&str, &str> = configs
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect();
        plan.entries
            .iter()
            .filter_map(|(id, entry)| {
                id_to_name
                    .get(id.as_str())
                    .map(|name| (name.to_string(), entry.priority))
            })
            .collect()
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn score_release_results(
        &self,
        raw_results: Vec<IndexerSearchResult>,
        quality_profile: &QualityProfile,
        title_id: &str,
        indexer_routing: Option<&IndexerRoutingPlan>,
        runtime_minutes: Option<i32>,
        parse_context: &ReleaseParseContext,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let mut prepared = None;
        self.score_release_results_with_prepared(
            raw_results,
            quality_profile,
            title_id,
            indexer_routing,
            runtime_minutes,
            parse_context,
            season,
            episode,
            absolute_episode,
            &mut prepared,
            None,
            false,
            None,
            now,
            ScoringLane::Full,
        )
        .await
        .map(|batch| batch.results)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "release scoring needs the full search context to produce deterministic ranking decisions"
    )]
    pub(crate) async fn score_release_results_with_prepared(
        &self,
        mut raw_results: Vec<IndexerSearchResult>,
        quality_profile: &QualityProfile,
        title_id: &str,
        // The library and settings scope used to be resolved here; the canonical
        // scoring context resolves them from the title, the same way import
        // does, so passing them separately could only make the two disagree.
        indexer_routing: Option<&IndexerRoutingPlan>,
        // The search `category` and the title's tags used to be read here for
        // audio-language inference; the canonical scorer now infers it from
        // the title's own facet and tags, so every lane infers identically.
        runtime_minutes: Option<i32>,
        parse_context: &ReleaseParseContext,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        prepared: &mut Option<PreparedReleaseScoringInputs>,
        catalog_reads: Option<&crate::acquisition::title_reads::TitleCatalogReads>,
        preserve_duplicate_sources: bool,
        // Set only for an operator's whole-season search: the season whose
        // multi-episode releases are the only ones kept. Applied here, before
        // the result list is truncated, so single episodes that would be
        // dropped cannot crowd the packs out of it.
        multi_episode_season: Option<u32>,
        // The lane's clock: listing age is read against it, and the snapshot a
        // grab or park persists is captured at it.
        now: chrono::DateTime<chrono::Utc>,
        // The background walk caps how many results it scores; every other
        // search scores them all.
        lane: ScoringLane,
    ) -> AppResult<ScoredReleaseBatch> {
        let mut timer = crate::rules::metrics::StageTimer::new(
            "scan_batch",
            crate::rules::metrics::Purpose::Live,
        );
        timer.outcome("error");
        metrics::counter!("scryer_scoring_batch_candidates_total", "purpose" => "live")
            .increment(raw_results.len() as u64);
        let local_reads = crate::acquisition::title_reads::TitleCatalogReads::new(title_id);
        let reads = catalog_reads
            .filter(|reads| reads.covers(title_id))
            .unwrap_or(&local_reads);
        let mut routing_priorities = indexer_routing
            .into_iter()
            .flat_map(|plan| plan.entries.iter())
            .map(|(id, entry)| (id.clone(), entry.priority))
            .collect::<Vec<_>>();
        routing_priorities.sort();
        if prepared.as_ref().is_some_and(|inputs| {
            inputs.title.id != title_id
                || inputs.quality_profile != *quality_profile
                || inputs.routing_priorities != routing_priorities
        }) {
            *prepared = None;
        }
        // RSS evaluates multiple scopes and may block a failed release between
        // them. Share catalog/policy inputs, but refresh this mutable exclusion.
        if let Some(prepared) = prepared.as_mut() {
            prepared.blocklist = self.load_title_release_blocklist_signatures(title_id).await;
        }
        if prepared.is_none() {
            #[cfg(test)]
            reads
                .scoring_preparations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let blocklist = self.load_title_release_blocklist_signatures(title_id).await;
            let DownloadSourceCapabilities {
                has_usenet: has_usenet_client,
                has_torrent: has_torrent_client,
                has_http: has_http_client,
                has_unavailable_client_plugin,
                preferred: client_preferred_source_kind,
            } = self.download_source_capabilities().await;
            let scored_title = self.services.catalog.titles.get_by_id(title_id).await?;
            let Some(scored_title) = scored_title else {
                return Err(AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {title_id} for release scoring"),
                ));
            };
            let delay_profiles = self.load_delay_profiles().await;
            let delay_profile = crate::delay_profile::resolve_delay_profile(
                &delay_profiles,
                &scored_title.tags,
                &scored_title.facet,
                &scored_title.library_id,
            );
            let preferred_source_kind = delay_profile
                .map(|profile| match profile.preferred_protocol {
                    crate::delay_profile::PreferredProtocol::Usenet => "nzb",
                    crate::delay_profile::PreferredProtocol::Torrent => "torrent",
                })
                .unwrap_or(client_preferred_source_kind.as_str())
                .to_string();
            let enabled_protocols =
                delay_profile.map(|profile| (profile.enable_usenet, profile.enable_torrent));
            let canonical_context = self
                .resolve_canonical_scoring_context(&scored_title, quality_profile)
                .await;
            let rule_evaluation_batch = crate::canonical_scoring::RuleEvaluationBatch::from_context(
                &canonical_context
                    .view(crate::quality_profile::CoverageSizeBasis::default(), false),
            );
            // A reusable RSS snapshot must never turn a failed catalog read
            // into an empty success retained for every later scope.
            let (catalog_episodes, catalog_collections) = if catalog_reads.is_some() {
                (reads.episodes(self).await?, reads.collections(self).await?)
            } else {
                (
                    reads.episodes(self).await.unwrap_or_default(),
                    reads.collections(self).await.unwrap_or_default(),
                )
            };
            // Read once per title and reused for every release scored against
            // it; `None` for every title with no stored bridge.
            let bridge = reads.anime_numbering_bridge(self).await;
            let anime_numbering_bridge = if catalog_reads.is_some() {
                bridge?
            } else {
                bridge.unwrap_or_default()
            };
            let indexer_priority_by_name =
                self.build_indexer_priority_by_name(indexer_routing).await;
            *prepared = Some(PreparedReleaseScoringInputs {
                quality_profile: quality_profile.clone(),
                routing_priorities,
                blocklist,
                has_usenet_client,
                has_torrent_client,
                has_http_client,
                has_unavailable_client_plugin,
                preferred_source_kind,
                enabled_protocols,
                title: scored_title,
                canonical_context,
                catalog_episodes,
                catalog_collections,
                anime_numbering_bridge,
                primary_episode_ids: None,
                indexer_priority_by_name,
                now,
                rule_evaluation_batch,
            });
        }
        let inputs = prepared
            .as_mut()
            .expect("release scoring inputs initialized");

        let unroutable_by_indexer = retain_routable_releases(
            &mut raw_results,
            inputs.has_usenet_client,
            inputs.has_torrent_client,
            inputs.has_http_client,
            inputs.has_unavailable_client_plugin,
        );
        let requested_episode_index =
            resolve_requested_episode(&inputs.catalog_episodes, season, episode, absolute_episode)
                .and_then(|found| {
                    inputs
                        .catalog_episodes
                        .iter()
                        .position(|candidate| std::ptr::eq(candidate, found))
                });

        // The cheap filters run first, on the async thread: a duplicate or a
        // blocklisted release is dropped before anything parses it.
        let mut seen = HashSet::new();
        let mut duplicates = 0usize;
        let mut blocklisted = 0usize;
        let mut survivors = Vec::with_capacity(raw_results.len());
        for result in raw_results {
            if !seen.insert(release_search_key(&result)) {
                duplicates += 1;
                continue;
            }
            if is_release_blocklisted(
                result.indexer_id.as_deref(),
                &result.title,
                result.info_hash(),
                &inputs.blocklist,
            ) {
                blocklisted += 1;
                continue;
            }
            survivors.push(result);
        }
        let (survivors, over_cap) = cap_evaluated_results(survivors, lane.evaluation_cap());
        lane.record_evaluated(duplicates, blocklisted, over_cap, survivors.len());

        // Parsing and rule scoring are CPU work: they run on the blocking
        // pool, a chunk at a time, with the prepared snapshot moved into each
        // chunk and back so every candidate is scored against the same one.
        let parse_context = Arc::new(parse_context.clone());
        let tracing_title_id: Arc<str> = Arc::from(title_id);
        let parsed = score_in_blocking_chunks(prepared, survivors, move |inputs, result| {
            Some(parse_candidate(
                inputs,
                &parse_context,
                &tracing_title_id,
                result,
            ))
        })
        .await?;

        // A series pack is judged against the episodes that already have a
        // file. That read is async, so it is made once, before scoring, and
        // only when some candidate is a pack.
        let inputs = prepared
            .as_mut()
            .expect("release scoring inputs initialized");
        if parsed.iter().any(|candidate| candidate.is_series_pack)
            && (inputs.primary_episode_ids.is_none()
                || (catalog_reads.is_some() && matches!(inputs.primary_episode_ids, Some(None))))
        {
            inputs.primary_episode_ids = Some(match reads.primary_file_cutoffs(self).await {
                Ok(files) => Some(
                    files
                        .iter()
                        .filter_map(|file| file.episode_id.clone())
                        .collect::<std::collections::HashSet<_>>(),
                ),
                Err(error) => {
                    tracing::warn!(
                        title_id,
                        error = %error,
                        "release scoring: failed to load media ownership; rejecting series packs"
                    );
                    None
                }
            });
        }

        let request = CandidateRequest {
            season,
            episode,
            absolute_episode,
            multi_episode_season,
            runtime_minutes,
            requested_episode_index,
        };
        let evaluated = score_in_blocking_chunks(prepared, parsed, move |inputs, candidate| {
            evaluate_candidate(inputs, &request, candidate)
        })
        .await?;
        let prepared = prepared
            .as_mut()
            .expect("release scoring inputs initialized");
        let mut rank_by_key: HashMap<String, crate::acquisition::scoring::SearchRank> =
            HashMap::with_capacity(evaluated.len());
        let mut scored = Vec::with_capacity(evaluated.len());
        for candidate in evaluated {
            rank_by_key.insert(candidate.key, candidate.rank);
            scored.push(candidate.result);
        }

        let mut quota_exhausted_by_indexer: HashMap<String, usize> = HashMap::new();
        let mut scored = if preserve_duplicate_sources {
            for candidate in &scored {
                if grab_quota_preference(candidate).0 == 3 {
                    *quota_exhausted_by_indexer
                        .entry(candidate.indexer_id.clone().unwrap_or_default())
                        .or_default() += 1;
                }
            }
            scored.retain(|candidate| grab_quota_preference(candidate).0 != 3);
            scored
        } else {
            dedupe_cross_indexer_release_results(
                scored,
                &prepared.indexer_priority_by_name,
                &prepared.preferred_source_kind,
            )
        };

        scored.sort_by(|left, right| {
            crate::acquisition::scoring::compare_ranked_results(
                left,
                right,
                &rank_by_key,
                release_search_key,
            )
            .then_with(|| left.indexer_id.cmp(&right.indexer_id))
        });
        // Admission supplies related-incumbent revision preference. Do not
        // discard candidates before that context has been applied.

        timer.outcome("ok");
        Ok(ScoredReleaseBatch {
            results: scored,
            unroutable_by_indexer,
            quota_exhausted_by_indexer,
        })
    }

    /// Internal search+score pipeline shared by both user-facing search and background acquisition.
    /// Returns the scored releases plus the set of indexer ids that actually
    /// **fired** a query and returned a response (empty included), aggregated across
    /// all queries. The fired
    /// set — never the routed set — is what background acquisition records as
    /// Convergence coverage.
    async fn search_and_score_releases_with_context(
        &self,
        request: ReleaseSearchRequest<'_>,
        context: &mut WalkSearchContext,
    ) -> AppResult<ScoredSearchOutcome> {
        let ReleaseSearchRequest {
            queries,
            imdb_id,
            tmdb_id,
            tvdb_id,
            anidb_id,
            mal_id,
            category,
            owner_facet,
            search_facet,
            id_search_facet,
            newznab_categories,
            title_id,
            title_tags,
            library_id,
            caller_label,
            mode,
            runtime_minutes,
            parse_context,
            season,
            episode,
            numbering_context,
            absolute_episode,
            year,
            tagged_aliases,
            search_subject_kind,
            multi_episode_season,
            cancel_token,
            restrict_to_indexer_ids,
            background_value,
            scoring_now,
        } = request;
        let identity = (title_id.to_string(), owner_facet.clone());
        if context.reads.is_none() || context.search_identity.as_ref() != Some(&identity) {
            context.prepared = None;
            context.search_identity = Some(identity);
        }
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("indexer search canceled"));
        }
        let quality_profile_lookup = QualityProfileLookup {
            title_tags,
            library_id,
            imdb_id: imdb_id.as_deref(),
            tvdb_id: tvdb_id.as_deref(),
            category_hint: Some(owner_facet.as_str()),
        };
        let quality_profile = self.resolve_quality_profile(quality_profile_lookup).await?;

        let scope_id = self.quality_profile_scope_id(quality_profile_lookup);
        let mut indexer_routing = self
            .resolve_indexer_routing(library_id, scope_id.as_deref())
            .await;
        let newznab_categories = if newznab_categories.is_empty() {
            None
        } else {
            if let Some(plan) = indexer_routing.as_mut() {
                merge_series_movie_categories_into_routing(plan, &owner_facet, &newznab_categories);
            }
            Some(merge_newznab_category_codes(
                default_indexer_routing_categories_for_scope(owner_facet.as_str()),
                &newznab_categories,
            ))
        };

        // If routing exists and no indexer is eligible for this search, skip it entirely.
        if !has_eligible_search_indexer(indexer_routing.as_ref(), restrict_to_indexer_ids.as_ref())
        {
            info!(
                caller = caller_label,
                scope_id = scope_id.as_deref().unwrap_or("none"),
                "no indexers eligible for search, skipping search"
            );
            return Ok(ScoredSearchOutcome {
                results: Vec::new(),
                unroutable_by_indexer: HashMap::new(),
                quota_exhausted_by_indexer: HashMap::new(),
                complete_indexer_ids: Vec::new(),
                unsupported_indexer_ids: Vec::new(),
                incomplete_indexer_reasons: HashMap::new(),
                search_session_id: Uuid::new_v4().to_string(),
            });
        }

        let configured_indexers = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_else(|error| {
                warn!(error = %error, "failed to load indexer configs for search planning");
                vec![]
            });
        let collapse_structured_queries = search_subject_kind == ReleaseSearchSubjectKind::Episode
            && should_collapse_structured_nab_queries(
                &configured_indexers,
                indexer_routing.as_ref(),
                restrict_to_indexer_ids.as_ref(),
                mode,
                chrono::Utc::now(),
            );
        // Keep the restriction on the page sink as the source of truth for
        // classification, while projecting it into the routing map for
        // IndexerClient implementations that override only the legacy search method.
        let dispatch_indexer_routing = dispatch_routing_for_search_restriction(
            &configured_indexers,
            indexer_routing.as_ref(),
            restrict_to_indexer_ids.as_ref(),
        );

        // Auto mode normally conserves API calls by using the first query, but
        // episode acquisition keeps season/title fallbacks so packs and ranges
        // can be considered for a single requested episode. Broad structured
        // collapse is only safe when provider dispatch uses season/episode
        // parameters; text dispatch still needs distinct SxxEyy/Sxx/title forms.
        let effective_queries = match mode {
            SearchMode::Auto if search_subject_kind == ReleaseSearchSubjectKind::Episode => queries,
            SearchMode::Auto => queries.into_iter().take(1).collect(),
            SearchMode::Interactive => queries,
        };
        let effective_queries = if collapse_structured_queries {
            dedupe_structured_dispatch_queries(effective_queries, season, episode, absolute_episode)
        } else if mode == SearchMode::Auto
            && search_subject_kind == ReleaseSearchSubjectKind::Episode
        {
            dedupe_text_safe_structured_dispatch_queries(
                effective_queries,
                season,
                episode,
                absolute_episode,
            )
        } else {
            effective_queries
        };
        let required_query_count = usize::from(!effective_queries.is_empty());

        let mut set = JoinSet::new();
        let (page_tx, mut page_source) = mpsc::channel(2);
        let page_sink = crate::IndexerSearchPageSink::new(page_tx, 2)
            .with_indexer_restriction(restrict_to_indexer_ids.clone());
        let mut ids = HashMap::new();
        if let Some(imdb_id) = imdb_id.clone() {
            ids.insert("imdb_id".to_string(), imdb_id);
        }
        if let Some(tmdb_id) = tmdb_id.clone() {
            ids.insert("tmdb_id".to_string(), tmdb_id);
        }
        if let Some(tvdb_id) = tvdb_id.clone() {
            ids.insert("tvdb_id".to_string(), tvdb_id);
        }
        if let Some(anidb_id) = anidb_id.clone() {
            ids.insert("anidb_id".to_string(), anidb_id);
        }
        if let Some(mal_id) = mal_id.clone() {
            ids.insert("mal_id".to_string(), mal_id);
        }
        let search_session_id = Uuid::new_v4().to_string();
        let learning_context = if !title_id.trim().is_empty() {
            Some(IndexerSearchLearningContext {
                title_id: title_id.to_string(),
                facet: search_facet.as_str().to_string(),
                subject_kind: search_subject_kind,
                search_session_id: search_session_id.clone(),
                // The convergence value hint rides the Auto background context so
                // the scheduler can lane-rank this scope.
                background_value,
                // Only the background convergence lanes set a value hint, and
                // only they may be served from the persisted candidate corpus.
                // An explicit operator search (queue-best-release, the UI search
                // buttons) must fire the indexer live: the user is asking what
                // exists *now*, and a corpus snapshot persisted before a new
                // release appeared would hide it for the whole reuse window.
                candidate_reuse_allowed: background_value.is_some(),
                background_pages_per_pass: background_value
                    .map(|value| self.background_pages_per_pass(value)),
            })
        } else {
            None
        };
        let indexer_error_operation = match mode {
            SearchMode::Interactive => IndexerErrorOperation::InteractiveSearch,
            SearchMode::Auto => IndexerErrorOperation::AutomaticSearch,
        };

        let indexer_client = self.services.integrations.indexer_client.clone();
        let facet = Some(search_facet.as_str().to_string());
        let id_search_facet = id_search_facet
            .as_ref()
            .map(|facet| facet.as_str().to_string());
        let tagged_aliases = tagged_aliases.to_vec();
        let query_cancel_token = cancel_token.child_token();
        let plan_page_sink = page_sink.clone();
        let plan_indexer_routing = dispatch_indexer_routing;
        if context.should_yield() {
            return Ok(ScoredSearchOutcome {
                results: Vec::new(),
                unroutable_by_indexer: HashMap::new(),
                quota_exhausted_by_indexer: HashMap::new(),
                complete_indexer_ids: Vec::new(),
                unsupported_indexer_ids: Vec::new(),
                incomplete_indexer_reasons: HashMap::new(),
                search_session_id,
            });
        }
        set.spawn(async move {
            indexer_client
                .search_queries_stream(
                    effective_queries,
                    ids,
                    category,
                    facet,
                    id_search_facet,
                    newznab_categories,
                    plan_indexer_routing,
                    mode,
                    indexer_error_operation,
                    season,
                    episode,
                    numbering_context,
                    absolute_episode,
                    year,
                    tagged_aliases,
                    learning_context,
                    query_cancel_token,
                    plan_page_sink,
                )
                .await
        });

        drop(page_sink);

        let mut query_failures = 0usize;
        let mut successful_searches = 0usize;
        let mut first_failure: Option<String> = None;
        let mut scored_results: Vec<IndexerSearchResult> = Vec::new();
        let mut page_source_open = true;
        let mut search_tasks_open = true;
        // Coverage is an intersection across every effective title/alias
        // query. Candidates from incomplete queries remain usable, but one
        // partial, missing, or failed query withholds coverage for that indexer.
        let mut coverage_by_indexer = HashMap::new();
        let mut incomplete_indexer_reasons = HashMap::new();

        loop {
            if !page_source_open && !search_tasks_open {
                break;
            }
            let result = tokio::select! {
                _ = cancel_token.cancelled() => {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    return Err(AppError::canceled("indexer search canceled"));
                }
                page = page_source.recv(), if page_source_open => {
                    match page {
                        Some(mut page) => {
                            for result in &mut page.results {
                                let provenance = result.provenance.get_or_insert(
                                    ReleaseCandidateProvenance {
                                        search_subject_kind,
                                        strategy_kind: ReleaseStrategyKind::Fallback,
                                        title_validated_upstream: false,
                                    },
                                );
                                provenance.search_subject_kind = search_subject_kind;
                            }
                            // Keep the bounded search response until all pages
                            // arrive, then score once rather than rescoring the
                            // growing retained set on every page.
                            scored_results.append(&mut page.results);
                            continue;
                        }
                        None => {
                            page_source_open = false;
                            continue;
                        }
                    }
                }
                result = set.join_next(), if search_tasks_open => result,
            };

            let Some(result) = result else {
                search_tasks_open = false;
                continue;
            };

            match result {
                Ok(Ok(response)) => {
                    successful_searches += 1;
                    for outcome in &response.indexer_outcomes {
                        if let Some(reason) = incomplete_indexer_reason(outcome.outcome) {
                            incomplete_indexer_reasons
                                .entry(outcome.indexer_id.clone())
                                .or_insert(reason);
                        }
                    }
                    record_query_coverage_outcomes(
                        &mut coverage_by_indexer,
                        &response.indexer_outcomes,
                    );
                }
                Ok(Err(error)) => {
                    if error.is_canceled() {
                        set.abort_all();
                        while set.join_next().await.is_some() {}
                        return Err(error);
                    }
                    query_failures += 1;
                    first_failure = first_failure.or_else(|| Some(error.to_string()));
                    warn!(
                        caller = caller_label,
                        error = %error,
                        "indexer search query failed"
                    );
                }
                Err(error) => {
                    query_failures += 1;
                    first_failure = first_failure.or_else(|| Some(error.to_string()));
                    warn!(
                        caller = caller_label,
                        error = %error,
                        "indexer search task panicked"
                    );
                }
            }
        }

        let mut unroutable_by_indexer = HashMap::new();
        let mut quota_exhausted_by_indexer = HashMap::new();
        if !scored_results.is_empty() {
            let batch = self
                .score_release_results_with_prepared(
                    scored_results,
                    &quality_profile,
                    title_id,
                    indexer_routing.as_ref(),
                    runtime_minutes,
                    parse_context,
                    season,
                    episode,
                    absolute_episode,
                    &mut context.prepared,
                    context.reads.as_deref(),
                    mode == SearchMode::Auto,
                    multi_episode_season,
                    scoring_now,
                    match background_value {
                        Some(value) => ScoringLane::Walk {
                            cap: self.background_evaluation_cap(value),
                        },
                        None => ScoringLane::Full,
                    },
                )
                .await?;
            scored_results = batch.results;
            unroutable_by_indexer = batch.unroutable_by_indexer;
            log_unroutable_releases(caller_label, mode, &unroutable_by_indexer);
            quota_exhausted_by_indexer = batch.quota_exhausted_by_indexer;
            log_quota_exhausted_releases(caller_label, &quota_exhausted_by_indexer);
        }
        if scored_results.is_empty() && successful_searches == 0 && query_failures > 0 {
            let details =
                first_failure.unwrap_or_else(|| "all indexer search queries failed".to_string());
            return Err(AppError::Repository(details));
        }
        let unsupported_indexer_ids =
            unsupported_indexers(&coverage_by_indexer, required_query_count);
        Ok(ScoredSearchOutcome {
            results: scored_results,
            unroutable_by_indexer,
            quota_exhausted_by_indexer,
            complete_indexer_ids: completely_covered_indexers(
                coverage_by_indexer,
                required_query_count,
            ),
            unsupported_indexer_ids,
            incomplete_indexer_reasons,
            search_session_id,
        })
    }

    pub(crate) async fn search_and_evaluate_subject(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        scoring_now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        self.search_and_evaluate_subject_restricted(
            title,
            subject,
            caller_label,
            mode,
            cancel_token,
            None,
            None,
            scoring_now,
        )
        .await
    }

    /// Search and evaluate `subject`, optionally restricted to a subset of
    /// indexers. Automatic acquisition passes the scope's uncovered subset;
    /// a covered indexer's catalog holds no new information for that scope.
    #[expect(
        clippy::too_many_arguments,
        reason = "background search threads the convergence subset and value hint alongside the subject"
    )]
    pub(crate) async fn search_and_evaluate_subject_restricted(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
        scoring_now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        Ok(self
            .search_and_evaluate_subject_restricted_with_outcome(
                title,
                subject,
                caller_label,
                mode,
                cancel_token,
                restrict_to_indexer_ids,
                background_value,
                scoring_now,
            )
            .await?
            .results)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "interactive search also needs the restricted indexer's completion status"
    )]
    pub(crate) async fn search_and_evaluate_subject_restricted_with_outcome(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
        scoring_now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<ScoredSearchOutcome> {
        self.search_and_evaluate_subject_with_context(
            title,
            subject,
            caller_label,
            mode,
            cancel_token,
            restrict_to_indexer_ids,
            background_value,
            scoring_now,
            &mut WalkSearchContext::default(),
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the walk shares preparation while each subject keeps its search inputs"
    )]
    pub(crate) async fn search_and_evaluate_subject_with_context(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
        scoring_now: chrono::DateTime<chrono::Utc>,
        context: &mut WalkSearchContext,
    ) -> AppResult<ScoredSearchOutcome> {
        let mut outcome = self
            .search_and_score_subject_with_context(
                title,
                subject,
                caller_label,
                mode,
                cancel_token,
                restrict_to_indexer_ids,
                background_value,
                scoring_now,
                context,
            )
            .await?;
        // A background search ties its session to the convergence scope, so
        // an indexer that ended contained is held there until it is due.
        if background_value.is_some()
            && let Some(convergence) = self.resolve_scope_convergence(title, subject).await
        {
            self.link_search_session_coverage_scope(
                &outcome.search_session_id,
                &convergence.scope_key,
            )
            .await;
        }
        let local_reads = crate::acquisition::title_reads::TitleCatalogReads::new(&title.id);
        let reads = context.reads.as_deref().unwrap_or(&local_reads);
        outcome.results = self
            .evaluate_search_results_for_subject_with_reads(
                title,
                subject,
                outcome.results,
                // A background value is the mark of the convergence sweep; a
                // search without one was started by an operator even when it
                // runs auto decisioning (`queue_best_release`), and Sonarr
                // skips the monitored check for exactly those searches.
                matches!(mode, SearchMode::Interactive) || background_value.is_none(),
                reads,
            )
            .await?;
        if self
            .finalize_evaluated_search_session_or_warn(
                &outcome.search_session_id,
                &outcome.results,
                &title.id,
            )
            .await
        {
            self.record_search_coverage(
                title,
                subject,
                &outcome.complete_indexer_ids,
                &outcome.unsupported_indexer_ids,
            )
            .await;
        }
        Ok(outcome)
    }

    /// Search and score without writing generic scope coverage. The series-pack
    /// title lane uses this to record its set and qualifying collection keys;
    /// callers keep the operation's catalog and scoring preparation together.
    #[expect(
        clippy::too_many_arguments,
        reason = "background search threads the convergence subset and value hint alongside the subject"
    )]
    #[cfg(test)]
    pub(crate) async fn search_and_score_subject_restricted_with_fired_indexers(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
        scoring_now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<ScoredSearchOutcome> {
        self.search_and_score_subject_with_context(
            title,
            subject,
            caller_label,
            mode,
            cancel_token,
            restrict_to_indexer_ids,
            background_value,
            scoring_now,
            &mut WalkSearchContext::default(),
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the walk shares preparation while each subject keeps its search inputs"
    )]
    pub(crate) async fn search_and_score_subject_with_context(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
        scoring_now: chrono::DateTime<chrono::Utc>,
        context: &mut WalkSearchContext,
    ) -> AppResult<ScoredSearchOutcome> {
        let languages = match title.search_languages_override() {
            Some(languages) => languages,
            None => self
                .read_setting_json_value::<Vec<String>>(
                    crate::SEARCH_LANGUAGES_KEY,
                    Some(&title.library_id),
                )
                .await?
                .unwrap_or_default(),
        };
        // Writes are validated; a stored value from outside the picker (a
        // hand-edited tag or setting) is ignored rather than searched.
        let languages = languages
            .iter()
            .filter_map(|language| crate::normalize_search_language_code(language))
            .collect::<Vec<_>>();
        let names = localized_search_names(title, &languages);
        let shared_reads = context.reads.clone();
        let mut outcome = self
            .search_and_score_subject_query_round(
                title,
                subject,
                caller_label,
                mode,
                cancel_token.clone(),
                restrict_to_indexer_ids.clone(),
                background_value,
                scoring_now,
                context,
            )
            .await?;
        let mut listing_keys = outcome
            .results
            .iter()
            .map(|candidate| (candidate.indexer_id.clone(), release_search_key(candidate)))
            .collect::<HashSet<_>>();
        for name in names {
            if context.should_yield() {
                // The skipped alias has no answer. Retain candidates already
                // obtained, but never claim this incomplete scope is covered.
                outcome.complete_indexer_ids.clear();
                outcome.unsupported_indexer_ids.clear();
                break;
            }
            if cancel_token.is_cancelled() {
                return Err(AppError::canceled("indexer search canceled"));
            }
            if mode == SearchMode::Auto {
                let local_reads =
                    crate::acquisition::title_reads::TitleCatalogReads::new(&title.id);
                let reads = shared_reads.as_deref().unwrap_or(&local_reads);
                let evaluated = self
                    .evaluate_search_results_for_subject_with_reads(
                        title,
                        subject,
                        outcome.results.clone(),
                        background_value.is_none(),
                        reads,
                    )
                    .await?;
                if evaluated.iter().any(candidate_is_reusable) {
                    break;
                }
            }
            let mut localized = subject.clone();
            localized.queries = localized_search_queries(&title.name, &name, &subject.queries);
            if localized.queries.is_empty() {
                continue;
            }
            // IDs have already been tried. This round asks the indexer a new
            // textual question and cannot be collapsed into its ID strategy.
            localized.imdb_id = None;
            localized.tmdb_id = None;
            localized.tvdb_id = None;
            localized.anidb_id = None;
            localized.mal_id = None;
            let next = self
                .search_and_score_subject_query_round(
                    title,
                    &localized,
                    caller_label,
                    mode,
                    cancel_token.clone(),
                    restrict_to_indexer_ids.clone(),
                    background_value,
                    scoring_now,
                    context,
                )
                .await;
            let mut next = match next {
                Ok(next) => next,
                Err(error) if error.is_canceled() => return Err(error),
                Err(error) => {
                    for id in outcome.complete_indexer_ids.drain(..) {
                        outcome.incomplete_indexer_reasons.insert(
                            id,
                            IncompleteIndexerReason::failed(format!(
                                "localized query for {name} failed: {error}"
                            )),
                        );
                    }
                    outcome.unsupported_indexer_ids.clear();
                    warn!(title_id = title.id, query_name = name, error = %error, "localized query failed; retaining earlier results");
                    continue;
                }
            };
            let local_reads = crate::acquisition::title_reads::TitleCatalogReads::new(&title.id);
            let reads = shared_reads.as_deref().unwrap_or(&local_reads);
            let evaluated = self
                .evaluate_search_results_for_subject_with_reads(
                    title,
                    subject,
                    next.results.clone(),
                    mode == SearchMode::Interactive || background_value.is_none(),
                    reads,
                )
                .await?;
            if !self
                .finalize_evaluated_search_session_or_warn(
                    &next.search_session_id,
                    &evaluated,
                    &title.id,
                )
                .await
            {
                next.complete_indexer_ids.clear();
            }
            for candidate in &mut next.results {
                candidate
                    .extra
                    .insert("searched_title".into(), Value::String(name.clone()));
            }
            outcome
                .results
                .extend(next.results.into_iter().filter(|candidate| {
                    listing_keys
                        .insert((candidate.indexer_id.clone(), release_search_key(candidate)))
                }));
            merge_localized_round_coverage(
                &mut outcome.complete_indexer_ids,
                &mut outcome.unsupported_indexer_ids,
                &next.complete_indexer_ids,
                &next.unsupported_indexer_ids,
            );
            outcome
                .incomplete_indexer_reasons
                .extend(next.incomplete_indexer_reasons);
            for (indexer_id, counts) in next.unroutable_by_indexer {
                let merged = outcome.unroutable_by_indexer.entry(indexer_id).or_default();
                for (reason, count) in counts {
                    *merged.entry(reason).or_default() += count;
                }
            }
            for (indexer_id, count) in next.quota_exhausted_by_indexer {
                *outcome
                    .quota_exhausted_by_indexer
                    .entry(indexer_id)
                    .or_default() += count;
            }
        }
        outcome.results.sort_by(compare_release_search_results);
        Ok(outcome)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one localized round preserves the original search scope"
    )]
    async fn search_and_score_subject_query_round(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<HashSet<String>>,
        background_value: Option<f64>,
        scoring_now: chrono::DateTime<chrono::Utc>,
        context: &mut WalkSearchContext,
    ) -> AppResult<ScoredSearchOutcome> {
        let tagged_aliases = release_search_tagged_aliases(title);
        self.search_and_score_releases_with_context(
            ReleaseSearchRequest {
                queries: subject.queries.clone(),
                imdb_id: subject.imdb_id.clone(),
                tmdb_id: subject.tmdb_id.clone(),
                tvdb_id: subject.tvdb_id.clone(),
                anidb_id: subject.anidb_id.clone(),
                mal_id: subject.mal_id.clone(),
                category: Some(subject.category.clone()),
                owner_facet: subject.owner_facet.clone(),
                search_facet: subject.search_facet.clone(),
                id_search_facet: subject.id_search_facet.clone(),
                newznab_categories: subject.newznab_categories.clone(),
                title_id: subject.title_id.as_str(),
                title_tags: &subject.title_tags,
                library_id: Some(title.library_id.as_str()),
                caller_label,
                mode,
                runtime_minutes: subject.runtime_minutes,
                // The subject carries season 0 so the acceptance layer can veto a
                // season-1 release for a special; `&season=0` is not a query an
                // indexer answers, so it is dropped here and the search runs on the
                // title/absolute queries the specials lane already builds.
                season: subject.season.filter(|season| *season > 0),
                episode: subject.episode,
                numbering_context: subject.numbering_context.clone(),
                absolute_episode: subject.absolute_episode,
                // `title` here is the *search* title — for a series-movie link it is
                // the derived movie record, whose facet and year both come from the
                // movie. Reading the year and the facet gate off the same record is
                // what keeps a series year from ever being sent as a release year.
                year: (title.facet == MediaFacet::Movie)
                    .then_some(title.year)
                    .flatten(),
                tagged_aliases: &tagged_aliases,
                search_subject_kind: subject.subject_kind,
                // Only an operator's whole-season search narrows its list to
                // multi-episode releases; the automatic season-pack walk applies
                // its own pack check when it picks a grab.
                multi_episode_season: (mode == SearchMode::Interactive
                    && subject.subject_kind == ReleaseSearchSubjectKind::Season)
                    .then_some(subject.season)
                    .flatten(),
                parse_context: &subject.title_evidence.parse_context,
                cancel_token,
                restrict_to_indexer_ids,
                background_value,
                scoring_now,
            },
            context,
        )
        .await
    }

    pub(crate) async fn finalize_evaluated_search_session(
        &self,
        search_session_id: &str,
        evaluated: &[IndexerSearchResult],
    ) -> AppResult<()> {
        let mut fingerprints = evaluated
            .iter()
            .filter(|candidate| candidate_is_reusable(candidate))
            .map(release_candidate_fingerprint)
            .collect::<Vec<_>>();
        fingerprints.sort_unstable();
        fingerprints.dedup();
        self.services
            .integrations
            .indexer_client
            .finalize_search_session(search_session_id, &fingerprints)
            .await
    }

    /// Finalization is retention bookkeeping and must never veto acquisition:
    /// a search that just returned live candidates still grabs, whatever the
    /// corpus tables did. `false` means the session was not finalized — the
    /// caller must then withhold convergence coverage, because coverage
    /// without a pruned corpus behind it is the claim the retention model
    /// forbids. Nothing else is owed: the staged runs stay `received_*`, which
    /// excludes them from reuse, and the scope simply re-searches next cycle.
    pub(crate) async fn finalize_evaluated_search_session_or_warn(
        &self,
        search_session_id: &str,
        evaluated: &[IndexerSearchResult],
        title_id: &str,
    ) -> bool {
        match self
            .finalize_evaluated_search_session(search_session_id, evaluated)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                warn!(
                    title_id,
                    search_session_id,
                    error = %error,
                    "search candidate cache finalization failed; withholding coverage and continuing"
                );
                false
            }
        }
    }

    /// Interactive search for a title (movie or standalone). Resolves all
    /// external IDs and search category from the title record so the frontend
    /// only needs to pass the title ID.
    pub(crate) async fn attach_candidate_tokens(
        &self,
        actor: &User,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        results: &mut [IndexerSearchResult],
        preserve_subject_scope: bool,
    ) {
        let signing_key = match self.release_candidate_signing_key_for_actor(actor).await {
            Ok(signing_key) => signing_key,
            Err(err) => {
                warn!(
                    actor = actor.id.as_str(),
                    title_id = title.id.as_str(),
                    scope = ?subject.submission_scope,
                    error = %err,
                    "failed to resolve candidate-token signing key for title-aware search"
                );
                for result in results.iter_mut() {
                    result.candidate_token = None;
                }
                return;
            }
        };

        let catalog_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        let catalog_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let requested_episode = resolve_requested_episode(
            &catalog_episodes,
            subject.season,
            subject.episode,
            subject.absolute_episode,
        );

        let now = self.runtime.environment.now();

        for result in results.iter_mut() {
            let scope = if preserve_subject_scope {
                subject.submission_scope.clone()
            } else {
                result
                    .parsed_release_metadata
                    .as_ref()
                    .map(|parsed| {
                        crate::acquisition_coverage::resolve_release_coverage(
                            parsed,
                            &catalog_episodes,
                            &catalog_collections,
                            requested_episode,
                        )
                        .submission_scope_or(&subject.submission_scope)
                    })
                    .unwrap_or_else(|| subject.submission_scope.clone())
            };
            result.queue_scope = Some(scope.clone());
            let canonical_source = result.canonical_download_source();
            let selection = QueuedReleaseSelection {
                indexer_id: result.indexer_id.clone(),
                source_hint: canonical_source.as_ref().map(|(source, _)| source.clone()),
                source_kind: canonical_source
                    .as_ref()
                    .map(|(_, kind)| *kind)
                    .or(result.source_kind),
                source_title: Some(result.title.clone()),
                source_password: result.password_hint.clone(),
                info_hash_hint: result
                    .extra
                    .get("info_hash")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                size_bytes: result.size_bytes,
                seeders: crate::acquisition::seed_goals::seeders_from_extra(&result.extra),
            };
            result.candidate_token = if selection.source_hint.is_some() {
                match self.issue_release_candidate_token_with_signing_key(
                    actor,
                    &title.id,
                    &scope,
                    &selection,
                    Some(
                        crate::quality::release_listing::ReleaseListingSnapshot::capture_from_search_result(
                            result, now,
                        ),
                    ),
                    result.http.clone(),
                    &signing_key,
                ) {
                    Ok(token) => Some(token),
                    Err(err) => {
                        warn!(
                            actor = actor.id.as_str(),
                            title_id = title.id.as_str(),
                            scope = ?scope,
                            release_title = result.title.as_str(),
                            error = %err,
                            "failed to attach candidate token to title-aware search result"
                        );
                        None
                    }
                }
            } else {
                None
            };
        }
    }

    pub async fn search_indexers_for_title(
        &self,
        actor: &User,
        title_id: String,
        cancel_token: CancellationToken,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {title_id}"),
                )
            })?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let subject = self
            .resolve_release_search_subject_for_title(&title)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for title"
        );

        let mut results = self
            .search_and_evaluate_subject(
                &title,
                &subject,
                &actor.id,
                SearchMode::Interactive,
                cancel_token,
                self.runtime.environment.now(),
            )
            .await?;
        self.attach_candidate_tokens(actor, &title, &subject, &mut results, false)
            .await;

        Ok(results)
    }

    pub async fn search_indexers_for_series_movie(
        &self,
        actor: &User,
        title_id: String,
        series_movie_link_id: String,
        cancel_token: CancellationToken,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {title_id}"),
                )
            })?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let link = self
            .services
            .catalog
            .shows
            .get_series_movie_link_by_id(&series_movie_link_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::SERIES_MOVIE_NOT_FOUND,
                    format!("series movie {series_movie_link_id}"),
                )
            })?;
        if link.series_title_id != title.id {
            return Err(AppError::validation_refused(
                crate::error_reasons::catalog::SERIES_MOVIE_DOES_NOT_BELONG_TITLE,
                "series movie does not belong to title",
            ));
        }

        let (search_title, subject) = self
            .resolve_release_search_subject_for_series_movie(&title, &link)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            series_movie_link_id = series_movie_link_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for series movie"
        );

        let mut results = self
            .search_and_evaluate_subject(
                &search_title,
                &subject,
                &actor.id,
                SearchMode::Interactive,
                cancel_token,
                self.runtime.environment.now(),
            )
            .await?;
        self.attach_candidate_tokens(actor, &search_title, &subject, &mut results, true)
            .await;

        Ok(results)
    }

    /// Interactive search for a specific episode. Resolves all external IDs,
    /// search category, and absolute episode number from the title/episode
    /// records so the frontend only needs to pass title ID + season + episode.
    pub async fn search_indexers_for_episode(
        &self,
        actor: &User,
        title_id: String,
        season: String,
        episode: String,
        cancel_token: CancellationToken,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {title_id}"),
                )
            })?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let subject = self
            .resolve_release_search_subject_for_episode(&title, &season, &episode)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for episode"
        );

        let mut results = self
            .search_and_evaluate_subject(
                &title,
                &subject,
                &actor.id,
                SearchMode::Interactive,
                cancel_token,
                self.runtime.environment.now(),
            )
            .await?;
        self.attach_candidate_tokens(actor, &title, &subject, &mut results, false)
            .await;

        Ok(results)
    }
}

/// Upper bound on blocklist entries read per title for search-time exclusion.
const TITLE_RELEASE_BLOCKLIST_READ_LIMIT: usize = 1_000;

/// A title's blocked releases, keyed the way [`is_release_blocklisted`] keys a
/// candidate. The per-title `blocklist` table is the single search-time
/// exclusion source; `release_download_attempts` is history only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleReleaseBlocklistSignatures {
    /// `(indexer_id, normalized_release_name)`. An empty indexer id blocks the
    /// name on every indexer -- see [`scryer_domain::BlocklistEntry::indexer_id`].
    pub(crate) release_names: HashSet<(String, String)>,
    /// Infohashes of blocked torrents, which match regardless of indexer.
    pub(crate) info_hashes: HashSet<String>,
}

impl AppUseCase {
    /// Loads the title's blocked releases. A repository error warns and yields
    /// an empty set: a storage hiccup degrades to "nothing excluded" rather
    /// than failing the search.
    pub(crate) async fn load_title_release_blocklist_signatures(
        &self,
        title_id: &str,
    ) -> TitleReleaseBlocklistSignatures {
        let entries = match self
            .services
            .workflow
            .blocklist_repo
            .list_for_title(title_id, TITLE_RELEASE_BLOCKLIST_READ_LIMIT)
            .await
        {
            Ok(entries) => entries,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id,
                    "failed to load title release blocklist; excluding nothing this search"
                );
                Vec::new()
            }
        };

        let mut signatures = TitleReleaseBlocklistSignatures::default();
        for entry in entries {
            if let Some(info_hash) = entry.info_hash {
                signatures.info_hashes.insert(info_hash);
            }
            signatures
                .release_names
                .insert((entry.indexer_id, entry.normalized_release_name));
        }
        signatures
    }
}

/// Whether this release is blocked for the title these signatures belong to.
///
/// Infohash first -- content identity is the same wherever the torrent came
/// from -- then the release name scoped to the indexer offering it. A signature
/// carrying an empty indexer id blocks the name on every indexer.
pub(crate) fn is_release_blocklisted(
    indexer_id: Option<&str>,
    release_name: &str,
    info_hash: Option<&str>,
    blocklist: &TitleReleaseBlocklistSignatures,
) -> bool {
    if let Some(info_hash) = scryer_plugin_sdk::torrent::normalize_info_hash(info_hash)
        && blocklist.info_hashes.contains(&info_hash)
    {
        return true;
    }
    let Some(release_name) = normalize_release_name(Some(release_name)) else {
        return false;
    };
    blocklist
        .release_names
        .contains(&(String::new(), release_name.clone()))
        || indexer_id.is_some_and(|indexer_id| {
            blocklist
                .release_names
                .contains(&(indexer_id.trim().to_string(), release_name.clone()))
        })
}

#[derive(Clone, Copy)]
pub(crate) struct QualityProfileLookup<'a> {
    pub(crate) title_tags: &'a [String],
    pub(crate) library_id: Option<&'a str>,
    pub(crate) imdb_id: Option<&'a str>,
    pub(crate) tvdb_id: Option<&'a str>,
    pub(crate) category_hint: Option<&'a str>,
}

pub(crate) struct ReleaseSearchRequest<'a> {
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) mal_id: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) owner_facet: MediaFacet,
    pub(crate) search_facet: MediaFacet,
    pub(crate) id_search_facet: Option<MediaFacet>,
    pub(crate) newznab_categories: Vec<String>,
    pub(crate) title_id: &'a str,
    pub(crate) title_tags: &'a [String],
    pub(crate) library_id: Option<&'a str>,
    pub(crate) caller_label: &'a str,
    pub(crate) mode: SearchMode,
    pub(crate) runtime_minutes: Option<i32>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) numbering_context: crate::IndexerSearchNumberingContext,
    pub(crate) absolute_episode: Option<u32>,
    /// Movie release year, when the searched title is a movie that has one.
    /// Series years never travel here: a season/episode search has no single
    /// release year, so passing the series year would be a guess.
    pub(crate) year: Option<i32>,
    pub(crate) tagged_aliases: &'a [TaggedAlias],
    pub(crate) search_subject_kind: ReleaseSearchSubjectKind,
    /// An operator's whole-season search: keep only releases of several
    /// episodes that include this season. `None` for every other search.
    pub(crate) multi_episode_season: Option<u32>,
    pub(crate) parse_context: &'a ReleaseParseContext,
    pub(crate) cancel_token: CancellationToken,
    /// When set, only these indexer ids are queried (the convergence cursor's
    /// uncovered subset). `None` = every routed indexer.
    pub(crate) restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
    /// Background convergence value hint: the target's recency
    /// lane maps to a scheduler candidate value (hot → high, cold → low) so
    /// the quota-pressure gate can drain cold work first. Only the Auto
    /// background path carries it; interactive/RSS leave it `None` (neutral).
    pub(crate) background_value: Option<f64>,
    /// The lane's clock for this search. Candidates are scored against it and
    /// the listing snapshot they carry is captured at it, so a grab or park
    /// stamped with the same instant persists exactly what was scored.
    pub(crate) scoring_now: chrono::DateTime<chrono::Utc>,
}

/// The configured scope that supplied an effective quality profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QualityProfileResolutionSource {
    Title,
    Library,
    Category,
    Global,
    Builtin,
}

impl QualityProfileResolutionSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Library => "library",
            Self::Category => "category",
            Self::Global => "global",
            Self::Builtin => "builtin",
        }
    }
}

/// Effective profile data and the precedence scope that selected it.
#[derive(Clone, Debug)]
pub(crate) struct QualityProfileResolution {
    pub(crate) profile: QualityProfile,
    pub(crate) profile_id: String,
    pub(crate) source: QualityProfileResolutionSource,
}

/// An indexer routing plan and whether any settings read behind it failed.
pub(crate) struct ObservedIndexerRouting {
    pub(crate) plan: Option<IndexerRoutingPlan>,
    pub(crate) read_failed: bool,
}

impl AppUseCase {
    pub(crate) async fn resolve_quality_profile(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> AppResult<QualityProfile> {
        let resolution = self.resolve_quality_profile_resolution(lookup).await?;
        debug!(
            quality_profile_id = resolution.profile_id,
            quality_profile_source = resolution.source.as_str(),
            "resolved quality profile"
        );
        Ok(resolution.profile)
    }

    /// Resolves the effective profile together with the precedence scope that
    /// supplied it. Callers that only need scoring can use
    /// [`Self::resolve_quality_profile`]; diagnostics should retain this
    /// provenance instead of attempting to recompute it later.
    pub(crate) async fn resolve_quality_profile_resolution(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> AppResult<QualityProfileResolution> {
        let catalog = self.load_quality_profiles().await?;
        let category_scope_id = self.quality_profile_scope_id(lookup);

        let title_profile_id = lookup
            .title_tags
            .iter()
            .find(|t| t.starts_with("scryer:quality-profile:"))
            .map(|t| {
                t.trim_start_matches("scryer:quality-profile:")
                    .trim()
                    .to_string()
            })
            .filter(|value| !value.is_empty() && value != QUALITY_PROFILE_INHERIT_VALUE);

        let category_profile_id = self
            .read_setting_string_value_explicit(
                QUALITY_PROFILE_ID_KEY,
                category_scope_id.as_deref(),
            )
            .await?;
        let library_profile_id = match lookup.library_id {
            Some(library_id) => {
                self.read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(library_id))
                    .await?
            }
            None => None,
        };
        let global_profile_id = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?;

        let (active_profile_id, source) = if let Some(profile_id) = title_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Title)
        } else if let Some(profile_id) = library_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Library)
        } else if let Some(profile_id) = category_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Category)
        } else if let Some(profile_id) = global_profile_id {
            (Some(profile_id), QualityProfileResolutionSource::Global)
        } else {
            (None, QualityProfileResolutionSource::Builtin)
        };

        if let Some(profile_id) = active_profile_id {
            let profile = crate::settings::runtime::quality_profile_by_id(&catalog, &profile_id)?
                .cloned()
                .ok_or_else(|| {
                    AppError::validation_refused(crate::error_reasons::catalog::CONFIGURED_QUALITY_PROFILE_MISSING_CATALOG, format!(
                        "configured quality profile '{profile_id}' from {} is missing from the catalog",
                        source.as_str()
                    ))
                })?;
            return Ok(QualityProfileResolution {
                profile_id: profile.id.clone(),
                profile,
                source,
            });
        }

        if !catalog.is_empty() {
            return Err(AppError::validation_refused(crate::error_reasons::catalog::NO_QUALITY_PROFILE_NOT_CONFIGURED, "no quality profile is configured; choose a global, category, library, or title profile"
                    .to_string()));
        }

        let profile = builtin_default_quality_profile();
        Ok(QualityProfileResolution {
            profile_id: profile.id.clone(),
            profile,
            source: QualityProfileResolutionSource::Builtin,
        })
    }

    async fn load_quality_profiles(&self) -> AppResult<Vec<QualityProfile>> {
        self.services
            .config
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
    }

    pub(crate) async fn read_setting_string_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, key_name, scope_id)
            .await
    }

    pub(crate) async fn read_setting_string_value_explicit(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_for_scope_explicit(SETTINGS_SCOPE_SYSTEM, key_name, scope_id)
            .await
    }

    pub(crate) async fn read_setting_string_value_for_scope(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let scope_id = scope_id.map(std::string::ToString::to_string);
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json(scope, key_name, scope_id)
            .await?
        else {
            return Ok(None);
        };

        Ok(normalize_setting_string_value(&raw_value))
    }

    pub(crate) async fn read_setting_string_value_for_scope_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let scope_id = scope_id.map(std::string::ToString::to_string);
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json_explicit(scope, key_name, scope_id)
            .await?
        else {
            return Ok(None);
        };

        Ok(normalize_setting_string_value(&raw_value))
    }

    pub(crate) fn quality_profile_scope_id(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> Option<String> {
        if let Some(value) = lookup.category_hint {
            let normalized = value.to_ascii_lowercase();
            match normalized.as_str() {
                "movie" => return Some("movie".to_string()),
                "series" => return Some("series".to_string()),
                "anime" => return Some("anime".to_string()),
                "5070" => return Some("series".to_string()),
                _ => {}
            }
        }

        if lookup.imdb_id.is_some() {
            return Some("movie".to_string());
        }
        if lookup.tvdb_id.is_some() {
            return Some("series".to_string());
        }

        None
    }

    /// Resolve Newznab category codes from the user's indexer routing settings
    /// for the given scope_id (movie/series/anime).
    ///
    /// Returns `None` if no routing is configured (caller falls back to
    /// hardcoded defaults). Returns `Some(vec![])` if all indexers are
    /// disabled for this scope (caller should skip search).
    pub(crate) async fn resolve_indexer_routing(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> Option<IndexerRoutingPlan> {
        self.resolve_indexer_routing_observed(library_id, scope_id)
            .await
            .plan
    }

    /// [`Self::resolve_indexer_routing`], also reporting whether a settings
    /// read failed along the way. The plan is the same either way; the flag
    /// lets a memoizing caller decline to keep an answer that came from a
    /// fallback rather than from the stored settings.
    pub(crate) async fn resolve_indexer_routing_observed(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> ObservedIndexerRouting {
        let mut read_failed = false;
        if let Some(library_id) = library_id {
            match self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(library_id))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(plan) = self.parse_indexer_routing_plan(library_id, &value) {
                        return ObservedIndexerRouting {
                            plan: Some(plan),
                            read_failed,
                        };
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    read_failed = true;
                    warn!(
                        error = %err,
                        library_id = library_id,
                        "failed to read library indexer routing setting, falling back to facet defaults"
                    );
                }
            }
        }

        let Some(scope_id) = scope_id else {
            return ObservedIndexerRouting {
                plan: None,
                read_failed,
            };
        };

        let raw_json = match self
            .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
        {
            Ok(Some(value)) => value,
            Ok(None) => {
                return ObservedIndexerRouting {
                    plan: None,
                    read_failed,
                };
            }
            Err(err) => {
                warn!(
                    error = %err,
                    scope_id = scope_id,
                    "failed to read indexer routing setting, falling back to defaults"
                );
                return ObservedIndexerRouting {
                    plan: None,
                    read_failed: true,
                };
            }
        };

        ObservedIndexerRouting {
            plan: self.parse_indexer_routing_plan(scope_id, &raw_json),
            read_failed,
        }
    }

    pub(crate) fn parse_indexer_routing_plan(
        &self,
        scope_id: &str,
        raw_json: &str,
    ) -> Option<IndexerRoutingPlan> {
        let parsed: Value = match serde_json::from_str(raw_json) {
            Ok(value) => value,
            Err(_) => return None,
        };

        let obj = parsed.as_object()?;
        if obj.is_empty() {
            return None;
        }

        let mut entries = std::collections::HashMap::new();

        // The canonical write paths in settings.rs and the startup
        // `normalize_routing_settings` migration always emit `enabled` and
        // `priority`. The `unwrap_or` fallbacks here are transitional
        // legacy-compat for installs that haven't yet been normalized.
        for (indexer_id, config) in obj {
            let enabled = match config.get("enabled").and_then(|v| v.as_bool()) {
                Some(value) => value,
                None => {
                    debug!(
                        scope_id,
                        indexer_id,
                        "indexer routing entry missing `enabled`; using legacy default `true`"
                    );
                    true
                }
            };

            let mut categories: Vec<String> = Vec::new();
            if let Some(cats) = config.get("categories").and_then(|v| v.as_array()) {
                for cat in cats {
                    if let Some(cat_str) = cat.as_str() {
                        let trimmed = cat_str.trim();
                        if !trimmed.is_empty() {
                            categories.push(trimmed.to_string());
                        }
                    }
                }
            }

            let priority = match config.get("priority").and_then(|v| v.as_i64()) {
                Some(value) => value,
                None => {
                    debug!(
                        scope_id,
                        indexer_id,
                        "indexer routing entry missing `priority`; using legacy default `i64::MAX`"
                    );
                    i64::MAX
                }
            };

            entries.insert(
                indexer_id.clone(),
                IndexerRoutingEntry {
                    enabled,
                    categories,
                    priority,
                },
            );
        }

        debug!(
            scope_id = scope_id,
            indexer_count = entries.len(),
            "resolved per-indexer routing plan"
        );
        Some(IndexerRoutingPlan { entries })
    }
}

#[cfg(test)]
mod structured_dispatch_query_tests {
    use super::*;

    fn outcome(indexer_id: &str, outcome: IndexerSearchOutcome) -> IndexerQueryOutcome {
        IndexerQueryOutcome {
            page: None,
            indexer_id: indexer_id.to_string(),
            outcome,
        }
    }

    #[test]
    fn text_safe_dedupe_preserves_distinct_episode_season_absolute_and_title_queries() {
        let queries = vec![
            "Silver Horizon 033".to_string(),
            "Silver Horizon S02E05".to_string(),
            "Silver Horizon S02".to_string(),
            "Silver Horizon".to_string(),
        ];

        let deduped = dedupe_text_safe_structured_dispatch_queries(
            queries.clone(),
            Some(2),
            Some(5),
            Some(33),
        );

        assert_eq!(deduped, queries);
    }

    #[test]
    fn broad_structured_dedupe_still_collapses_equivalent_parameterized_queries() {
        let queries = vec![
            "Silver Horizon 033".to_string(),
            "Silver Horizon S02E05".to_string(),
            "Silver Horizon S02".to_string(),
            "Silver Horizon".to_string(),
        ];

        let deduped = dedupe_structured_dispatch_queries(queries, Some(2), Some(5), Some(33));

        assert_eq!(deduped, vec!["Silver Horizon 033".to_string()]);
    }

    #[test]
    fn coverage_requires_every_effective_query_to_complete() {
        let mut aggregate = HashMap::new();
        record_query_coverage_outcomes(
            &mut aggregate,
            &[
                outcome("complete", IndexerSearchOutcome::Complete { empty: false }),
                outcome("partial", IndexerSearchOutcome::Complete { empty: false }),
                outcome("missing", IndexerSearchOutcome::Complete { empty: true }),
            ],
        );
        record_query_coverage_outcomes(
            &mut aggregate,
            &[
                outcome("complete", IndexerSearchOutcome::Complete { empty: true }),
                outcome(
                    "partial",
                    IndexerSearchOutcome::Partial {
                        empty: false,
                        reason: Some(IndexerSearchIncompleteReason::UpstreamFailure),
                        retry_after: None,
                    },
                ),
            ],
        );

        assert_eq!(
            completely_covered_indexers(aggregate, 2),
            vec!["complete".to_string()]
        );
    }

    #[test]
    fn duplicate_outcomes_cannot_satisfy_multiple_queries() {
        let mut aggregate = HashMap::new();
        record_query_coverage_outcomes(
            &mut aggregate,
            &[
                outcome("duplicate", IndexerSearchOutcome::Complete { empty: false }),
                outcome("duplicate", IndexerSearchOutcome::Complete { empty: true }),
            ],
        );

        assert!(completely_covered_indexers(aggregate, 2).is_empty());
    }

    #[test]
    fn every_incomplete_outcome_withholds_multi_query_coverage() {
        let incomplete = [
            IndexerSearchOutcome::Partial {
                empty: false,
                reason: Some(IndexerSearchIncompleteReason::UpstreamFailure),
                retry_after: None,
            },
            IndexerSearchOutcome::Deferred { retry_after: None },
            IndexerSearchOutcome::Skipped { retry_after: None },
            IndexerSearchOutcome::Unsupported,
            IndexerSearchOutcome::BackedOff { until: None },
            IndexerSearchOutcome::ClientUnavailable {
                reason: "SYNTHETIC_CLIENT_UNAVAILABLE",
            },
            IndexerSearchOutcome::Errored,
        ];

        for incomplete in incomplete {
            let mut aggregate = HashMap::new();
            record_query_coverage_outcomes(
                &mut aggregate,
                &[outcome(
                    "idx",
                    IndexerSearchOutcome::Complete { empty: false },
                )],
            );
            record_query_coverage_outcomes(&mut aggregate, &[outcome("idx", incomplete)]);

            assert!(completely_covered_indexers(aggregate, 2).is_empty());
        }
    }

    #[test]
    fn an_indexer_whose_client_could_not_be_built_fails_with_its_setup_code() {
        let reason = super::incomplete_indexer_reason(IndexerSearchOutcome::ClientUnavailable {
            reason: "SYNTHETIC_CLIENT_UNAVAILABLE",
        })
        .expect("a client that could not be built is an incomplete answer");
        assert_eq!(reason.code, Some("SYNTHETIC_CLIENT_UNAVAILABLE"));
        assert!(!reason.message.is_empty());
        assert!(
            super::incomplete_indexer_reason(IndexerSearchOutcome::Errored)
                .is_some_and(|reason| reason.code.is_none()),
            "an ordinary failure carries no code"
        );

        let mut aggregate = HashMap::new();
        record_query_coverage_outcomes(
            &mut aggregate,
            &[outcome(
                "unloaded",
                IndexerSearchOutcome::ClientUnavailable {
                    reason: "SYNTHETIC_CLIENT_UNAVAILABLE",
                },
            )],
        );
        assert!(
            unsupported_indexers(&aggregate, 1).is_empty(),
            "an unloaded plugin is not reported as lacking text search"
        );
        assert!(completely_covered_indexers(aggregate, 1).is_empty());
    }

    #[test]
    fn an_indexer_is_unsupported_only_when_it_could_answer_no_query() {
        let mut aggregate = HashMap::new();
        record_query_coverage_outcomes(
            &mut aggregate,
            &[
                outcome("unsupported", IndexerSearchOutcome::Unsupported),
                outcome("mixed", IndexerSearchOutcome::Unsupported),
                outcome("once", IndexerSearchOutcome::Unsupported),
                outcome("complete", IndexerSearchOutcome::Complete { empty: true }),
            ],
        );
        record_query_coverage_outcomes(
            &mut aggregate,
            &[
                outcome("unsupported", IndexerSearchOutcome::Unsupported),
                outcome(
                    "mixed",
                    IndexerSearchOutcome::Deferred { retry_after: None },
                ),
                outcome("complete", IndexerSearchOutcome::Complete { empty: true }),
            ],
        );

        assert_eq!(
            unsupported_indexers(&aggregate, 2),
            vec!["unsupported".to_string()]
        );
        assert_eq!(
            completely_covered_indexers(aggregate, 2),
            vec!["complete".to_string()]
        );
    }
}

#[cfg(test)]
#[path = "app_usecase_discovery_tests.rs"]
mod app_usecase_discovery_tests;

#[cfg(test)]
mod season_multi_episode_filter_tests {
    use super::release_is_multi_episode_for_season;
    use scryer_release_parser::{ContextFacetHint, ContextTitle, ReleaseParseContext};

    fn parse(release: &str, facet_hint: ContextFacetHint) -> crate::ParsedReleaseMetadata {
        let context = ReleaseParseContext {
            facet_hint,
            title: ContextTitle {
                name: "Glass Harbor".into(),
            },
            aliases: Vec::new(),
            known_years: Vec::new(),
            imdb_ids: Vec::new(),
            episodes: Vec::new(),
        };
        crate::parse_release_metadata_for_target(release, &context)
    }

    fn kept_in_season_two(release: &str, facet_hint: ContextFacetHint) -> bool {
        release_is_multi_episode_for_season(parse(release, facet_hint).episode.as_ref(), 2)
    }

    #[test]
    fn season_search_keeps_releases_of_several_episodes_that_include_the_season() {
        for release in [
            "Glass.Harbor.S02.1080p.WEB-DL.DDP5.1.H.264-QUILLFOX",
            "Glass Harbor Season 2 1080p WEB-DL-QUILLFOX",
            "Glass.Harbor.S02E01-E06.1080p.WEB-DL-QUILLFOX",
            "Glass.Harbor.S02E01E02.1080p.WEB-DL-QUILLFOX",
            "Glass.Harbor.S02.Part.1.1080p.WEB-DL-QUILLFOX",
            "Glass.Harbor.S01-S03.1080p.BluRay.x264-QUILLFOX",
            "Glass Harbor Complete Series 1080p BluRay x264-QUILLFOX",
        ] {
            assert!(
                kept_in_season_two(release, ContextFacetHint::Series),
                "{release} should be kept: {:?}",
                parse(release, ContextFacetHint::Series).episode
            );
        }
    }

    #[test]
    fn season_search_keeps_an_absolute_numbered_batch_that_names_no_season() {
        let release = "[Lanternfish] Glass Harbor - 01-12 [1080p]";
        let parsed = parse(release, ContextFacetHint::Anime);
        let episode = parsed.episode.as_ref().expect("batch numbering");
        assert_eq!(
            episode.release_type,
            crate::ParsedEpisodeReleaseType::RangePack
        );
        assert!(release_is_multi_episode_for_season(Some(episode), 2));
    }

    #[test]
    fn season_search_drops_single_episodes_extras_and_other_seasons() {
        for (release, facet_hint) in [
            (
                "Glass.Harbor.S02E05.1080p.WEB-DL-QUILLFOX",
                ContextFacetHint::Series,
            ),
            (
                "Glass.Harbor.2024.03.15.1080p.WEB-DL-QUILLFOX",
                ContextFacetHint::Series,
            ),
            (
                "[Lanternfish] Glass Harbor - 05 [1080p]",
                ContextFacetHint::Anime,
            ),
            (
                "Glass.Harbor.S02.Extras.1080p.BluRay-QUILLFOX",
                ContextFacetHint::Series,
            ),
            (
                "Glass.Harbor.S03.1080p.WEB-DL-QUILLFOX",
                ContextFacetHint::Series,
            ),
            (
                "Glass.Harbor.S03E01-E06.1080p.WEB-DL-QUILLFOX",
                ContextFacetHint::Series,
            ),
            (
                "Glass.Harbor.S03-S05.1080p.BluRay.x264-QUILLFOX",
                ContextFacetHint::Series,
            ),
        ] {
            assert!(
                !kept_in_season_two(release, facet_hint),
                "{release} should be dropped: {:?}",
                parse(release, facet_hint).episode
            );
        }
    }

    #[test]
    fn season_search_keeps_a_release_with_no_episode_numbering() {
        let release = "Glass.Harbor.1080p.WEB-DL-QUILLFOX";
        let parsed = parse(release, ContextFacetHint::Series);
        assert!(parsed.episode.is_none(), "{:?}", parsed.episode);
        assert!(release_is_multi_episode_for_season(
            parsed.episode.as_ref(),
            2
        ));
    }
}

/// Narrows coverage after a localized alias round. An indexer stays
/// complete when the round covered it too, or when the round could not
/// apply to it at all: the alias query carries no external ids, so an
/// id-only indexer that covered the primary round is unsupported there,
/// which says nothing against the primary coverage. An indexer stays
/// unsupported only when every round left it unsupported.
fn merge_localized_round_coverage(
    complete: &mut Vec<String>,
    unsupported: &mut Vec<String>,
    round_complete: &[String],
    round_unsupported: &[String],
) {
    complete.retain(|id| round_complete.contains(id) || round_unsupported.contains(id));
    unsupported.retain(|id| round_unsupported.contains(id));
}

#[cfg(test)]
mod localized_query_tests {
    use super::*;

    #[test]
    fn an_id_only_indexer_keeps_its_primary_coverage_through_an_alias_round() {
        let ids = |values: &[&str]| {
            values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        };
        let mut complete = ids(&["id-only", "text", "dropped"]);
        let mut unsupported = ids(&["never"]);

        merge_localized_round_coverage(
            &mut complete,
            &mut unsupported,
            &ids(&["text"]),
            &ids(&["id-only", "never"]),
        );

        assert_eq!(complete, ids(&["id-only", "text"]));
        assert_eq!(unsupported, ids(&["never"]));
    }

    #[test]
    fn localized_queries_preserve_episode_and_year_boundaries() {
        let queries = vec![
            "Over the Atlantic S11E02".into(),
            "Over the Atlantic 2019".into(),
            "Another Cour 02".into(),
            "Over the AtlanticOcean".into(),
        ];
        assert_eq!(
            localized_search_queries("Over the Atlantic", "Över Atlanten", &queries),
            vec!["Över Atlanten S11E02", "Över Atlanten 2019"]
        );
    }

    fn alias(name: &str, language: &str) -> TaggedAlias {
        TaggedAlias {
            name: name.to_string(),
            language: language.to_string(),
            country: None,
        }
    }

    fn languages(codes: &[&str]) -> Vec<String> {
        codes
            .iter()
            .filter_map(|code| crate::normalize_search_language_code(code))
            .collect()
    }

    fn title_with(aliases: Vec<TaggedAlias>) -> Title {
        Title {
            id: "title-1".to_string(),
            name: "Harbor Lantern".to_string(),
            facet: MediaFacet::Anime,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Anime),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: chrono::DateTime::UNIX_EPOCH,
            year: Some(2022),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: aliases,
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            movie_release_dates: None,
            folder_path: None,
        }
    }

    #[test]
    fn localized_search_names_attribute_an_untagged_kana_alias_to_japanese() {
        let title = title_with(vec![
            alias("ハーバーランタン", ""),
            alias("ほしぞらのかなた", "und"),
        ]);
        assert_eq!(
            localized_search_names(&title, &languages(&["jpn"])),
            vec!["ほしぞらのかなた", "ハーバーランタン"]
        );
        assert!(localized_search_names(&title, &languages(&["kor", "zho"])).is_empty());
    }

    #[test]
    fn localized_search_names_attribute_an_untagged_hangul_alias_to_korean() {
        let title = title_with(vec![alias("항구의 등불", "")]);
        assert_eq!(
            localized_search_names(&title, &languages(&["kor"])),
            vec!["항구의 등불"]
        );
        assert!(localized_search_names(&title, &languages(&["jpn"])).is_empty());
    }

    #[test]
    fn localized_search_names_do_not_make_an_untagged_han_alias_japanese() {
        let title = title_with(vec![alias("星空彼方", "")]);
        assert!(localized_search_names(&title, &languages(&["jpn", "zho"])).is_empty());

        // A country, when the catalog supplies one, settles it.
        let mut title = title;
        title.tagged_aliases[0].country = Some("JP".to_string());
        assert_eq!(
            localized_search_names(&title, &languages(&["jpn"])),
            vec!["星空彼方"]
        );
    }

    #[test]
    fn localized_search_names_search_romaji_aliases_under_japanese() {
        let title = title_with(vec![
            alias("Hoshizora no Kanata", "x-jat"),
            alias("Minato no Akari", "ja-Latn"),
            alias("Hangu Deungbul", "ko-Latn"),
        ]);
        assert_eq!(
            localized_search_names(&title, &languages(&["jpn"])),
            vec!["Hoshizora no Kanata", "Minato no Akari"]
        );
        assert_eq!(
            localized_search_names(&title, &languages(&["kor"])),
            vec!["Hangu Deungbul"]
        );
    }

    /// The romanization tags SMG serves for the catalog's Chinese, Korean and
    /// Thai romanized titles resolve to the language they romanize.
    #[test]
    fn localized_search_names_search_latn_romanizations_under_their_language() {
        let title = title_with(vec![
            alias("Xingkong Bi'an", "zh-Latn"),
            alias("Hangu Deungbul", "ko-Latn"),
            alias("Khom Fai Tha Ruea", "th-Latn"),
        ]);
        assert_eq!(
            localized_search_names(&title, &languages(&["zho", "kor", "tha"])),
            vec!["Xingkong Bi'an", "Hangu Deungbul", "Khom Fai Tha Ruea"]
        );
        assert!(localized_search_names(&title, &languages(&["jpn", "eng"])).is_empty());
    }

    #[test]
    fn localized_search_names_leave_an_untagged_latin_alias_out() {
        let title = title_with(vec![
            alias("Lantern of the Harbor", ""),
            alias("Kanata", "und"),
        ]);
        assert!(
            localized_search_names(&title, &languages(&["eng", "jpn", "kor", "fra"])).is_empty()
        );
    }

    #[test]
    fn localized_search_names_keep_tagged_alias_order_and_exclusions() {
        let mut title = title_with(vec![
            alias("Lanterne du Port", "fr"),
            alias("Hafenlaterne", "deu"),
            alias("Harbor  Lantern", "eng"),
            alias("Ein Hafenlicht", "de-DE"),
            alias("ハーバーランタン", "ja"),
        ]);
        title.tags = vec![format!(
            "{}Harbor Lamp",
            scryer_domain::SEARCH_ALIASES_TAG_PREFIX
        )];
        assert_eq!(
            localized_search_names(&title, &languages(&["deu", "eng", "fra"])),
            vec![
                "Harbor Lamp",
                "Ein Hafenlicht",
                "Hafenlaterne",
                "Lanterne du Port"
            ]
        );
    }

    #[test]
    fn localized_search_names_attribute_every_script_to_its_picker_language() {
        for (name, code) in [
            ("ほしぞら", "jpn"),
            ("항구", "kor"),
            ("Φανός", "ell"),
            ("פנס", "heb"),
            ("โคมไฟ", "tha"),
            ("ფარანი", "kat"),
            ("լապտեր", "hye"),
            ("فانوس", "ara"),
        ] {
            let title = title_with(vec![alias(name, "")]);
            assert_eq!(
                localized_search_names(&title, &languages(&[code])),
                vec![name],
                "{name} under {code}"
            );
        }
    }
}
