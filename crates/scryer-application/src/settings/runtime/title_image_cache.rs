use scryer_domain::{MediaFacet, Title};

use crate::ports::{EpisodeImageUrlUpdate, TitleArtworkUrlUpdate};
use crate::{AppError, AppResult, AppUseCase, User};

const TITLE_IMAGE_CACHE_REFRESH_BATCH_SIZE: usize = 100;

struct TitleImageCacheClearScheduledGuard {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for TitleImageCacheClearScheduledGuard {
    fn drop(&mut self) {
        self.flag.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TitleImageCacheRefreshSummary {
    pub titles_scanned: u64,
    pub titles_linked: u64,
    pub title_urls_updated: u64,
    pub episode_urls_updated: u64,
    pub missing_artwork_results: u64,
    pub missing_title_artwork_results: u64,
    pub missing_episode_matches: u64,
    pub missing_incoming_image_urls: u64,
    pub cache_cleared: bool,
}

impl AppUseCase {
    pub async fn clear_title_image_cache(&self, actor: &User) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let app = self.clone();
        tokio::spawn(async move {
            match app.run_title_image_cache_refresh().await {
                Ok(summary) => {
                    info!(
                        titles_scanned = summary.titles_scanned,
                        title_urls_updated = summary.title_urls_updated,
                        episode_urls_updated = summary.episode_urls_updated,
                        missing_artwork_results = summary.missing_artwork_results,
                        missing_title_artwork_results = summary.missing_title_artwork_results,
                        missing_episode_matches = summary.missing_episode_matches,
                        missing_incoming_image_urls = summary.missing_incoming_image_urls,
                        "title image cache refresh completed"
                    );
                }
                Err(error) => {
                    warn!(error = %error, "title image cache refresh failed");
                }
            }
        });

        Ok(true)
    }

    pub async fn run_title_image_cache_refresh(&self) -> AppResult<TitleImageCacheRefreshSummary> {
        let scheduled = self
            .runtime
            .catalog
            .title_image_cache_clear_scheduled
            .clone();
        if scheduled.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(AppError::Validation(
                "title image cache refresh is already running".to_string(),
            ));
        }
        let _scheduled_guard = TitleImageCacheClearScheduledGuard { flag: scheduled };

        let _maintenance_guard = loop {
            let active_scans = self.runtime.library.library_scan_tracker.list_active().await;
            if !active_scans.is_empty() {
                info!(
                    active_scans = active_scans.len(),
                    "title image cache refresh pausing while library scan is active"
                );
                self.runtime
                    .library
                    .library_scan_tracker
                    .wait_until_idle()
                    .await;
                info!("title image cache refresh resuming after library scan");
            }
            let guard = self
                .runtime
                .catalog
                .title_image_maintenance_lock
                .write()
                .await;
            if self
                .runtime
                .library
                .library_scan_tracker
                .list_active()
                .await
                .is_empty()
            {
                break guard;
            }
        };

        let mut summary = self.rehydrate_remote_artwork_urls().await?;
        self.services
            .library
            .title_images
            .clear_title_image_cache()
            .await?;
        self.services
            .library
            .image_proxy_cache_control
            .clear_cache()
            .await?;
        summary.cache_cleared = true;
        info!(
            titles_scanned = summary.titles_scanned,
            titles_linked = summary.titles_linked,
            title_urls_updated = summary.title_urls_updated,
            episode_urls_updated = summary.episode_urls_updated,
            missing_artwork_results = summary.missing_artwork_results,
            missing_title_artwork_results = summary.missing_title_artwork_results,
            missing_episode_matches = summary.missing_episode_matches,
            missing_incoming_image_urls = summary.missing_incoming_image_urls,
            "title image cache reset completed"
        );
        self.wake_title_image_loops();
        Ok(summary)
    }

    async fn rehydrate_remote_artwork_urls(&self) -> AppResult<TitleImageCacheRefreshSummary> {
        let language = self.metadata_language().await;
        let mut after_id = None;
        let mut summary = TitleImageCacheRefreshSummary::default();

        loop {
            let titles = self
                .services
                .catalog
                .titles
                .list_page_after_id(after_id.clone(), TITLE_IMAGE_CACHE_REFRESH_BATCH_SIZE)
                .await?;
            if titles.is_empty() {
                break;
            }

            after_id = titles.last().map(|title| title.id.clone());
            let batch_summary = self
                .rehydrate_remote_artwork_urls_for_title_batch(&titles, &language)
                .await?;
            summary.titles_scanned += batch_summary.titles_scanned;
            summary.titles_linked += batch_summary.titles_linked;
            summary.title_urls_updated += batch_summary.title_urls_updated;
            summary.episode_urls_updated += batch_summary.episode_urls_updated;
            summary.missing_artwork_results += batch_summary.missing_artwork_results;
            summary.missing_title_artwork_results += batch_summary.missing_title_artwork_results;
            summary.missing_episode_matches += batch_summary.missing_episode_matches;
            summary.missing_incoming_image_urls += batch_summary.missing_incoming_image_urls;

            info!(
                titles_scanned = summary.titles_scanned,
                title_urls_updated = summary.title_urls_updated,
                episode_urls_updated = summary.episode_urls_updated,
                missing_artwork_results = summary.missing_artwork_results,
                missing_title_artwork_results = summary.missing_title_artwork_results,
                missing_episode_matches = summary.missing_episode_matches,
                missing_incoming_image_urls = summary.missing_incoming_image_urls,
                batch_titles_scanned = batch_summary.titles_scanned,
                batch_title_urls_updated = batch_summary.title_urls_updated,
                batch_episode_urls_updated = batch_summary.episode_urls_updated,
                batch_missing_artwork_results = batch_summary.missing_artwork_results,
                batch_missing_title_artwork_results =
                    batch_summary.missing_title_artwork_results,
                batch_missing_episode_matches = batch_summary.missing_episode_matches,
                batch_missing_incoming_image_urls = batch_summary.missing_incoming_image_urls,
                "title image cache refresh rehydrated artwork url batch"
            );
        }

        Ok(summary)
    }

    async fn rehydrate_remote_artwork_urls_for_title_batch(
        &self,
        titles: &[Title],
        language: &str,
    ) -> AppResult<TitleImageCacheRefreshSummary> {
        let mut summary = TitleImageCacheRefreshSummary {
            titles_scanned: titles.len() as u64,
            ..Default::default()
        };
        let mut movie_targets = Vec::new();
        let mut series_ids = Vec::new();
        let mut series_title_by_tvdb = HashMap::<i64, &Title>::new();

        for title in titles {
            match title.facet {
                MediaFacet::Movie => {
                    if let Some(movie_ref) = crate::catalog_workflow::movie_title_ref(title) {
                        summary.titles_linked += 1;
                        movie_targets.push((title, movie_ref));
                    }
                }
                MediaFacet::Series | MediaFacet::Anime => {
                    let Some(tvdb_id) = title_tvdb_id(title) else {
                        continue;
                    };
                    summary.titles_linked += 1;
                    series_ids.push(tvdb_id);
                    series_title_by_tvdb.insert(tvdb_id, title);
                }
            }
        }

        if movie_targets.is_empty() && series_ids.is_empty() {
            return Ok(summary);
        }

        let mut title_updates = Vec::new();
        let mut episode_updates = Vec::new();

        if !movie_targets.is_empty() {
            let refs = movie_targets
                .iter()
                .map(|(_, movie_ref)| movie_ref.clone())
                .collect::<Vec<_>>();
            match self
                .services
                .library
                .metadata_gateway
                .get_movie_titles(&refs, language)
                .await
            {
                Ok(movie_result) => {
                    for (ref_index, (title, _)) in movie_targets.iter().enumerate() {
                        let title = *title;
                        let Some(movie) = movie_result.by_ref_index.get(&ref_index) else {
                            summary.missing_artwork_results += 1;
                            summary.missing_title_artwork_results += 1;
                            continue;
                        };
                        let poster_url = (!movie.poster_url.trim().is_empty())
                            .then_some(&movie.poster_url);
                        let background_url = movie
                            .background_url
                            .as_ref()
                            .filter(|url| !url.trim().is_empty());
                        if poster_url.is_none() && background_url.is_none() {
                            summary.missing_artwork_results += 1;
                            summary.missing_incoming_image_urls += 1;
                        }
                        if let Some(update) = title_artwork_update(title, poster_url, background_url)
                        {
                            title_updates.push(update);
                        }
                    }
                }
                Err(error)
                    if crate::catalog_workflow::movie_title_queries_not_supported(&error) =>
                {
                    let legacy_movie_ids = movie_targets
                        .iter()
                        .filter_map(|(_, movie_ref)| movie_ref.tvdb_id)
                        .collect::<Vec<_>>();
                    let legacy_artwork = if legacy_movie_ids.is_empty() {
                        None
                    } else {
                        Some(
                            self.services
                                .library
                                .metadata_gateway
                                .get_artwork_urls_bulk(&legacy_movie_ids, &[], language)
                                .await?,
                        )
                    };
                    for (title, movie_ref) in &movie_targets {
                        let title = *title;
                        let Some(tvdb_id) = movie_ref.tvdb_id else {
                            summary.missing_artwork_results += 1;
                            summary.missing_title_artwork_results += 1;
                            continue;
                        };
                        let Some(urls) = legacy_artwork
                            .as_ref()
                            .and_then(|artwork| artwork.movies.get(&tvdb_id))
                        else {
                            summary.missing_artwork_results += 1;
                            summary.missing_title_artwork_results += 1;
                            continue;
                        };
                        if urls.poster_url.is_none() && urls.background_url.is_none() {
                            summary.missing_artwork_results += 1;
                            summary.missing_incoming_image_urls += 1;
                        }
                        if let Some(update) = title_artwork_update(
                            title,
                            urls.poster_url.as_ref(),
                            urls.background_url.as_ref(),
                        ) {
                            title_updates.push(update);
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }

        let series_artwork = if series_ids.is_empty() {
            None
        } else {
            Some(
                self.services
                    .library
                    .metadata_gateway
                    .get_artwork_urls_bulk(&[], &series_ids, language)
                    .await?,
            )
        };

        for tvdb_id in series_ids {
            let Some(title) = series_title_by_tvdb.get(&tvdb_id) else {
                continue;
            };
            let Some(urls) = series_artwork
                .as_ref()
                .and_then(|artwork| artwork.series.get(&tvdb_id))
            else {
                summary.missing_artwork_results += 1;
                summary.missing_title_artwork_results += 1;
                tracing::debug!(
                    title_id = %title.id,
                    tvdb_id,
                    "title image cache refresh skipped series with missing artwork result"
                );
                continue;
            };
            if urls.poster_url.is_none() && urls.background_url.is_none() {
                summary.missing_artwork_results += 1;
                summary.missing_incoming_image_urls += 1;
                tracing::debug!(
                    title_id = %title.id,
                    tvdb_id,
                    "title image cache refresh skipped series artwork update with no usable image URLs"
                );
            }
            if let Some(update) =
                title_artwork_update(title, urls.poster_url.as_ref(), urls.background_url.as_ref())
            {
                title_updates.push(update);
            }

            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await?;
            let mut episode_by_tvdb = HashMap::<i64, &scryer_domain::Episode>::new();
            let mut episode_by_numbers = HashMap::<(String, String), &scryer_domain::Episode>::new();
            for episode in &episodes {
                if let Some(tvdb_id) = episode
                    .tvdb_id
                    .as_deref()
                    .and_then(|value| value.trim().parse::<i64>().ok())
                {
                    episode_by_tvdb.insert(tvdb_id, episode);
                }
                if let (Some(season), Some(number)) = (
                    episode.season_number.as_deref(),
                    episode.episode_number.as_deref(),
                ) {
                    episode_by_numbers.insert((season.to_string(), number.to_string()), episode);
                }
            }

            for incoming in &urls.episodes {
                let existing = episode_by_tvdb
                    .get(&incoming.tvdb_id)
                    .copied()
                    .or_else(|| {
                        episode_by_numbers
                            .get(&(
                                incoming.season_number.to_string(),
                                incoming.episode_number.to_string(),
                            ))
                            .copied()
                    });
                let Some(existing) = existing else {
                    summary.missing_artwork_results += 1;
                    summary.missing_episode_matches += 1;
                    tracing::debug!(
                        title_id = %title.id,
                        series_tvdb_id = tvdb_id,
                        episode_tvdb_id = incoming.tvdb_id,
                        season_number = incoming.season_number,
                        episode_number = incoming.episode_number,
                        "title image cache refresh skipped incoming episode still with no local episode match"
                    );
                    continue;
                };
                if incoming.image_url.is_none() {
                    summary.missing_artwork_results += 1;
                    summary.missing_incoming_image_urls += 1;
                    tracing::debug!(
                        title_id = %title.id,
                        episode_id = %existing.id,
                        episode_tvdb_id = incoming.tvdb_id,
                        "title image cache refresh skipped incoming episode still with no usable image URL"
                    );
                    continue;
                };
                if let Some(update) = episode_image_url_update(existing, incoming.image_url.as_ref())
                {
                    episode_updates.push(update);
                }
            }
        }

        summary.title_urls_updated = self
            .services
            .catalog
            .titles
            .update_title_artwork_urls(&title_updates)
            .await?;
        summary.episode_urls_updated = self
            .services
            .catalog
            .shows
            .update_episode_image_urls(&episode_updates)
            .await?;
        Ok(summary)
    }
}

fn title_tvdb_id(title: &Title) -> Option<i64> {
    title
        .external_ids
        .iter()
        .find(|external_id| external_id.source.trim().eq_ignore_ascii_case("tvdb"))
        .and_then(|external_id| external_id.value.trim().parse::<i64>().ok())
}

fn title_artwork_update(
    title: &Title,
    incoming_poster_url: Option<&String>,
    incoming_background_url: Option<&String>,
) -> Option<TitleArtworkUrlUpdate> {
    let current_poster = title
        .poster_source_url
        .as_ref()
        .or(title.poster_url.as_ref())
        .cloned();
    let current_background = title
        .background_source_url
        .as_ref()
        .or(title.background_url.as_ref())
        .cloned();
    let next_poster = incoming_poster_url.cloned().or(current_poster.clone());
    let next_background = incoming_background_url.cloned().or(current_background.clone());

    if next_poster == current_poster && next_background == current_background {
        return None;
    }

    Some(TitleArtworkUrlUpdate {
        title_id: title.id.clone(),
        poster_url: next_poster,
        background_url: next_background,
    })
}

fn episode_image_url_update(
    episode: &scryer_domain::Episode,
    incoming_image_url: Option<&String>,
) -> Option<EpisodeImageUrlUpdate> {
    let image_url = incoming_image_url?;
    if episode.image_url.as_deref() == Some(image_url.as_str()) {
        return None;
    }

    Some(EpisodeImageUrlUpdate {
        episode_id: episode.id.clone(),
        image_url: Some(image_url.clone()),
    })
}

#[cfg(test)]
mod title_image_cache_refresh_tests {
    use super::*;

    fn test_title() -> Title {
        Title {
            id: "title-1".to_string(),
            library_id: "library-1".to_string(),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            name: "Example".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: Vec::new(),
            canonical_tags: vec![],
            external_ids: vec![scryer_domain::ExternalId {
                source: "tvdb".to_string(),
                value: "123".to_string(),
            }],
            created_by: None,
            created_at: chrono::Utc::now(),
            year: None,
            overview: None,
            poster_url: Some("https://old.example/poster.jpg".to_string()),
            poster_source_url: None,
            background_url: Some("https://old.example/background.jpg".to_string()),
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
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn test_episode() -> scryer_domain::Episode {
        scryer_domain::Episode {
            id: "episode-1".to_string(),
            title_id: "title-1".to_string(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: None,
            title: None,
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: Some("456".to_string()),
            image_url: Some("https://old.example/still.jpg".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn title_artwork_update_preserves_existing_urls_when_incoming_is_missing() {
        let title = test_title();

        assert!(title_artwork_update(&title, None, None).is_none());
    }

    #[test]
    fn title_artwork_update_updates_only_when_incoming_url_changes() {
        let title = test_title();
        let incoming_poster = "https://new.example/poster.jpg".to_string();

        let update = title_artwork_update(&title, Some(&incoming_poster), None)
            .expect("changed poster should create update");

        assert_eq!(update.title_id, "title-1");
        assert_eq!(update.poster_url, Some(incoming_poster));
        assert_eq!(
            update.background_url,
            Some("https://old.example/background.jpg".to_string())
        );
    }

    #[test]
    fn episode_image_url_update_preserves_existing_url_when_incoming_is_missing() {
        let episode = test_episode();

        assert!(episode_image_url_update(&episode, None).is_none());
    }

    #[test]
    fn episode_image_url_update_updates_only_when_incoming_url_changes() {
        let episode = test_episode();
        let incoming_image = "https://new.example/still.jpg".to_string();

        let update = episode_image_url_update(&episode, Some(&incoming_image))
            .expect("changed still should create update");

        assert_eq!(update.episode_id, "episode-1");
        assert_eq!(update.image_url, Some(incoming_image));
    }
}
