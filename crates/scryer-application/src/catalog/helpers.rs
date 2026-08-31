use super::*;
use scryer_domain::{MovieEntity, SeriesMovieLink};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DownloadClientRoutingEntry {
    pub(crate) enabled: bool,
    pub(crate) category: Option<String>,
    pub(crate) recent_queue_priority: Option<String>,
    pub(crate) older_queue_priority: Option<String>,
    pub(crate) remove_completed: bool,
    pub(crate) remove_failed: bool,
    pub(crate) seeding_profile_id: Option<String>,
}

pub(crate) fn default_download_client_routing_entry() -> DownloadClientRoutingEntry {
    DownloadClientRoutingEntry {
        enabled: true,
        category: None,
        recent_queue_priority: None,
        older_queue_priority: None,
        remove_completed: true,
        remove_failed: true,
        seeding_profile_id: None,
    }
}

pub(crate) fn is_logical_specials_collection(collection: &Collection) -> bool {
    collection.collection_type == CollectionType::Specials
        || (collection.collection_type == CollectionType::Season
            && collection.collection_index == "0")
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

/// Parse a stored download-client routing JSON object into the typed entry.
///
/// Per-field default fallbacks (`removeCompleted` → `true`, `removeFailed` →
/// `true`) and legacy key aliases (`removeComplete`, `remove_completed`,
/// `recentPriority`, etc.) are transitional legacy-compat behavior. The
/// canonical write paths in `settings.rs` and the startup
/// `normalize_routing_settings` migration always emit fully-materialized
/// entries; once every install has been normalized, the fallbacks here can be
/// removed.
pub(crate) fn parse_download_client_routing_entry(
    config: &serde_json::Value,
) -> DownloadClientRoutingEntry {
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
            true,
        ),
        remove_failed: read_routing_bool(
            config
                .get("removeFailed")
                .or_else(|| config.get("remove_failed"))
                .or_else(|| config.get("removeFailure")),
            true,
        ),
        seeding_profile_id: read_routing_string(
            config
                .get("seedingProfileId")
                .or_else(|| config.get("seeding_profile_id")),
        ),
    }
}

pub(crate) fn parse_download_client_routing_map(
    raw_json: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str::<serde_json::Value>(raw_json)
        .ok()?
        .as_object()
        .cloned()
}

pub(crate) fn build_rematched_external_ids(
    title: &Title,
    replacement_identity_ids: &[ExternalId],
    rematch_replaced_external_id_sources: &[&str],
) -> Vec<ExternalId> {
    let mut next: Vec<ExternalId> = title
        .external_ids
        .iter()
        .filter(|eid| {
            !rematch_replaced_external_id_sources
                .iter()
                .any(|source| eid.source.eq_ignore_ascii_case(source))
        })
        .cloned()
        .collect();

    for external_id in replacement_identity_ids {
        let source = external_id.source.trim();
        let value = external_id.value.trim();
        if source.is_empty() || value.is_empty() {
            continue;
        }
        next.push(ExternalId {
            source: source.to_string(),
            value: value.to_string(),
        });
    }

    next
}

pub(crate) fn strip_derived_match_tags(
    tags: &[String],
    rematch_derived_tag_prefixes: &[&str],
) -> Vec<String> {
    tags.iter()
        .filter(|tag| {
            !rematch_derived_tag_prefixes
                .iter()
                .any(|prefix| tag.starts_with(prefix))
        })
        .cloned()
        .collect()
}

pub(crate) fn release_is_recent_for_queue_priority(
    baseline_date: Option<&str>,
    recent_queue_priority_window_days: i64,
) -> bool {
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
    (0..=recent_queue_priority_window_days).contains(&age_days)
}

pub(crate) fn movie_entity_from_anime_movie(movie: &AnimeMovie) -> MovieEntity {
    let now = Utc::now();
    MovieEntity {
        id: Id::new().0,
        title: movie.name.clone(),
        sort_title: non_empty_owned(movie.sort_title.as_str()),
        slug: non_empty_owned(movie.slug.as_str()),
        year: movie.year,
        overview: non_empty_owned(movie.overview.as_str()),
        poster_url: non_empty_owned(movie.poster_url.as_str()),
        background_url: None,
        language: non_empty_owned(movie.language.as_str()),
        runtime_minutes: (movie.runtime_minutes > 0).then_some(movie.runtime_minutes),
        content_status: non_empty_owned(movie.content_status.as_str()),
        studio: non_empty_owned(movie.studio.as_str()),
        digital_release_date: movie.digital_release_date.clone(),
        imdb_id: movie
            .movie_imdb_id
            .as_deref()
            .and_then(non_empty_owned)
            .or_else(|| non_empty_owned(movie.imdb_id.as_str())),
        tvdb_id: movie.movie_tvdb_id.map(|id| id.to_string()),
        tmdb_id: movie.movie_tmdb_id.map(|id| id.to_string()),
        mal_id: movie.movie_mal_id.map(|id| id.to_string()),
        anidb_id: movie.movie_anidb_id.map(|id| id.to_string()),
        ratings: None,
        credits: None,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn series_movie_link_from_anime_movie(
    title_id: &str,
    anime_movie: &AnimeMovie,
    movie_entity: MovieEntity,
    narrative_order: String,
    after_season: i32,
    linked_episode_id: Option<String>,
    monitored: bool,
) -> SeriesMovieLink {
    let now = Utc::now();
    SeriesMovieLink {
        id: Id::new().0,
        series_title_id: title_id.to_string(),
        movie: movie_entity,
        placement: non_empty_owned(anime_movie.placement.as_str()),
        narrative_order: Some(narrative_order),
        after_season: Some(after_season),
        before_season: None,
        linked_episode_id,
        association_confidence: non_empty_owned(anime_movie.association_confidence.as_str()),
        continuity_status: non_empty_owned(anime_movie.continuity_status.as_str()),
        movie_form: non_empty_owned(anime_movie.movie_form.as_str()),
        confidence: non_empty_owned(anime_movie.confidence.as_str()),
        signal_summary: non_empty_owned(anime_movie.signal_summary.as_str()),
        source: Some("anibridge".to_string()),
        monitoring_override: None,
        metadata_active: true,
        monitored,
        legacy_collection_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn non_empty_owned(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn anime_movie_identity_keys(movie: &AnimeMovie) -> Vec<String> {
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

pub(crate) fn anime_mapping_identity_keys(mapping: &AnimeMapping) -> Vec<String> {
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

pub(crate) fn anime_movie_after_season(
    movie: &AnimeMovie,
    season_last_aired: &std::collections::BTreeMap<i32, String>,
) -> i32 {
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

pub(crate) fn anime_movie_release_sort_key(movie: &AnimeMovie) -> (&str, &str) {
    (
        movie
            .digital_release_date
            .as_deref()
            .unwrap_or("9999-12-31"),
        movie.sort_title.as_str(),
    )
}

#[cfg(test)]
mod routing_tests {
    use super::parse_download_client_routing_entry;
    use serde_json::json;

    #[test]
    fn routing_entry_parses_legacy_and_new_queue_priority_fields() {
        let entry = parse_download_client_routing_entry(&json!({
            "enabled": true,
            "category": "series",
            "recentPriority": "high",
            "olderQueuePriority": "low",
            "removeCompleted": true,
            "remove_failed": true
        }));

        assert!(entry.enabled);
        assert_eq!(entry.category.as_deref(), Some("series"));
        assert_eq!(entry.recent_queue_priority.as_deref(), Some("high"));
        assert_eq!(entry.older_queue_priority.as_deref(), Some("low"));
        assert!(entry.remove_completed);
        assert!(entry.remove_failed);
    }

    #[test]
    fn routing_entry_defaults_remove_completed_and_failed_when_flags_are_missing() {
        let entry = parse_download_client_routing_entry(&json!({
            "enabled": true,
            "category": "series"
        }));

        assert!(entry.enabled);
        assert_eq!(entry.category.as_deref(), Some("series"));
        assert!(entry.remove_completed);
        assert!(entry.remove_failed);
        assert_eq!(entry.seeding_profile_id, None);
    }

    #[test]
    fn routing_entry_parses_seeding_profile_id_from_either_key_shape() {
        let camel = parse_download_client_routing_entry(&json!({
            "enabled": true,
            "seedingProfileId": "  profile-1  "
        }));
        assert_eq!(camel.seeding_profile_id.as_deref(), Some("profile-1"));

        let snake = parse_download_client_routing_entry(&json!({
            "enabled": true,
            "seeding_profile_id": "profile-2"
        }));
        assert_eq!(snake.seeding_profile_id.as_deref(), Some("profile-2"));

        let cleared = parse_download_client_routing_entry(&json!({
            "enabled": true,
            "seedingProfileId": null
        }));
        assert_eq!(cleared.seeding_profile_id, None);

        let blank = parse_download_client_routing_entry(&json!({
            "enabled": true,
            "seedingProfileId": "   "
        }));
        assert_eq!(blank.seeding_profile_id, None);
    }
}

#[cfg(test)]
mod anime_movie_mapping_tests {
    use super::{movie_entity_from_anime_movie, series_movie_link_from_anime_movie};
    use crate::AnimeMovie;

    #[test]
    fn series_movies_split_metadata_from_timeline_classification() {
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
            studio: "Studio".into(),
            digital_release_date: Some("2024-02-01".into()),
            association_confidence: "high".into(),
            continuity_status: "canon".into(),
            movie_form: "movie".into(),
            placement: "ordered".into(),
            confidence: "high".into(),
            signal_summary: "TVDB marked special as critical to story".into(),
        };

        let entity = movie_entity_from_anime_movie(&movie);
        assert_eq!(entity.tvdb_id.as_deref(), Some("200"));
        assert_eq!(entity.tmdb_id.as_deref(), Some("300"));
        assert_eq!(entity.mal_id.as_deref(), Some("400"));
        assert_eq!(entity.imdb_id.as_deref(), Some("tt123"));

        let link = series_movie_link_from_anime_movie(
            "title-1",
            &movie,
            entity,
            "1.5".to_string(),
            1,
            Some("episode-1".to_string()),
            false,
        );
        assert_eq!(link.series_title_id, "title-1");
        assert_eq!(link.linked_episode_id.as_deref(), Some("episode-1"));
        assert_eq!(link.continuity_status.as_deref(), Some("canon"));
        assert_eq!(link.association_confidence.as_deref(), Some("high"));
        assert_eq!(link.confidence.as_deref(), Some("high"));
        assert_eq!(link.placement.as_deref(), Some("ordered"));
        assert_eq!(link.narrative_order.as_deref(), Some("1.5"));
    }
}
