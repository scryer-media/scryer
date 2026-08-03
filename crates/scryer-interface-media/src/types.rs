use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};

pub use crate::conversions::{FromApplication, IntoApplication};
pub use scryer_interface_media_types::*;

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct TitlePayload {
    pub id: ID,
    pub library_id: ID,
    pub name: String,
    pub facet: MediaFacetValue,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub external_ids: Vec<ExternalIdPayload>,
    pub created_at: DateTime<Utc>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub poster_source_url: Option<String>,
    pub background_url: Option<String>,
    pub background_source_url: Option<String>,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub imdb_id: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub popularity: Option<f64>,
    pub canonical_tags: Vec<CanonicalMediaTagPayload>,
    pub content_status: Option<String>,
    pub language: Option<String>,
    pub first_aired: Option<Date>,
    pub network: Option<String>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub aliases: Vec<String>,
    pub metadata_language: Option<String>,
    pub metadata_fetched_at: Option<DateTime<Utc>>,
    pub quality_profile_id: Option<ID>,
    pub root_folder_id: ID,
    pub monitor_type: Option<MonitorTypeValue>,
    pub use_season_folders: Option<bool>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub filler_policy: Option<FillerPolicyValue>,
    pub recap_policy: Option<RecapPolicyValue>,
}

#[derive(SimpleObject, Clone)]
pub struct TitleExternalRatingPayload {
    pub source: String,
    pub value: Option<f64>,
    pub score: Option<f64>,
    pub normalized: f64,
    pub votes: Option<i32>,
    pub url: String,
}

#[derive(SimpleObject, Clone)]
pub struct TitleRatingPayload {
    pub rating: Option<f64>,
    pub rating_sources: Vec<String>,
    pub external_ratings: Vec<TitleExternalRatingPayload>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TitleCatalogSortKeyValue {
    Title,
    Library,
    Monitored,
    Quality,
    Episodes,
    Status,
    Size,
    Added,
    Year,
    Runtime,
    Root,
    Popularity,
    MediaResolution,
    MediaHdr,
    MediaAudioCodec,
    RatingScryer,
    RatingImdb,
    RatingRottenTomatoes,
    RatingPopcornmeter,
    RatingMetacritic,
    RatingMetacriticUser,
    RatingLetterboxd,
    RatingTmdb,
    RatingTvdb,
    RatingTrakt,
    RatingMyanimelist,
    RatingAnilist,
    RatingAnidb,
    RatingMdblist,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TitleCatalogContentStatusValue {
    Continuing,
    Ended,
}

#[derive(InputObject, Clone)]
pub struct TitleCatalogSortInput {
    pub key: TitleCatalogSortKeyValue,
    pub direction: Option<SortDirectionValue>,
}

#[derive(InputObject, Clone, Default)]
pub struct TitleCatalogFilterInput {
    pub monitored: Option<bool>,
    pub content_statuses: Option<Vec<TitleCatalogContentStatusValue>>,
    pub root_folder_ids: Option<Vec<ID>>,
    pub genre_tag_keys: Option<Vec<String>>,
    pub theme_tag_keys: Option<Vec<String>>,
    pub minimum_year: Option<i32>,
    pub maximum_year: Option<i32>,
    pub minimum_rating: Option<f64>,
}

#[derive(SimpleObject, Clone)]
pub struct TitleCatalogFilterOptionsPayload {
    pub genres: Vec<CanonicalTagFilterOptionPayload>,
    pub themes: Vec<CanonicalTagFilterOptionPayload>,
    pub minimum_year: Option<i32>,
    pub maximum_year: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct TitleCatalogPayload {
    pub items: Vec<TitlePayload>,
    pub has_more: bool,
    pub total_count: i32,
    pub filter_counts: TitleCatalogFilterCountsPayload,
    pub managed_bytes: Long,
}

#[derive(SimpleObject, Clone)]
pub struct TitleCatalogFilterCountsPayload {
    pub all: i32,
    pub monitored: i32,
    pub unmonitored: i32,
    pub continuing: i32,
    pub ended: i32,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct CollectionPayload {
    pub id: ID,
    pub title_id: ID,
    pub collection_type: CollectionTypeValue,
    pub collection_index: String,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub narrative_order: Option<String>,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
    pub monitored: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct SetCollectionMonitoredPayload {
    pub id: ID,
    pub monitored: bool,
    pub episodes: Vec<EpisodePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct MovieEntityPayload {
    pub id: ID,
    pub title: String,
    pub slug: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub content_status: Option<String>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub mal_id: Option<String>,
    pub anidb_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct SeriesMovieLinkPayload {
    pub id: ID,
    pub movie: MovieEntityPayload,
    pub narrative_order: Option<String>,
    pub after_season: Option<i32>,
    pub before_season: Option<i32>,
    pub linked_episode_id: Option<ID>,
    pub continuity_status: Option<String>,
    pub movie_form: Option<String>,
    pub signal_summary: Option<String>,
    pub monitored: bool,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct EpisodePayload {
    pub id: ID,
    pub title_id: ID,
    pub collection_id: Option<ID>,
    pub episode_type: EpisodeTypeValue,
    pub episode_number: Option<String>,
    pub season_number: Option<String>,
    pub episode_label: Option<String>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<Date>,
    pub duration_seconds: Option<i64>,
    pub has_multi_audio: bool,
    pub has_subtitle: bool,
    pub is_filler: bool,
    pub is_recap: bool,
    pub absolute_number: Option<String>,
    pub image_url: Option<String>,
    pub monitored: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct TitleMediaFilePayload {
    pub id: ID,
    pub title_id: ID,
    pub episode_id: Option<ID>,
    pub series_movie_link_ids: Vec<ID>,
    pub file_path: String,
    pub size_bytes: Long,
    pub role: String,
    pub quality_label: Option<String>,
    pub scan_status: String,
    pub created_at: DateTime<Utc>,
    // Media analysis (populated after media scan; null until scan_status = "scanned")
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub video_frame_rate: Option<String>,
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    pub audio_languages: Vec<String>,
    pub audio_streams: Vec<AudioStreamDetailPayload>,
    pub subtitle_languages: Vec<String>,
    pub subtitle_codecs: Vec<String>,
    pub subtitle_streams: Vec<SubtitleStreamDetailPayload>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub num_chapters: Option<i32>,
    pub container_format: Option<String>,
    // Rich metadata (populated at import from parsed release name)
    pub scene_name: Option<String>,
    pub release_group: Option<String>,
    pub source_type: Option<String>,
    pub resolution: Option<String>,
    pub video_codec_parsed: Option<String>,
    pub audio_codec_parsed: Option<String>,
    pub acquisition_score: Option<i32>,
    pub scoring_log: Option<String>,
    pub indexer_source: Option<String>,
    pub grabbed_release_title: Option<String>,
    pub grabbed_at: Option<DateTime<Utc>>,
    pub edition: Option<String>,
    pub original_file_path: Option<String>,
    pub release_hash: Option<String>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct LibraryPayload {
    pub id: ID,
    pub facet: MediaFacetValue,
    pub name: String,
    pub slug: String,
    pub is_default: bool,
    pub is_bootstrap_default_root_set: bool,
    pub roots: Vec<LibraryRootPayload>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct DownloadQueueItemPayload {
    pub id: ID,
    pub title_id: Option<ID>,
    pub episode_id: Option<ID>,
    pub title_name: String,
    pub facet: Option<MediaFacetValue>,
    pub is_scryer_origin: bool,
    pub source_provider: Option<String>,
    pub client_id: ID,
    pub client_name: String,
    pub client_type: String,
    pub state: DownloadQueueStateValue,
    pub display_state: DownloadDisplayStateValue,
    pub progress_percent: i32,
    pub import_transfer_phase: Option<ImportTransferPhaseValue>,
    pub import_transfer_bytes: Option<Long>,
    pub import_transfer_total_bytes: Option<Long>,
    pub import_transfer_started_at: Option<DateTime<Utc>>,
    pub import_transfer_updated_at: Option<DateTime<Utc>>,
    pub size_bytes: Option<Long>,
    pub remaining_seconds: Option<i32>,
    pub queued_at: Option<DateTime<Utc>>,
    pub last_updated_at: Option<DateTime<Utc>>,
    pub attention_required: bool,
    pub attention_reason: Option<String>,
    pub download_client_item_id: String,
    pub download_id: Option<String>,
    pub import_status: Option<ImportStatusValue>,
    pub import_error_code: Option<ImportErrorCodeValue>,
    pub import_error_message: Option<String>,
    pub imported_at: Option<DateTime<Utc>>,
    pub delete_status: Option<DownloadQueueDeleteStatusValue>,
    pub delete_error_message: Option<String>,
    pub tracked_state: Option<TrackedDownloadStateValue>,
    pub tracked_status: Option<TrackedDownloadStatusValue>,
    pub tracked_status_messages: Vec<String>,
    pub tracked_match_type: Option<TitleMatchTypeValue>,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadHistoryPagePayload {
    pub items: Vec<DownloadQueueItemPayload>,
    pub has_more: bool,
    pub total_count: i32,
    pub available_clients: Vec<DownloadClientFilterOptionPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadImportPagePayload {
    pub items: Vec<DownloadQueueItemPayload>,
    pub has_more: bool,
    pub total_count: i32,
}

#[derive(SimpleObject, Clone)]
pub struct AddTitleResult {
    pub title: TitlePayload,
    pub metadata_hydration_state: AddTitleHydrationStateValue,
    pub reused_existing_title: bool,
    pub reused_queued_download: bool,
    pub download_job_id: Option<ID>,
    pub queued_download: Option<QueueDownloadPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct FixTitleMatchPayload {
    pub title: TitlePayload,
    pub hydrated: bool,
    pub library_scan: Option<LibraryScanSummaryPayload>,
    pub warnings: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct PendingImportBindingPreviewPayload {
    pub title: TitlePayload,
    pub file: PendingImportBindingFilePreviewPayload,
    pub available_episodes: Vec<EpisodePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ResolvePendingImportPayload {
    pub title: TitlePayload,
    pub created: bool,
    pub library_scan: Option<LibraryScanSummaryPayload>,
    pub metadata_hydration_state: AddTitleHydrationStateValue,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadQueueActionPayload {
    pub kind: DownloadQueueActionKindValue,
    pub download_client_item_id: String,
    pub client_id: Option<ID>,
    pub client_type: Option<String>,
    pub import_id: Option<ID>,
    pub command_id: Option<ID>,
    pub removed: bool,
    pub queue_item: Option<DownloadQueueItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ManualImportPreviewPayload {
    pub files: Vec<ManualImportFilePreviewPayload>,
    pub available_episodes: Vec<EpisodePayload>,
    pub available_series_movies: Vec<ManualImportSeriesMovieTargetPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ManualImportSelectionPayload {
    pub selection_id: ID,
    pub files: Vec<ManualImportFilePreviewPayload>,
    pub available_episodes: Vec<EpisodePayload>,
    pub available_series_movies: Vec<ManualImportSeriesMovieTargetPayload>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct WantedItemPayload {
    /// Scope identity: the acquisition-state row id when a state row
    /// exists for this scope, otherwise the convergence scope key itself
    /// (`episode:<uuid>`, `title:<uuid>`, `series_movie:<uuid>`, …). A derived
    /// target with no state row is still addressable by this id (pause/resume and
    /// the interactive search job both accept a scope key here).
    pub id: ID,
    pub title_id: ID,
    pub title_name: Option<String>,
    pub title_slug: Option<String>,
    pub title_facet: Option<String>,
    pub library_id: Option<ID>,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    pub episode_id: Option<ID>,
    pub collection_id: Option<ID>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub media_type: WantedMediaTypeValue,
    pub last_search_at: Option<DateTime<Utc>>,
    pub status: WantedStatusValue,
    pub grabbed_release: Option<String>,
    /// Safe indexer/provider label for the grabbed release, when known.
    pub source_provider: Option<String>,
    pub current_score: Option<i32>,
    pub latest_release_decision: Option<ReleaseDecisionPayload>,
    pub mismatch_recovery_eligible: bool,
    /// Convergence progress for this scope: whether the scope is
    /// still sweeping indexers, has converged (watching RSS), is queued, or is
    /// deferred because every uncovered indexer is currently unavailable.
    pub convergence_state: ConvergenceStateValue,
    /// Routed indexers already searched under the current fingerprint.
    pub indexers_covered: i32,
    /// Routed-enabled indexers for this scope.
    pub indexers_routed: i32,
    /// Recency lane: `Hot` converges promptly, `Cold` drains under
    /// scheduler backpressure.
    pub recency_lane: RecencyLaneValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct WantedItemsListPayload {
    pub items: Vec<WantedItemPayload>,
    pub total_count: i64,
    pub has_more: bool,
}

#[derive(SimpleObject, Clone)]
pub struct WantedItemsPagePayload {
    pub items: Vec<WantedItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct TitleAcquisitionDiagnosticsPayload {
    pub recent_decisions: Vec<ReleaseDecisionPayload>,
    pub decision_counts: Vec<DecisionCodeCountPayload>,
    pub wanted_status_counts: Vec<WantedStatusCountPayload>,
    pub pending_release_counts: Vec<PendingReleaseStatusCountPayload>,
    pub mismatch_recovery_eligible_count: i64,
    pub latest_decision_at: Option<DateTime<Utc>>,
    pub latest_wanted_search_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct ReleaseDecisionPayload {
    pub id: ID,
    pub wanted_item_id: ID,
    pub title_id: ID,
    pub release_title: String,
    pub release_url: Option<String>,
    pub release_size_bytes: Option<Long>,
    pub decision_code: String,
    pub candidate_score: i32,
    pub current_score: Option<i32>,
    pub score_delta: Option<i32>,
    pub explanation_json: Option<Json<serde_json::Value>>,
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct ReleaseDecisionsPagePayload {
    pub items: Vec<ReleaseDecisionPayload>,
    pub total_count: i64,
    pub has_more: bool,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct PendingReleasePayload {
    pub id: ID,
    pub wanted_item_id: ID,
    pub title_id: ID,
    pub release_title: String,
    pub release_url: Option<String>,
    pub release_size_bytes: Option<Long>,
    pub release_score: i32,
    pub scoring_log_json: Option<Json<serde_json::Value>>,
    pub indexer_source: Option<String>,
    pub added_at: DateTime<Utc>,
    pub delay_until: DateTime<Utc>,
    pub status: PendingReleaseStatusValue,
}

#[derive(InputObject, Clone)]
pub struct PendingReleaseFilterInput {
    pub title_id: Option<ID>,
    pub wanted_item_id: Option<ID>,
    pub statuses: Option<Vec<PendingReleaseStatusValue>>,
}

#[derive(SimpleObject, Clone)]
pub struct PendingReleasesPayload {
    pub items: Vec<PendingReleasePayload>,
    pub has_more: bool,
    pub total_count: i32,
}
