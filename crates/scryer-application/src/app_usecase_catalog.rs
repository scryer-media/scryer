use super::*;
use scryer_domain::{InterstitialMovieMetadata, NotificationEventType};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tracing::{info, warn};

/// Extract the `category` from the highest-priority client in a `nzbget.client_routing`
/// JSON blob (`{ "client_id": { "category": "Movies", ... }, ... }`).
/// With `serde_json` `preserve_order`, the first key is the highest-priority client.
fn extract_highest_priority_nzbget_category(raw_json: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    let obj = parsed.as_object()?;
    for (_client_id, config) in obj {
        if let Some(cat) = config.get("category").and_then(|v| v.as_str()) {
            let trimmed = cat.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn interstitial_movie_from_metadata(movie: &MovieMetadata) -> InterstitialMovieMetadata {
    InterstitialMovieMetadata {
        tvdb_id: movie.tvdb_id.to_string(),
        name: movie.name.clone(),
        slug: movie.slug.clone(),
        year: movie.year,
        content_status: movie.content_status.clone(),
        overview: movie.overview.clone(),
        poster_url: movie.poster_url.clone(),
        language: movie.language.clone(),
        runtime_minutes: movie.runtime_minutes,
        sort_title: movie.sort_title.clone(),
        imdb_id: movie.imdb_id.clone(),
        genres: movie.genres.clone(),
        studio: movie.studio.clone(),
        digital_release_date: movie.tmdb_release_date.clone(),
    }
}

impl AppUseCase {
    pub async fn list_titles(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.titles.list(facet, query).await
    }

    pub async fn list_title_release_blocklist(
        &self,
        actor: &User,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        require(actor, &Entitlement::ViewCatalog)?;
        let bounded_limit = limit.clamp(1, 1_000);
        self.services
            .release_attempts
            .list_failed_release_signatures_for_title(title_id, bounded_limit)
            .await
    }

    pub async fn add_title(&self, actor: &User, request: NewTitle) -> AppResult<Title> {
        require(actor, &Entitlement::ManageTitle)?;

        if request.name.trim().is_empty() {
            return Err(AppError::Validation("title name is required".into()));
        }

        let title = Title {
            id: Id::new().0,
            name: request.name.trim().to_string(),
            facet: request.facet,
            monitored: request.monitored,
            tags: normalize_tags(&request.tags),
            external_ids: sanitize_ids(request.external_ids),
            created_by: Some(actor.id.clone()),
            created_at: Utc::now(),
            year: request.year,
            overview: request.overview,
            poster_url: request.poster_url,
            sort_title: request.sort_title,
            slug: request.slug,
            imdb_id: None,
            runtime_minutes: request.runtime_minutes,
            genres: vec![],
            content_status: request.content_status,
            language: request.language,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: request.min_availability,
            digital_release_date: None,
        };

        let title = self.services.titles.create(title).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                EventType::TitleAdded,
                format!("title added: {}", title.name),
            )
            .await?;

        if let Some(handler) = self.facet_registry.get(&title.facet) {
            if let Some(activity_kind) = handler.title_added_activity_kind() {
                self.services
                    .record_activity_event(
                        Some(actor.id.clone()),
                        Some(title.id.clone()),
                        activity_kind,
                        format!("new {} added: {}", handler.facet_id(), title.name),
                        ActivitySeverity::Info,
                        vec![ActivityChannel::WebUi],
                    )
                    .await?;
            }
        }

        // Dispatch notification for title added
        {
            let facet_str = format!("{:?}", title.facet).to_lowercase();
            let mut metadata = HashMap::new();
            metadata.insert("title_name".to_string(), serde_json::json!(title.name));
            if let Some(ref year) = title.year {
                metadata.insert("title_year".to_string(), serde_json::json!(year));
            }
            metadata.insert("title_facet".to_string(), serde_json::json!(facet_str));
            if let Some(ref poster) = title.poster_url {
                metadata.insert("poster_url".to_string(), serde_json::json!(poster));
            }
            self.dispatch_notification(
                NotificationEventType::TitleAdded.as_str(),
                &format!("{} added: {}", facet_str, title.name),
                &format!("{} has been added to your library.", title.name),
                &metadata,
            )
            .await;
        }

        // Wake the background hydration loop to fetch rich metadata from SMG.
        // The title is already persisted — hydration happens asynchronously.
        self.services.hydration_wake.notify_one();

        Ok(title)
    }

    /// Hydrate a single title by fetching metadata from SMG.
    /// Used for the interactive single-title path (e.g. user adds one title via UI).
    async fn hydrate_title_metadata(&self, title: Title) -> Title {
        let tvdb_id = match extract_tvdb_id(&title) {
            Some(id) => id,
            None => {
                warn!(
                    title_id = %title.id,
                    external_ids = ?title.external_ids,
                    "no tvdb external id found, skipping metadata hydration"
                );
                return title;
            }
        };

        let language = "eng";

        let Some(handler) = self.facet_registry.get(&title.facet) else {
            return title;
        };

        match handler
            .hydrate_metadata(self.services.metadata_gateway.as_ref(), tvdb_id, language)
            .await
        {
            Ok(result) => self.apply_hydration_result(title, result).await,
            Err(err) => {
                warn!(
                    title_id = %title.id,
                    tvdb_id = tvdb_id,
                    error = %err,
                    "failed to fetch metadata from gateway"
                );
                title
            }
        }
    }

    /// Apply a [`HydrationResult`] to a title: persist metadata, create
    /// seasons/episodes, and enrich with anime mapping data.
    async fn apply_hydration_result(&self, title: Title, result: super::HydrationResult) -> Title {
        let has_episodes = self
            .facet_registry
            .get(&title.facet)
            .is_some_and(|h| h.has_episodes());

        if has_episodes {
            info!(
                title_id = %title.id,
                seasons = result.seasons.len(),
                episodes = result.episodes.len(),
                "received series metadata from gateway"
            );
        }

        // Build extra external IDs from the primary anime mapping only.
        let mut metadata_update = result.metadata_update;
        if let Some(mapping) = result.anime_mappings.first() {
            if let Some(mal_id) = mapping.mal_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "mal".to_string(),
                    value: mal_id.to_string(),
                });
            }
            if let Some(anilist_id) = mapping.anilist_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "anilist".to_string(),
                    value: anilist_id.to_string(),
                });
            }
            if let Some(anidb_id) = mapping.anidb_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "anidb".to_string(),
                    value: anidb_id.to_string(),
                });
            }
            if let Some(kitsu_id) = mapping.kitsu_id {
                metadata_update.extra_external_ids.push(ExternalId {
                    source: "kitsu".to_string(),
                    value: kitsu_id.to_string(),
                });
            }
        }

        // Store anime-specific metadata as tags on the title
        if let Some(primary) = result.anime_mappings.first() {
            if let Some(score) = primary.score {
                metadata_update
                    .extra_tags
                    .push(format!("scryer:mal-score:{score}"));
            }
            if !primary.anime_media_type.is_empty() {
                metadata_update.extra_tags.push(format!(
                    "scryer:anime-media-type:{}",
                    primary.anime_media_type
                ));
            }
            if !primary.status.is_empty() {
                metadata_update
                    .extra_tags
                    .push(format!("scryer:anime-status:{}", primary.status));
            }
        }

        let title = match self
            .services
            .titles
            .update_title_hydrated_metadata(&title.id, metadata_update)
            .await
        {
            Ok(updated) => updated,
            Err(err) => {
                warn!(
                    title_id = %title.id,
                    error = %err,
                    "failed to persist metadata"
                );
                title
            }
        };

        if !result.seasons.is_empty() || !result.episodes.is_empty() {
            self.create_series_seasons_and_episodes(
                &title,
                &result.seasons,
                &result.episodes,
                &result.anime_mappings,
            )
            .await;
        }

        title
    }

    async fn create_series_seasons_and_episodes(
        &self,
        title: &Title,
        seasons: &[SeasonMetadata],
        episodes: &[EpisodeMetadata],
        anime_mappings: &[AnimeMapping],
    ) {
        let monitor_type = if title.monitored {
            extract_monitor_type(&title.tags)
        } else {
            "none".to_string()
        };
        info!(
            title_id = %title.id,
            monitor_type = %monitor_type,
            tags = ?title.tags,
            episode_count = episodes.len(),
            "creating series seasons and episodes"
        );

        // Build a map from season number -> collection_id for episode assignment.
        // Only create one collection per season number, preferring "official" episode_type.
        let mut best_season_by_number: std::collections::HashMap<i32, &SeasonMetadata> =
            std::collections::HashMap::new();
        for season in seasons {
            let existing = best_season_by_number.get(&season.number);
            if existing.is_none() || season.episode_type == "official" {
                best_season_by_number.insert(season.number, season);
            }
        }

        let monitor_specials = if title.facet == MediaFacet::Anime {
            // Per-title tag overrides global setting
            if let Some(per_title) = extract_tag_bool(&title.tags, "scryer:monitor-specials:") {
                per_title
            } else {
                self.read_setting_string_value("anime.monitor_specials", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some("true") // Default: false
            }
        } else {
            false
        };

        let inter_season_movies = if title.facet == MediaFacet::Anime {
            if let Some(per_title) = extract_tag_bool(&title.tags, "scryer:inter-season-movies:") {
                per_title
            } else {
                self.read_setting_string_value("anime.inter_season_movies", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .as_deref()
                    != Some("false") // Default: true
            }
        } else {
            false
        };

        // Seasons that have no episodes should not be auto-monitored.
        let seasons_with_episodes: std::collections::HashSet<i32> =
            episodes.iter().map(|ep| ep.season_number).collect();

        let mut season_number_to_collection: std::collections::HashMap<i32, String> =
            std::collections::HashMap::new();

        for season in best_season_by_number.values() {
            let season_monitored = seasons_with_episodes.contains(&season.number)
                && should_monitor_season(&monitor_type, season.number, monitor_specials);
            let collection_type = if season.number == 0 && title.facet == MediaFacet::Anime {
                "specials".to_string()
            } else {
                "season".to_string()
            };
            let collection = Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type,
                collection_index: season.number.to_string(),
                label: Some(season.label.clone()),
                ordered_path: None,
                narrative_order: Some(season.number.to_string()),
                first_episode_number: None,
                last_episode_number: None,
                interstitial_movie: None,
                monitored: season_monitored,
                created_at: Utc::now(),
            };

            match self
                .services
                .shows
                .create_collection(collection.clone())
                .await
            {
                Ok(created) => {
                    season_number_to_collection.insert(season.number, created.id);
                }
                Err(err) => {
                    warn!(
                        title_id = %title.id,
                        season_number = season.number,
                        error = %err,
                        "failed to create season collection"
                    );
                }
            }
        }

        // Build last-aired date per regular season from the episode data so
        // we can determine where each interstitial movie falls narratively.
        let mut season_last_aired: std::collections::BTreeMap<i32, String> =
            std::collections::BTreeMap::new();
        for ep in episodes.iter() {
            if ep.season_number > 0 && !ep.aired.is_empty() {
                season_last_aired
                    .entry(ep.season_number)
                    .and_modify(|d| {
                        if ep.aired > *d {
                            *d = ep.aired.clone();
                        }
                    })
                    .or_insert_with(|| ep.aired.clone());
            }
        }

        // Create interstitial movie collections for anime titles.
        // Movies (global_media_type == "movie") are positioned narratively between
        // seasons using their episode aired dates (e.g. Demon Slayer: Mugen Train
        // between S1 and S2). Build a lookup from (season_number, episode_number) →
        // collection_id so that episodes get routed to the correct collection.
        let mut interstitial_episode_lookup: std::collections::HashMap<(i32, i32), String> =
            std::collections::HashMap::new();

        if title.facet == MediaFacet::Anime && inter_season_movies && !anime_mappings.is_empty() {
            let movie_mappings: Vec<&AnimeMapping> = anime_mappings
                .iter()
                .filter(|m| m.global_media_type == "movie")
                .filter(|m| !m.episode_mappings.is_empty())
                .collect();

            if !movie_mappings.is_empty() {
                let metadata_language = title.metadata_language.as_deref().unwrap_or("eng");
                let movie_tvdb_ids: Vec<i64> = movie_mappings
                    .iter()
                    .filter_map(|mapping| mapping.thetvdb_id.or(mapping.alt_tvdb_id))
                    .collect();
                let interstitial_movie_metadata = match self
                    .services
                    .metadata_gateway
                    .get_movies_bulk(&movie_tvdb_ids, metadata_language)
                    .await
                {
                    Ok(metadata) => metadata,
                    Err(err) => {
                        warn!(
                            title_id = %title.id,
                            error = %err,
                            "failed to fetch interstitial movie metadata"
                        );
                        HashMap::new()
                    }
                };

                // For each movie, find its earliest episode aired date, then
                // find the last regular season that ended on or before that date.
                let mut movies_by_position: std::collections::BTreeMap<i32, Vec<&AnimeMapping>> =
                    std::collections::BTreeMap::new();
                for m in &movie_mappings {
                    let movie_aired: Option<String> = m
                        .episode_mappings
                        .iter()
                        .flat_map(|em| {
                            episodes.iter().filter(move |ep| {
                                ep.season_number == em.tvdb_season
                                    && ep.episode_number >= em.episode_start
                                    && ep.episode_number <= em.episode_end
                            })
                        })
                        .filter(|ep| !ep.aired.is_empty())
                        .map(|ep| ep.aired.clone())
                        .min();

                    let after_season = if let Some(aired) = &movie_aired {
                        season_last_aired
                            .iter()
                            .filter(|(_, last)| last.as_str() <= aired.as_str())
                            .max_by_key(|(&num, _)| num)
                            .map(|(&num, _)| num)
                            .unwrap_or(0)
                    } else {
                        0
                    };

                    info!(
                        title_id = %title.id,
                        movie_type = %m.global_media_type,
                        thetvdb_season = ?m.thetvdb_season,
                        movie_aired = ?movie_aired,
                        after_season = after_season,
                        "positioning interstitial movie"
                    );

                    movies_by_position.entry(after_season).or_default().push(m);
                }

                for (after_season, movies) in &movies_by_position {
                    for (seq, movie) in movies.iter().enumerate() {
                        let narrative_order = format!("{}.{}", after_season, seq + 1);
                        let label = format!("Movie {}", seq + 1);

                        let collection = Collection {
                            id: Id::new().0,
                            title_id: title.id.clone(),
                            collection_type: "interstitial".to_string(),
                            collection_index: narrative_order.clone(),
                            label: Some(label.clone()),
                            ordered_path: None,
                            narrative_order: Some(narrative_order.clone()),
                            first_episode_number: None,
                            last_episode_number: None,
                            interstitial_movie: movie
                                .thetvdb_id
                                .or(movie.alt_tvdb_id)
                                .and_then(|tvdb_id| interstitial_movie_metadata.get(&tvdb_id))
                                .map(interstitial_movie_from_metadata),
                            monitored: true,
                            created_at: Utc::now(),
                        };

                        match self.services.shows.create_collection(collection).await {
                            Ok(created) => {
                                info!(
                                    title_id = %title.id,
                                    label = %label,
                                    narrative_order = %narrative_order,
                                    "created interstitial movie collection"
                                );
                                for em in &movie.episode_mappings {
                                    for ep_num in em.episode_start..=em.episode_end {
                                        interstitial_episode_lookup
                                            .insert((em.tvdb_season, ep_num), created.id.clone());
                                    }
                                }
                            }
                            Err(err) => {
                                warn!(
                                    title_id = %title.id,
                                    label = %label,
                                    error = %err,
                                    "failed to create interstitial movie collection"
                                );
                            }
                        }
                    }
                }
            }
        }

        // Build a lookup from season number → season episode_type for deriving episode type.
        let season_episode_types: std::collections::HashMap<i32, &str> = best_season_by_number
            .iter()
            .map(|(&num, s)| (num, s.episode_type.as_str()))
            .collect();

        let today = Utc::now().format("%Y-%m-%d").to_string();

        let skip_filler = if title.facet == MediaFacet::Anime {
            let effective = match extract_tag_string(&title.tags, "scryer:filler-policy:") {
                Some(v) => v.to_string(),
                None => self
                    .read_setting_string_value("anime.filler_policy", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            };
            effective == "skip_filler"
        } else {
            false
        };
        let skip_recap = if title.facet == MediaFacet::Anime {
            let effective = match extract_tag_string(&title.tags, "scryer:recap-policy:") {
                Some(v) => v.to_string(),
                None => self
                    .read_setting_string_value("anime.recap_policy", Some("anime"))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            };
            effective == "skip_recap"
        } else {
            false
        };

        // Track which interstitial collections have had their label updated
        // to the first episode's name (e.g. "Movie 1" → "Mugen Train").
        let mut labeled_collections: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for ep in episodes {
            // Check interstitial episode lookup first (routes movie episodes to their
            // interstitial collections), then fall back to the season-based lookup.
            let collection_id = interstitial_episode_lookup
                .get(&(ep.season_number, ep.episode_number))
                .cloned()
                .or_else(|| season_number_to_collection.get(&ep.season_number).cloned());

            // If this episode is routed to an interstitial collection, update the
            // collection label to the episode's name (once per collection).
            if let Some(ref cid) = collection_id {
                if interstitial_episode_lookup.contains_key(&(ep.season_number, ep.episode_number))
                    && !ep.name.is_empty()
                    && labeled_collections.insert(cid.clone())
                {
                    if let Err(err) = self
                        .services
                        .shows
                        .update_collection(
                            cid,
                            None,
                            None,
                            Some(ep.name.clone()),
                            None,
                            None,
                            None,
                            None,
                        )
                        .await
                    {
                        warn!(
                            collection_id = %cid,
                            error = %err,
                            "failed to update interstitial collection label"
                        );
                    }
                }
            }

            let air_date = if ep.aired.is_empty() {
                None
            } else {
                Some(ep.aired.clone())
            };
            let episode_monitored = if (skip_filler && ep.is_filler) || (skip_recap && ep.is_recap)
            {
                false
            } else {
                should_monitor_episode(
                    &monitor_type,
                    ep.season_number,
                    air_date.as_deref(),
                    &today,
                    monitor_specials,
                )
            };

            let anime_media_type = if title.facet == MediaFacet::Anime {
                anime_mappings
                    .iter()
                    .find(|m| m.thetvdb_season == Some(ep.season_number))
                    .map(|m| m.anime_media_type.as_str())
            } else {
                None
            };

            let episode_type = derive_episode_type(
                ep.season_number,
                season_episode_types.get(&ep.season_number).copied(),
                anime_media_type,
            );

            let episode = Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id,
                episode_type,
                episode_number: Some(ep.episode_number.to_string()),
                season_number: Some(ep.season_number.to_string()),
                episode_label: Some(ep.name.clone()),
                title: Some(ep.name.clone()),
                air_date,
                duration_seconds: if ep.runtime_minutes > 0 {
                    Some(i64::from(ep.runtime_minutes) * 60)
                } else {
                    None
                },
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: ep.is_filler,
                is_recap: ep.is_recap,
                absolute_number: if ep.absolute_number.is_empty() {
                    None
                } else {
                    Some(ep.absolute_number.clone())
                },
                overview: if ep.overview.trim().is_empty() {
                    None
                } else {
                    Some(ep.overview.clone())
                },
                monitored: episode_monitored,
                created_at: Utc::now(),
            };

            if let Err(err) = self.services.shows.create_episode(episode).await {
                warn!(
                    title_id = %title.id,
                    episode_number = ep.episode_number,
                    error = %err,
                    "failed to create episode"
                );
            }
        }
    }

    pub async fn add_title_and_queue_download(
        &self,
        actor: &User,
        request: NewTitle,
        source_hint: Option<String>,
        source_title: Option<String>,
    ) -> AppResult<(Title, String)> {
        let title = self.add_title(actor, request).await?;
        let source_hint_for_attempt = normalize_release_attempt_value(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_value(source_title.as_deref());
        let source_password: Option<String> = None;
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

        let category = self.derive_download_category(&title.facet).await;
        let job_result = self
            .services
            .download_client
            .submit_to_download_queue(
                &title,
                source_hint,
                source_title,
                source_password.clone(),
                Some(category),
            )
            .await;

        let grab = match job_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => "manual", "facet" => facet_label).increment(1);
                }
                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password.clone(),
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
                        download_client_type: grab.client_type.clone(),
                        download_client_item_id: grab.job_id.clone(),
                        source_title: source_title_for_attempt.clone(),
                    })
                    .await;
                grab
            }
            Err(error) => {
                let error_message = error.to_string();
                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(error_message),
                        source_password,
                    )
                    .await;
                return Err(error);
            }
        };

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                EventType::ActionTriggered,
                format!(
                    "download queued for title {} with job {}",
                    title.name, grab.job_id
                ),
            )
            .await?;
        self.services
            .record_activity_event(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                ActivityKind::MovieDownloaded,
                format!("movie downloaded: {}", title.name),
                ActivitySeverity::Success,
                vec![ActivityChannel::Toast, ActivityChannel::WebUi],
            )
            .await?;

        Ok((title, grab.job_id))
    }

    pub async fn queue_existing_title_download(
        &self,
        actor: &User,
        title_id: &str,
        source_hint: Option<String>,
        source_title: Option<String>,
    ) -> AppResult<String> {
        require(actor, &Entitlement::TriggerActions)?;

        let title = self
            .services
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;

        let source_hint_for_attempt = normalize_release_attempt_value(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_value(source_title.as_deref());
        let source_password: Option<String> = None;
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

        let category = self.derive_download_category(&title.facet).await;
        let job_result = self
            .services
            .download_client
            .submit_to_download_queue(
                &title,
                source_hint,
                source_title,
                source_password.clone(),
                Some(category),
            )
            .await;

        let grab = match job_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => "manual", "facet" => facet_label).increment(1);
                }
                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password.clone(),
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
                        download_client_type: grab.client_type.clone(),
                        download_client_item_id: grab.job_id.clone(),
                        source_title: source_title_for_attempt.clone(),
                    })
                    .await;
                grab
            }
            Err(error) => {
                let error_message = error.to_string();
                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt,
                        source_title_for_attempt,
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(error_message),
                        source_password,
                    )
                    .await;
                return Err(error);
            }
        };

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                EventType::ActionTriggered,
                format!(
                    "download queued for existing title {} with job {}",
                    title.name, grab.job_id
                ),
            )
            .await?;
        self.services
            .record_activity_event(
                Some(actor.id.clone()),
                Some(title.id.clone()),
                ActivityKind::MovieDownloaded,
                format!("movie downloaded: {}", title.name),
                ActivitySeverity::Success,
                vec![ActivityChannel::Toast, ActivityChannel::WebUi],
            )
            .await?;

        Ok(grab.job_id)
    }

    /// Resolve the NZBGet download category for a facet.
    /// Reads the per-facet `nzbget.client_routing` setting first (which stores
    /// per-client per-scope routing with a `category` field), falling back to the
    /// hardcoded `FacetHandler::download_category()` value.
    pub(crate) async fn derive_download_category(&self, facet: &MediaFacet) -> String {
        let scope_id = match facet {
            MediaFacet::Movie => "movie",
            MediaFacet::Tv => "series",
            MediaFacet::Anime => "anime",
            _ => "other",
        };

        // Try the per-client routing config first (set via the download client routing UI)
        if let Ok(Some(raw_json)) = self
            .read_setting_string_value("nzbget.client_routing", Some(scope_id))
            .await
        {
            if let Some(cat) = extract_highest_priority_nzbget_category(&raw_json) {
                return cat;
            }
        }

        // Fall back to the legacy nzbget.category setting
        if let Ok(Some(configured)) = self
            .read_setting_string_value("nzbget.category", Some(scope_id))
            .await
        {
            let trimmed = configured.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        self.facet_registry
            .get(facet)
            .map(|h| h.download_category().to_string())
            .unwrap_or_else(|| "other".to_string())
    }

    pub async fn set_title_monitored(
        &self,
        actor: &User,
        id: &str,
        monitored: bool,
    ) -> AppResult<Title> {
        require(actor, &Entitlement::MonitorTitle)?;

        let title = self.services.titles.update_monitored(id, monitored).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(id.to_string()),
                EventType::TitleUpdated,
                format!("title {} monitoring set to {}", title.name, title.monitored),
            )
            .await?;
        Ok(title)
    }

    pub async fn set_collection_monitored(
        &self,
        actor: &User,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<Collection> {
        require(actor, &Entitlement::MonitorTitle)?;

        let collection = self
            .services
            .shows
            .update_collection(
                collection_id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(monitored),
            )
            .await?;
        self.services
            .shows
            .set_collection_episodes_monitored(collection_id, monitored)
            .await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(collection.title_id.clone()),
                EventType::TitleUpdated,
                format!(
                    "collection {} monitoring set to {}",
                    collection_id, monitored
                ),
            )
            .await?;
        Ok(collection)
    }

    pub async fn set_episode_monitored(
        &self,
        actor: &User,
        episode_id: &str,
        monitored: bool,
    ) -> AppResult<Episode> {
        require(actor, &Entitlement::MonitorTitle)?;

        let episode = self
            .services
            .shows
            .update_episode(
                episode_id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(monitored),
                None,
            )
            .await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(episode.title_id.clone()),
                EventType::TitleUpdated,
                format!("episode {} monitoring set to {}", episode_id, monitored),
            )
            .await?;
        Ok(episode)
    }

    pub async fn delete_title(
        &self,
        actor: &User,
        id: &str,
        delete_files_on_disk: bool,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        let title = self
            .services
            .titles
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;

        if delete_files_on_disk {
            let media_files = self
                .services
                .media_files
                .list_media_files_for_title(id)
                .await?;

            let mut unique_paths = HashSet::new();
            for media_file in media_files {
                unique_paths.insert(media_file.file_path.trim().to_string());
            }

            let media_root = crate::recycle_bin::media_root_for_title(self, &title).await;
            let recycle_config =
                crate::recycle_bin::resolve_recycle_config(self, media_root.as_deref()).await;

            let mut delete_failures = Vec::new();
            for file_path in unique_paths {
                if file_path.is_empty() {
                    continue;
                }

                let manifest = crate::recycle_bin::RecycleManifest {
                    recycled_at: chrono::Utc::now().to_rfc3339(),
                    original_path: file_path.clone(),
                    size_bytes: fs::metadata(&file_path).await.map(|m| m.len()).unwrap_or(0),
                    title_id: Some(id.to_string()),
                    reason: "title_deleted".to_string(),
                };

                if let Err(error) = crate::recycle_bin::recycle_file(
                    &recycle_config,
                    Path::new(&file_path),
                    manifest,
                )
                .await
                {
                    warn!(
                        title_id = %id,
                        title_name = %title.name,
                        file_path = %file_path,
                        error = %error,
                        "failed to recycle media file while removing title from catalog"
                    );
                    delete_failures.push(format!("{file_path}: {error}"));
                }
            }

            if !delete_failures.is_empty() {
                return Err(AppError::Repository(format!(
                    "failed to delete one or more files from disk for title {}: {}",
                    title.name,
                    delete_failures.join(", ")
                )));
            }
        }

        // Cancel any inflight downloads for this title
        match self.services.download_client.list_queue().await {
            Ok(queue_items) => {
                for item in queue_items {
                    if item.title_id.as_deref() == Some(id) {
                        if let Err(err) = self
                            .services
                            .download_client
                            .delete_queue_item(&item.download_client_item_id, false)
                            .await
                        {
                            warn!(
                                title_id = %id,
                                download_item_id = %item.download_client_item_id,
                                error = %err,
                                "failed to cancel inflight download while deleting title"
                            );
                        }
                    }
                }
            }
            Err(err) => {
                warn!(
                    title_id = %id,
                    error = %err,
                    "failed to list download queue while deleting title; skipping download cancellation"
                );
            }
        }

        // Clean up wanted items for this title
        if let Err(err) = self
            .services
            .wanted_items
            .delete_wanted_items_for_title(id)
            .await
        {
            warn!(
                title_id = %id,
                error = %err,
                "failed to delete wanted items while deleting title"
            );
        }

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(id.to_string()),
                EventType::ActionTriggered,
                format!("title deleted: {}", title.name),
            )
            .await?;
        self.services.titles.delete(id).await?;

        // Dispatch notification for title deleted
        {
            let facet_str = format!("{:?}", title.facet).to_lowercase();
            let mut metadata = HashMap::new();
            metadata.insert("title_name".to_string(), serde_json::json!(title.name));
            if let Some(ref year) = title.year {
                metadata.insert("title_year".to_string(), serde_json::json!(year));
            }
            metadata.insert("title_facet".to_string(), serde_json::json!(facet_str));
            self.dispatch_notification(
                NotificationEventType::TitleDeleted.as_str(),
                &format!("{} deleted: {}", facet_str, title.name),
                &format!("{} has been removed from your library.", title.name),
                &metadata,
            )
            .await;
        }

        Ok(())
    }

    pub async fn update_title_metadata(
        &self,
        actor: &User,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        require(actor, &Entitlement::ManageTitle)?;

        if name.is_none() && facet.is_none() && tags.is_none() {
            return Err(AppError::Validation(
                "at least one title field must be provided".into(),
            ));
        }

        let title = self
            .services
            .titles
            .update_metadata(id, name, facet, tags)
            .await?;

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(id.to_string()),
                EventType::TitleUpdated,
                format!("title metadata updated: {}", title.name),
            )
            .await?;
        Ok(title)
    }

    pub async fn get_title(&self, actor: &User, id: &str) -> AppResult<Option<Title>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.titles.get_by_id(id).await
    }

    async fn validate_title_exists(&self, title_id: &str) -> AppResult<()> {
        self.services
            .titles
            .get_by_id(title_id)
            .await?
            .map(|_| ())
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))
    }

    pub async fn list_primary_collection_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .shows
            .list_primary_collection_summaries(title_ids)
            .await
    }

    pub async fn list_collections(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<Collection>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.validate_title_exists(title_id).await?;
        self.services
            .shows
            .list_collections_for_title(title_id)
            .await
    }

    pub async fn get_collection(
        &self,
        actor: &User,
        collection_id: &str,
    ) -> AppResult<Option<Collection>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .shows
            .get_collection_by_id(collection_id)
            .await
    }

    pub async fn create_collection(
        &self,
        actor: &User,
        title_id: String,
        collection_type: String,
        collection_index: String,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
    ) -> AppResult<Collection> {
        require(actor, &Entitlement::ManageTitle)?;

        if collection_type.trim().is_empty() {
            return Err(AppError::Validation("collection type is required".into()));
        }
        if collection_index.trim().is_empty() {
            return Err(AppError::Validation("collection index is required".into()));
        }

        self.validate_title_exists(&title_id).await?;

        let collection = Collection {
            id: Id::new().0,
            title_id,
            collection_type: collection_type.trim().to_lowercase(),
            collection_index: collection_index.trim().to_string(),
            label: normalize_show_text_opt(label),
            ordered_path: normalize_show_text_opt(ordered_path),
            narrative_order: None,
            first_episode_number: normalize_show_text_opt(first_episode_number),
            last_episode_number: normalize_show_text_opt(last_episode_number),
            interstitial_movie: None,
            monitored: true,
            created_at: Utc::now(),
        };

        let collection = self.services.shows.create_collection(collection).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(collection.title_id.clone()),
                EventType::ActionTriggered,
                format!(
                    "collection created: {} ({})",
                    collection.collection_index, collection.title_id
                ),
            )
            .await?;

        Ok(collection)
    }

    pub async fn update_collection(
        &self,
        actor: &User,
        collection_id: String,
        collection_type: Option<String>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection> {
        require(actor, &Entitlement::ManageTitle)?;

        if collection_type.as_ref().is_none()
            && collection_index.is_none()
            && label.is_none()
            && ordered_path.is_none()
            && first_episode_number.is_none()
            && last_episode_number.is_none()
            && monitored.is_none()
        {
            return Err(AppError::Validation(
                "at least one collection field must be provided".into(),
            ));
        }

        if let Some(raw) = &collection_type {
            if raw.trim().is_empty() {
                return Err(AppError::Validation(
                    "collection type cannot be empty".into(),
                ));
            }
        }

        if let Some(raw) = &collection_index {
            if raw.trim().is_empty() {
                return Err(AppError::Validation(
                    "collection index cannot be empty".into(),
                ));
            }
        }

        let collection = self
            .services
            .shows
            .update_collection(
                &collection_id,
                collection_type.map(|value| value.trim().to_lowercase()),
                collection_index.map(|value| value.trim().to_string()),
                normalize_show_text_opt(label),
                normalize_show_text_opt(ordered_path),
                normalize_show_text_opt(first_episode_number),
                normalize_show_text_opt(last_episode_number),
                monitored,
            )
            .await?;

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(collection.title_id.clone()),
                EventType::ActionTriggered,
                format!(
                    "collection updated: {} ({})",
                    collection.collection_index, collection.title_id
                ),
            )
            .await?;

        Ok(collection)
    }

    pub async fn create_episode(
        &self,
        actor: &User,
        title_id: String,
        collection_id: Option<String>,
        episode_type: String,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: bool,
        has_subtitle: bool,
    ) -> AppResult<Episode> {
        require(actor, &Entitlement::ManageTitle)?;

        if episode_type.trim().is_empty() {
            return Err(AppError::Validation("episode type is required".into()));
        }

        self.validate_title_exists(&title_id).await?;

        let episode = Episode {
            id: Id::new().0,
            title_id,
            collection_id,
            episode_type: episode_type.trim().to_lowercase(),
            episode_number: normalize_show_text_opt(episode_number),
            season_number: normalize_show_text_opt(season_number),
            episode_label: normalize_show_text_opt(episode_label),
            title: normalize_show_text_opt(title),
            air_date: normalize_show_text_opt(air_date),
            duration_seconds,
            has_multi_audio,
            has_subtitle,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            monitored: true,
            created_at: Utc::now(),
        };

        let episode = self.services.shows.create_episode(episode).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(episode.title_id.clone()),
                EventType::ActionTriggered,
                format!("episode created for title {}", episode.title_id),
            )
            .await?;

        Ok(episode)
    }

    pub async fn update_episode(
        &self,
        actor: &User,
        episode_id: String,
        episode_type: Option<String>,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: Option<bool>,
        has_subtitle: Option<bool>,
        monitored: Option<bool>,
        collection_id: Option<String>,
    ) -> AppResult<Episode> {
        require(actor, &Entitlement::ManageTitle)?;

        if episode_type.as_ref().is_none()
            && episode_number.is_none()
            && season_number.is_none()
            && episode_label.is_none()
            && title.is_none()
            && air_date.is_none()
            && duration_seconds.is_none()
            && has_multi_audio.is_none()
            && has_subtitle.is_none()
            && monitored.is_none()
            && collection_id.is_none()
        {
            return Err(AppError::Validation(
                "at least one episode field must be provided".into(),
            ));
        }

        if let Some(raw) = &episode_type {
            if raw.trim().is_empty() {
                return Err(AppError::Validation("episode type cannot be empty".into()));
            }
        }

        let episode = self
            .services
            .shows
            .update_episode(
                &episode_id,
                episode_type.map(|value| value.trim().to_lowercase()),
                normalize_show_text_opt(episode_number),
                normalize_show_text_opt(season_number),
                normalize_show_text_opt(episode_label),
                normalize_show_text_opt(title),
                normalize_show_text_opt(air_date),
                duration_seconds,
                has_multi_audio,
                has_subtitle,
                monitored,
                collection_id,
            )
            .await?;

        self.services
            .record_event(
                Some(actor.id.clone()),
                Some(episode.title_id.clone()),
                EventType::ActionTriggered,
                format!("episode updated for title {}", episode.title_id),
            )
            .await?;

        Ok(episode)
    }

    pub async fn delete_collection(&self, actor: &User, collection_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        self.services.shows.delete_collection(collection_id).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("collection deleted: {}", collection_id),
            )
            .await?;

        Ok(())
    }

    pub async fn delete_episode(&self, actor: &User, episode_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        self.services.shows.delete_episode(episode_id).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("episode deleted: {}", episode_id),
            )
            .await?;

        Ok(())
    }

    pub async fn list_episodes(
        &self,
        actor: &User,
        collection_id: &str,
    ) -> AppResult<Vec<Episode>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .shows
            .list_episodes_for_collection(collection_id)
            .await
    }

    pub async fn get_episode(&self, actor: &User, episode_id: &str) -> AppResult<Option<Episode>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.shows.get_episode_by_id(episode_id).await
    }

    pub async fn list_calendar_episodes(
        &self,
        actor: &User,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .shows
            .list_episodes_in_date_range(start_date, end_date)
            .await
    }

    /// Re-fetch metadata from SMG for all monitored series/anime titles.
    /// This updates episode air dates (TBA → actual), adds newly announced
    /// episodes, and refreshes other metadata fields.
    pub(crate) async fn refresh_monitored_series_metadata(&self) {
        let titles = match self.services.titles.list(None, None).await {
            Ok(t) => t,
            Err(err) => {
                warn!(error = %err, "metadata refresh: failed to list titles");
                return;
            }
        };

        let mut refreshed = 0u32;
        for title in titles {
            if !title.monitored {
                continue;
            }
            let Some(handler) = self.facet_registry.get(&title.facet) else {
                continue;
            };
            if !handler.has_episodes() {
                continue;
            }

            self.hydrate_title_metadata(title).await;
            refreshed += 1;

            // Small delay between titles to avoid hammering SMG
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        if refreshed > 0 {
            info!(count = refreshed, "periodic metadata refresh completed");
        }
    }
}

fn normalize_release_attempt_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Extract the monitor type from title tags (e.g. "scryer:monitor-type:none").
/// Defaults to "allEpisodes" when no tag is present for backward compatibility.
fn extract_monitor_type(tags: &[String]) -> String {
    // Tags are lowercased by normalize_tag(), so values like "futureEpisodes"
    // become "futureepisodes". We return the lowercased value.
    for tag in tags {
        if let Some(value) = tag.strip_prefix("scryer:monitor-type:") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "allepisodes".to_string()
}

/// Extract a boolean from a `scryer:{prefix}:true/false` tag.
/// Returns `None` when no matching tag exists (caller falls back to global setting).
fn extract_tag_bool(tags: &[String], prefix: &str) -> Option<bool> {
    for tag in tags {
        if let Some(value) = tag.strip_prefix(prefix) {
            return Some(!value.trim().eq_ignore_ascii_case("false"));
        }
    }
    None
}

/// Extract a string value from a `scryer:{prefix}:{value}` tag.
/// Returns `None` when no matching tag exists (caller falls back to global setting).
fn extract_tag_string<'a>(tags: &'a [String], prefix: &str) -> Option<&'a str> {
    for tag in tags {
        if let Some(value) = tag.strip_prefix(prefix) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

/// Determine whether an individual episode should be monitored based on
/// the user's monitor type selection and the episode's air date.
///
/// NOTE: All values are lowercase because tags go through `normalize_tag`
/// which calls `.to_lowercase()`. The frontend sends camelCase values like
/// "futureEpisodes" which become "futureepisodes" after normalization.
fn should_monitor_season(monitor_type: &str, season_number: i32, monitor_specials: bool) -> bool {
    if season_number == 0 {
        return monitor_specials;
    }

    monitor_type != "none" && monitor_type != "unmonitored"
}

fn should_monitor_episode(
    monitor_type: &str,
    season_number: i32,
    air_date: Option<&str>,
    today: &str,
    monitor_specials: bool,
) -> bool {
    if season_number == 0 {
        return monitor_specials;
    }

    match monitor_type {
        "none" | "unmonitored" => false,
        "allepisodes" | "monitored" => true,
        "futureepisodes" => {
            // Monitor only episodes that haven't aired yet
            match air_date {
                Some(date) if !date.is_empty() => date >= today,
                _ => true, // no air date = assume future
            }
        }
        "missingandfutureepisodes" => {
            // Monitor episodes that haven't aired or are missing (not on disk).
            // At add time, no episodes are on disk yet, so all are "missing" — monitor all.
            true
        }
        _ => true,
    }
}

/// Derive the episode type from the season number, season episode_type, and anime media type.
fn derive_episode_type(
    season_number: i32,
    season_episode_type: Option<&str>,
    anime_media_type: Option<&str>,
) -> String {
    if season_number == 0 {
        return match anime_media_type {
            Some("OVA") => "ova".to_string(),
            Some("ONA") => "ona".to_string(),
            _ => "special".to_string(),
        };
    }
    match season_episode_type {
        Some("alternate") => "alternate".to_string(),
        _ => "standard".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Background metadata hydration loop
// ---------------------------------------------------------------------------

/// How long to wait after a wake signal to let titles accumulate before
/// draining.  Keeps the loop from firing per-title during bulk imports.
const HYDRATION_COLLECT_WINDOW: Duration = Duration::from_millis(50);

/// Safety cap so a single query doesn't pull the entire DB in a degenerate
/// case.  In practice this is never hit during normal operation.
const HYDRATION_MAX_BATCH: usize = 10_000;

/// How long to wait before retrying titles that failed hydration (e.g. SMG
/// was down).  Doubles on consecutive failures up to 5 minutes.
const HYDRATION_RETRY_BASE: Duration = Duration::from_secs(10);
const HYDRATION_RETRY_MAX: Duration = Duration::from_secs(300);

fn extract_tvdb_id(title: &scryer_domain::Title) -> Option<i64> {
    title
        .external_ids
        .iter()
        .find(|eid| eid.source == "tvdb")
        .and_then(|eid| eid.value.parse::<i64>().ok())
}

/// Spawns a background loop that hydrates titles whose `metadata_fetched_at`
/// is NULL.
///
/// On each `hydration_wake` signal the loop sleeps for 50 ms to let more
/// titles accumulate, then drains *all* unhydrated titles in a single pair
/// of bulk GraphQL calls (one for movies, one for series) instead of N
/// individual round-trips.
///
/// When the queue is empty the loop parks indefinitely until the next wake.
/// If any titles fail hydration (e.g. SMG down), the loop retries with
/// exponential backoff instead of parking.
pub async fn start_background_hydration_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    use scryer_domain::MediaFacet;

    info!("background hydration loop started");

    loop {
        // Park until someone signals new work.
        tokio::select! {
            _ = token.cancelled() => {
                info!("background hydration loop shutting down");
                return;
            }
            _ = app.services.hydration_wake.notified() => {}
        }

        // Inner drain loop — retries with backoff on failures without
        // re-entering the outer `notified()` wait (which could deadlock
        // if a stale permit was already consumed).
        let mut retry_delay = HYDRATION_RETRY_BASE;
        'drain: loop {
            // Let titles accumulate for a short window so bulk adds
            // coalesce into a single drain pass.
            tokio::time::sleep(HYDRATION_COLLECT_WINDOW).await;

            if token.is_cancelled() {
                return;
            }

            let batch = match app
                .services
                .titles
                .list_unhydrated(HYDRATION_MAX_BATCH)
                .await
            {
                Ok(titles) => titles,
                Err(err) => {
                    warn!(error = %err, "hydration loop: failed to list unhydrated titles");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue 'drain;
                }
            };

            if batch.is_empty() {
                break 'drain;
            }

            let count = batch.len();
            info!(count, "hydration loop: processing batch");

            // ---- Partition by facet and extract TVDB IDs ----
            let mut movie_titles: Vec<scryer_domain::Title> = Vec::new();
            let mut series_titles: Vec<scryer_domain::Title> = Vec::new();
            for title in batch {
                match title.facet {
                    MediaFacet::Movie => movie_titles.push(title),
                    MediaFacet::Tv | MediaFacet::Anime | MediaFacet::Other => {
                        series_titles.push(title)
                    }
                }
            }

            let mut had_failures = false;
            let language = "eng";

            // ---- Bulk-fetch movies (single GraphQL call) ----
            if !movie_titles.is_empty() {
                let tvdb_ids: Vec<i64> = movie_titles.iter().filter_map(extract_tvdb_id).collect();

                match app
                    .services
                    .metadata_gateway
                    .get_movies_bulk(&tvdb_ids, language)
                    .await
                {
                    Ok(metadata_map) => {
                        for title in movie_titles {
                            let tvdb_id = extract_tvdb_id(&title);
                            if let Some(movie) = tvdb_id.and_then(|id| metadata_map.get(&id)) {
                                let result =
                                    super::movie_to_hydration_result(movie.clone(), language);
                                let hydrated = app.apply_hydration_result(title, result).await;
                                sync_wanted_after_hydration(&app, &hydrated).await;
                            } else {
                                had_failures = true;
                            }
                        }
                    }
                    Err(err) => {
                        warn!(error = %err, "hydration loop: bulk movie fetch failed");
                        had_failures = true;
                    }
                }
            }

            // ---- Bulk-fetch series + anime (single GraphQL call) ----
            if !series_titles.is_empty() {
                let tvdb_ids: Vec<i64> = series_titles.iter().filter_map(extract_tvdb_id).collect();

                match app
                    .services
                    .metadata_gateway
                    .get_series_bulk(&tvdb_ids, language)
                    .await
                {
                    Ok(metadata_map) => {
                        for title in series_titles {
                            let tvdb_id = extract_tvdb_id(&title);
                            if let Some(series) = tvdb_id.and_then(|id| metadata_map.get(&id)) {
                                let result =
                                    super::series_to_hydration_result(series.clone(), language);
                                let hydrated = app.apply_hydration_result(title, result).await;
                                sync_wanted_after_hydration(&app, &hydrated).await;
                            } else {
                                had_failures = true;
                            }
                        }
                    }
                    Err(err) => {
                        warn!(error = %err, "hydration loop: bulk series fetch failed");
                        had_failures = true;
                    }
                }
            }

            info!(count, "hydration loop: batch complete");

            if had_failures {
                info!(
                    retry_secs = retry_delay.as_secs(),
                    "hydration loop: some titles failed, scheduling retry"
                );
                let new_work = tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(retry_delay) => false,
                    _ = app.services.hydration_wake.notified() => true,
                };
                if new_work {
                    retry_delay = HYDRATION_RETRY_BASE;
                } else {
                    retry_delay = (retry_delay * 2).min(HYDRATION_RETRY_MAX);
                }
                continue 'drain;
            }
        }

        info!("hydration loop: queue drained, parking");
    }
}

/// After successful hydration, sync wanted items for monitored titles.
async fn sync_wanted_after_hydration(app: &AppUseCase, title: &scryer_domain::Title) {
    if title.monitored && title.metadata_fetched_at.is_some() {
        let now = Utc::now();
        if let Some(handler) = app.facet_registry.get(&title.facet) {
            if handler.has_episodes() {
                app.sync_wanted_series_inner(title, &now, true).await;
            } else {
                app.sync_wanted_movie_inner(title, &now, true).await;
            }
        }
        app.services.acquisition_wake.notify_one();
    }
}
