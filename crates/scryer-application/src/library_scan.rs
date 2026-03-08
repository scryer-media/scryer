use async_trait::async_trait;

use crate::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct LibraryFile {
    pub path: String,
    pub display_name: String,
    /// Absolute path to the companion `.nfo` sidecar file, if one was found
    /// alongside this video file during scanning.
    pub nfo_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LibraryScanSummary {
    pub scanned: usize,
    pub matched: usize,
    pub imported: usize,
    pub skipped: usize,
    pub unmatched: usize,
}

#[derive(Debug, Clone)]
pub struct MetadataSearchItem {
    pub tvdb_id: String,
    pub name: String,
    pub year: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct RichMetadataSearchItem {
    pub tvdb_id: String,
    pub name: String,
    pub imdb_id: Option<String>,
    pub slug: Option<String>,
    pub type_hint: Option<String>,
    pub year: Option<i32>,
    pub status: Option<String>,
    pub overview: Option<String>,
    pub popularity: Option<f64>,
    pub poster_url: Option<String>,
    pub language: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub sort_title: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MultiMetadataSearchResult {
    pub movies: Vec<RichMetadataSearchItem>,
    pub series: Vec<RichMetadataSearchItem>,
    pub anime: Vec<RichMetadataSearchItem>,
}

#[derive(Debug, Clone)]
pub struct MovieMetadata {
    pub tvdb_id: i64,
    pub name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub content_status: String,
    pub overview: String,
    pub poster_url: String,
    pub language: String,
    pub runtime_minutes: i32,
    pub sort_title: String,
    pub imdb_id: String,
    pub genres: Vec<String>,
    pub studio: String,
    pub tmdb_release_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SeriesMetadata {
    pub tvdb_id: i64,
    pub name: String,
    pub sort_name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub content_status: String,
    pub first_aired: String,
    pub overview: String,
    pub network: String,
    pub runtime_minutes: i32,
    pub poster_url: String,
    pub country: String,
    pub genres: Vec<String>,
    pub aliases: Vec<String>,
    pub seasons: Vec<SeasonMetadata>,
    pub episodes: Vec<EpisodeMetadata>,
    pub anime_mappings: Vec<AnimeMapping>,
}

#[derive(Debug, Clone)]
pub struct AnimeMapping {
    pub mal_id: Option<i64>,
    pub anilist_id: Option<i64>,
    pub anidb_id: Option<i64>,
    pub kitsu_id: Option<i64>,
    pub thetvdb_season: Option<i32>,
    pub score: Option<f64>,
    pub anime_media_type: String,
    pub global_media_type: String,
    pub status: String,
    pub episode_mappings: Vec<AnimeEpisodeMapping>,
}

#[derive(Debug, Clone)]
pub struct AnimeEpisodeMapping {
    pub tvdb_season: i32,
    pub episode_start: i32,
    pub episode_end: i32,
}

#[derive(Debug, Clone)]
pub struct SeasonMetadata {
    pub tvdb_id: i64,
    pub number: i32,
    pub label: String,
    pub episode_type: String,
}

#[derive(Debug, Clone)]
pub struct EpisodeMetadata {
    pub tvdb_id: i64,
    pub episode_number: i32,
    pub name: String,
    pub aired: String,
    pub runtime_minutes: i32,
    pub is_filler: bool,
    pub is_recap: bool,
    pub overview: String,
    pub absolute_number: String,
    pub season_number: i32,
}

#[async_trait]
pub trait MetadataGateway: Send + Sync {
    async fn search_tvdb(
        &self,
        query: &str,
        type_hint: &str,
    ) -> AppResult<Vec<MetadataSearchItem>>;

    async fn search_tvdb_rich(
        &self,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<Vec<RichMetadataSearchItem>>;

    async fn search_tvdb_multi(
        &self,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult>;

    async fn get_movie(&self, tvdb_id: i64, language: &str) -> AppResult<MovieMetadata>;

    async fn get_series(&self, tvdb_id: i64, language: &str) -> AppResult<SeriesMetadata>;

    /// Fetch metadata for multiple movies in a single round-trip.
    /// Returns a map from tvdb_id → metadata. IDs that fail to resolve are omitted.
    async fn get_movies_bulk(
        &self,
        tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<std::collections::HashMap<i64, MovieMetadata>>;

    /// Fetch metadata for multiple series in a single round-trip.
    async fn get_series_bulk(
        &self,
        tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<std::collections::HashMap<i64, SeriesMetadata>>;
}

#[async_trait]
pub trait LibraryScanner: Send + Sync {
    async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>>;
}

#[derive(Default)]
pub struct NullLibraryScanner;

#[async_trait]
impl LibraryScanner for NullLibraryScanner {
    async fn scan_library(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
        Err(AppError::Repository(
            "library scanner is not configured".into(),
        ))
    }
}

#[derive(Default)]
pub struct NullMetadataGateway;

#[async_trait]
impl MetadataGateway for NullMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn get_movies_bulk(
        &self,
        _tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<std::collections::HashMap<i64, MovieMetadata>> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn get_series_bulk(
        &self,
        _tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<std::collections::HashMap<i64, SeriesMetadata>> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }
}
