use super::*;
use scryer_domain::{InterstitialMovieMetadata, NotificationEventType};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tracing::{info, warn};

pub const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
pub const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
const DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY: &str = "download_client.default_category";
const LEGACY_NZBGET_CATEGORY_SETTING_KEY: &str = "nzbget.category";
const RECENT_QUEUE_PRIORITY_WINDOW_DAYS: i64 = 14;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DownloadClientRoutingEntry {
    enabled: bool,
    category: Option<String>,
    recent_queue_priority: Option<String>,
    older_queue_priority: Option<String>,
    remove_completed: bool,
    remove_failed: bool,
}

fn routing_entry_enabled(config: &serde_json::Value) -> bool {
    match config.get("enabled") {
        Some(serde_json::Value::Bool(enabled)) => *enabled,
        Some(serde_json::Value::String(enabled)) => !matches!(
            enabled.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no"
        ),
        Some(serde_json::Value::Number(number)) => number.as_i64() != Some(0),
        _ => true,
    }
}

fn read_routing_string(raw_value: Option<&serde_json::Value>) -> Option<String> {
    raw_value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn read_routing_bool(raw_value: Option<&serde_json::Value>, default: bool) -> bool {
    match raw_value {
        Some(serde_json::Value::Bool(value)) => *value,
        Some(serde_json::Value::String(value)) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no"
        ),
        Some(serde_json::Value::Number(value)) => value.as_i64() != Some(0),
        _ => default,
    }
}

fn parse_download_client_routing_entry(config: &serde_json::Value) -> DownloadClientRoutingEntry {
    DownloadClientRoutingEntry {
        enabled: routing_entry_enabled(config),
        category: read_routing_string(config.get("category")),
        recent_queue_priority: read_routing_string(
            config
                .get("recentQueuePriority")
                .or_else(|| config.get("recentPriority"))
                .or_else(|| config.get("recent_priority")),
        ),
        older_queue_priority: read_routing_string(
            config
                .get("olderQueuePriority")
                .or_else(|| config.get("olderPriority"))
                .or_else(|| config.get("older_priority")),
        ),
        remove_completed: read_routing_bool(
            config
                .get("removeCompleted")
                .or_else(|| config.get("remove_completed"))
                .or_else(|| config.get("removeComplete")),
            false,
        ),
        remove_failed: read_routing_bool(
            config
                .get("removeFailed")
                .or_else(|| config.get("remove_failed"))
                .or_else(|| config.get("removeFailure")),
            false,
        ),
    }
}

fn parse_download_client_routing_map(
    raw_json: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str::<serde_json::Value>(raw_json)
        .ok()?
        .as_object()
        .cloned()
}

fn release_is_recent_for_queue_priority(baseline_date: Option<&str>) -> bool {
    let Some(baseline_date) = baseline_date else {
        return false;
    };
    let baseline_date = baseline_date.trim();
    let parsed_date = chrono::NaiveDate::parse_from_str(baseline_date, "%Y-%m-%d")
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(baseline_date)
                .ok()
                .map(|value| value.date_naive())
        })
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(baseline_date)
                .ok()
                .map(|value| value.date_naive())
        });
    let Some(parsed_date) = parsed_date else {
        return false;
    };
    let now = chrono::Utc::now().date_naive();
    let age_days = now.signed_duration_since(parsed_date).num_days();
    (0..=RECENT_QUEUE_PRIORITY_WINDOW_DAYS).contains(&age_days)
}

#[cfg(test)]
mod routing_tests {
    use super::parse_download_client_routing_entry;
    use serde_json::json;

    #[test]
    fn routing_entry_parses_legacy_and_new_queue_priority_fields() {
        let entry = parse_download_client_routing_entry(&json!({
            "enabled": true,
            "category": "tv",
            "recentPriority": "high",
            "olderQueuePriority": "low",
            "removeCompleted": true,
            "remove_failed": true
        }));

        assert!(entry.enabled);
        assert_eq!(entry.category.as_deref(), Some("tv"));
        assert_eq!(entry.recent_queue_priority.as_deref(), Some("high"));
        assert_eq!(entry.older_queue_priority.as_deref(), Some("low"));
        assert!(entry.remove_completed);
        assert!(entry.remove_failed);
    }
}

fn interstitial_movie_from_anime_movie(movie: &AnimeMovie) -> InterstitialMovieMetadata {
    InterstitialMovieMetadata {
        tvdb_id: movie
            .movie_tvdb_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
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
        digital_release_date: movie.digital_release_date.clone(),
        association_confidence: Some(movie.association_confidence.clone()),
        continuity_status: Some(movie.continuity_status.clone()),
        movie_form: Some(movie.movie_form.clone()),
        confidence: Some(movie.confidence.clone()),
        signal_summary: Some(movie.signal_summary.clone()),
        placement: Some(movie.placement.clone()),
        movie_tmdb_id: movie.movie_tmdb_id.map(|id| id.to_string()),
        movie_mal_id: movie.movie_mal_id.map(|id| id.to_string()),
        movie_anidb_id: movie.movie_anidb_id.map(|id| id.to_string()),
    }
}

fn anime_movie_identity_keys(movie: &AnimeMovie) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(tvdb_id) = movie.movie_tvdb_id {
        keys.push(format!("tvdb:{tvdb_id}"));
    }
    if let Some(tmdb_id) = movie.movie_tmdb_id {
        keys.push(format!("tmdb:{tmdb_id}"));
    }
    if let Some(imdb_id) = movie
        .movie_imdb_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        keys.push(format!("imdb:{}", imdb_id.trim().to_ascii_lowercase()));
    }
    keys
}

fn anime_mapping_identity_keys(mapping: &AnimeMapping) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(tvdb_id) = mapping.alt_tvdb_id {
        keys.push(format!("tvdb:{tvdb_id}"));
    }
    if let Some(tmdb_id) = mapping.themoviedb_id {
        keys.push(format!("tmdb:{tmdb_id}"));
    }
    if mapping.global_media_type == "movie"
        && let Some(tvdb_id) = mapping.thetvdb_id
    {
        keys.push(format!("tvdb:{tvdb_id}"));
    }
    keys
}

fn anime_movie_after_season(
    movie: &AnimeMovie,
    season_last_aired: &std::collections::BTreeMap<i32, String>,
) -> i32 {
    // Prefer digital_release_date for precise placement.
    if let Some(release_date) = movie
        .digital_release_date
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return season_last_aired
            .iter()
            .filter(|(_, last)| last.as_str() <= release_date)
            .max_by_key(|(season, _)| *season)
            .map(|(season, _)| *season)
            .unwrap_or(0);
    }

    // Fall back to movie year — place after the last season whose final episode
    // aired in a year <= the movie's release year.
    if let Some(year) = movie.year {
        return season_last_aired
            .iter()
            .filter(|(_, last)| {
                last.get(..4)
                    .and_then(|y| y.parse::<i32>().ok())
                    .is_some_and(|aired_year| aired_year <= year)
            })
            .max_by_key(|(season, _)| *season)
            .map(|(season, _)| *season)
            .unwrap_or(0);
    }

    0
}

fn anime_movie_release_sort_key(movie: &AnimeMovie) -> (&str, &str) {
    (
        movie
            .digital_release_date
            .as_deref()
            .unwrap_or("9999-12-31"),
        movie.sort_title.as_str(),
    )
}

#[cfg(test)]
mod anime_movie_mapping_tests {
    use super::interstitial_movie_from_anime_movie;
    use crate::AnimeMovie;

    #[test]
    fn interstitial_movies_preserve_classification_metadata() {
        let movie = AnimeMovie {
            movie_tvdb_id: Some(200),
            movie_tmdb_id: Some(300),
            movie_imdb_id: Some("tt123".into()),
            movie_mal_id: Some(400),
            movie_anidb_id: None,
            name: "Sample Movie".into(),
            slug: "sample-movie".into(),
            year: Some(2024),
            content_status: "released".into(),
            overview: "Overview".into(),
            poster_url: "poster".into(),
            language: "eng".into(),
            runtime_minutes: 95,
            sort_title: "Sample Movie".into(),
            imdb_id: "tt123".into(),
            genres: vec!["Action".into()],
            studio: "Studio".into(),
            digital_release_date: Some("2024-02-01".into()),
            association_confidence: "high".into(),
            continuity_status: "canon".into(),
            movie_form: "movie".into(),
            placement: "ordered".into(),
            confidence: "high".into(),
            signal_summary: "TVDB marked special as critical to story".into(),
        };

        let mapped = interstitial_movie_from_anime_movie(&movie);
        assert_eq!(mapped.tvdb_id, "200");
        assert_eq!(mapped.continuity_status.as_deref(), Some("canon"));
        assert_eq!(mapped.association_confidence.as_deref(), Some("high"));
        assert_eq!(mapped.confidence.as_deref(), Some("high"));
        assert_eq!(mapped.placement.as_deref(), Some("ordered"));
        assert_eq!(mapped.movie_tmdb_id.as_deref(), Some("300"));
        assert_eq!(mapped.movie_mal_id.as_deref(), Some("400"));
    }
}

impl AppUseCase {
    async fn emit_hydration_activity(
        &self,
        title: &Title,
        kind: ActivityKind,
        severity: ActivitySeverity,
        message: String,
    ) {
        if let Err(err) = self
            .services
            .record_activity_event(
                None,
                Some(title.id.clone()),
                Some(format!("{:?}", title.facet).to_lowercase()),
                kind,
                message,
                severity,
                vec![ActivityChannel::WebUi],
            )
            .await
        {
            warn!(
                title_id = %title.id,
                error = %err,
                "failed to record hydration activity event"
            );
        }
    }

    async fn emit_hydration_started(&self, title: &Title) {
        self.emit_hydration_activity(
            title,
            ActivityKind::MetadataHydrationStarted,
            ActivitySeverity::Info,
            format!("hydrating metadata for {}", title.name),
        )
        .await;
    }

    async fn emit_hydration_completed(&self, title: &Title) {
        self.emit_hydration_activity(
            title,
            ActivityKind::MetadataHydrationCompleted,
            ActivitySeverity::Success,
            format!("metadata hydrated for {}", title.name),
        )
        .await;
    }

    async fn emit_hydration_failed(&self, title: &Title, reason: &str) {
        self.emit_hydration_activity(
            title,
            ActivityKind::MetadataHydrationFailed,
            ActivitySeverity::Warning,
            format!("metadata hydration failed for {}: {}", title.name, reason),
        )
        .await;
    }

    async fn read_download_client_routing_value(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(value) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await?
        {
            return Ok(Some(value));
        }

        self.read_setting_string_value(LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
    }

    async fn read_download_client_routing_entry(
        &self,
        facet: &MediaFacet,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientRoutingEntry>> {
        let scope_id = facet.as_str();

        let Some(raw_json) = self.read_download_client_routing_value(scope_id).await? else {
            return Ok(None);
        };

        let Some(routing_map) = parse_download_client_routing_map(&raw_json) else {
            return Ok(None);
        };

        Ok(routing_map
            .get(client_id)
            .map(parse_download_client_routing_entry))
    }

    pub(crate) async fn should_remove_completed_download(
        &self,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        self.read_download_client_routing_entry(facet, client_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|entry| entry.remove_completed)
    }

    pub(crate) async fn should_remove_failed_download(
        &self,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        self.read_download_client_routing_entry(facet, client_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|entry| entry.remove_failed)
    }

    pub(crate) fn is_recent_for_queue_priority(&self, baseline_date: Option<&str>) -> Option<bool> {
        baseline_date.map(|_| release_is_recent_for_queue_priority(baseline_date))
    }

    pub(crate) async fn metadata_language(&self) -> String {
        self.read_setting_string_value_for_scope("system", "metadata_language", None)
            .await
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "eng".to_string())
    }

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

    /// Return the configured root folders for a facet.
    ///
    /// Reads the `<facet>.root_folders` JSON setting.  When absent or empty,
    /// falls back to the single `<facet>.path` setting and returns it as the
    /// sole default entry.
    pub async fn root_folders_for_facet(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        let handler = self.facet_registry.get(facet);
        let root_folders_key = handler.map(|h| h.root_folders_key());
        let library_path_key = handler.map(|h| h.library_path_key());
        let default_path = handler
            .map(|h| h.default_library_path())
            .unwrap_or("/media");

        // Try the root_folders JSON array first.
        if let Some(key) = root_folders_key
            && let Some(raw) = self
                .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, key, None)
                .await?
        {
            let trimmed = raw.trim();
            if !trimmed.is_empty()
                && trimmed != "[]"
                && let Ok(entries) =
                    serde_json::from_str::<Vec<scryer_domain::RootFolderEntry>>(trimmed)
                && !entries.is_empty()
            {
                return Ok(entries);
            }
        }

        // Fall back to the single path setting.
        let path = if let Some(key) = library_path_key {
            self.read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, key, None)
                .await?
                .unwrap_or_else(|| default_path.to_string())
        } else {
            default_path.to_string()
        };

        Ok(vec![scryer_domain::RootFolderEntry {
            path,
            is_default: true,
        }])
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
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
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
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: request.min_availability,
            digital_release_date: None,
            folder_path: None,
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

        {
            let facet_str = format!("{:?}", title.facet).to_lowercase();
            let activity_kind = self
                .facet_registry
                .get(&title.facet)
                .and_then(|h| h.title_added_activity_kind())
                .unwrap_or(ActivityKind::MovieAdded);
            let mut metadata = HashMap::new();
            metadata.insert("title_name".to_string(), serde_json::json!(title.name));
            if let Some(ref year) = title.year {
                metadata.insert("title_year".to_string(), serde_json::json!(year));
            }
            metadata.insert("title_facet".to_string(), serde_json::json!(facet_str));
            if let Some(ref poster) = title.poster_url {
                metadata.insert("poster_url".to_string(), serde_json::json!(poster));
            }
            let envelope = crate::activity::NotificationEnvelope {
                event_type: NotificationEventType::TitleAdded,
                title: format!("{} added: {}", facet_str, title.name),
                body: format!("{} has been added to your library.", title.name),
                facet: Some(facet_str),
                metadata,
            };
            self.services
                .record_activity_event_with_notification(
                    Some(actor.id.clone()),
                    Some(title.id.clone()),
                    None,
                    activity_kind,
                    format!("new title added: {}", title.name),
                    ActivitySeverity::Info,
                    vec![ActivityChannel::WebUi],
                    envelope,
                )
                .await?;
        }

        // Wake the background hydration loop to fetch rich metadata from SMG.
        // The title is already persisted — hydration happens asynchronously.
        self.emit_hydration_started(&title).await;
        self.services.hydration_wake.notify_one();
        if title
            .poster_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.services.poster_wake.notify_one();
        }
        if title
            .banner_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.services.banner_wake.notify_one();
        }
        if title
            .background_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.services.fanart_wake.notify_one();
        }

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
                self.emit_hydration_failed(&title, "no tvdb external id found")
                    .await;
                return title;
            }
        };

        let language = self.metadata_language().await;

        let Some(handler) = self.facet_registry.get(&title.facet) else {
            return title;
        };

        self.emit_hydration_started(&title).await;

        match handler
            .hydrate_metadata(self.services.metadata_gateway.as_ref(), tvdb_id, &language)
            .await
        {
            Ok(result) => {
                let hydrated = self.apply_hydration_result(title, result).await;
                if hydrated.metadata_fetched_at.is_some() {
                    self.emit_hydration_completed(&hydrated).await;
                } else {
                    self.emit_hydration_failed(&hydrated, "metadata could not be persisted")
                        .await;
                }
                hydrated
            }
            Err(err) => {
                warn!(
                    title_id = %title.id,
                    tvdb_id = tvdb_id,
                    error = %err,
                    "failed to fetch metadata from gateway"
                );
                self.emit_hydration_failed(&title, &err.to_string()).await;
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
        // Prefer non-special (R/regular) mappings over specials (S) to avoid
        // OVA metadata clobbering the main series (e.g. Bleach anilist 834 vs 269).
        let mut metadata_update = result.metadata_update;
        if let Some(mapping) = result
            .anime_mappings
            .iter()
            .find(|m| m.mapping_type != "S")
            .or(result.anime_mappings.first())
        {
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
        if let Some(primary) = result
            .anime_mappings
            .iter()
            .find(|m| m.mapping_type != "S")
            .or(result.anime_mappings.first())
        {
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
                &result.anime_movies,
            )
            .await;
        }

        if title
            .poster_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.services.poster_wake.notify_one();
        }
        if title
            .banner_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.services.banner_wake.notify_one();
        }
        if title
            .background_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.services.fanart_wake.notify_one();
        }

        title
    }

    pub(crate) async fn create_series_seasons_and_episodes(
        &self,
        title: &Title,
        seasons: &[SeasonMetadata],
        episodes: &[EpisodeMetadata],
        anime_mappings: &[AnimeMapping],
        anime_movies: &[AnimeMovie],
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

        // Fetch existing collections so we can reuse them instead of creating
        // duplicates on every metadata refresh cycle.
        let existing_collections = self
            .services
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let existing_collection_map: std::collections::HashMap<(CollectionType, String), String> =
            existing_collections
                .iter()
                .map(|c| {
                    (
                        (c.collection_type, c.collection_index.clone()),
                        c.id.clone(),
                    )
                })
                .collect();

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

        let derived_anime_movies: Vec<&AnimeMovie> =
            if title.facet == MediaFacet::Anime && inter_season_movies {
                anime_movies
                    .iter()
                    .filter(|movie| {
                        !movie.name.trim().is_empty()
                            && matches!(movie.association_confidence.as_str(), "medium" | "high")
                    })
                    .collect()
            } else {
                vec![]
            };
        let specials_movies: Vec<InterstitialMovieMetadata> = derived_anime_movies
            .iter()
            .copied()
            .filter(|movie| movie.placement == "specials")
            .map(interstitial_movie_from_anime_movie)
            .collect();
        let ordered_movies: Vec<&AnimeMovie> = derived_anime_movies
            .iter()
            .copied()
            .filter(|movie| movie.placement != "specials")
            .collect();

        let mut season_number_to_collection: std::collections::HashMap<i32, String> =
            std::collections::HashMap::new();

        for season in best_season_by_number.values() {
            let season_monitored = seasons_with_episodes.contains(&season.number)
                && should_monitor_season(&monitor_type, season.number, monitor_specials);
            let collection_type = if season.number == 0 && title.facet == MediaFacet::Anime {
                CollectionType::Specials
            } else {
                CollectionType::Season
            };
            let collection_index = season.number.to_string();
            if let Some(existing_id) =
                existing_collection_map.get(&(collection_type, collection_index.clone()))
            {
                // Update language-sensitive label if it changed
                if !season.label.is_empty()
                    && let Ok(Some(existing)) =
                        self.services.shows.get_collection_by_id(existing_id).await
                    && existing.label.as_deref() != Some(&season.label)
                {
                    let _ = self
                        .services
                        .shows
                        .update_collection(
                            existing_id,
                            None,
                            None,
                            Some(season.label.clone()),
                            None,
                            None,
                            None,
                            None,
                        )
                        .await;
                }
                season_number_to_collection.insert(season.number, existing_id.clone());
                continue;
            }

            let collection = Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type,
                collection_index,
                label: Some(season.label.clone()),
                ordered_path: None,
                narrative_order: Some(season.number.to_string()),
                first_episode_number: None,
                last_episode_number: None,
                interstitial_movie: None,
                specials_movies: if season.number == 0 && title.facet == MediaFacet::Anime {
                    specials_movies.clone()
                } else {
                    vec![]
                },
                interstitial_season_episode: None,
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

        // Create interstitial movie collections for anime titles using the
        // derived anime_movies payload from SMG. Episode mappings are only used
        // to route any linked season-0 episode records into the movie collection
        // when a matching mapping still exists.
        let mut interstitial_episode_lookup: std::collections::HashMap<(i32, i32), String> =
            std::collections::HashMap::new();

        if title.facet == MediaFacet::Anime && inter_season_movies && !ordered_movies.is_empty() {
            let mut mapping_episode_links: HashMap<String, Vec<(i32, i32)>> = HashMap::new();
            for mapping in anime_mappings {
                let identity_keys = anime_mapping_identity_keys(mapping);
                if identity_keys.is_empty() || mapping.episode_mappings.is_empty() {
                    continue;
                }
                let mut linked_episodes = Vec::new();
                for em in &mapping.episode_mappings {
                    for ep_num in em.episode_start..=em.episode_end {
                        linked_episodes.push((em.tvdb_season, ep_num));
                    }
                }
                for key in identity_keys {
                    mapping_episode_links
                        .entry(key)
                        .or_default()
                        .extend(linked_episodes.iter().copied());
                }
            }

            let mut movies_by_position: std::collections::BTreeMap<i32, Vec<&AnimeMovie>> =
                std::collections::BTreeMap::new();
            for movie in &ordered_movies {
                let after_season = anime_movie_after_season(movie, &season_last_aired);
                movies_by_position
                    .entry(after_season)
                    .or_default()
                    .push(*movie);
            }

            for (after_season, movies) in &mut movies_by_position {
                movies.sort_by(|left, right| {
                    anime_movie_release_sort_key(left)
                        .cmp(&anime_movie_release_sort_key(right))
                        .then_with(|| left.name.cmp(&right.name))
                });

                for (seq, movie) in movies.iter().enumerate() {
                    let narrative_order = format!("{}.{}", after_season, seq + 1);
                    let label = if movie.continuity_status == "canon" {
                        movie.name.clone()
                    } else {
                        format!("Movie {}", seq + 1)
                    };

                    // Reuse existing interstitial collection if one already exists.
                    if let Some(existing_id) = existing_collection_map
                        .get(&(CollectionType::Interstitial, narrative_order.clone()))
                    {
                        // Update language-sensitive label if it changed
                        if !label.is_empty()
                            && let Ok(Some(existing_coll)) =
                                self.services.shows.get_collection_by_id(existing_id).await
                            && existing_coll.label.as_deref() != Some(&label)
                        {
                            let _ = self
                                .services
                                .shows
                                .update_collection(
                                    existing_id,
                                    None,
                                    None,
                                    Some(label.clone()),
                                    None,
                                    None,
                                    None,
                                    None,
                                )
                                .await;
                        }

                        // Update interstitial_season_episode if it changed or was missing
                        let new_season_episode = anime_movie_identity_keys(movie)
                            .iter()
                            .filter_map(|key| mapping_episode_links.get(key.as_str()))
                            .flatten()
                            .find(|(s, _)| *s == 0)
                            .map(|(_, ep)| format!("S00E{:0>2}", ep));
                        if let Some(ref se) = new_season_episode
                            && let Ok(Some(existing_coll)) =
                                self.services.shows.get_collection_by_id(existing_id).await
                            && existing_coll.interstitial_season_episode.as_deref()
                                != Some(se.as_str())
                        {
                            let _ = self
                                .services
                                .shows
                                .update_interstitial_season_episode(existing_id, Some(se.clone()))
                                .await;
                        }

                        for key in anime_movie_identity_keys(movie) {
                            if let Some(linked_episodes) = mapping_episode_links.get(&key) {
                                for (season_num, episode_num) in linked_episodes {
                                    interstitial_episode_lookup
                                        .insert((*season_num, *episode_num), existing_id.clone());
                                }
                            }
                        }
                        continue;
                    }

                    // Compute the S00Exx episode number from the linked episode data
                    let season_episode = anime_movie_identity_keys(movie)
                        .iter()
                        .filter_map(|key| mapping_episode_links.get(key.as_str()))
                        .flatten()
                        .find(|(s, _)| *s == 0)
                        .map(|(_, ep)| format!("S00E{:0>2}", ep));

                    let collection = Collection {
                        id: Id::new().0,
                        title_id: title.id.clone(),
                        collection_type: CollectionType::Interstitial,
                        collection_index: narrative_order.clone(),
                        label: Some(label.clone()),
                        ordered_path: None,
                        narrative_order: Some(narrative_order.clone()),
                        first_episode_number: None,
                        last_episode_number: None,
                        interstitial_movie: Some(interstitial_movie_from_anime_movie(movie)),
                        specials_movies: vec![],
                        interstitial_season_episode: season_episode,
                        monitored: true,
                        created_at: Utc::now(),
                    };

                    match self.services.shows.create_collection(collection).await {
                        Ok(created) => {
                            info!(
                                title_id = %title.id,
                                label = %label,
                                narrative_order = %narrative_order,
                                placement = %movie.placement,
                                "created interstitial movie collection"
                            );
                            for key in anime_movie_identity_keys(movie) {
                                if let Some(linked_episodes) = mapping_episode_links.get(&key) {
                                    for (season_num, episode_num) in linked_episodes {
                                        interstitial_episode_lookup.insert(
                                            (*season_num, *episode_num),
                                            created.id.clone(),
                                        );
                                    }
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
            if let Some(ref cid) = collection_id
                && interstitial_episode_lookup.contains_key(&(ep.season_number, ep.episode_number))
                && !ep.name.is_empty()
                && labeled_collections.insert(cid.clone())
                && let Err(err) = self
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

            // If episode already exists, update language-sensitive fields instead of skipping.
            if let Ok(Some(existing)) = self
                .services
                .shows
                .find_episode_by_title_and_numbers(
                    &title.id,
                    &ep.season_number.to_string(),
                    &ep.episode_number.to_string(),
                )
                .await
            {
                let new_title = if ep.name.is_empty() {
                    None
                } else {
                    Some(ep.name.clone())
                };
                let new_overview = if ep.overview.trim().is_empty() {
                    None
                } else {
                    Some(ep.overview.clone())
                };
                // Only update if the new data differs from existing
                let title_changed = new_title.as_deref() != existing.title.as_deref();
                let overview_changed = new_overview.as_deref() != existing.overview.as_deref();
                let new_tvdb_id = if ep.tvdb_id > 0 {
                    Some(ep.tvdb_id.to_string())
                } else {
                    None
                };
                let tvdb_id_changed = new_tvdb_id.as_deref() != existing.tvdb_id.as_deref();
                if title_changed || overview_changed || tvdb_id_changed {
                    let _ = self
                        .services
                        .shows
                        .update_episode(
                            &existing.id,
                            None,
                            None,
                            None,
                            if title_changed {
                                new_title.clone()
                            } else {
                                None
                            },
                            if title_changed { new_title } else { None },
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            if overview_changed { new_overview } else { None },
                            if tvdb_id_changed { new_tvdb_id } else { None },
                        )
                        .await;
                }
                continue;
            }

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
                tvdb_id: if ep.tvdb_id > 0 {
                    Some(ep.tvdb_id.to_string())
                } else {
                    None
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
        source_kind: Option<DownloadSourceKind>,
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
        let is_recent = self.is_recent_for_queue_priority(
            title
                .first_aired
                .as_deref()
                .or(title.digital_release_date.as_deref()),
        );
        let job_result = self
            .services
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                source_hint,
                source_kind,
                source_title,
                source_password: source_password.clone(),
                category: Some(category),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: None,
            })
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
                        collection_id: None,
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
        {
            let facet_str = format!("{:?}", title.facet).to_lowercase();
            let mut grab_meta = HashMap::new();
            grab_meta.insert("title_name".to_string(), serde_json::json!(title.name));
            grab_meta.insert("title_facet".to_string(), serde_json::json!(facet_str));
            if let Some(ref poster) = title.poster_url {
                grab_meta.insert("poster_url".to_string(), serde_json::json!(poster));
            }
            let envelope = crate::activity::NotificationEnvelope {
                event_type: NotificationEventType::Grab,
                title: format!("Grabbed: {}", title.name),
                body: format!("Download queued for {}", title.name),
                facet: Some(facet_str),
                metadata: grab_meta,
            };
            self.services
                .record_activity_event_with_notification(
                    Some(actor.id.clone()),
                    Some(title.id.clone()),
                    None,
                    ActivityKind::MovieDownloaded,
                    format!("movie downloaded: {}", title.name),
                    ActivitySeverity::Success,
                    vec![ActivityChannel::Toast, ActivityChannel::WebUi],
                    envelope,
                )
                .await?;
        }

        Ok((title, grab.job_id))
    }

    pub async fn queue_existing_title_download(
        &self,
        actor: &User,
        title_id: &str,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
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
        let is_recent = self.is_recent_for_queue_priority(
            title
                .first_aired
                .as_deref()
                .or(title.digital_release_date.as_deref()),
        );
        let job_result = self
            .services
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                source_hint,
                source_kind,
                source_title,
                source_password: source_password.clone(),
                category: Some(category),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: None,
            })
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
                        collection_id: None,
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
        {
            let facet_str = format!("{:?}", title.facet).to_lowercase();
            let mut grab_meta = HashMap::new();
            grab_meta.insert("title_name".to_string(), serde_json::json!(title.name));
            grab_meta.insert("title_facet".to_string(), serde_json::json!(facet_str));
            if let Some(ref poster) = title.poster_url {
                grab_meta.insert("poster_url".to_string(), serde_json::json!(poster));
            }
            let envelope = crate::activity::NotificationEnvelope {
                event_type: NotificationEventType::Grab,
                title: format!("Grabbed: {}", title.name),
                body: format!("Download queued for {}", title.name),
                facet: Some(facet_str),
                metadata: grab_meta,
            };
            self.services
                .record_activity_event_with_notification(
                    Some(actor.id.clone()),
                    Some(title.id.clone()),
                    None,
                    ActivityKind::MovieDownloaded,
                    format!("movie downloaded: {}", title.name),
                    ActivitySeverity::Success,
                    vec![ActivityChannel::Toast, ActivityChannel::WebUi],
                    envelope,
                )
                .await?;
        }

        Ok(grab.job_id)
    }

    /// Resolve the per-facet fallback category used when the selected client
    /// does not declare an explicit routing category.
    pub(crate) async fn derive_download_category(&self, facet: &MediaFacet) -> String {
        let scope_id = facet.as_str();

        if let Ok(Some(configured)) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, Some(scope_id))
            .await
        {
            let trimmed = configured.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        if let Ok(Some(configured)) = self
            .read_setting_string_value(LEGACY_NZBGET_CATEGORY_SETTING_KEY, Some(scope_id))
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

        if title.monitored {
            let now = Utc::now();
            if let Some(handler) = self.facet_registry.get(&title.facet) {
                if handler.has_episodes() {
                    self.sync_wanted_series_inner(&title, &now, true).await;
                } else {
                    self.sync_wanted_movie_inner(&title, &now, true).await;
                }
            }
        } else if let Err(err) = self
            .services
            .wanted_items
            .delete_wanted_items_for_title(&title.id)
            .await
        {
            warn!(
                title_id = title.id.as_str(),
                error = %err,
                "failed to delete wanted items after disabling monitoring"
            );
        }

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

        // Auto-monitor the parent title + immediate wanted sync
        if monitored
            && let Ok(Some(title)) = self.services.titles.get_by_id(&collection.title_id).await
        {
            if !title.monitored {
                let _ = self.services.titles.update_monitored(&title.id, true).await;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because a collection was monitored"
                );
            }

            let now = Utc::now();
            if let Some(handler) = self.facet_registry.get(&title.facet)
                && handler.has_episodes()
            {
                // Re-fetch title in case monitoring was just updated
                if let Ok(Some(title)) = self.services.titles.get_by_id(&title.id).await {
                    self.sync_wanted_series_inner(&title, &now, true).await;
                }
            }
        }

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
                None,
                None,
            )
            .await?;

        // When monitoring an episode, ensure the parent title and collection are
        // also monitored — matching Sonarr behavior where monitoring any item
        // implies the title should be monitored.
        if monitored {
            if let Ok(Some(title)) = self.services.titles.get_by_id(&episode.title_id).await
                && !title.monitored
            {
                let _ = self.services.titles.update_monitored(&title.id, true).await;
                tracing::info!(
                    title_id = %title.id,
                    title_name = %title.name,
                    "auto-monitored title because an episode was monitored"
                );
            }

            if let Some(ref collection_id) = episode.collection_id
                && let Ok(Some(collection)) = self
                    .services
                    .shows
                    .get_collection_by_id(collection_id)
                    .await
                && !collection.monitored
            {
                let _ = self
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
                        Some(true),
                    )
                    .await;
                tracing::info!(
                    collection_id = %collection_id,
                    "auto-monitored collection because an episode was monitored"
                );
            }

            // Immediately sync wanted items for this title so the episode
            // appears on the wanted page without waiting for the hourly sync.
            if let Ok(Some(title)) = self.services.titles.get_by_id(&episode.title_id).await {
                let now = Utc::now();
                if let Some(handler) = self.facet_registry.get(&title.facet)
                    && handler.has_episodes()
                {
                    self.sync_wanted_series_inner(&title, &now, true).await;
                }
            }
        }

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
            if let Some(ref folder_path) = title.folder_path {
                let folder = Path::new(folder_path);
                if folder.exists() {
                    if let Err(err) = fs::remove_dir_all(folder).await {
                        return Err(AppError::Repository(format!(
                            "failed to delete title folder {}: {err}",
                            folder.display()
                        )));
                    }
                    info!(
                        path = %folder.display(),
                        title = %title.name,
                        "deleted title folder"
                    );
                }
            } else {
                info!(
                    title_id = %id,
                    title_name = %title.name,
                    "no folder_path set, skipping file deletion"
                );
            }
        }

        // Purge recycle bin entries that belonged to this title.
        if let Some(media_root) = crate::recycle_bin::media_root_for_title(self, &title).await {
            let config = crate::recycle_bin::resolve_recycle_config(self, Some(&media_root)).await;
            match crate::recycle_bin::purge_for_title(&config, id).await {
                Ok(n) if n > 0 => info!(
                    purged = n,
                    title_id = %id,
                    "purged recycle bin entries for deleted title"
                ),
                Err(e) => warn!(
                    error = %e,
                    title_id = %id,
                    "failed to purge recycle entries for deleted title"
                ),
                _ => {}
            }
        }

        let queued_submission_keys = match self
            .services
            .download_submissions
            .list_for_title(id)
            .await
        {
            Ok(submissions) => submissions
                .into_iter()
                .map(|submission| {
                    (
                        submission.download_client_type,
                        submission.download_client_item_id,
                    )
                })
                .collect::<HashSet<_>>(),
            Err(err) => {
                warn!(
                    title_id = %id,
                    error = %err,
                    "failed to list download submissions while deleting title; falling back to embedded queue metadata only"
                );
                HashSet::new()
            }
        };

        // Cancel any inflight downloads for this title
        match self.services.download_client.list_queue().await {
            Ok(queue_items) => {
                for item in queue_items {
                    let matches_title = item.title_id.as_deref() == Some(id)
                        || queued_submission_keys.contains(&(
                            item.client_type.clone(),
                            item.download_client_item_id.clone(),
                        ));
                    if matches_title
                        && let Err(err) = self
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
            Err(err) => {
                warn!(
                    title_id = %id,
                    error = %err,
                    "failed to list download queue while deleting title; skipping download cancellation"
                );
            }
        }

        if let Err(err) = self
            .services
            .pending_releases
            .delete_pending_releases_for_title(id)
            .await
        {
            warn!(
                title_id = %id,
                error = %err,
                "failed to delete pending releases while deleting title"
            );
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

        if let Err(err) = self
            .services
            .download_submissions
            .delete_for_title(id)
            .await
        {
            warn!(
                title_id = %id,
                error = %err,
                "failed to delete download submissions while deleting title"
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

        // Emit activity event with notification envelope for title deleted
        {
            let facet_str = format!("{:?}", title.facet).to_lowercase();
            let mut metadata = HashMap::new();
            metadata.insert("title_name".to_string(), serde_json::json!(title.name));
            if let Some(ref year) = title.year {
                metadata.insert("title_year".to_string(), serde_json::json!(year));
            }
            metadata.insert("title_facet".to_string(), serde_json::json!(facet_str));
            let envelope = crate::activity::NotificationEnvelope {
                event_type: NotificationEventType::TitleDeleted,
                title: format!("{} deleted: {}", facet_str, title.name),
                body: format!("{} has been removed from your library.", title.name),
                facet: Some(facet_str),
                metadata,
            };
            let _ = self
                .services
                .record_activity_event_with_notification(
                    Some(actor.id.clone()),
                    Some(id.to_string()),
                    None,
                    ActivityKind::SystemNotice,
                    format!("title deleted: {}", title.name),
                    ActivitySeverity::Info,
                    vec![ActivityChannel::WebUi],
                    envelope,
                )
                .await;
        }

        Ok(())
    }

    pub async fn delete_media_file(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageTitle)?;

        let media_file = self
            .services
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;

        if delete_from_disk {
            let file_path = media_file.file_path.trim().to_string();
            if !file_path.is_empty() {
                let recycle_config =
                    crate::recycle_bin::config_from_file_path(Path::new(&file_path));

                let manifest = crate::recycle_bin::RecycleManifest {
                    recycled_at: chrono::Utc::now().to_rfc3339(),
                    original_path: file_path.clone(),
                    size_bytes: fs::metadata(&file_path).await.map(|m| m.len()).unwrap_or(0),
                    title_id: Some(media_file.title_id.clone()),
                    reason: "file_deleted".to_string(),
                };

                if let Err(error) = crate::recycle_bin::recycle_file(
                    &recycle_config,
                    Path::new(&file_path),
                    manifest,
                )
                .await
                {
                    warn!(
                        file_id = %file_id,
                        file_path = %file_path,
                        error = %error,
                        "failed to recycle media file"
                    );
                    return Err(error);
                }
            }
        }

        self.services.media_files.delete_media_file(file_id).await?;

        info!(
            file_id = %file_id,
            file_path = %media_file.file_path,
            delete_from_disk = %delete_from_disk,
            "media file deleted"
        );

        // Record title history: FileDeleted
        {
            let mut data = HashMap::new();
            data.insert("file_path".into(), serde_json::json!(&media_file.file_path));
            data.insert(
                "size_bytes".into(),
                serde_json::json!(media_file.size_bytes),
            );
            data.insert(
                "reason".into(),
                serde_json::json!(if delete_from_disk {
                    "manual_disk"
                } else {
                    "manual_db_only"
                }),
            );
            let _ = self
                .services
                .record_title_history(NewTitleHistoryEvent {
                    title_id: media_file.title_id.clone(),
                    episode_id: media_file.episode_id.clone(),
                    collection_id: None,
                    event_type: TitleHistoryEventType::FileDeleted,
                    source_title: Some(media_file.file_path.clone()),
                    quality: None,
                    download_id: None,
                    data,
                })
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

    pub async fn list_title_media_size_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services
            .media_files
            .list_title_media_size_summaries(title_ids)
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
        let parsed_type = CollectionType::parse(collection_type.trim().to_lowercase().as_str())
            .ok_or_else(|| {
                AppError::Validation(format!("unknown collection type: {}", collection_type))
            })?;
        if collection_index.trim().is_empty() {
            return Err(AppError::Validation("collection index is required".into()));
        }

        self.validate_title_exists(&title_id).await?;

        let collection = Collection {
            id: Id::new().0,
            title_id,
            collection_type: parsed_type,
            collection_index: collection_index.trim().to_string(),
            label: normalize_show_text_opt(label),
            ordered_path: normalize_show_text_opt(ordered_path),
            narrative_order: None,
            first_episode_number: normalize_show_text_opt(first_episode_number),
            last_episode_number: normalize_show_text_opt(last_episode_number),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
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

        if let Some(raw) = &collection_type
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation(
                "collection type cannot be empty".into(),
            ));
        }
        let parsed_type = collection_type
            .map(|raw| {
                CollectionType::parse(raw.trim().to_lowercase().as_str()).ok_or_else(|| {
                    AppError::Validation(format!("unknown collection type: {}", raw))
                })
            })
            .transpose()?;

        if let Some(raw) = &collection_index
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation(
                "collection index cannot be empty".into(),
            ));
        }

        let collection = self
            .services
            .shows
            .update_collection(
                &collection_id,
                parsed_type,
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

        let parsed_episode_type =
            scryer_domain::EpisodeType::parse(episode_type.trim().to_lowercase().as_str())
                .ok_or_else(|| {
                    AppError::Validation(format!("unknown episode type: {}", episode_type))
                })?;

        self.validate_title_exists(&title_id).await?;

        let episode = Episode {
            id: Id::new().0,
            title_id,
            collection_id,
            episode_type: parsed_episode_type,
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
            tvdb_id: None,
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
        overview: Option<String>,
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
            && overview.is_none()
        {
            return Err(AppError::Validation(
                "at least one episode field must be provided".into(),
            ));
        }

        if let Some(raw) = &episode_type
            && raw.trim().is_empty()
        {
            return Err(AppError::Validation("episode type cannot be empty".into()));
        }

        let parsed_episode_type = episode_type
            .map(|value| {
                scryer_domain::EpisodeType::parse(value.trim().to_lowercase().as_str())
                    .ok_or_else(|| AppError::Validation(format!("unknown episode type: {}", value)))
            })
            .transpose()?;

        let episode = self
            .services
            .shows
            .update_episode(
                &episode_id,
                parsed_episode_type,
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
                overview,
                None,
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
) -> scryer_domain::EpisodeType {
    use scryer_domain::EpisodeType;
    if season_number == 0 {
        return match anime_media_type {
            Some("OVA") => EpisodeType::Ova,
            Some("ONA") => EpisodeType::Ona,
            _ => EpisodeType::Special,
        };
    }
    match season_episode_type {
        Some("alternate") => EpisodeType::Alternate,
        _ => EpisodeType::Standard,
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

            let language = app.metadata_language().await;

            let batch = match app
                .services
                .titles
                .list_unhydrated(HYDRATION_MAX_BATCH, &language)
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
                    MediaFacet::Series | MediaFacet::Anime => series_titles.push(title),
                }
            }

            let mut had_failures = false;

            // ---- Single bulk fetch for all movies + series ----
            let movie_ids: Vec<i64> = movie_titles.iter().filter_map(extract_tvdb_id).collect();
            let series_ids: Vec<i64> = series_titles.iter().filter_map(extract_tvdb_id).collect();

            match app
                .services
                .metadata_gateway
                .get_metadata_bulk(&movie_ids, &series_ids, &language)
                .await
            {
                Ok(bulk) => {
                    for title in movie_titles {
                        let tvdb_id = extract_tvdb_id(&title);
                        if let Some(movie) = tvdb_id.and_then(|id| bulk.movies.get(&id)) {
                            let result = super::movie_to_hydration_result(movie.clone(), &language);
                            let hydrated = app.apply_hydration_result(title, result).await;
                            if hydrated.metadata_fetched_at.is_some() {
                                app.emit_hydration_completed(&hydrated).await;
                            }
                            sync_wanted_after_hydration(&app, &hydrated).await;
                        } else {
                            had_failures = true;
                        }
                    }
                    for title in series_titles {
                        let tvdb_id = extract_tvdb_id(&title);
                        if let Some(series) = tvdb_id.and_then(|id| bulk.series.get(&id)) {
                            let result =
                                super::series_to_hydration_result(series.clone(), &language);
                            let hydrated = app.apply_hydration_result(title, result).await;
                            if hydrated.metadata_fetched_at.is_some() {
                                app.emit_hydration_completed(&hydrated).await;
                            }
                            sync_wanted_after_hydration(&app, &hydrated).await;
                        } else {
                            had_failures = true;
                        }
                    }
                }
                Err(err) => {
                    warn!(error = %err, "hydration loop: bulk metadata fetch failed");
                    had_failures = true;
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
