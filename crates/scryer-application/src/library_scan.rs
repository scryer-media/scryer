use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFile {
    pub path: String,
    pub display_name: String,
    /// Absolute path to the companion `.nfo` sidecar file, if one was found
    /// alongside this video file during scanning.
    pub nfo_path: Option<String>,
    pub size_bytes: Option<i64>,
    pub source_signature_scheme: Option<String>,
    pub source_signature_value: Option<String>,
}

pub type LibraryFileBatch = Vec<LibraryFile>;
pub type LibraryFileBatchReceiver = mpsc::Receiver<AppResult<LibraryFileBatch>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryDirectoryScanResult {
    pub files: Vec<LibraryFile>,
    pub walk_ms: u64,
    pub stat_ms: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    pub banner_url: Option<String>,
    pub background_url: Option<String>,
    pub language: String,
    pub runtime_minutes: i32,
    pub sort_title: String,
    pub imdb_id: String,
    pub anidb_id: Option<i64>,
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
    pub banner_url: Option<String>,
    pub background_url: Option<String>,
    pub country: String,
    pub genres: Vec<String>,
    pub aliases: Vec<String>,
    pub tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    pub seasons: Vec<SeasonMetadata>,
    pub episodes: Vec<EpisodeMetadata>,
    pub anime_mappings: Vec<AnimeMapping>,
    pub anime_movies: Vec<AnimeMovie>,
}

#[derive(Debug, Clone)]
pub struct AnimeMapping {
    pub mal_id: Option<i64>,
    pub anilist_id: Option<i64>,
    pub anidb_id: Option<i64>,
    pub kitsu_id: Option<i64>,
    pub thetvdb_id: Option<i64>,
    pub themoviedb_id: Option<i64>,
    pub alt_tvdb_id: Option<i64>,
    pub thetvdb_season: Option<i32>,
    pub score: Option<f64>,
    pub anime_media_type: String,
    pub global_media_type: String,
    pub status: String,
    pub mapping_type: String,
    pub episode_mappings: Vec<AnimeEpisodeMapping>,
}

#[derive(Debug, Clone)]
pub struct AnimeEpisodeMapping {
    pub tvdb_season: i32,
    pub episode_start: i32,
    pub episode_end: i32,
}

#[derive(Debug, Clone)]
pub struct AnimeMovie {
    pub movie_tvdb_id: Option<i64>,
    pub movie_tmdb_id: Option<i64>,
    pub movie_imdb_id: Option<String>,
    pub movie_mal_id: Option<i64>,
    pub movie_anidb_id: Option<i64>,
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
    pub digital_release_date: Option<String>,
    pub association_confidence: String,
    pub continuity_status: String,
    pub movie_form: String,
    pub placement: String,
    pub confidence: String,
    pub signal_summary: String,
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
    async fn search_tvdb(&self, query: &str, type_hint: &str)
    -> AppResult<Vec<MetadataSearchItem>>;

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

    /// Fetch metadata for movies and series in a single GraphQL round-trip.
    /// Returns resolved results; IDs that fail to resolve are omitted from the maps.
    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<BulkMetadataResult>;

    /// Resolve all anibridge source entries covering a specific TVDB episode.
    /// Returns entries like (anidb, 15449, R), (anilist, 116674, ), (mal, 41467, ).
    async fn anibridge_mappings_for_episode(
        &self,
        tvdb_id: i64,
        season: i32,
        episode: i32,
    ) -> AppResult<Vec<AnibridgeSourceMapping>>;
}

#[derive(Debug, Clone)]
pub struct AnibridgeSourceMapping {
    pub source_type: String,
    pub source_id: i64,
    pub source_scope: String,
}

#[derive(Debug, Clone, Default)]
pub struct BulkMetadataResult {
    pub movies: std::collections::HashMap<i64, MovieMetadata>,
    pub series: std::collections::HashMap<i64, SeriesMetadata>,
}

#[async_trait]
pub trait LibraryScanner: Send + Sync {
    async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>>;

    async fn scan_directory(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        self.scan_library(root).await
    }

    async fn scan_library_batched(
        &self,
        root: &str,
        batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver>;

    async fn scan_directory_batched(
        &self,
        root: &str,
        batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver>;

    async fn scan_directory_with_metrics(
        &self,
        root: &str,
    ) -> AppResult<LibraryDirectoryScanResult> {
        Ok(LibraryDirectoryScanResult {
            files: self.scan_directory(root).await?,
            ..Default::default()
        })
    }

    async fn scan_directory_for_progress_with_metrics(
        &self,
        root: &str,
    ) -> AppResult<LibraryDirectoryScanResult> {
        let mut result = self.scan_directory_with_metrics(root).await?;
        for file in &mut result.files {
            file.size_bytes = None;
            file.source_signature_scheme = None;
            file.source_signature_value = None;
        }
        Ok(result)
    }
}

pub fn source_signature_from_std_metadata(
    metadata: &std::fs::Metadata,
) -> Option<(String, String)> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        Some((
            "windows_last_write_100ns_v1".to_string(),
            metadata.last_write_time().to_string(),
        ))
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Some((
            "unix_mtime_nsec_v1".to_string(),
            format!("{}:{}", metadata.mtime(), metadata.mtime_nsec()),
        ))
    }

    #[cfg(not(any(unix, windows)))]
    {
        use std::time::UNIX_EPOCH;

        metadata
            .modified()
            .ok()
            .and_then(|modified| match modified.duration_since(UNIX_EPOCH) {
                Ok(duration) => Some((
                    "system_time_nsec_v1".to_string(),
                    format!("{}:{}", duration.as_secs(), duration.subsec_nanos()),
                )),
                Err(error) => {
                    let duration = error.duration();
                    Some((
                        "system_time_nsec_v1".to_string(),
                        format!("-{}:{}", duration.as_secs(), duration.subsec_nanos()),
                    ))
                }
            })
    }
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

    async fn scan_library_batched(
        &self,
        _root: &str,
        batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        if batch_size == 0 {
            return Err(AppError::Validation(
                "batch size must be greater than 0".into(),
            ));
        }

        Err(AppError::Repository(
            "library scanner is not configured".into(),
        ))
    }

    async fn scan_directory_batched(
        &self,
        _root: &str,
        batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        if batch_size == 0 {
            return Err(AppError::Validation(
                "batch size must be greater than 0".into(),
            ));
        }

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

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn anibridge_mappings_for_episode(
        &self,
        _tvdb_id: i64,
        _season: i32,
        _episode: i32,
    ) -> AppResult<Vec<AnibridgeSourceMapping>> {
        Ok(vec![])
    }
}
