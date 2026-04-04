use super::*;
use crate::quality_profile::ScoringSource;
use crate::quality_profile::evaluate_against_profile_for_category;
use scryer_domain::TaggedAlias;
use serde_json::Value;
use std::collections::HashMap;
use tokio::task::JoinSet;
use tracing::{info, warn};

fn source_kind_matches_preference(result: &IndexerSearchResult, preferred: &str) -> bool {
    match result.source_kind {
        Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => preferred == "nzb",
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
            preferred == "torrent"
        }
        None => false,
    }
}

const INDEXER_ROUTING_KEY: &str = "indexer.routing";

fn parse_search_facet(facet: Option<String>) -> Option<String> {
    facet
        .and_then(|value| MediaFacet::parse(&value))
        .map(|f| f.as_str().to_string())
}

fn activity_media_label(facet: Option<&str>) -> &'static str {
    match facet {
        Some("series") => "series",
        Some("anime") => "anime",
        _ => "movie",
    }
}

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

            let new_wins = if new_prio != existing_prio {
                new_prio < existing_prio
            } else {
                let existing_preferred =
                    source_kind_matches_preference(existing, preferred_source_kind);
                let new_preferred = source_kind_matches_preference(result, preferred_source_kind);
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
    info!(before, after = deduped.len(), "cross-indexer release dedup");
    deduped
}

impl AppUseCase {
    pub(crate) async fn download_source_capabilities(&self) -> (bool, bool, String) {
        let clients = self
            .services
            .download_client_configs
            .list(None)
            .await
            .unwrap_or_default();
        let enabled: Vec<_> = clients.iter().filter(|c| c.is_enabled).collect();
        let plugin_provider = self.services.download_client_plugin_provider.as_ref();
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

    pub(crate) async fn score_release_results(
        &self,
        mut raw_results: Vec<IndexerSearchResult>,
        quality_profile: &QualityProfile,
        title_id: Option<&str>,
        scope_id: Option<&str>,
        indexer_routing: Option<&IndexerRoutingPlan>,
        category: Option<&str>,
        title_tags: &[String],
        runtime_minutes: Option<i32>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    ) -> Vec<IndexerSearchResult> {
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

        let (has_usenet_client, has_torrent_client, preferred_source_kind) =
            self.download_source_capabilities().await;

        raw_results.retain(|result| match result.source_kind {
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => has_usenet_client,
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                has_torrent_client
            }
            None => true,
        });

        let user_rules_engine = self
            .services
            .user_rules
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
        let mut user_evaluator = user_rules_engine.evaluator();
        let resolved_persona = self
            .resolve_scoring_persona(scope_id, Some(quality_profile), category)
            .await
            .unwrap_or_else(|error| {
                warn!(error = %error, "failed to resolve scoring persona, using canonical default");
                crate::ScoringPersona::default()
            });
        let required_audio_languages = self
            .resolve_required_audio_languages(title_id, scope_id, Some(quality_profile))
            .await
            .unwrap_or_else(|error| {
                warn!(
                    error = %error,
                    "failed to resolve required audio languages, using canonical default"
                );
                Vec::new()
            });
        let mut resolved_profile = quality_profile.clone();
        resolved_profile.criteria.required_audio_languages = required_audio_languages;
        resolved_profile.criteria.scoring_persona = resolved_persona.clone();
        resolved_profile.criteria.facet_persona_overrides.clear();

        let mut scored = Vec::new();
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
            let mut scored_release_metadata = parsed_release_metadata.clone();
            scored_release_metadata.languages_audio = crate::release_audio_language_hints(
                &parsed_release_metadata,
                result.indexer_languages.as_deref(),
            );

            if let Some(ref ep_meta) = scored_release_metadata.episode {
                if let Some(wanted_season) = season
                    && let Some(parsed_season) = ep_meta.season
                    && parsed_season != wanted_season
                {
                    continue;
                }
                if let Some(wanted_episode) = episode {
                    if !ep_meta.episode_numbers.is_empty()
                        && !ep_meta.episode_numbers.contains(&wanted_episode)
                    {
                        continue;
                    }
                    if ep_meta.episode_numbers.is_empty()
                        && let (Some(parsed_abs), Some(wanted_abs)) =
                            (ep_meta.absolute_episode, absolute_episode)
                        && parsed_abs != wanted_abs
                    {
                        continue;
                    }
                }
            }

            let weights = crate::scoring_weights::build_weights_for_category(
                &resolved_persona,
                &resolved_profile.criteria.scoring_overrides,
                category,
            );
            let mut decision = evaluate_against_profile_for_category(
                &resolved_profile,
                &scored_release_metadata,
                false,
                &weights,
                category,
            );
            apply_age_scoring(&mut decision, result.published_at.as_deref());
            crate::quality_profile::apply_size_scoring_for_category(
                &mut decision,
                &scored_release_metadata,
                result.size_bytes,
                category,
                runtime_minutes,
                &weights,
            );

            if !user_rules_engine.is_empty() {
                let user_input = crate::app_usecase_discovery::build_user_rule_input(
                    &scored_release_metadata,
                    &resolved_profile,
                    &result,
                    &decision,
                    category,
                    title_tags,
                    runtime_minutes,
                );
                let facet = category.unwrap_or("movie");
                match user_evaluator.evaluate(&user_input, facet) {
                    Ok(eval_result) => {
                        for entry in eval_result.entries {
                            decision.log_with_source(
                                &entry.code,
                                entry.delta,
                                ScoringSource::UserRule {
                                    id: entry.rule_set_id,
                                    name: entry.rule_set_name,
                                },
                            );
                        }
                        for err in eval_result.errors {
                            decision.log_with_source(
                                "user_rule_error",
                                0,
                                ScoringSource::UserRule {
                                    id: err.rule_set_id,
                                    name: err.rule_set_name,
                                },
                            );
                        }
                    }
                    Err(error) => {
                        warn!(error = %error, "user rule evaluation failed for release");
                    }
                }
            }

            scored.push(IndexerSearchResult {
                parsed_release_metadata: Some(scored_release_metadata),
                quality_profile_decision: Some(decision),
                ..result
            });
        }

        let indexer_priority_by_name = self.build_indexer_priority_by_name(indexer_routing).await;
        let mut scored = dedupe_cross_indexer_release_results(
            scored,
            &indexer_priority_by_name,
            preferred_source_kind.as_str(),
        );

        scored.sort_by(|left, right| {
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

        scored
    }

    /// Internal search+score pipeline shared by both user-facing search and background acquisition.
    pub(crate) async fn search_and_score_releases(
        &self,
        queries: Vec<String>,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        facet: Option<String>,
        title_id: Option<&str>,
        title_tags: &[String],
        caller_label: &str,
        mode: SearchMode,
        runtime_minutes: Option<i32>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: &[TaggedAlias],
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

        // Auto mode: conserve API calls by using only the first (canonical) query variant
        let effective_queries: Vec<String> = match mode {
            SearchMode::Auto => queries.into_iter().take(1).collect(),
            SearchMode::Interactive => queries,
        };

        let mut set = JoinSet::new();
        let mut ids = HashMap::new();
        if let Some(imdb_id) = imdb_id.clone() {
            ids.insert("imdb_id".to_string(), imdb_id);
        }
        if let Some(tvdb_id) = tvdb_id.clone() {
            ids.insert("tvdb_id".to_string(), tvdb_id);
        }
        if let Some(anidb_id) = anidb_id.clone() {
            ids.insert("anidb_id".to_string(), anidb_id);
        }

        for query in effective_queries {
            let indexer_client = self.services.indexer_client.clone();
            let ids = ids.clone();
            let category = category.clone();
            let facet = facet.clone();
            let indexer_routing = indexer_routing.clone();
            let tagged_aliases = tagged_aliases.to_vec();

            set.spawn(async move {
                indexer_client
                    .search(
                        query,
                        ids,
                        category.clone(),
                        facet,
                        None,
                        indexer_routing,
                        mode,
                        season,
                        episode,
                        absolute_episode,
                        tagged_aliases,
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

        if raw_results.is_empty() && query_failures > 0 {
            let details =
                first_failure.unwrap_or_else(|| "all indexer search queries failed".to_string());
            return Err(AppError::Repository(details));
        }

        Ok(self
            .score_release_results(
                raw_results,
                &quality_profile,
                title_id,
                scope_id.as_deref(),
                indexer_routing.as_ref(),
                category.as_deref(),
                title_tags,
                runtime_minutes,
                season,
                episode,
                absolute_episode,
            )
            .await)
    }

    async fn search_indexer_queries(
        &self,
        actor: &User,
        queries: Vec<String>,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        facet: Option<String>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: &[TaggedAlias],
    ) -> AppResult<Vec<IndexerSearchResult>> {
        self.search_and_score_releases(
            queries,
            imdb_id,
            tvdb_id,
            anidb_id,
            category,
            facet,
            None,
            &[],
            &actor.id,
            SearchMode::Interactive,
            None,
            season,
            episode,
            absolute_episode,
            tagged_aliases,
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
        let normalized_category = parse_search_facet(category);

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
                normalized_category.clone(),
                None,
                None,
                None,
                &[],
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
        let activity_media_label = activity_media_label(normalized_category.as_deref());

        let results = results?;

        info!(
            actor = actor.id.as_str(),
            count = results.len(),
            "indexer search returned results"
        );
        self.emit_discovery_search_completed_event(
            Some(actor.id.clone()),
            activity_media_label.to_string(),
            Some(display_source),
            results.len() as i64,
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
        let normalized_category = parse_search_facet(category);

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
                normalized_category.clone(),
                Some(season_num as u32),
                Some(episode_num as u32),
                absolute_episode,
                &[],
            )
            .await?;

        let activity_media_label = activity_media_label(normalized_category.as_deref());

        self.emit_discovery_search_completed_event(
            Some(actor.id.clone()),
            activity_media_label.to_string(),
            Some(format!(
                "{} S{:0>2}E{:0>2}",
                normalized_title, season_num, episode_num
            )),
            results.len() as i64,
        )
        .await;

        Ok(results)
    }

    /// Interactive search for a title (movie or standalone). Resolves all
    /// external IDs and search category from the title record so the frontend
    /// only needs to pass the title ID.
    pub async fn search_indexers_for_title(
        &self,
        actor: &User,
        title_id: String,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        require(actor, &Entitlement::ViewCatalog)?;

        let title = self
            .services
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;

        let imdb_id = normalize_imdb_id(title.imdb_id);
        let tvdb_id = normalize_numeric_id(
            crate::app_usecase_acquisition::tvdb_id_from_external_ids(&title.external_ids),
        );
        let anidb_id = normalize_numeric_id(
            crate::app_usecase_acquisition::anidb_id_from_external_ids(&title.external_ids),
        );
        let category = self
            .facet_registry
            .get(&title.facet)
            .map(|h| h.search_category().to_string())
            .unwrap_or_else(|| "movie".to_string());
        let facet = Some(title.facet.as_str().to_string());

        let query = title.name.trim().to_string();
        if query.is_empty() && imdb_id.is_none() && tvdb_id.is_none() && anidb_id.is_none() {
            return Err(AppError::Validation(
                "title has no name or external IDs".into(),
            ));
        }

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = query.as_str(),
            category = category.as_str(),
            "searching indexers for title"
        );

        let results = self
            .search_indexer_queries(
                actor,
                vec![query.clone()],
                imdb_id,
                tvdb_id,
                anidb_id,
                Some(category.clone()),
                facet,
                None,
                None,
                None,
                &title.tagged_aliases,
            )
            .await?;

        self.emit_discovery_search_completed_event(
            Some(actor.id.clone()),
            category,
            Some(query),
            results.len() as i64,
        )
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
    ) -> AppResult<Vec<IndexerSearchResult>> {
        require(actor, &Entitlement::ViewCatalog)?;

        let title = self
            .services
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;

        let season = season.trim().to_string();
        let episode = episode.trim().to_string();
        if season.is_empty() || episode.is_empty() {
            return Err(AppError::Validation(
                "season and episode are required".into(),
            ));
        }

        let season_digits: String = season.chars().filter(|c| c.is_ascii_digit()).collect();
        let episode_digits: String = episode.chars().filter(|c| c.is_ascii_digit()).collect();
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

        let imdb_id = normalize_imdb_id(title.imdb_id.clone());
        let tvdb_id = normalize_numeric_id(
            crate::app_usecase_acquisition::tvdb_id_from_external_ids(&title.external_ids),
        );
        let title_anidb_id = normalize_numeric_id(
            crate::app_usecase_acquisition::anidb_id_from_external_ids(&title.external_ids),
        );
        let category = self
            .facet_registry
            .get(&title.facet)
            .map(|h| h.search_category().to_string())
            .unwrap_or_else(|| "series".to_string());
        let facet = Some(title.facet.as_str().to_string());

        // Resolve episode-specific anidb_id from anibridge (e.g. Bleach S17E08 → 15449)
        let anidb_id = if let Some(ref tvdb) = tvdb_id {
            if let Ok(tvdb_num) = tvdb.parse::<i64>() {
                match self
                    .services
                    .metadata_gateway
                    .anibridge_mappings_for_episode(tvdb_num, season_num as i32, episode_num as i32)
                    .await
                {
                    Ok(mappings) => mappings
                        .iter()
                        .find(|m| m.source_type == "anidb" && m.source_scope == "R")
                        .map(|m| m.source_id.to_string())
                        .or(title_anidb_id),
                    Err(_) => title_anidb_id,
                }
            } else {
                title_anidb_id
            }
        } else {
            title_anidb_id
        };

        // Look up absolute episode number from the episode record
        let absolute_episode: Option<u32> = self
            .services
            .shows
            .find_episode_by_title_and_numbers(&title_id, &season_digits, &episode_digits)
            .await
            .ok()
            .flatten()
            .and_then(|ep| {
                ep.absolute_number.as_ref().and_then(|n: &String| {
                    n.trim()
                        .replace(|c: char| !c.is_ascii_digit(), "")
                        .parse::<u32>()
                        .ok()
                })
            });

        let queries = vec![format!(
            "{} S{:0>2}E{:0>2}",
            title.name.trim(),
            season_num,
            episode_num
        )];

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = queries[0].as_str(),
            category = category.as_str(),
            "searching indexers for episode"
        );

        let results = self
            .search_indexer_queries(
                actor,
                queries.clone(),
                imdb_id,
                tvdb_id,
                anidb_id,
                Some(category.clone()),
                facet,
                Some(season_num as u32),
                Some(episode_num as u32),
                absolute_episode,
                &title.tagged_aliases,
            )
            .await?;

        self.emit_discovery_search_completed_event(
            Some(actor.id.clone()),
            category,
            queries.into_iter().next(),
            results.len() as i64,
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
    raw.as_deref().and_then(crate::normalize::normalize_imdb_id)
}

fn normalize_numeric_id(raw: Option<String>) -> Option<String> {
    raw.as_deref()
        .and_then(crate::normalize::normalize_numeric_id)
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
    pub(crate) async fn resolve_indexer_routing(
        &self,
        scope_id: Option<&str>,
    ) -> Option<IndexerRoutingPlan> {
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

#[cfg(test)]
#[path = "app_usecase_discovery_tests.rs"]
mod app_usecase_discovery_tests;
