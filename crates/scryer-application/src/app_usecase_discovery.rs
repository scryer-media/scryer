use super::*;
use crate::quality_profile::ScoringSource;
use serde_json::Value;
use tokio::task::JoinSet;
use tracing::{info, warn};

fn source_kind_matches_preference(result: &IndexerSearchResult, preferred: &str) -> bool {
    match result.source_kind {
        Some(DownloadSourceKind::NzbUrl) => preferred == "nzb",
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
            preferred == "torrent"
        }
        None => false,
    }
}

const INDEXER_ROUTING_KEY: &str = "indexer.routing";

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

pub(crate) fn is_4xx_or_5xx_status(status: u16) -> bool {
    (400..=599).contains(&status)
}

fn extract_indexer_http_status(error: &AppError) -> Option<u16> {
    match error {
        AppError::Repository(message) => extract_http_status_from_message(message),
        _ => None,
    }
}

pub(crate) fn is_indexer_http_error(error: &AppError) -> bool {
    extract_indexer_http_status(error).is_some_and(is_4xx_or_5xx_status)
}

fn release_search_key(result: &IndexerSearchResult) -> String {
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

impl AppUseCase {
    pub(crate) async fn record_indexer_http_error_timestamp(&self, error: &AppError) {
        if !is_indexer_http_error(error) {
            return;
        }

        if let Err(err) = self
            .services
            .indexer_configs
            .touch_last_error(super::INDEXER_PROVIDER_NZBGEEK)
            .await
        {
            warn!(error = %err, "failed to update indexer last_error_at");
        }
    }

    /// Internal search+score pipeline shared by both user-facing search and background acquisition.
    pub(crate) async fn search_and_score_releases(
        &self,
        queries: Vec<String>,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        title_tags: &[String],
        caller_label: &str,
        mode: SearchMode,
        runtime_minutes: Option<i32>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let quality_profile = self
            .resolve_quality_profile(
                title_tags,
                imdb_id.as_deref(),
                tvdb_id.as_deref(),
                category.as_deref(),
            )
            .await?;

        let scope_id = self.quality_profile_scope_id(
            imdb_id.as_deref(),
            tvdb_id.as_deref(),
            category.as_deref(),
        );
        let indexer_routing = self.resolve_indexer_routing(scope_id.as_deref()).await;

        // If routing exists and every indexer is disabled, skip the search entirely.
        if let Some(ref plan) = indexer_routing {
            let any_enabled = plan.entries.values().any(|e| e.enabled);
            if !any_enabled {
                info!(
                    caller = caller_label,
                    scope_id = scope_id.as_deref().unwrap_or("none"),
                    "all indexers disabled for scope, skipping search"
                );
                return Ok(Vec::new());
            }
        }

        let title_hint = extract_title_hint(&queries);

        // Auto mode: conserve API calls by using only the first (canonical) query variant
        let effective_queries: Vec<String> = match mode {
            SearchMode::Auto => queries.into_iter().take(1).collect(),
            SearchMode::Interactive => queries,
        };

        let mut set = JoinSet::new();
        for query in effective_queries {
            let indexer_client = self.services.indexer_client.clone();
            let imdb_id = imdb_id.clone();
            let tvdb_id = tvdb_id.clone();
            let anidb_id = anidb_id.clone();
            let category = category.clone();
            let indexer_routing = indexer_routing.clone();

            set.spawn(async move {
                indexer_client
                    .search(
                        query,
                        imdb_id,
                        tvdb_id,
                        anidb_id,
                        category,
                        None,
                        indexer_routing,
                        mode,
                        season,
                        episode,
                    )
                    .await
            });
        }

        let mut query_failures = 0usize;
        let mut first_failure: Option<String> = None;
        let mut raw_results: Vec<IndexerSearchResult> = Vec::new();

        while let Some(result) = set.join_next().await {
            match result {
                Ok(Ok(mut response)) => {
                    raw_results.append(&mut response.results);
                }
                Ok(Err(error)) => {
                    query_failures += 1;
                    first_failure = first_failure.or_else(|| Some(error.to_string()));
                    self.record_indexer_http_error_timestamp(&error).await;
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

        // Filter out results whose title doesn't match any of the search queries.
        // This prevents RSS feeds (which return their entire recent feed) from
        // polluting results with unrelated releases.
        if let Some(ref hint) = title_hint {
            let hint_normalized = crate::app_usecase_rss::normalize_for_matching(hint);
            if !hint_normalized.is_empty() {
                let before = raw_results.len();
                raw_results.retain(|r| {
                    crate::app_usecase_rss::normalize_for_matching(&r.title)
                        .contains(&hint_normalized)
                });
                let filtered = before - raw_results.len();
                if filtered > 0 {
                    info!(
                        before,
                        after = raw_results.len(),
                        filtered,
                        title_hint = hint.as_str(),
                        "filtered non-matching releases from search results"
                    );
                }
            }
        }

        if raw_results.is_empty() && query_failures > 0 {
            let details =
                first_failure.unwrap_or_else(|| "all indexer search queries failed".to_string());
            return Err(AppError::Repository(details));
        }

        let failed_signatures = match self
            .services
            .release_attempts
            .list_failed_release_signatures(5000)
            .await
        {
            Ok(items) => items,
            Err(error) => {
                warn!(error = %error, "failed to load failed release blocklist signatures");
                Vec::new()
            }
        };

        let failed_source_hints: std::collections::HashSet<String> = failed_signatures
            .iter()
            .filter_map(|signature| {
                normalize_release_attempt_hint(signature.source_hint.as_deref())
            })
            .collect();
        let failed_source_titles: std::collections::HashSet<String> = failed_signatures
            .iter()
            .filter_map(|signature| {
                normalize_release_attempt_title(signature.source_title.as_deref())
            })
            .collect();

        // Determine which source kinds (NZB, torrent) the user can actually use
        // based on their enabled download clients, and which kind is preferred
        // (lowest client_priority wins). Done early so we can filter before parsing.
        let (has_usenet_client, has_torrent_client, preferred_source_kind) = {
            let clients = self
                .services
                .download_client_configs
                .list(None)
                .await
                .unwrap_or_default();
            let enabled: Vec<_> = clients.iter().filter(|c| c.is_enabled).collect();
            let has_usenet = enabled
                .iter()
                .any(|c| matches!(c.client_type.as_str(), "nzbget" | "sabnzbd"));
            let has_torrent = enabled.iter().any(|c| {
                matches!(
                    c.client_type.as_str(),
                    "qbittorrent" | "transmission" | "deluge" | "rtorrent"
                )
            });
            let preferred = enabled
                .iter()
                .min_by_key(|c| c.client_priority)
                .map(|c| {
                    if matches!(c.client_type.as_str(), "nzbget" | "sabnzbd") {
                        "nzb"
                    } else {
                        "torrent"
                    }
                })
                .unwrap_or("nzb");
            (has_usenet, has_torrent, preferred)
        };

        // Filter out results with no compatible download client before expensive parsing/scoring.
        raw_results.retain(|r| match r.source_kind {
            Some(DownloadSourceKind::NzbUrl) => has_usenet_client,
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                has_torrent_client
            }
            None => true,
        });

        // Clone the user rules engine for this batch (cheap Arc clone).
        let user_rules_engine = self
            .services
            .user_rules
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
        let mut user_evaluator = user_rules_engine.evaluator();

        let mut deduped = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for result in raw_results {
            let key = release_search_key(&result);
            if !seen.insert(key) {
                continue;
            }

            if is_release_blocklisted(&result, &failed_source_hints, &failed_source_titles) {
                continue;
            }

            let parsed_release_metadata = parse_release_metadata(&result.title);

            // Post-filter: skip results whose parsed season/episode doesn't match
            // the requested values. Indexers don't always respect API params.
            if let Some(ref ep_meta) = parsed_release_metadata.episode {
                if let Some(wanted_season) = season
                    && let Some(parsed_season) = ep_meta.season
                    && parsed_season != wanted_season
                {
                    continue;
                }
                if let Some(wanted_episode) = episode {
                    // For SxxExx-style releases, check episode_numbers
                    if !ep_meta.episode_numbers.is_empty()
                        && !ep_meta.episode_numbers.contains(&wanted_episode)
                    {
                        continue;
                    }
                    // For absolute-numbered releases (common in anime), check
                    // against the known absolute episode number from TVDB
                    if ep_meta.episode_numbers.is_empty()
                        && let (Some(parsed_abs), Some(wanted_abs)) =
                            (ep_meta.absolute_episode, absolute_episode)
                        && parsed_abs != wanted_abs
                    {
                        continue;
                    }
                }
            }

            let persona = quality_profile
                .criteria
                .resolve_persona(category.as_deref());
            let weights = crate::scoring_weights::build_weights(
                persona,
                &quality_profile.criteria.scoring_overrides,
            );
            let mut decision = evaluate_against_profile(
                &quality_profile,
                &parsed_release_metadata,
                false,
                &weights,
            );
            apply_age_scoring(&mut decision, result.published_at.as_deref());
            crate::quality_profile::apply_size_scoring_for_category(
                &mut decision,
                &parsed_release_metadata,
                result.size_bytes,
                category.as_deref(),
                runtime_minutes,
                &weights,
            );
            // ── User rules (additive, after all built-in scoring) ───────
            // NZBGeek vote scoring is now handled by plugin-declared Rego
            // policies (nzbgeek_vote_penalty, nzbgeek_language_bonus) that
            // run as part of the user rules evaluation below.
            if !user_rules_engine.is_empty() {
                let user_input = crate::app_usecase_discovery::build_user_rule_input(
                    &parsed_release_metadata,
                    &quality_profile,
                    &result,
                    &decision,
                    category.as_deref(),
                    title_tags,
                    runtime_minutes,
                );
                let facet = category.as_deref().unwrap_or("movie");
                match user_evaluator.evaluate(&user_input, facet) {
                    Ok(eval_result) => {
                        for entry in eval_result.entries {
                            decision.log_with_source(
                                &entry.code,
                                entry.delta,
                                ScoringSource::UserRule(entry.rule_set_id),
                            );
                        }
                        for err in eval_result.errors {
                            decision.log_with_source(
                                "user_rule_error",
                                0,
                                ScoringSource::UserRule(err.rule_set_id),
                            );
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "user rule evaluation failed for release");
                    }
                }
            }

            deduped.push(IndexerSearchResult {
                parsed_release_metadata: Some(parsed_release_metadata),
                quality_profile_decision: Some(decision),
                ..result
            });
        }

        // Cross-indexer dedup: same release from multiple indexers.
        // Prefer: (1) higher-priority indexer for this facet, (2) source kind
        // matching the user's highest-priority download client.
        {
            // Build indexer name → priority lookup from the routing plan.
            // Indexers not in the routing plan get MAX priority (lowest preference).
            let indexer_priority_by_name: std::collections::HashMap<String, i64> =
                if let Some(ref plan) = indexer_routing {
                    let configs = self
                        .services
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
                } else {
                    std::collections::HashMap::new()
                };

            let mut best_by_key: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let mut remove_indices: std::collections::HashSet<usize> =
                std::collections::HashSet::new();

            for (idx, result) in deduped.iter().enumerate() {
                let key = result
                    .parsed_release_metadata
                    .as_ref()
                    .map(crate::release_dedup::build_release_dedup_key)
                    .unwrap_or_default();
                if key.is_empty() {
                    continue;
                }

                if let Some(&existing_idx) = best_by_key.get(&key) {
                    let existing = &deduped[existing_idx];

                    // Compare indexer priority first (lower = better)
                    let existing_prio = indexer_priority_by_name
                        .get(&existing.source)
                        .copied()
                        .unwrap_or(i64::MAX);
                    let new_prio = indexer_priority_by_name
                        .get(&result.source)
                        .copied()
                        .unwrap_or(i64::MAX);

                    let new_wins = if new_prio != existing_prio {
                        new_prio < existing_prio
                    } else {
                        // Same indexer priority — break tie by download client preference
                        let existing_preferred =
                            source_kind_matches_preference(existing, preferred_source_kind);
                        let new_preferred =
                            source_kind_matches_preference(result, preferred_source_kind);
                        new_preferred && !existing_preferred
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

            if !remove_indices.is_empty() {
                let before = deduped.len();
                let mut idx = 0usize;
                deduped.retain(|_| {
                    let keep = !remove_indices.contains(&idx);
                    idx += 1;
                    keep
                });
                info!(before, after = deduped.len(), "cross-indexer release dedup");
            }
        }

        deduped.sort_by(|left, right| {
            let left_allowed = left
                .quality_profile_decision
                .as_ref()
                .map(|decision| decision.allowed)
                .unwrap_or(true);
            let right_allowed = right
                .quality_profile_decision
                .as_ref()
                .map(|decision| decision.allowed)
                .unwrap_or(true);

            match (left_allowed, right_allowed) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let left_score = left
                        .quality_profile_decision
                        .as_ref()
                        .map(|decision| decision.preference_score)
                        .unwrap_or(0);
                    let right_score = right
                        .quality_profile_decision
                        .as_ref()
                        .map(|decision| decision.preference_score)
                        .unwrap_or(0);

                    right_score.cmp(&left_score)
                }
            }
        });

        Ok(deduped)
    }

    async fn search_indexer_queries(
        &self,
        actor: &User,
        queries: Vec<String>,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        self.search_and_score_releases(
            queries,
            imdb_id,
            tvdb_id,
            anidb_id,
            category,
            &[],
            &actor.id,
            SearchMode::Interactive,
            None,
            season,
            episode,
            absolute_episode,
        )
        .await
    }

    pub async fn search_indexers(
        &self,
        actor: &User,
        query: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        require(actor, &Entitlement::ViewCatalog)?;

        let normalized_query = query.trim();
        let normalized_imdb_id = normalize_imdb_id(imdb_id);
        let normalized_tvdb_id = normalize_numeric_id(tvdb_id);
        let normalized_anidb_id = normalize_numeric_id(anidb_id);
        let normalized_category = category
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if normalized_query.is_empty()
            && normalized_tvdb_id.is_none()
            && normalized_imdb_id.is_none()
        {
            return Err(AppError::Validation("search query is required".into()));
        }

        info!(
            actor = actor.id.as_str(),
            query = normalized_query,
            imdb_id = normalized_imdb_id.as_deref(),
            tvdb_id = normalized_tvdb_id.as_deref(),
            anidb_id = normalized_anidb_id.as_deref(),
            category = normalized_category.as_deref(),
            "searching indexers"
        );

        let results = self
            .search_indexer_queries(
                actor,
                vec![normalized_query.to_string()],
                normalized_imdb_id.clone(),
                normalized_tvdb_id.clone(),
                normalized_anidb_id.clone(),
                normalized_category.clone(),
                None,
                None,
                None,
            )
            .await;

        let mut display_source = normalized_query.to_string();
        if display_source.is_empty() {
            if let Some(tvdb_id) = normalized_tvdb_id.as_deref() {
                display_source = format!("tvdb:{tvdb_id}");
            } else if let Some(imdb_id) = normalized_imdb_id.as_deref() {
                display_source = format!("imdb:{imdb_id}");
            }
        }
        let activity_media_label = normalized_category
            .as_deref()
            .map(
                |category| match category.trim().to_ascii_lowercase().as_str() {
                    "series" | "tv" => "series",
                    "anime" => "anime",
                    _ => "movie",
                },
            )
            .unwrap_or("movie");

        let results = results?;

        info!(
            actor = actor.id.as_str(),
            count = results.len(),
            "indexer search returned results"
        );
        let _ = self
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                ActivityKind::MovieFetched,
                format!(
                    "{} searched: {} ({} results)",
                    activity_media_label,
                    display_source,
                    results.len()
                ),
                ActivitySeverity::Info,
                vec![ActivityChannel::WebUi],
            )
            .await;

        Ok(results)
    }

    pub async fn search_indexers_episode(
        &self,
        actor: &User,
        title: String,
        season: String,
        episode: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        absolute_episode: Option<u32>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        require(actor, &Entitlement::ViewCatalog)?;

        let normalized_title = title.trim();
        let season = season.trim();
        let episode = episode.trim();

        if normalized_title.is_empty() || season.is_empty() || episode.is_empty() {
            return Err(AppError::Validation(
                "title, season, and episode are required".into(),
            ));
        }

        let normalized_imdb_id = normalize_imdb_id(imdb_id);
        let normalized_anidb_id = normalize_numeric_id(anidb_id);
        let normalized_tvdb_id = normalize_numeric_id(tvdb_id);
        let normalized_category = category
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let season_digits: String = season
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        let episode_digits: String = episode
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();

        if season_digits.is_empty() || episode_digits.is_empty() {
            return Err(AppError::Validation(
                "season and episode must include numeric values".into(),
            ));
        }

        let season_num = season_digits
            .parse::<usize>()
            .map_err(|_| AppError::Validation("invalid season value".into()))?;
        let episode_num = episode_digits
            .parse::<usize>()
            .map_err(|_| AppError::Validation("invalid episode value".into()))?;

        let queries = vec![format!(
            "{} S{:0>2}E{:0>2}",
            normalized_title, season_num, episode_num
        )];

        let results = self
            .search_indexer_queries(
                actor,
                queries,
                normalized_imdb_id.clone(),
                normalized_tvdb_id.clone(),
                normalized_anidb_id.clone(),
                normalized_category.clone(),
                Some(season_num as u32),
                Some(episode_num as u32),
                absolute_episode,
            )
            .await?;

        let activity_media_label = normalized_category
            .as_deref()
            .map(|value| match value.trim().to_ascii_lowercase().as_str() {
                "series" | "tv" => "series",
                "anime" => "anime",
                _ => "movie",
            })
            .unwrap_or("movie");

        let _ = self
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                ActivityKind::MovieFetched,
                format!(
                    "{} searched: {} S{:0>2}E{:0>2} ({} results)",
                    activity_media_label,
                    normalized_title,
                    season_num,
                    episode_num,
                    results.len()
                ),
                ActivitySeverity::Info,
                vec![ActivityChannel::WebUi],
            )
            .await;

        Ok(results)
    }

    pub async fn search_indexers_season(
        &self,
        actor: &User,
        title: String,
        season: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        require(actor, &Entitlement::ViewCatalog)?;

        let normalized_title = title.trim();
        let season = season.trim();

        if normalized_title.is_empty() || season.is_empty() {
            return Err(AppError::Validation("title and season are required".into()));
        }

        let normalized_imdb_id = normalize_imdb_id(imdb_id);
        let normalized_tvdb_id = normalize_numeric_id(tvdb_id);
        let normalized_category = category
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let season_digits: String = season
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        if season_digits.is_empty() {
            return Err(AppError::Validation(
                "season must include a numeric value".into(),
            ));
        }

        let season_num = season_digits
            .parse::<usize>()
            .map_err(|_| AppError::Validation("invalid season value".into()))?;

        let queries = vec![format!("{} S{:0>2}", normalized_title, season_num)];

        let results = self
            .search_indexer_queries(
                actor,
                queries,
                normalized_imdb_id.clone(),
                normalized_tvdb_id.clone(),
                None, // anidb_id — not available in season search
                normalized_category.clone(),
                Some(season_num as u32),
                None,
                None,
            )
            .await?;

        let activity_media_label = normalized_category
            .as_deref()
            .map(|value| match value.trim().to_ascii_lowercase().as_str() {
                "series" | "tv" => "series",
                "anime" => "anime",
                _ => "movie",
            })
            .unwrap_or("series");

        let _ = self
            .services
            .record_activity_event(
                Some(actor.id.clone()),
                None,
                ActivityKind::MovieFetched,
                format!(
                    "{} season pack searched: {} S{:0>2} ({} results)",
                    activity_media_label,
                    normalized_title,
                    season_num,
                    results.len()
                ),
                ActivitySeverity::Info,
                vec![ActivityChannel::WebUi],
            )
            .await;

        Ok(results)
    }
}

pub(crate) fn is_release_blocklisted(
    result: &IndexerSearchResult,
    failed_source_hints: &std::collections::HashSet<String>,
    failed_source_titles: &std::collections::HashSet<String>,
) -> bool {
    if let Some(download_url) = normalize_release_attempt_hint(result.download_url.as_deref())
        && failed_source_hints.contains(&download_url)
    {
        return true;
    }

    if let Some(link) = normalize_release_attempt_hint(result.link.as_deref())
        && failed_source_hints.contains(&link)
    {
        return true;
    }

    if let Some(title) = normalize_release_attempt_title(Some(result.title.as_str()))
        && failed_source_titles.contains(&title)
    {
        return true;
    }

    false
}

fn normalize_imdb_id(raw: Option<String>) -> Option<String> {
    let value = raw
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;

    if let Some(tt_index) = value.to_ascii_lowercase().find("tt") {
        let suffix: String = value[tt_index + 2..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        if !suffix.is_empty() {
            return Some(format!("tt{suffix}"));
        }
    }

    if value.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("tt{value}"))
    } else {
        None
    }
}

fn normalize_numeric_id(raw: Option<String>) -> Option<String> {
    let value = raw
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();

    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

impl AppUseCase {
    pub(crate) async fn resolve_quality_profile(
        &self,
        title_tags: &[String],
        imdb_id: Option<&str>,
        tvdb_id: Option<&str>,
        category_hint: Option<&str>,
    ) -> AppResult<QualityProfile> {
        let catalog = self.load_quality_profiles().await?;
        let category_scope_id = self.quality_profile_scope_id(imdb_id, tvdb_id, category_hint);

        let title_profile_id = title_tags
            .iter()
            .find(|t| t.starts_with("scryer:quality-profile:"))
            .map(|t| t.trim_start_matches("scryer:quality-profile:").to_string());

        let category_profile_id = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, category_scope_id.as_deref())
            .await?;
        let global_profile_id = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?;

        let active_profile_id = resolve_profile_id_for_title(
            title_profile_id.as_deref(),
            category_profile_id.as_deref(),
            global_profile_id.as_deref(),
        );
        if let Some(profile_id) = active_profile_id.as_deref()
            && let Some(profile) = catalog.iter().find(|profile| profile.id == profile_id)
        {
            return Ok(profile.clone());
        }

        warn!(
            active_profile_id = active_profile_id.as_deref().unwrap_or("none"),
            "quality profile id not found in catalog, using default"
        );

        Ok(default_quality_profile_for_search())
    }

    async fn load_quality_profiles(&self) -> AppResult<Vec<QualityProfile>> {
        match self
            .services
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
        {
            Ok(catalog) if !catalog.is_empty() => return Ok(catalog),
            Ok(_) => warn!("quality profile DB catalog is empty; using default"),
            Err(err) => {
                warn!(error = %err, "failed to load quality profiles from DB; using default")
            }
        }

        Ok(vec![default_quality_profile_for_search()])
    }

    pub(crate) async fn read_setting_string_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, key_name, scope_id)
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

    fn quality_profile_scope_id(
        &self,
        imdb_id: Option<&str>,
        tvdb_id: Option<&str>,
        category_hint: Option<&str>,
    ) -> Option<String> {
        if let Some(value) = category_hint {
            let normalized = value.to_ascii_lowercase();
            match normalized.as_str() {
                "movie" => return Some("movie".to_string()),
                "tv" | "series" => return Some("series".to_string()),
                "anime" => return Some("anime".to_string()),
                "5070" => return Some("series".to_string()),
                _ => {}
            }
        }

        if imdb_id.is_some() {
            return Some("movie".to_string());
        }
        if tvdb_id.is_some() {
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
    async fn resolve_indexer_routing(&self, scope_id: Option<&str>) -> Option<IndexerRoutingPlan> {
        let scope_id = scope_id?;

        let raw_json = match self
            .read_setting_string_value(INDEXER_ROUTING_KEY, Some(scope_id))
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

        let parsed: Value = match serde_json::from_str(&raw_json) {
            Ok(value) => value,
            Err(_) => return None,
        };

        let obj = parsed.as_object()?;
        if obj.is_empty() {
            return None;
        }

        let mut entries = std::collections::HashMap::new();

        for (indexer_id, config) in obj {
            let enabled = config
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

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

            let priority = config
                .get("priority")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);

            entries.insert(
                indexer_id.clone(),
                IndexerRoutingEntry {
                    enabled,
                    categories,
                    priority,
                },
            );
        }

        info!(
            scope_id = scope_id,
            indexer_count = entries.len(),
            "resolved per-indexer routing plan"
        );
        Some(IndexerRoutingPlan { entries })
    }
}

pub(crate) fn build_user_rule_input(
    parsed: &ParsedReleaseMetadata,
    profile: &QualityProfile,
    result: &IndexerSearchResult,
    decision: &QualityProfileDecision,
    category: Option<&str>,
    title_tags: &[String],
    runtime_minutes: Option<i32>,
) -> scryer_rules::UserRuleInput {
    crate::user_rule_input::build_search_rule_input(
        parsed,
        profile,
        result,
        decision,
        category,
        title_tags,
        runtime_minutes,
    )
}

/// Extract a title name hint from search queries by stripping S##E## patterns.
/// Returns None if no meaningful title text can be extracted.
fn extract_title_hint(queries: &[String]) -> Option<String> {
    for query in queries {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Strip season/episode patterns: S01E05, S01, 1x05, etc.
        let cleaned = trimmed
            .split_whitespace()
            .filter(|word| {
                let w = word.to_ascii_lowercase();
                // Skip pure season/episode tokens (S01E05, S01, 1x05, bare numbers)
                if w.starts_with('s') && w[1..].chars().all(|c| c.is_ascii_digit() || c == 'e') {
                    return false;
                }
                if w.contains('x') {
                    return false;
                }
                !w.chars().all(|c| c.is_ascii_digit())
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }
    None
}

#[cfg(test)]
#[path = "app_usecase_discovery_tests.rs"]
mod app_usecase_discovery_tests;
