use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{Collection, MediaFacet, Title};

use crate::{
    ActivityKind, AnimeMapping, AnimeMovie, AppResult, EpisodeMetadata, MetadataGateway,
    MovieMetadata, RenameCollisionPolicy, RenameMissingMetadataPolicy, RenamePlanItem,
    SeasonMetadata, SeriesMetadata, TitleMetadataUpdate,
};

/// Result of hydrating a title's metadata from a metadata gateway.
/// Movies return empty seasons/episodes. Series include full season/episode data.
pub struct HydrationResult {
    pub metadata_update: TitleMetadataUpdate,
    pub seasons: Vec<SeasonMetadata>,
    pub episodes: Vec<EpisodeMetadata>,
    pub anime_mappings: Vec<AnimeMapping>,
    pub anime_movies: Vec<AnimeMovie>,
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Build a [`HydrationResult`] from an already-fetched [`MovieMetadata`].
///
/// Shared by the single-title facet handler path and the bulk hydration loop.
pub fn movie_to_hydration_result(movie: MovieMetadata, language: &str) -> HydrationResult {
    let update = TitleMetadataUpdate {
        year: movie.year.filter(|&y| y > 0),
        overview: non_empty(movie.overview),
        poster_url: non_empty(movie.poster_url),
        banner_url: movie.banner_url.and_then(non_empty),
        background_url: movie.background_url.and_then(non_empty),
        sort_title: non_empty(movie.sort_title),
        slug: non_empty(movie.slug),
        imdb_id: non_empty(movie.imdb_id),
        runtime_minutes: if movie.runtime_minutes > 0 {
            Some(movie.runtime_minutes)
        } else {
            None
        },
        genres: movie.genres,
        content_status: non_empty(movie.content_status),
        language: non_empty(movie.language),
        first_aired: None,
        network: None,
        studio: non_empty(movie.studio),
        country: None,
        aliases: vec![],
        metadata_language: Some(language.to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        digital_release_date: movie.tmdb_release_date,
        ..Default::default()
    };
    HydrationResult {
        metadata_update: update,
        seasons: vec![],
        episodes: vec![],
        anime_mappings: vec![],
        anime_movies: vec![],
    }
}

/// Build a [`HydrationResult`] from an already-fetched [`SeriesMetadata`].
pub fn series_to_hydration_result(series: SeriesMetadata, language: &str) -> HydrationResult {
    let update = TitleMetadataUpdate {
        year: series.year.filter(|&y| y > 0),
        overview: non_empty(series.overview),
        poster_url: non_empty(series.poster_url),
        banner_url: series.banner_url.and_then(non_empty),
        background_url: series.background_url.and_then(non_empty),
        sort_title: non_empty(series.sort_name),
        slug: non_empty(series.slug),
        imdb_id: None,
        runtime_minutes: if series.runtime_minutes > 0 {
            Some(series.runtime_minutes)
        } else {
            None
        },
        genres: series.genres,
        content_status: non_empty(series.content_status),
        language: None,
        first_aired: non_empty(series.first_aired),
        network: non_empty(series.network),
        studio: None,
        country: non_empty(series.country),
        aliases: series.aliases,
        metadata_language: Some(language.to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        ..Default::default()
    };
    HydrationResult {
        metadata_update: update,
        seasons: series.seasons,
        episodes: series.episodes,
        anime_mappings: series.anime_mappings,
        anime_movies: series.anime_movies,
    }
}

/// Configuration and strategies for a specific media facet.
/// Each facet (movie, tv, anime) implements this trait to define
/// its metadata hydration, rename strategy, import routing, and
/// acquisition behavior.
#[async_trait]
pub trait FacetHandler: Send + Sync {
    /// The domain enum variant this handler covers.
    fn facet(&self) -> MediaFacet;

    /// String ID used in settings keys, database columns, audit logs.
    /// e.g. "movie", "tv", "anime"
    fn facet_id(&self) -> &str;

    /// Download client category string.
    fn download_category(&self) -> &str;

    /// Settings key for the library root path (e.g. "movies.path").
    fn library_path_key(&self) -> &str;

    /// Settings key for the root folders JSON array (e.g. "movies.root_folders").
    fn root_folders_key(&self) -> &str;

    /// Settings key for the global rename template (e.g. "rename.template.movie.global").
    fn rename_template_key(&self) -> &str;

    /// Settings key for the global collision policy (e.g. "rename.collision_policy.movie.global").
    fn collision_policy_key(&self) -> &str;

    /// Settings key for the global missing metadata policy.
    fn missing_metadata_policy_key(&self) -> &str;

    /// Default rename template when no setting is configured.
    fn default_rename_template(&self) -> &str;

    /// Default library root path.
    fn default_library_path(&self) -> &str;

    /// Whether this facet has episode-level structure.
    fn has_episodes(&self) -> bool;

    /// Optional activity kind emitted when a title of this facet is added.
    fn title_added_activity_kind(&self) -> Option<ActivityKind>;

    /// Indexer search category (e.g. "movie", "series", "anime").
    fn search_category(&self) -> &str;

    /// Scope ID used for scoped rename settings lookups (e.g. "movie", "series").
    fn rename_scope_id(&self) -> &str;

    /// Hydrate a title's metadata by calling the metadata gateway.
    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult>;

    /// Build a rename plan item for a single title+collection.
    fn build_rename_plan_item(
        &self,
        title: &Title,
        collection: &Collection,
        template: &str,
        collision_policy: &RenameCollisionPolicy,
        missing_metadata_policy: &RenameMissingMetadataPolicy,
        planned_targets: &mut HashSet<String>,
    ) -> RenamePlanItem;
}
