use super::*;
use tracing::{info, warn};
use tokio::fs;
use std::collections::HashSet;
use std::io::ErrorKind;

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
            year: None,
            overview: None,
            poster_url: None,
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

        // Hydrate rich metadata from the metadata gateway
        let title = self.hydrate_title_metadata(title).await;

        // Create wanted items immediately so the background poller picks them up
        // on the next 60-second tick. This runs regardless of whether metadata
        // hydration succeeded — a movie without metadata can still be searched.
        if title.monitored {
            info!(
                title_id = %title.id,
                title_name = %title.name,
                facet = ?title.facet,
                "creating immediate wanted items for new monitored title"
            );
            let now = Utc::now();
            if let Some(handler) = self.facet_registry.get(&title.facet) {
                if handler.has_episodes() {
                    self.sync_wanted_series_inner(&title, &now, true).await;
                } else {
                    self.sync_wanted_movie_inner(&title, &now, true).await;
                }
            }
            self.services.acquisition_wake.notify_one();
        } else {
            info!(
                title_id = %title.id,
                title_name = %title.name,
                "title not monitored, skipping wanted item creation"
            );
        }

        Ok(title)
    }

    async fn hydrate_title_metadata(&self, title: Title) -> Title {
        let tvdb_id = match title
            .external_ids
            .iter()
            .find(|eid| eid.source == "tvdb")
            .and_then(|eid| eid.value.parse::<i64>().ok())
        {
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

        match handler.hydrate_metadata(self.services.metadata_gateway.as_ref(), tvdb_id, language).await {
            Ok(result) => {
                if handler.has_episodes() {
                    info!(
                        title_id = %title.id,
                        tvdb_id = tvdb_id,
                        seasons = result.seasons.len(),
                        episodes = result.episodes.len(),
                        "received series metadata from gateway"
                    );
                }

                // Build extra external IDs from anime mappings
                let mut metadata_update = result.metadata_update;
                for mapping in &result.anime_mappings {
                    if let Some(mal_id) = mapping.mal_id {
                        metadata_update.extra_external_ids.push(ExternalId { source: "mal".to_string(), value: mal_id.to_string() });
                    }
                    if let Some(anilist_id) = mapping.anilist_id {
                        metadata_update.extra_external_ids.push(ExternalId { source: "anilist".to_string(), value: anilist_id.to_string() });
                    }
                    if let Some(anidb_id) = mapping.anidb_id {
                        metadata_update.extra_external_ids.push(ExternalId { source: "anidb".to_string(), value: anidb_id.to_string() });
                    }
                    if let Some(kitsu_id) = mapping.kitsu_id {
                        metadata_update.extra_external_ids.push(ExternalId { source: "kitsu".to_string(), value: kitsu_id.to_string() });
                    }
                }

                // Store anime-specific metadata as tags on the title
                if let Some(primary) = result.anime_mappings.first() {
                    if let Some(score) = primary.score {
                        metadata_update.extra_tags.push(format!("scryer:mal-score:{score}"));
                    }
                    if !primary.anime_media_type.is_empty() {
                        metadata_update.extra_tags.push(format!("scryer:anime-media-type:{}", primary.anime_media_type));
                    }
                    if !primary.status.is_empty() {
                        metadata_update.extra_tags.push(format!("scryer:anime-status:{}", primary.status));
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
                    self.create_series_seasons_and_episodes(&title, &result.seasons, &result.episodes, &result.anime_mappings).await;
                }

                title
            }
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

    async fn create_series_seasons_and_episodes(
        &self,
        title: &Title,
        seasons: &[SeasonMetadata],
        episodes: &[EpisodeMetadata],
        anime_mappings: &[AnimeMapping],
    ) {
        let monitor_type = extract_monitor_type(&title.tags);
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
        let seasons_with_episodes: std::collections::HashSet<i32> = episodes
            .iter()
            .map(|ep| ep.season_number)
            .collect();

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
                monitored: season_monitored,
                created_at: Utc::now(),
            };

            match self.services.shows.create_collection(collection.clone()).await {
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

        // Create interstitial movie collections for anime titles.
        // Movies with global_media_type == "movie" and a thetvdb_season are positioned
        // narratively between seasons (e.g. Demon Slayer: Mugen Train between S1 and S2).
        // Build a lookup from (season_number, episode_number) → collection_id so that
        // episodes can be routed to the correct interstitial collection later.
        let mut interstitial_episode_lookup: std::collections::HashMap<(i32, i32), String> =
            std::collections::HashMap::new();

        if title.facet == MediaFacet::Anime && inter_season_movies && !anime_mappings.is_empty() {
            let movie_mappings: Vec<&AnimeMapping> = anime_mappings
                .iter()
                .filter(|m| m.global_media_type == "movie")
                .filter(|m| m.thetvdb_season.is_some())
                .collect();

            if !movie_mappings.is_empty() {
                // Group by the season they follow: a movie with thetvdb_season=N
                // belongs narratively after season N-1 → narrative_order = "{N-1}.{seq}"
                let mut movies_by_position: std::collections::BTreeMap<i32, Vec<&AnimeMapping>> =
                    std::collections::BTreeMap::new();
                for m in &movie_mappings {
                    let target_season = m.thetvdb_season.unwrap() as i32;
                    let after_season = (target_season - 1).max(0);
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
                                // Map episode ranges from this movie's episode_mappings
                                // to this collection so episodes get routed correctly.
                                for em in &movie.episode_mappings {
                                    for ep_num in em.episode_start..=em.episode_end {
                                        interstitial_episode_lookup.insert(
                                            (em.tvdb_season, ep_num),
                                            created.id.clone(),
                                        );
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
            self.read_setting_string_value("anime.filler_policy", Some("anime"))
                .await
                .ok()
                .flatten()
                .as_deref()
                == Some("skip_filler")
        } else {
            false
        };
        let skip_recap = if title.facet == MediaFacet::Anime {
            self.read_setting_string_value("anime.recap_policy", Some("anime"))
                .await
                .ok()
                .flatten()
                .as_deref()
                == Some("skip_recap")
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
                if interstitial_episode_lookup
                    .contains_key(&(ep.season_number, ep.episode_number))
                    && !ep.name.is_empty()
                    && labeled_collections.insert(cid.clone())
                {
                    if let Err(err) = self
                        .services
                        .shows
                        .update_collection(cid, None, None, Some(ep.name.clone()), None, None, None, None)
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

            let air_date = if ep.aired.is_empty() { None } else { Some(ep.aired.clone()) };
            let episode_monitored = if (skip_filler && ep.is_filler) || (skip_recap && ep.is_recap) {
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
                absolute_number: if ep.absolute_number.is_empty() { None } else { Some(ep.absolute_number.clone()) },
                overview: if ep.overview.trim().is_empty() { None } else { Some(ep.overview.clone()) },
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

        let category = self.derive_download_category(&title.facet);
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

        let job_id = match job_result {
            Ok(job_id) => {
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
                job_id
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
                    title.name, job_id
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

        Ok((title, job_id))
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

        let category = self.derive_download_category(&title.facet);
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

        let job_id = match job_result {
            Ok(job_id) => {
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
                job_id
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
                    title.name, job_id
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

        Ok(job_id)
    }

    fn derive_download_category(&self, facet: &MediaFacet) -> String {
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
            .update_collection(collection_id, None, None, None, None, None, None, Some(monitored))
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
                format!("collection {} monitoring set to {}", collection_id, monitored),
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
            .update_episode(episode_id, None, None, None, None, None, None, None, None, None, Some(monitored), None)
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

            let mut delete_failures = Vec::new();
            for file_path in unique_paths {
                if file_path.is_empty() {
                    continue;
                }

                if let Err(error) = fs::remove_file(&file_path).await {
                    if error.kind() == ErrorKind::NotFound {
                        continue;
                    }

                    warn!(
                        title_id = %id,
                        title_name = %title.name,
                        file_path = %file_path,
                        error = %error,
                        "failed to delete media file while removing title from catalog"
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
        if let Err(err) = self.services.wanted_items.delete_wanted_items_for_title(id).await {
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
