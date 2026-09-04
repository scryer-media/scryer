use super::{Date, ExternalIdPayload, MediaServerPlaybackLinkPayload};
use async_graphql::{Enum, ID, InputObject, SimpleObject};

// ── Metadata Gateway (proxied from SMG) ────────────────────────────────────

#[derive(InputObject, Clone)]
/// Metadata gateway movie lookup by one supported identity and language.
pub struct MetadataMovieInput {
    /// TVDB movie ID, when known.
    pub tvdb_id: Option<String>,
    /// SMG canonical movie title ID, when known.
    pub smg_id: Option<i64>,
    /// TMDB movie ID, when known.
    pub tmdb_id: Option<i64>,
    /// IMDb movie ID, when known.
    pub imdb_id: Option<String>,
    /// Optional metadata language code.
    pub language: Option<String>,
}

#[derive(InputObject, Clone)]
/// Metadata gateway series lookup by provider ID and language.
pub struct MetadataSeriesInput {
    /// TVDB series ID.
    pub tvdb_id: String,
    /// Whether episode metadata should be included; omitted uses the service default.
    pub include_episodes: Option<bool>,
    /// Optional metadata language code.
    pub language: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Search result from the metadata gateway with nullable provider metadata.
pub struct MetadataSearchItemPayload {
    /// TVDB provider ID.
    pub tvdb_id: String,
    /// SMG canonical title ID, or null for legacy-only results.
    pub smg_id: Option<i64>,
    /// TMDB provider ID, or null when unavailable.
    pub tmdb_id: Option<i64>,
    /// Primary metadata-provider source, or null for legacy-only results.
    pub primary_source: Option<String>,
    /// Every external identity supplied by the metadata provider.
    pub external_ids: Vec<ExternalIdPayload>,
    /// Metadata title.
    pub name: String,
    /// IMDb ID, or null when unavailable.
    pub imdb_id: Option<String>,
    /// Provider slug, or null when unavailable.
    pub slug: Option<String>,
    #[graphql(name = "type")]
    /// Provider content-type hint, or null when unavailable.
    pub type_hint: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Provider status, or null when unavailable.
    pub status: Option<String>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Popularity score, or null when unavailable.
    pub popularity: Option<f64>,
    /// Poster URL, or null when unavailable.
    pub poster_url: Option<String>,
    /// Metadata language, or null when unavailable.
    pub language: Option<String>,
    /// Runtime in minutes, or null when unavailable.
    pub runtime_minutes: Option<i32>,
    /// Normalized sort title, or null when unavailable.
    pub sort_title: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Metadata search results grouped by content facet.
pub struct MetadataSearchMultiPayload {
    /// Movie results; empty when no movies matched.
    pub movies: Vec<MetadataSearchItemPayload>,
    /// Series results; empty when no series matched.
    pub series: Vec<MetadataSearchItemPayload>,
    /// Anime results; empty when no anime matched.
    pub anime: Vec<MetadataSearchItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Full metadata gateway movie record.
pub struct MetadataMoviePayload {
    /// TVDB movie ID.
    pub tvdb_id: String,
    /// SMG canonical movie title ID, or null when unavailable.
    pub smg_id: Option<i64>,
    /// TMDB movie ID, or null when unavailable.
    pub tmdb_id: Option<i64>,
    /// Movie title.
    pub name: String,
    /// Provider slug.
    pub slug: String,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Provider status.
    pub status: String,
    /// Overview text.
    pub overview: String,
    /// Metadata-provider URL for the poster image.
    pub poster_url: String,
    /// Metadata language code.
    pub language: String,
    /// Runtime in minutes.
    pub runtime_minutes: i32,
    /// Normalized sort title.
    pub sort_title: String,
    /// IMDb title identifier.
    pub imdb_id: String,
    /// Studio name.
    pub studio: String,
    /// TMDB release date, or null when unavailable.
    pub tmdb_release_date: Option<Date>,
}

#[derive(SimpleObject, Clone)]
/// Full metadata gateway series record with seasons and optional episodes.
pub struct MetadataSeriesPayload {
    /// TVDB series ID.
    pub tvdb_id: String,
    /// Series title.
    pub name: String,
    /// Normalized sort name.
    pub sort_name: String,
    /// Provider slug.
    pub slug: String,
    /// First release year, or null when unavailable.
    pub year: Option<i32>,
    /// Provider status.
    pub status: String,
    /// First-air date.
    pub first_aired: Date,
    /// Overview text.
    pub overview: String,
    /// Network name.
    pub network: String,
    /// Runtime in minutes.
    pub runtime_minutes: i32,
    /// Metadata-provider URL for the poster image.
    pub poster_url: String,
    /// Country code.
    pub country: String,
    /// Alternate titles.
    pub aliases: Vec<String>,
    /// Season metadata.
    pub seasons: Vec<MetadataSeasonPayload>,
    /// Episode metadata; empty when not requested or unavailable.
    pub episodes: Vec<MetadataEpisodePayload>,
    /// Companion movies reported for an anime series; empty for other series.
    pub anime_movies: Vec<MetadataAnimeMoviePayload>,
}

#[derive(SimpleObject, Clone)]
/// Metadata gateway companion-movie record for an anime series.
pub struct MetadataAnimeMoviePayload {
    /// Movie name.
    pub name: String,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// How confidently the provider associates the movie with the series.
    pub association_confidence: String,
    /// Whether the movie is canon to the series.
    pub continuity_status: String,
    /// Where the movie sits relative to the series seasons.
    pub placement: String,
    /// Provider identifiers for the movie.
    pub external_ids: Vec<ExternalIdPayload>,
}

#[derive(SimpleObject, Clone)]
/// Metadata gateway season record.
pub struct MetadataSeasonPayload {
    /// TVDB season ID.
    pub tvdb_id: String,
    /// Numeric season number assigned by the metadata provider.
    pub number: i32,
    /// Season label.
    pub label: String,
    /// Episode classification.
    pub episode_type: String,
}

#[derive(SimpleObject, Clone)]
/// Metadata gateway episode record.
pub struct MetadataEpisodePayload {
    /// TVDB episode ID.
    pub tvdb_id: String,
    /// Episode number within the season.
    pub episode_number: i32,
    /// Numeric season containing this episode.
    pub season_number: i32,
    /// Episode title.
    pub name: String,
    /// Original air date.
    pub aired: Date,
    /// Runtime in minutes.
    pub runtime_minutes: i32,
    /// Whether the episode is marked filler.
    pub is_filler: bool,
    /// Episode image URL.
    pub image_url: String,
}

#[derive(SimpleObject, Clone)]
/// Availability summary for an episode's primary media.
pub struct EpisodeMediaAvailabilityPayload {
    /// Current availability or scan state.
    pub state: EpisodeMediaAvailabilityStateValue,
    /// Quality label of the primary file, or null before a file is available.
    pub primary_quality_label: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// States used to describe an episode's media availability.
pub enum EpisodeMediaAvailabilityStateValue {
    /// A playable primary media file is available.
    Available,
    /// A library scan has not yet completed for the episode.
    PendingScan,
    /// The latest media scan failed.
    ScanFailed,
    /// No media file currently satisfies the episode requirements.
    Missing,
    /// The episode is not monitored and is excluded from acquisition.
    Unmonitored,
}

#[derive(SimpleObject, Clone)]
/// Calendar episode with title, library, monitoring, and air-date context.
pub struct CalendarEpisodePayload {
    /// Episode ID.
    pub id: ID,
    /// Parent title ID.
    pub title_id: ID,
    /// ID of the library containing the parent title.
    pub library_id: ID,
    /// Library name, or null when unavailable.
    pub library_name: Option<String>,
    /// Library slug, or null when unavailable.
    pub library_slug: Option<String>,
    /// Display name of the parent title.
    pub title_name: String,
    /// Title slug, or null when unavailable.
    pub title_slug: Option<String>,
    /// Title facet string.
    pub title_facet: String,
    /// Season number text, or null when unavailable.
    pub season_number: Option<String>,
    /// Episode number text, or null when unavailable.
    pub episode_number: Option<String>,
    /// Episode title, or null when unavailable.
    pub episode_title: Option<String>,
    /// Movie or episode overview, or null when unavailable.
    pub overview: Option<String>,
    /// Proxied movie poster or episode-still URL.
    pub image_url: Option<String>,
    /// Air date, or null when unavailable.
    pub air_date: Option<Date>,
    /// Whether both the episode and its parent title are monitored.
    pub monitored: bool,
    /// Compact availability derived from the episode's primary media file.
    pub media_availability: EpisodeMediaAvailabilityPayload,
    /// Provider-native playback links available to the current user.
    pub playback_links: Vec<MediaServerPlaybackLinkPayload>,
}
