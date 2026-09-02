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

struct PreparedReleaseScoringInputs {
    blocklist: TitleReleaseBlocklistSignatures,
    has_usenet_client: bool,
    has_torrent_client: bool,
    preferred_source_kind: String,
    enabled_protocols: Option<(bool, bool)>,
    title: Title,
    canonical_context: crate::quality::canonical_context::ResolvedScoringContext,
    catalog_episodes: Vec<Episode>,
    catalog_collections: Vec<Collection>,
    primary_episode_ids: Option<Option<HashSet<String>>>,
    indexer_priority_by_name: HashMap<String, i64>,
    now: chrono::DateTime<chrono::Utc>,
}

pub(crate) struct ScoredSearchOutcome {
    pub results: Vec<IndexerSearchResult>,
    pub complete_indexer_ids: Vec<String>,
    pub incomplete_indexer_reasons: HashMap<String, String>,
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

fn release_search_tagged_aliases(title: &Title) -> Vec<TaggedAlias> {
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
        None => false,
    }
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

    absolute_episode.and_then(|wanted_absolute| {
        episodes.iter().find(|candidate| {
            candidate
                .absolute_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                == Some(wanted_absolute)
        })
    })
}

#[cfg(test)]
fn extract_indexer_http_status(error: &AppError) -> Option<u16> {
    match error {
        AppError::Repository(message) => extract_http_status_from_message(message),
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

fn normalize_structured_dispatch_query(query: &str, absolute_episode: Option<u32>) -> String {
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
        let removable_structured = removable_numeric || looks_like_structured_query_token(trimmed);
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
        let normalized = normalize_structured_dispatch_query(&query, absolute_episode);
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
}

fn record_query_coverage_outcomes(
    aggregate: &mut HashMap<String, QueryCoverageAggregate>,
    outcomes: &[IndexerQueryOutcome],
) {
    // A provider may contribute more than one internal outcome for a query.
    // Collapse those first so duplicate rows cannot masquerade as completing
    // multiple effective title/alias queries.
    let mut per_query = HashMap::<String, bool>::new();
    for outcome in outcomes {
        per_query
            .entry(outcome.indexer_id.clone())
            .and_modify(|complete| *complete &= outcome.outcome.coverage_eligible())
            .or_insert_with(|| outcome.outcome.coverage_eligible());
    }
    for (indexer_id, query_complete) in per_query {
        aggregate
            .entry(indexer_id)
            .and_modify(|state| {
                state.completed_queries = state.completed_queries.saturating_add(1);
                state.all_complete &= query_complete;
            })
            .or_insert(QueryCoverageAggregate {
                completed_queries: 1,
                all_complete: query_complete,
            });
    }
}

pub(crate) fn incomplete_indexer_reason(outcome: IndexerSearchOutcome) -> Option<String> {
    let (reason, retry_after) = match outcome {
        IndexerSearchOutcome::Complete { .. } => return None,
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::RateLimited),
            retry_after,
            ..
        } => ("indexer search was rate limited", retry_after),
        // Unattested legacy responses are operationally successful; they only
        // withhold convergence coverage until the plugin declares semantics.
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::Unattested),
            ..
        } => return None,
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
        IndexerSearchOutcome::Errored => ("indexer search failed", None),
    };
    Some(match retry_after {
        Some(delay) => format!("{reason}; retry after {}s", delay.as_secs()),
        None => reason.to_string(),
    })
}

fn completely_covered_indexers(
    aggregate: HashMap<String, QueryCoverageAggregate>,
    required_query_count: usize,
) -> Vec<String> {
    if required_query_count == 0 {
        return Vec::new();
    }
    let mut covered = aggregate
        .into_iter()
        .filter_map(|(indexer_id, state)| {
            (state.completed_queries == required_query_count && state.all_complete)
                .then_some(indexer_id)
        })
        .collect::<Vec<_>>();
    covered.sort();
    covered
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
        let normalized = normalize_structured_dispatch_query(&query, absolute_episode);
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

        let routing_entry = routing.and_then(|plan| plan.entries.get(&config.id));
        if routing_entry.is_some_and(|entry| !entry.enabled) {
            continue;
        }

        match config.nab_transport_kind() {
            Some(_) => saw_nab_transport = true,
            None => return false,
        }
    }

    saw_nab_transport
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

impl AppUseCase {
    pub(crate) async fn download_source_capabilities(&self) -> (bool, bool, String) {
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

        (has_usenet, has_torrent, preferred)
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
            false,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "release scoring needs the full search context to produce deterministic ranking decisions"
    )]
    async fn score_release_results_with_prepared(
        &self,
        mut raw_results: Vec<IndexerSearchResult>,
        quality_profile: &QualityProfile,
        title_id: &str,
        // The library and settings scope used to be resolved here; the canonical
        // scoring context resolves them from the title, the same way import
        // does, so passing them separately could only make the two disagree.
        indexer_routing: Option<&IndexerRoutingPlan>,
        // The search `category` and the title's tags used to be read here for
        // audio-language inference; that now lives in
        // `canonical_context::announced_metadata_for_title`, keyed on the
        // title's own facet and tags so every lane infers identically.
        runtime_minutes: Option<i32>,
        parse_context: &ReleaseParseContext,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        prepared: &mut Option<PreparedReleaseScoringInputs>,
        preserve_duplicate_sources: bool,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        if prepared.is_none() {
            let blocklist = self.load_title_release_blocklist_signatures(title_id).await;
            let (has_usenet_client, has_torrent_client, client_preferred_source_kind) =
                self.download_source_capabilities().await;
            let scored_title = self.services.catalog.titles.get_by_id(title_id).await?;
            let Some(scored_title) = scored_title else {
                return Err(AppError::NotFound(format!(
                    "title {title_id} for release scoring"
                )));
            };
            let delay_profiles = self.load_delay_profiles().await;
            let delay_profile = crate::delay_profile::resolve_delay_profile(
                &delay_profiles,
                &scored_title.tags,
                &scored_title.facet,
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
            let catalog_episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(title_id)
                .await
                .unwrap_or_default();
            let catalog_collections = self
                .services
                .catalog
                .shows
                .list_collections_for_title(title_id)
                .await
                .unwrap_or_default();
            let indexer_priority_by_name =
                self.build_indexer_priority_by_name(indexer_routing).await;
            *prepared = Some(PreparedReleaseScoringInputs {
                blocklist,
                has_usenet_client,
                has_torrent_client,
                preferred_source_kind,
                enabled_protocols,
                title: scored_title,
                canonical_context,
                catalog_episodes,
                catalog_collections,
                primary_episode_ids: None,
                indexer_priority_by_name,
                now: chrono::Utc::now(),
            });
        }
        let prepared = prepared
            .as_mut()
            .expect("release scoring inputs initialized");

        raw_results.retain(|result| match result.source_kind {
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => {
                prepared.has_usenet_client
            }
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                prepared.has_torrent_client
            }
            None => true,
        });
        let requested_episode = resolve_requested_episode(
            &prepared.catalog_episodes,
            season,
            episode,
            absolute_episode,
        );
        let mut scored = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut rank_by_key: HashMap<String, crate::acquisition::scoring::SearchRank> =
            HashMap::new();

        for result in raw_results {
            let key = release_search_key(&result);
            if !seen.insert(key) {
                continue;
            }

            if is_release_blocklisted(
                result.indexer_id.as_deref(),
                &result.title,
                result.info_hash(),
                &prepared.blocklist,
            ) {
                continue;
            }

            let parsed_release_metadata =
                parse_release_metadata_for_target(&result.title, parse_context);
            let scored_release_metadata =
                crate::quality::canonical_context::announced_metadata_for_title(
                    &prepared.title,
                    &parsed_release_metadata,
                    prepared.canonical_context.required_audio_languages(),
                    result.indexer_languages.as_deref(),
                );

            let is_series_pack = scored_release_metadata
                .episode
                .as_ref()
                .is_some_and(|episode| episode.is_series_pack);
            let pack_below_missing_threshold = if is_series_pack {
                if prepared.primary_episode_ids.is_none() {
                    prepared.primary_episode_ids = Some(
                        match self
                            .services
                            .library
                            .media_files
                            .list_media_files_for_title(title_id)
                            .await
                        {
                            Ok(files) => Some(
                                files
                                    .into_iter()
                                    .filter(|file| file.role.is_primary())
                                    .filter_map(|file| file.episode_id)
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
                        },
                    );
                }
                !prepared
                    .primary_episode_ids
                    .as_ref()
                    .and_then(Option::as_ref)
                    .is_some_and(|primary_episode_ids| {
                        crate::acquisition_coverage::series_pack_missing_ratio_qualifies(
                            &scored_release_metadata,
                            &prepared.catalog_episodes,
                            primary_episode_ids,
                        )
                    })
            } else {
                false
            };

            let release_coverage = crate::acquisition_coverage::resolve_release_coverage(
                &scored_release_metadata,
                &prepared.catalog_episodes,
                &prepared.catalog_collections,
                requested_episode,
            );
            if let Some(wanted_episode) = requested_episode
                && !release_coverage.covers_episode(wanted_episode)
            {
                continue;
            }
            let candidate_size_basis = crate::acquisition_coverage::coverage_size_basis(
                &release_coverage,
                &scored_release_metadata,
                &prepared.catalog_episodes,
                runtime_minutes,
            );

            if requested_episode.is_none()
                && let Some(ref ep_meta) = scored_release_metadata.episode
                && let Some(wanted_season) = season
                && let Some(parsed_season) = ep_meta.season
                && parsed_season != wanted_season
            {
                continue;
            }
            if requested_episode.is_none()
                && let Some(ref ep_meta) = scored_release_metadata.episode
                && let Some(wanted_episode) = episode
            {
                if !ep_meta.episode_numbers.is_empty()
                    && !ep_meta.episode_numbers.contains(&wanted_episode)
                {
                    continue;
                }
                if ep_meta.episode_numbers.is_empty()
                    && ep_meta.absolute_episode_numbers.is_empty()
                    && let (Some(parsed_abs), Some(wanted_abs)) =
                        (ep_meta.absolute_episode, absolute_episode)
                    && parsed_abs != wanted_abs
                {
                    continue;
                }
            }

            // One canonical score, from the same function and the same resolved
            // context the import path uses. Everything that used to be added
            // here and nowhere else — the freshness bonus, the single-episode
            // pack penalty, the listing-metadata rule inputs — is gone from the
            // number; what survives of it orders the results, in
            // `acquisition::scoring`, and never crosses a comparison.
            let scored_release = crate::canonical_scoring::score_release(
                &crate::canonical_scoring::ReleaseEvidence::announced(
                    scored_release_metadata.clone(),
                    result.size_bytes,
                ),
                &prepared.canonical_context.view(candidate_size_basis, false),
            );
            let decision = scored_release.announced_decision;

            // Rank is built here, where the listing and the coverage are still
            // in hand, and dropped when the search ends. It is keyed by release
            // rather than carried on the result so it cannot leak into anything
            // that gets stored or compared later.
            let protocol_enabled = prepared.enabled_protocols.is_none_or(|(usenet, torrent)| {
                match result.source_kind {
                    Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => usenet,
                    Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                        torrent
                    }
                    None => true,
                }
            });
            rank_by_key.insert(
                release_search_key(&result),
                crate::acquisition::scoring::SearchRank {
                    head: crate::acquisition::scoring::RankHead {
                        blocked: !decision.allowed
                            || pack_below_missing_threshold
                            || !protocol_enabled,
                        tier_index: decision.tier_index.unwrap_or(usize::MAX),
                        negated_revision: -(i32::from(scored_release_metadata.is_proper_upload)
                            + i32::from(scored_release_metadata.is_repack)),
                        negated_score: decision.preference_score.saturating_neg(),
                    },
                    non_preferred_protocol: !source_kind_matches_preference(
                        &result,
                        &prepared.preferred_source_kind,
                    ),
                    coverage_distance: release_coverage.coverage_distance(requested_episode),
                    episode_number: scored_release_metadata
                        .episode
                        .as_ref()
                        .and_then(|episode| episode.episode_numbers.iter().min().copied())
                        .unwrap_or(0),
                    indexer_priority: source_grab_sort_priority(
                        &result,
                        prepared
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
                            prepared.now,
                        )
                    } else {
                        0
                    },
                    negated_size_bytes: result.size_bytes.unwrap_or_default().saturating_neg(),
                },
            );

            let mut scored_result = IndexerSearchResult {
                parsed_release_metadata: Some(scored_release_metadata),
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
                ..result
            };
            if pack_below_missing_threshold {
                crate::acquisition_release_search::annotate_auto_decision(
                    &mut scored_result,
                    crate::acquisition_release_search::ReleaseAutoDecisionCode::PackBelowMissingThreshold,
                );
            }
            scored.push(scored_result);
        }

        let mut scored = if preserve_duplicate_sources {
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
        scored.truncate(200);

        Ok(scored)
    }

    /// Internal search+score pipeline shared by both user-facing search and background acquisition.
    /// Returns the scored releases plus the set of indexer ids that actually
    /// **fired** a query and returned a response (empty included), aggregated across
    /// all queries. The fired
    /// set — never the routed set — is what background acquisition records as
    /// Convergence coverage.
    pub(crate) async fn search_and_score_releases(
        &self,
        request: ReleaseSearchRequest<'_>,
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
            absolute_episode,
            year,
            tagged_aliases,
            search_subject_kind,
            cancel_token,
            restrict_to_indexer_ids,
            background_value,
        } = request;
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
        // Restrict the search to the requested indexer subset (the convergence
        // cursor's uncovered indexers). With no routing plan
        // configured, synthesize one over the enabled indexers so the
        // restriction still applies.
        if let Some(allowed) = restrict_to_indexer_ids.as_ref() {
            let mut plan = match indexer_routing.take() {
                Some(plan) => plan,
                None => crate::contracts::IndexerRoutingPlan {
                    entries: self
                        .services
                        .integrations
                        .indexer_configs
                        .list(None)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|config| config.is_enabled)
                        .map(|config| {
                            (
                                config.id,
                                crate::contracts::IndexerRoutingEntry {
                                    enabled: true,
                                    categories: Vec::new(),
                                    priority: 0,
                                },
                            )
                        })
                        .collect(),
                },
            };
            for (indexer_id, entry) in plan.entries.iter_mut() {
                if !allowed.contains(indexer_id) {
                    entry.enabled = false;
                }
            }
            indexer_routing = Some(plan);
        }
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

        // If routing exists and every indexer is disabled, skip the search entirely.
        if let Some(ref plan) = indexer_routing {
            let any_enabled = plan.entries.values().any(|e| e.enabled);
            if !any_enabled {
                info!(
                    caller = caller_label,
                    scope_id = scope_id.as_deref().unwrap_or("none"),
                    "all indexers disabled for scope, skipping search"
                );
                return Ok(ScoredSearchOutcome {
                    results: Vec::new(),
                    complete_indexer_ids: Vec::new(),
                    incomplete_indexer_reasons: HashMap::new(),
                    search_session_id: Uuid::new_v4().to_string(),
                });
            }
        }

        let configured_indexers = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_else(|error| {
                warn!(error = %error, "failed to load indexer configs for transport-aware query collapse");
                vec![]
            });
        let collapse_structured_queries = search_subject_kind == ReleaseSearchSubjectKind::Episode
            && should_collapse_structured_nab_queries(
                &configured_indexers,
                indexer_routing.as_ref(),
                mode,
                chrono::Utc::now(),
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
        let page_sink = crate::IndexerSearchPageSink::new(page_tx, 2);
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
        let plan_indexer_routing = indexer_routing.clone();
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
        let mut scoring_inputs = None;
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
                            let mut retained_and_page = std::mem::take(&mut scored_results);
                            retained_and_page.append(&mut page.results);
                            scored_results = self
                                .score_release_results_with_prepared(
                                    retained_and_page,
                                    &quality_profile,
                                    title_id,
                                    indexer_routing.as_ref(),
                                    runtime_minutes,
                                    parse_context,
                                    season,
                                    episode,
                                    absolute_episode,
                                    &mut scoring_inputs,
                                    mode == SearchMode::Auto,
                                )
                                .await?;
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

        if scored_results.is_empty() && successful_searches == 0 && query_failures > 0 {
            let details =
                first_failure.unwrap_or_else(|| "all indexer search queries failed".to_string());
            return Err(AppError::Repository(details));
        }

        Ok(ScoredSearchOutcome {
            results: scored_results,
            complete_indexer_ids: completely_covered_indexers(
                coverage_by_indexer,
                required_query_count,
            ),
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
    ) -> AppResult<Vec<IndexerSearchResult>> {
        self.search_and_evaluate_subject_restricted(
            title,
            subject,
            caller_label,
            mode,
            cancel_token,
            None,
            None,
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
    ) -> AppResult<ScoredSearchOutcome> {
        let mut outcome = self
            .search_and_score_subject_restricted_with_fired_indexers(
                title,
                subject,
                caller_label,
                mode,
                cancel_token,
                restrict_to_indexer_ids,
                background_value,
            )
            .await?;
        outcome.results = self
            .evaluate_search_results_for_subject(
                title,
                subject,
                outcome.results,
                // A background value is the mark of the convergence sweep; a
                // search without one was started by an operator even when it
                // runs auto decisioning (`queue_best_release`), and Sonarr
                // skips the monitored check for exactly those searches.
                matches!(mode, SearchMode::Interactive) || background_value.is_none(),
            )
            .await;
        if self
            .finalize_evaluated_search_session_or_warn(
                &outcome.search_session_id,
                &outcome.results,
                &title.id,
            )
            .await
        {
            self.record_search_coverage(title, subject, &outcome.complete_indexer_ids)
                .await;
        }
        Ok(outcome)
    }

    /// Search and score `subject` while leaving admission evaluation to the
    /// caller. This is used by pack discovery lanes that must group candidates
    /// by their resolved submission scope before running canonical evaluation.
    #[expect(
        clippy::too_many_arguments,
        reason = "background search threads the convergence subset and value hint alongside the subject"
    )]
    pub(crate) async fn search_and_score_subject_restricted(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
    ) -> AppResult<ScoredSearchOutcome> {
        self.search_and_score_subject_restricted_with_fired_indexers(
            title,
            subject,
            caller_label,
            mode,
            cancel_token,
            restrict_to_indexer_ids,
            background_value,
        )
        .await
    }

    /// Search and score without writing generic scope coverage. The series-pack
    /// title lane uses this to record its set and qualifying collection keys;
    /// all normal search callers should use `search_and_score_subject_restricted`.
    #[expect(
        clippy::too_many_arguments,
        reason = "background search threads the convergence subset and value hint alongside the subject"
    )]
    pub(crate) async fn search_and_score_subject_restricted_with_fired_indexers(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
        cancel_token: CancellationToken,
        restrict_to_indexer_ids: Option<std::collections::HashSet<String>>,
        background_value: Option<f64>,
    ) -> AppResult<ScoredSearchOutcome> {
        let tagged_aliases = release_search_tagged_aliases(title);
        self.search_and_score_releases(ReleaseSearchRequest {
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
            season: subject.season,
            episode: subject.episode,
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
            parse_context: &subject.title_evidence.parse_context,
            cancel_token,
            restrict_to_indexer_ids,
            background_value,
        })
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
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
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
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
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
            .ok_or_else(|| AppError::NotFound(format!("series movie {series_movie_link_id}")))?;
        if link.series_title_id != title.id {
            return Err(AppError::Validation(
                "series movie does not belong to title".into(),
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
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
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
    pub(crate) absolute_episode: Option<u32>,
    /// Movie release year, when the searched title is a movie that has one.
    /// Series years never travel here: a season/episode search has no single
    /// release year, so passing the series year would be a guess.
    pub(crate) year: Option<i32>,
    pub(crate) tagged_aliases: &'a [TaggedAlias],
    pub(crate) search_subject_kind: ReleaseSearchSubjectKind,
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
                    AppError::Validation(format!(
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
            return Err(AppError::Validation(
                "no quality profile is configured; choose a global, category, library, or title profile"
                    .to_string(),
            ));
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

        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        if trimmed == INHERIT_QUALITY_PROFILE_VALUE {
            return Ok(None);
        }

        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(Some(trimmed.to_string()));
        };
        match parsed {
            Value::Null => Ok(None),
            Value::String(value) => {
                let normalized = value.trim();
                if normalized.is_empty() || normalized == INHERIT_QUALITY_PROFILE_VALUE {
                    Ok(None)
                } else {
                    Ok(Some(normalized.to_string()))
                }
            }
            _ => Ok(Some(trimmed.to_string())),
        }
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

        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        if trimmed == INHERIT_QUALITY_PROFILE_VALUE {
            return Ok(None);
        }

        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(Some(trimmed.to_string()));
        };
        match parsed {
            Value::Null => Ok(None),
            Value::String(value) => {
                let normalized = value.trim();
                if normalized.is_empty() || normalized == INHERIT_QUALITY_PROFILE_VALUE {
                    Ok(None)
                } else {
                    Ok(Some(normalized.to_string()))
                }
            }
            _ => Ok(Some(trimmed.to_string())),
        }
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
        if let Some(library_id) = library_id {
            match self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(library_id))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(plan) = self.parse_indexer_routing_plan(library_id, &value) {
                        return Some(plan);
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        error = %err,
                        library_id = library_id,
                        "failed to read library indexer routing setting, falling back to facet defaults"
                    );
                }
            }
        }

        let scope_id = scope_id?;

        let raw_json = match self
            .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
        {
            Ok(Some(value)) => value,
            Ok(None) => return None,
            Err(err) => {
                warn!(
                    error = %err,
                    scope_id = scope_id,
                    "failed to read indexer routing setting, falling back to defaults"
                );
                return None;
            }
        };

        self.parse_indexer_routing_plan(scope_id, &raw_json)
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
}

#[cfg(test)]
#[path = "app_usecase_discovery_tests.rs"]
mod app_usecase_discovery_tests;
