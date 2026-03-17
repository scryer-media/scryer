use super::*;
use crate::acquisition_policy::{AcquisitionThresholds, evaluate_upgrade};
use crate::delay_profile::DelayProfile;
use chrono::{DateTime, Utc};
use scryer_domain::NotificationEventType;
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};

const RSS_SYNC_MAX_GUIDS: usize = 2000;

/// Normalize a title string for fuzzy matching: lowercase, strip non-alphanumeric,
/// collapse whitespace.
pub(crate) fn normalize_for_matching(title: &str) -> String {
    title
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c.is_whitespace() || c == '.' || c == '-' || c == '_' {
                Some(' ')
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build a lookup index: normalized title name → Vec<(title, category, tvdb_id, imdb_id)>
/// Includes aliases for broader matching.
fn build_title_lookup(titles: &[Title]) -> HashMap<String, Vec<TitleMatchInfo>> {
    let mut lookup: HashMap<String, Vec<TitleMatchInfo>> = HashMap::new();

    for title in titles {
        if !title.monitored {
            continue;
        }

        let info = TitleMatchInfo {
            title_id: title.id.clone(),
            year: title.year,
        };

        // Index by primary name
        let normalized = normalize_for_matching(&title.name);
        if !normalized.is_empty() {
            lookup.entry(normalized).or_default().push(info.clone());
        }

        // Index by aliases
        for alias in &title.aliases {
            let normalized_alias = normalize_for_matching(alias);
            if !normalized_alias.is_empty() {
                lookup
                    .entry(normalized_alias)
                    .or_default()
                    .push(info.clone());
            }
        }
    }

    lookup
}

#[derive(Clone)]
struct TitleMatchInfo {
    title_id: String,
    year: Option<i32>,
}

/// Extract the series/movie title portion from a release name by taking
/// everything before the first recognized quality/episode marker.
fn extract_title_from_release(parsed: &ParsedReleaseMetadata) -> String {
    normalize_for_matching(&parsed.normalized_title)
}

/// Try to match a parsed release against the title lookup.
/// Returns the best matching title info, if any.
fn match_release_to_title<'a>(
    parsed: &ParsedReleaseMetadata,
    lookup: &'a HashMap<String, Vec<TitleMatchInfo>>,
) -> Option<&'a TitleMatchInfo> {
    let release_title = extract_title_from_release(parsed);
    if release_title.is_empty() {
        return None;
    }

    // Exact match first
    if let Some(matches) = lookup.get(&release_title) {
        // If the release has a year, prefer matching title with same year
        if let Some(year) = parsed.year
            && let Some(m) = matches.iter().find(|m| m.year == Some(year as i32))
        {
            return Some(m);
        }
        return matches.first();
    }

    // Try with year stripped (release "Title 2024" → lookup "title")
    if let Some(year) = parsed.year {
        let year_str = format!(" {year}");
        if let Some(without_year) = release_title.strip_suffix(&year_str)
            && let Some(matches) = lookup.get(without_year)
        {
            return matches.first();
        }
    }

    // Try adding year from lookup (lookup has "title 2024", release has "title")
    for (key, matches) in lookup {
        for m in matches {
            if let Some(year) = m.year {
                let with_year = format!("{release_title} {year}");
                if *key == with_year {
                    return Some(m);
                }
            }
        }
    }

    None
}

impl AppUseCase {
    /// Run a single RSS sync cycle: fetch latest releases from all enabled indexers,
    /// match against monitored titles, score, and grab approved releases.
    pub async fn run_rss_sync(&self) -> AppResult<RssSyncReport> {
        let now = Utc::now();
        let sync_start = std::time::Instant::now();
        info!("starting RSS sync cycle");

        // Load all monitored titles for matching
        let titles = self.services.titles.list(None, None).await?;
        let lookup = build_title_lookup(&titles);

        if lookup.is_empty() {
            info!("RSS sync: no monitored titles, skipping");
            return Ok(RssSyncReport::default());
        }

        // Fetch RSS feed (empty query = latest releases) from all indexers
        let rss_results = self
            .services
            .indexer_client
            .search(
                String::new(), // empty query = RSS feed
                None,
                None,
                None,
                None, // no category filter
                None,
                None, // no routing filter
                500,  // generous limit for RSS
                SearchMode::Auto,
                None,
                None,
            )
            .await;

        let response = match rss_results {
            Ok(r) => r,
            Err(err) => {
                warn!(error = %err, "RSS sync: failed to fetch RSS feed from indexers");
                return Ok(RssSyncReport::default());
            }
        };

        if response.results.is_empty() {
            info!("RSS sync: no results from indexers");
            return Ok(RssSyncReport::default());
        }

        info!(
            result_count = response.results.len(),
            "RSS sync: fetched releases from indexers"
        );

        // Dedup against previously seen GUIDs (in-memory, resets on restart)
        let mut seen_guids = self.services.rss_seen_guids.write().await;
        let initial_seen_count = seen_guids.len();

        let mut new_results: Vec<IndexerSearchResult> = Vec::new();
        for result in response.results {
            let guid = result
                .guid
                .as_deref()
                .or(result.download_url.as_deref())
                .or(result.link.as_deref())
                .unwrap_or(&result.title);

            if seen_guids.insert(guid.to_string()) {
                new_results.push(result);
            }
        }

        // Cap the seen set to prevent unbounded growth
        if seen_guids.len() > RSS_SYNC_MAX_GUIDS {
            let excess = seen_guids.len() - RSS_SYNC_MAX_GUIDS;
            let to_remove: Vec<String> = seen_guids.iter().take(excess).cloned().collect();
            for key in to_remove {
                seen_guids.remove(&key);
            }
        }

        // Release the write lock before doing any I/O
        drop(seen_guids);

        info!(
            new_count = new_results.len(),
            previously_seen = initial_seen_count,
            "RSS sync: filtered to new releases"
        );

        if new_results.is_empty() {
            return Ok(RssSyncReport::default());
        }

        // Parse each release and match against monitored titles
        let mut matched_by_title: HashMap<String, Vec<IndexerSearchResult>> = HashMap::new();
        let mut matched_count = 0usize;
        let total_new = new_results.len();

        for result in new_results {
            let parsed = parse_release_metadata(&result.title);

            if let Some(title_info) = match_release_to_title(&parsed, &lookup) {
                matched_count += 1;
                matched_by_title
                    .entry(title_info.title_id.clone())
                    .or_default()
                    .push(result);
            }
        }

        info!(
            matched = matched_count,
            titles_matched = matched_by_title.len(),
            "RSS sync: matched releases to monitored titles"
        );

        // Snapshot download client state
        let dl_snapshot = super::app_usecase_acquisition::DownloadClientSnapshot::fetch(self).await;
        let delay_profiles = self.load_delay_profiles().await;
        let mut grabbed_urls: HashSet<String> = HashSet::new();
        let mut report = RssSyncReport {
            releases_fetched: total_new,
            releases_matched: matched_count,
            ..Default::default()
        };

        // For each matched title, score and potentially grab
        for (title_id, releases) in &matched_by_title {
            let title = match self.services.titles.get_by_id(title_id).await {
                Ok(Some(t)) => t,
                _ => continue,
            };

            // Check if there's a wanted item for this title
            let wanted = self
                .services
                .wanted_items
                .get_wanted_item_for_title(title_id, None)
                .await
                .ok()
                .flatten();

            // For series, we need to match individual episodes
            let has_episodes = self
                .facet_registry
                .get(&title.facet)
                .map(|h| h.has_episodes())
                .unwrap_or(false);

            if has_episodes {
                // For series: match each release to a specific episode's wanted item
                self.process_rss_series_releases(
                    &title,
                    releases,
                    &dl_snapshot,
                    &delay_profiles,
                    &mut grabbed_urls,
                    &mut report,
                    &now,
                )
                .await;
            } else {
                // For movies: use the title-level wanted item
                let Some(wanted) = wanted else {
                    continue;
                };
                if wanted.status == "grabbed" && wanted.current_score.is_some() {
                    // Already grabbed — only proceed if upgrade is possible
                }
                self.process_rss_title_releases(
                    &title,
                    &wanted,
                    releases,
                    &dl_snapshot,
                    &delay_profiles,
                    &mut grabbed_urls,
                    &mut report,
                    &now,
                )
                .await;
            }
        }

        info!(
            fetched = report.releases_fetched,
            matched = report.releases_matched,
            grabbed = report.releases_grabbed,
            held = report.releases_held,
            "RSS sync cycle completed"
        );

        metrics::counter!("scryer_rss_sync_total").increment(1);
        metrics::histogram!("scryer_rss_sync_duration_seconds")
            .record(sync_start.elapsed().as_secs_f64());
        metrics::counter!("scryer_rss_releases_fetched_total")
            .increment(report.releases_fetched as u64);
        metrics::counter!("scryer_rss_releases_matched_total")
            .increment(report.releases_matched as u64);
        metrics::counter!("scryer_rss_releases_grabbed_total")
            .increment(report.releases_grabbed as u64);

        Ok(report)
    }

    /// Process RSS releases matched to a movie title.
    async fn process_rss_title_releases(
        &self,
        title: &Title,
        wanted: &WantedItem,
        releases: &[IndexerSearchResult],
        dl_snapshot: &super::app_usecase_acquisition::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        let category = self
            .facet_registry
            .get(&title.facet)
            .map(|h| h.search_category().to_string())
            .unwrap_or_else(|| "movie".to_string());

        let tvdb_id = title
            .external_ids
            .iter()
            .find(|id| id.source == "tvdb")
            .map(|id| id.value.clone());

        // Score all releases against quality profile
        let scored = match self
            .score_rss_releases(
                releases,
                title.imdb_id.clone(),
                tvdb_id.clone(),
                Some(category.clone()),
                &title.tags,
                title.runtime_minutes,
            )
            .await
        {
            Ok(s) => s,
            Err(err) => {
                warn!(
                    title = title.name.as_str(),
                    error = %err,
                    "RSS sync: failed to score releases"
                );
                return;
            }
        };

        // Try to grab the best candidate using the same logic as acquisition
        self.try_grab_rss_release(
            title,
            wanted,
            &scored,
            &category,
            dl_snapshot,
            delay_profiles,
            grabbed_urls,
            report,
            now,
        )
        .await;
    }

    /// Process RSS releases matched to a series title — match episodes individually.
    async fn process_rss_series_releases(
        &self,
        title: &Title,
        releases: &[IndexerSearchResult],
        dl_snapshot: &super::app_usecase_acquisition::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        let category = self
            .facet_registry
            .get(&title.facet)
            .map(|h| h.search_category().to_string())
            .unwrap_or_else(|| "series".to_string());

        let tvdb_id = title
            .external_ids
            .iter()
            .find(|id| id.source == "tvdb")
            .map(|id| id.value.clone());

        // Group releases by (season, episode) from parsed metadata
        let mut by_episode: HashMap<(Option<u32>, Vec<u32>), Vec<&IndexerSearchResult>> =
            HashMap::new();

        for release in releases {
            let parsed = parse_release_metadata(&release.title);
            if let Some(ref ep) = parsed.episode {
                let key = (ep.season, ep.episode_numbers.clone());
                by_episode.entry(key).or_default().push(release);
            }
        }

        for ((season, episode_numbers), episode_releases) in &by_episode {
            // Find the wanted item for this specific episode
            let episode_id =
                if let (Some(season_num), Some(ep_num)) = (season, episode_numbers.first()) {
                    // Look up the episode by season/episode number
                    self.find_episode_id_for_title(&title.id, *season_num, *ep_num)
                        .await
                } else {
                    None
                };

            let episode_id = match episode_id {
                Some(id) => id,
                None => continue, // No matching episode in our DB
            };

            let wanted = match self
                .services
                .wanted_items
                .get_wanted_item_for_title(&title.id, Some(&episode_id))
                .await
            {
                Ok(Some(w)) => w,
                _ => continue, // Not wanted
            };

            if wanted.status != "wanted" && wanted.status != "grabbed" {
                continue;
            }

            // Score these releases
            let owned_releases: Vec<IndexerSearchResult> =
                episode_releases.iter().map(|r| (*r).clone()).collect();
            let scored = match self
                .score_rss_releases(
                    &owned_releases,
                    title.imdb_id.clone(),
                    tvdb_id.clone(),
                    Some(category.clone()),
                    &title.tags,
                    title.runtime_minutes,
                )
                .await
            {
                Ok(s) => s,
                Err(_) => continue,
            };

            self.try_grab_rss_release(
                title,
                &wanted,
                &scored,
                &category,
                dl_snapshot,
                delay_profiles,
                grabbed_urls,
                report,
                now,
            )
            .await;
        }
    }

    /// Find episode ID by title_id + season + episode number.
    async fn find_episode_id_for_title(
        &self,
        title_id: &str,
        season: u32,
        episode: u32,
    ) -> Option<String> {
        // List collections (seasons) for this title, find the right one
        let collections = self
            .services
            .shows
            .list_collections_for_title(title_id)
            .await
            .ok()?;

        let season_str = season.to_string();
        let collection = collections
            .iter()
            .find(|c| c.collection_index == season_str)?;

        let episodes = self
            .services
            .shows
            .list_episodes_for_collection(&collection.id)
            .await
            .ok()?;

        let episode_str = episode.to_string();
        episodes
            .iter()
            .find(|ep| ep.episode_number.as_deref() == Some(&episode_str))
            .map(|ep| ep.id.clone())
    }

    /// Score a batch of RSS releases against the quality profile.
    async fn score_rss_releases(
        &self,
        releases: &[IndexerSearchResult],
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        title_tags: &[String],
        runtime_minutes: Option<i32>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let quality_profile = self
            .resolve_quality_profile(
                title_tags,
                imdb_id.as_deref(),
                tvdb_id.as_deref(),
                category.as_deref(),
            )
            .await?;

        let failed_signatures = self
            .services
            .release_attempts
            .list_failed_release_signatures(5000)
            .await
            .unwrap_or_default();

        let failed_source_hints: HashSet<String> = failed_signatures
            .iter()
            .filter_map(|s| normalize_release_attempt_hint(s.source_hint.as_deref()))
            .collect();
        let failed_source_titles: HashSet<String> = failed_signatures
            .iter()
            .filter_map(|s| normalize_release_attempt_title(s.source_title.as_deref()))
            .collect();

        let user_rules_engine = self
            .services
            .user_rules
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
        let mut user_evaluator = user_rules_engine.evaluator();

        let mut scored = Vec::new();
        let mut seen = HashSet::new();

        for result in releases {
            let key = result
                .download_url
                .as_deref()
                .or(result.link.as_deref())
                .unwrap_or(&result.title)
                .to_string();
            if !seen.insert(key) {
                continue;
            }

            if crate::app_usecase_discovery::is_release_blocklisted(
                result,
                &failed_source_hints,
                &failed_source_titles,
            ) {
                continue;
            }

            let parsed = parse_release_metadata(&result.title);
            let persona = quality_profile
                .criteria
                .resolve_persona(category.as_deref());
            let weights = crate::scoring_weights::build_weights(
                persona,
                &quality_profile.criteria.scoring_overrides,
            );
            let mut decision = evaluate_against_profile(&quality_profile, &parsed, false, &weights);
            apply_age_scoring(&mut decision, result.published_at.as_deref());
            crate::quality_profile::apply_size_scoring_for_category(
                &mut decision,
                &parsed,
                result.size_bytes,
                category.as_deref(),
                runtime_minutes,
                &weights,
            );

            if !user_rules_engine.is_empty() {
                let user_input = crate::app_usecase_discovery::build_user_rule_input(
                    &parsed,
                    &quality_profile,
                    result,
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
                                crate::quality_profile::ScoringSource::UserRule(entry.rule_set_id),
                            );
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "RSS sync: user rule evaluation failed");
                    }
                }
            }

            scored.push(IndexerSearchResult {
                parsed_release_metadata: Some(parsed),
                quality_profile_decision: Some(decision),
                ..result.clone()
            });
        }

        scored.sort_by(|a, b| {
            let a_allowed = a
                .quality_profile_decision
                .as_ref()
                .map(|d| d.allowed)
                .unwrap_or(false);
            let b_allowed = b
                .quality_profile_decision
                .as_ref()
                .map(|d| d.allowed)
                .unwrap_or(false);

            match (a_allowed, b_allowed) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let a_score = a
                        .quality_profile_decision
                        .as_ref()
                        .map(|d| d.preference_score)
                        .unwrap_or(0);
                    let b_score = b
                        .quality_profile_decision
                        .as_ref()
                        .map(|d| d.preference_score)
                        .unwrap_or(0);
                    b_score.cmp(&a_score)
                }
            }
        });

        Ok(scored)
    }

    /// Try to grab the best candidate from scored RSS releases.
    /// Reuses the same logic as process_single_wanted_item for consistency.
    async fn try_grab_rss_release(
        &self,
        title: &Title,
        wanted: &WantedItem,
        scored: &[IndexerSearchResult],
        category: &str,
        dl_snapshot: &super::app_usecase_acquisition::DownloadClientSnapshot,
        delay_profiles: &[DelayProfile],
        grabbed_urls: &mut HashSet<String>,
        report: &mut RssSyncReport,
        now: &DateTime<Utc>,
    ) {
        // Load DB blocklist
        let db_blocklist: HashSet<String> = self
            .services
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 200)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| e.source_title)
            .map(|t| t.to_ascii_lowercase())
            .collect();

        let mut selected: Option<&IndexerSearchResult> = None;

        for candidate in scored {
            let is_allowed = candidate
                .quality_profile_decision
                .as_ref()
                .map(|d| d.allowed)
                .unwrap_or(false);
            if !is_allowed {
                continue;
            }

            if dl_snapshot.is_active(&candidate.title) {
                continue;
            }
            if dl_snapshot.failed_item(&candidate.title).is_some() {
                continue;
            }
            if db_blocklist.contains(&candidate.title.to_ascii_lowercase()) {
                continue;
            }

            selected = Some(candidate);
            break;
        }

        let Some(best) = selected else {
            return;
        };

        let candidate_score = best
            .quality_profile_decision
            .as_ref()
            .map(|d| d.preference_score)
            .unwrap_or(0);

        // Evaluate upgrade decision
        let tvdb_id = title
            .external_ids
            .iter()
            .find(|id| id.source == "tvdb")
            .map(|id| id.value.clone());

        let profile = self
            .resolve_quality_profile(
                &title.tags,
                title.imdb_id.as_deref(),
                tvdb_id.as_deref(),
                Some(category),
            )
            .await
            .unwrap_or_else(|_| crate::quality_profile::default_quality_profile_for_search());

        // Cutoff tier check
        if crate::quality_profile::has_reached_cutoff(
            wanted.grabbed_release.as_deref(),
            profile.criteria.cutoff_tier.as_deref(),
            &profile.criteria.quality_tiers,
        ) {
            return;
        }

        let thresholds = AcquisitionThresholds::default();
        let decision = evaluate_upgrade(
            candidate_score,
            wanted.current_score,
            profile.criteria.allow_upgrades,
            wanted.last_search_at.as_deref(),
            now,
            &thresholds,
        );

        // Record the decision
        let decision_record = ReleaseDecision {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: best.title.clone(),
            release_url: best.download_url.clone().or_else(|| best.link.clone()),
            release_size_bytes: best.size_bytes,
            decision_code: decision.code().to_string(),
            candidate_score,
            current_score: wanted.current_score,
            score_delta: wanted.current_score.map(|c| candidate_score - c),
            explanation_json: best.quality_profile_decision.as_ref().map(|d| {
                serde_json::to_string(
                    &d.scoring_log
                        .iter()
                        .map(|e| serde_json::json!({"code": e.code, "delta": e.delta}))
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()
            }),
            created_at: now.to_rfc3339(),
        };

        let _ = self
            .services
            .wanted_items
            .insert_release_decision(&decision_record)
            .await;

        if !decision.is_accept() {
            return;
        }

        // Check delay profile — hold release instead of grabbing immediately
        if let Some(dp) =
            crate::delay_profile::resolve_delay_profile(delay_profiles, &title.tags, &title.facet)
            && !crate::delay_profile::should_bypass_delay(dp, candidate_score)
        {
            // Hold release as pending instead of grabbing
            let scoring_json = best.quality_profile_decision.as_ref().map(|d| {
                serde_json::to_string(
                    &d.scoring_log
                        .iter()
                        .map(|e| serde_json::json!({"code": e.code, "delta": e.delta}))
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()
            });
            self.insert_pending_release(
                wanted,
                title,
                &best.title,
                best.download_url.as_deref().or(best.link.as_deref()),
                best.source_kind,
                best.size_bytes,
                candidate_score,
                scoring_json,
                Some(best.source.as_str()),
                best.guid.as_deref(),
                dp.delay_hours,
            )
            .await;
            report.releases_held += 1;
            return;
        }

        // Submit to download client
        let source_hint = best.download_url.clone().or_else(|| best.link.clone());

        if let Some(url) = source_hint.as_deref()
            && !grabbed_urls.insert(url.to_string())
        {
            return; // Already submitted this cycle
        }

        let source_title = Some(best.title.clone());
        let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_title(source_title.as_deref());
        let source_password =
            normalize_release_password(best.nzbgeek_password_protected.as_deref());

        let _ = self
            .services
            .release_attempts
            .record_release_attempt(
                Some(title.id.clone()),
                source_hint_for_attempt.clone(),
                source_title_for_attempt.clone(),
                ReleaseDownloadAttemptOutcome::Pending,
                None,
                source_password.clone(),
            )
            .await;

        let download_cat = self.derive_download_category(&title.facet).await;
        let is_recent = self.is_recent_for_queue_priority(
            best.published_at
                .as_deref()
                .or(title.first_aired.as_deref())
                .or(title.digital_release_date.as_deref()),
        );

        info!(
            title = title.name.as_str(),
            release = best.title.as_str(),
            score = candidate_score,
            "RSS sync: auto-grabbing release"
        );

        let grab_result = self
            .services
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                source_hint: source_hint.clone(),
                source_kind: best.source_kind,
                source_title: source_title.clone(),
                source_password: source_password.clone(),
                category: Some(download_cat),
                queue_priority: None,
                download_directory: None,
                release_title: Some(best.title.clone()),
                indexer_name: Some(best.source.clone()),
                info_hash_hint: best
                    .extra
                    .get("info_hash")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: None,
            })
            .await;

        match grab_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => best.source.clone(), "facet" => facet_label).increment(1);
                }

                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password,
                    )
                    .await;

                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let _ = self
                    .services
                    .download_submissions
                    .record_submission(DownloadSubmission {
                        title_id: title.id.clone(),
                        facet: facet_str.trim_matches('"').to_string(),
                        download_client_type: grab.client_type,
                        download_client_item_id: grab.job_id,
                        source_title: source_title.clone(),
                        collection_id: None,
                    })
                    .await;

                let grabbed_json = serde_json::json!({
                    "title": best.title,
                    "score": candidate_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source": "rss_sync",
                })
                .to_string();

                let _ = self
                    .services
                    .wanted_items
                    .update_wanted_item_status(
                        &wanted.id,
                        "grabbed",
                        None,
                        Some(&now.to_rfc3339()),
                        wanted.search_count,
                        Some(candidate_score),
                        Some(&grabbed_json),
                    )
                    .await;

                {
                    let mut grab_meta = HashMap::new();
                    grab_meta.insert("title_name".to_string(), serde_json::json!(title.name));
                    grab_meta.insert("release_title".to_string(), serde_json::json!(best.title));
                    grab_meta.insert("indexer".to_string(), serde_json::json!(best.source));
                    grab_meta.insert("score".to_string(), serde_json::json!(candidate_score));
                    let grab_envelope = crate::activity::NotificationEnvelope {
                        event_type: NotificationEventType::Grab,
                        title: format!("Grabbed: {}", title.name),
                        body: format!(
                            "RSS sync grabbed '{}' for {} (score: {})",
                            best.title, title.name, candidate_score
                        ),
                        facet: Some(format!("{:?}", title.facet).to_lowercase()),
                        metadata: grab_meta,
                    };
                    let _ = self
                        .services
                        .record_activity_event_with_notification(
                            None,
                            Some(title.id.clone()),
                            ActivityKind::MovieDownloaded,
                            format!(
                                "RSS sync grabbed: {} (score: {})",
                                best.title, candidate_score
                            ),
                            ActivitySeverity::Success,
                            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                            grab_envelope,
                        )
                        .await;
                }

                report.releases_grabbed += 1;
            }
            Err(err) => {
                warn!(
                    title = title.name.as_str(),
                    release = best.title.as_str(),
                    error = %err,
                    "RSS sync: download submission failed"
                );

                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(err.to_string()),
                        source_password,
                    )
                    .await;
            }
        }
    }
}

#[derive(Default, Debug)]
pub struct RssSyncReport {
    pub releases_fetched: usize,
    pub releases_matched: usize,
    pub releases_grabbed: usize,
    pub releases_held: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{MediaFacet, Title};

    fn make_title(id: &str, name: &str, year: Option<i32>) -> Title {
        Title {
            id: id.to_string(),
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: chrono::Utc::now(),
            year,
            overview: None,
            poster_url: None,
            banner_url: None,
            background_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
        }
    }

    fn make_title_with_aliases(
        id: &str,
        name: &str,
        year: Option<i32>,
        aliases: Vec<&str>,
    ) -> Title {
        let mut t = make_title(id, name, year);
        t.aliases = aliases.into_iter().map(|s| s.to_string()).collect();
        t
    }

    fn make_unmonitored(id: &str, name: &str) -> Title {
        let mut t = make_title(id, name, None);
        t.monitored = false;
        t
    }

    // ── normalize_for_matching ──────────────────────────────────────

    #[test]
    fn normalize_basic_title() {
        assert_eq!(normalize_for_matching("The Dark Knight"), "the dark knight");
    }

    #[test]
    fn normalize_dots_and_dashes() {
        assert_eq!(
            normalize_for_matching("The.Dark.Knight-2008"),
            "the dark knight 2008"
        );
    }

    #[test]
    fn normalize_underscores() {
        assert_eq!(normalize_for_matching("the_dark_knight"), "the dark knight");
    }

    #[test]
    fn normalize_strips_special_chars() {
        assert_eq!(
            normalize_for_matching("Spider-Man: Across the Spider-Verse"),
            "spider man across the spider verse"
        );
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(
            normalize_for_matching("  The   Dark   Knight  "),
            "the dark knight"
        );
    }

    #[test]
    fn normalize_empty() {
        assert_eq!(normalize_for_matching(""), "");
    }

    #[test]
    fn normalize_unicode_alphanumeric() {
        // é is alphanumeric in Unicode, so it's preserved
        assert_eq!(normalize_for_matching("café"), "café");
    }

    // ── build_title_lookup ──────────────────────────────────────────

    #[test]
    fn lookup_indexes_by_primary_name() {
        let titles = vec![make_title("t1", "Inception", Some(2010))];
        let lookup = build_title_lookup(&titles);
        assert!(lookup.contains_key("inception"));
        assert_eq!(lookup["inception"].len(), 1);
        assert_eq!(lookup["inception"][0].title_id, "t1");
    }

    #[test]
    fn lookup_skips_unmonitored() {
        let titles = vec![make_unmonitored("t1", "Inception")];
        let lookup = build_title_lookup(&titles);
        assert!(lookup.is_empty());
    }

    #[test]
    fn lookup_indexes_aliases() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "Spirited Away",
            Some(2001),
            vec!["Sen to Chihiro no Kamikakushi"],
        )];
        let lookup = build_title_lookup(&titles);
        assert!(lookup.contains_key("spirited away"));
        assert!(lookup.contains_key("sen to chihiro no kamikakushi"));
    }

    #[test]
    fn lookup_multiple_titles_same_normalized_name() {
        let titles = vec![
            make_title("t1", "Dune", Some(1984)),
            make_title("t2", "Dune", Some(2021)),
        ];
        let lookup = build_title_lookup(&titles);
        assert_eq!(lookup["dune"].len(), 2);
    }

    // ── match_release_to_title ──────────────────────────────────────

    #[test]
    fn match_exact_title() {
        let titles = vec![make_title("t1", "Inception", Some(2010))];
        let lookup = build_title_lookup(&titles);
        let parsed = crate::parse_release_metadata("Inception.2010.1080p.BluRay.x264");
        let result = match_release_to_title(&parsed, &lookup);
        assert!(result.is_some(), "exact match should succeed");
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_prefers_year_match() {
        let titles = vec![
            make_title("t1", "Dune", Some(1984)),
            make_title("t2", "Dune", Some(2021)),
        ];
        let lookup = build_title_lookup(&titles);
        let parsed = crate::parse_release_metadata("Dune.2021.1080p.BluRay.x264");
        let result = match_release_to_title(&parsed, &lookup);
        assert!(result.is_some(), "result was None");
        assert_eq!(result.unwrap().title_id, "t2");
    }

    #[test]
    fn match_with_year_stripped_from_release() {
        // Release has "Title 2010", lookup only has "Title" (with year in metadata)
        let t = make_title("t1", "Inception", Some(2010));
        // Name doesn't include the year
        let titles = vec![t];
        let lookup = build_title_lookup(&titles);
        let parsed = crate::parse_release_metadata("Inception.2010.1080p.BluRay");
        let result = match_release_to_title(&parsed, &lookup);
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_release_title_without_year_finds_title_with_year() {
        // Lookup has "title 2024", release only has "title"
        let titles = vec![make_title("t1", "Dune 2024", Some(2024))];
        let lookup = build_title_lookup(&titles);
        let parsed = ParsedReleaseMetadata {
            raw_title: "Dune".to_string(),
            normalized_title: "Dune".to_string(),
            year: None,
            ..Default::default()
        };
        let result = match_release_to_title(&parsed, &lookup);
        // Should match via the reverse year-addition path
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    #[test]
    fn match_no_match_returns_none() {
        let titles = vec![make_title("t1", "Inception", Some(2010))];
        let lookup = build_title_lookup(&titles);
        let parsed = crate::parse_release_metadata("Totally.Unknown.Movie.2024.1080p");
        let result = match_release_to_title(&parsed, &lookup);
        assert!(result.is_none());
    }

    #[test]
    fn match_empty_release_title_returns_none() {
        let titles = vec![make_title("t1", "Inception", Some(2010))];
        let lookup = build_title_lookup(&titles);
        let parsed = ParsedReleaseMetadata {
            raw_title: String::new(),
            normalized_title: String::new(),
            ..Default::default()
        };
        let result = match_release_to_title(&parsed, &lookup);
        assert!(result.is_none());
    }

    #[test]
    fn match_via_alias() {
        let titles = vec![make_title_with_aliases(
            "t1",
            "Spirited Away",
            Some(2001),
            vec!["Sen to Chihiro no Kamikakushi"],
        )];
        let lookup = build_title_lookup(&titles);
        let parsed = ParsedReleaseMetadata {
            raw_title: "Sen.to.Chihiro.no.Kamikakushi".to_string(),
            normalized_title: "Sen to Chihiro no Kamikakushi".to_string(),
            ..Default::default()
        };
        let result = match_release_to_title(&parsed, &lookup);
        assert!(result.is_some());
        assert_eq!(result.unwrap().title_id, "t1");
    }

    // ── extract_title_from_release ──────────────────────────────────

    #[test]
    fn extract_title_normalizes() {
        let parsed = crate::parse_release_metadata("The.Dark.Knight.2008.1080p.BluRay");
        let title = extract_title_from_release(&parsed);
        assert_eq!(title, "the dark knight");
    }
}
