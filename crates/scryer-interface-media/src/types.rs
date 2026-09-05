use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};

pub use crate::conversions::{FromApplication, IntoApplication};
pub use scryer_interface_media_types::*;

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// Catalog metadata, monitoring settings, and media identity for a title.
pub struct TitlePayload {
    /// Stable title identifier.
    pub id: ID,
    /// Identifier of the library containing this title.
    pub library_id: ID,
    /// Display title from the catalog metadata.
    pub name: String,
    /// Media facet that determines the title's domain, such as movie or series.
    pub facet: MediaFacetValue,
    /// Whether acquisition and monitoring are enabled for the title.
    pub monitored: bool,
    /// User-defined tags plus reserved `scryer:` settings entries.
    pub tags: Vec<String>,
    /// External catalog identifiers attached to the title.
    pub external_ids: Vec<ExternalIdPayload>,
    /// Time when the title record was created, in UTC.
    pub created_at: DateTime<Utc>,
    /// Release year, when catalog metadata supplies one.
    pub year: Option<i32>,
    /// Plot or synopsis text, or null before metadata provides it.
    pub overview: Option<String>,
    /// Proxied poster URL, or null when no poster source is available.
    pub poster_url: Option<String>,
    /// Poster source URL exposed alongside `poster_url`, or null when unavailable.
    pub poster_source_url: Option<String>,
    /// Proxied background URL, or null when no background source is available.
    pub background_url: Option<String>,
    /// Background source URL exposed alongside `background_url`, or null when unavailable.
    pub background_source_url: Option<String>,
    /// Normalized title used for ordering, or null when not computed.
    pub sort_title: Option<String>,
    /// Stable URL-friendly title key, or null when not assigned.
    pub slug: Option<String>,
    /// IMDb identifier, or null when the title has no IMDb match.
    pub imdb_id: Option<String>,
    /// Runtime in minutes, or null when unknown.
    pub runtime_minutes: Option<i32>,
    /// Catalog popularity score, or null when unavailable.
    pub popularity: Option<f64>,
    /// Canonical genre and theme tags resolved from source metadata.
    pub canonical_tags: Vec<CanonicalMediaTagPayload>,
    /// Catalog content status, or null before metadata provides it.
    pub content_status: Option<String>,
    /// Original content language code, or null when unknown.
    pub language: Option<String>,
    /// First air date as a calendar date, or null when unavailable.
    pub first_aired: Option<Date>,
    /// Broadcasting network, or null when unknown.
    pub network: Option<String>,
    /// Production studio, or null when unknown.
    pub studio: Option<String>,
    /// Production country, or null when unknown.
    pub country: Option<String>,
    /// Alternate title strings from catalog metadata.
    pub aliases: Vec<String>,
    /// Language used for the latest metadata fetch, or null when not recorded.
    pub metadata_language: Option<String>,
    /// Time of the latest metadata fetch in UTC, or null before a fetch completes.
    pub metadata_fetched_at: Option<DateTime<Utc>>,
    /// Identifier of the selected quality profile, or null when the title uses no profile.
    pub quality_profile_id: Option<ID>,
    /// Identifier of the root folder that stores this title's files.
    pub root_folder_id: ID,
    /// Monitoring mode, or null when no explicit mode is configured.
    pub monitor_type: Option<MonitorTypeValue>,
    /// Whether episodic files use season directories, or null when unset.
    pub use_season_folders: Option<bool>,
    /// Whether specials are monitored, or null when unset.
    pub monitor_specials: Option<bool>,
    /// Whether movies between seasons are included, or null when unset.
    pub inter_season_movies: Option<bool>,
    /// Policy for filler episodes, or null when unset.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Policy for recap episodes, or null when unset.
    pub recap_policy: Option<RecapPolicyValue>,
}

#[derive(SimpleObject, Clone)]
/// One external rating source and its normalized score.
pub struct TitleExternalRatingPayload {
    /// Rating provider name.
    pub source: String,
    /// Provider rating value before normalization, or null when absent.
    pub value: Option<f64>,
    /// Provider score on its native scale, or null when absent.
    pub score: Option<f64>,
    /// Score normalized to the shared comparison scale.
    pub normalized: f64,
    /// Number of votes reported by the provider, or null when absent.
    pub votes: Option<i32>,
    /// Provider page for this rating.
    pub url: String,
}

#[derive(SimpleObject, Clone)]
/// Combined title rating and the sources that contributed it.
pub struct TitleRatingPayload {
    /// Combined rating, or null when no source supplied one.
    pub rating: Option<f64>,
    /// Names of rating sources included in the combined value.
    pub rating_sources: Vec<String>,
    /// Per-provider ratings used to calculate or explain the combined value.
    pub external_ratings: Vec<TitleExternalRatingPayload>,
}

#[derive(SimpleObject, Clone)]
/// One cast or crew credit cached from the title's last metadata hydration.
pub struct TitleCreditPayload {
    /// Credit kind exactly as the metadata provider spelled it, e.g. `actor`,
    /// `voice_actor`, `director`, `writer`, `creator`, `composer`. Kept as a
    /// string so a new provider kind reaches clients without a schema break.
    pub kind: String,
    /// Person's display name in the hydration language.
    pub person_name: String,
    /// Person's name in their original language, or empty when the provider has none.
    pub person_original_name: String,
    /// Proxied portrait for this person, or null when the provider has no image.
    pub person_image_url: Option<String>,
    /// Character played, or empty for crew credits.
    pub character: String,
    /// Language tag this credit was hydrated in, or empty when the provider has none.
    pub language: String,
    /// Provider billing rank; lower sorts closer to top billing.
    pub billing_order: i32,
    /// Episodes this person appears in, or null for titles the provider does not count.
    pub episode_count: Option<i32>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Fields supported when sorting the title catalog.
pub enum TitleCatalogSortKeyValue {
    /// Sort by display title.
    Title,
    /// Sort by library name.
    Library,
    /// Sort by monitored status.
    Monitored,
    /// Sort by quality profile or media quality.
    Quality,
    /// Sort by episode count.
    Episodes,
    /// Sort by content status.
    Status,
    /// Sort by managed file size.
    Size,
    /// Sort by title creation time.
    Added,
    /// Sort by release year.
    Year,
    /// Sort by runtime in minutes.
    Runtime,
    /// Sort by root folder.
    Root,
    /// Sort by catalog popularity.
    Popularity,
    /// Sort by media resolution.
    MediaResolution,
    /// Sort by HDR format.
    MediaHdr,
    /// Sort by audio codec.
    MediaAudioCodec,
    /// Sort by the combined Scryer rating.
    RatingScryer,
    /// Sort by IMDb rating.
    RatingImdb,
    /// Sort by Rotten Tomatoes rating.
    RatingRottenTomatoes,
    /// Sort by Popcornmeter rating.
    RatingPopcornmeter,
    /// Sort by Metacritic critic rating.
    RatingMetacritic,
    /// Sort by Metacritic user rating.
    RatingMetacriticUser,
    /// Sort by Letterboxd rating.
    RatingLetterboxd,
    /// Sort by TMDB rating.
    RatingTmdb,
    /// Sort by TVDB rating.
    RatingTvdb,
    /// Sort by Trakt rating.
    RatingTrakt,
    /// Sort by MyAnimeList rating.
    RatingMyanimelist,
    /// Sort by AniList rating.
    RatingAnilist,
    /// Sort by AniDB rating.
    RatingAnidb,
    /// Sort by MDBList rating.
    RatingMdblist,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Broad content lifecycle states used by title catalog filters.
pub enum TitleCatalogContentStatusValue {
    /// Content is still being released.
    Continuing,
    /// Content has finished releasing.
    Ended,
}

#[derive(InputObject, Clone)]
/// Ordering applied to a title catalog page.
pub struct TitleCatalogSortInput {
    /// Field used for ordering.
    pub key: TitleCatalogSortKeyValue,
    /// Ascending or descending direction; omitted or null defaults to ascending.
    pub direction: Option<SortDirectionValue>,
}

#[derive(InputObject, Clone, Default)]
/// Optional predicates for narrowing a title catalog page.
pub struct TitleCatalogFilterInput {
    /// Restrict results to monitored or unmonitored titles; null leaves both included.
    pub monitored: Option<bool>,
    /// Restrict results to these content statuses; null leaves all statuses included.
    pub content_statuses: Option<Vec<TitleCatalogContentStatusValue>>,
    /// Restrict results to titles stored under these root folder IDs; null leaves all roots included.
    pub root_folder_ids: Option<Vec<ID>>,
    /// Restrict results to titles having one of these canonical genre tag keys; null leaves all genres included.
    pub genre_tag_keys: Option<Vec<String>>,
    /// Restrict results to titles having one of these canonical theme tag keys; null leaves all themes included.
    pub theme_tag_keys: Option<Vec<String>>,
    /// Restrict results to titles carrying at least one of these user-defined tag labels; null or empty leaves all titles included. Labels are normalized the way the tag registry stores them, and reserved `scryer:` entries are rejected.
    pub tags: Option<Vec<String>>,
    /// Inclusive lower bound for release year; null imposes no lower bound.
    pub minimum_year: Option<i32>,
    /// Inclusive upper bound for release year; null imposes no upper bound.
    pub maximum_year: Option<i32>,
    /// Minimum combined rating on the catalog rating scale; null imposes no rating bound.
    pub minimum_rating: Option<f64>,
}

#[derive(SimpleObject, Clone)]
/// Available canonical tag filters and the observed release-year range.
pub struct TitleCatalogFilterOptionsPayload {
    /// Genre tag options available to the current catalog scope.
    pub genres: Vec<CanonicalTagFilterOptionPayload>,
    /// Theme tag options available to the current catalog scope.
    pub themes: Vec<CanonicalTagFilterOptionPayload>,
    /// Lowest observed release year, or null when no title has a year.
    pub minimum_year: Option<i32>,
    /// Highest observed release year, or null when no title has a year.
    pub maximum_year: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// A page of catalog titles with counts and managed storage usage.
pub struct TitleCatalogPayload {
    /// Titles in the requested page, in the requested stable sort order.
    pub items: Vec<TitlePayload>,
    /// Whether another page exists after the current page.
    pub has_more: bool,
    /// Number of titles matching the filters across all pages.
    pub total_count: i32,
    /// Counts for the standard monitored and content-status facets.
    pub filter_counts: TitleCatalogFilterCountsPayload,
    /// Total managed media bytes represented by the matching catalog scope.
    pub managed_bytes: Long,
}

#[derive(SimpleObject, Clone)]
/// Counts for common title catalog filter facets.
pub struct TitleCatalogFilterCountsPayload {
    /// Number of matching titles before the facet is applied.
    pub all: i32,
    /// Number of matching monitored titles.
    pub monitored: i32,
    /// Number of matching unmonitored titles.
    pub unmonitored: i32,
    /// Number of matching continuing titles.
    pub continuing: i32,
    /// Number of matching ended titles.
    pub ended: i32,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// An ordered season, collection, or other grouped set of episodes for a title.
pub struct CollectionPayload {
    /// Stable collection identifier.
    pub id: ID,
    /// Identifier of the title owning this collection.
    pub title_id: ID,
    /// Collection kind, such as a season or special grouping.
    pub collection_type: CollectionTypeValue,
    /// Domain-specific collection index, preserved as text for non-numeric values.
    pub collection_index: String,
    /// Human-readable collection label, or null when unavailable.
    pub label: Option<String>,
    /// Ordered path used to place this collection among its parent collections, or null when absent.
    pub ordered_path: Option<String>,
    /// Narrative ordering value, or null when the source does not provide one.
    pub narrative_order: Option<String>,
    /// First episode number in the collection, or null when no episode number is known.
    pub first_episode_number: Option<String>,
    /// Last episode number in the collection, or null when no episode number is known.
    pub last_episode_number: Option<String>,
    /// Whether acquisition and monitoring are enabled for this collection.
    pub monitored: bool,
    /// Time when the collection record was created, in UTC.
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Result of changing a collection's monitored state.
pub struct SetCollectionMonitoredPayload {
    /// Identifier of the collection that was updated.
    pub id: ID,
    /// The resulting monitored state.
    pub monitored: bool,
    /// Episodes in the collection after the monitoring change.
    pub episodes: Vec<EpisodePayload>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// Persisted movie metadata.
pub struct MovieEntityPayload {
    #[graphql(skip)]
    pub permission_title_id: ID,
    /// Stable movie identifier.
    pub id: ID,
    /// Movie title.
    pub title: String,
    /// URL-friendly movie key, or null when unavailable.
    pub slug: Option<String>,
    /// Release year, or null when unknown.
    pub year: Option<i32>,
    /// Plot or synopsis text, or null before metadata provides it.
    pub overview: Option<String>,
    /// Poster URL, or null when no poster source is available.
    pub poster_url: Option<String>,
    /// Runtime in minutes, or null when unknown.
    pub runtime_minutes: Option<i32>,
    /// Catalog content status, or null when unavailable.
    pub content_status: Option<String>,
    /// IMDb identifier, or null when unmatched.
    pub imdb_id: Option<String>,
    /// TVDB identifier, or null when unmatched.
    pub tvdb_id: Option<String>,
    /// TMDB identifier, or null when unmatched.
    pub tmdb_id: Option<String>,
    /// MyAnimeList identifier, or null when unmatched.
    pub mal_id: Option<String>,
    /// AniDB identifier, or null when unmatched.
    pub anidb_id: Option<String>,
    /// Aggregated ratings cached during the movie's latest metadata hydration.
    pub ratings: TitleRatingPayload,
}

#[derive(SimpleObject, Clone)]
/// A movie's placement and continuity metadata within a series.
pub struct SeriesMovieLinkPayload {
    /// Stable series-movie link identifier.
    pub id: ID,
    /// Movie metadata referenced by this link.
    pub movie: MovieEntityPayload,
    /// Narrative position within the series, or null when unknown.
    pub narrative_order: Option<String>,
    /// Season after which the movie is placed, or null when not applicable.
    pub after_season: Option<i32>,
    /// Season before which the movie is placed, or null when not applicable.
    pub before_season: Option<i32>,
    /// Linked episode identifier, or null when this link has no episode anchor.
    pub linked_episode_id: Option<ID>,
    /// Continuity classification, or null when not determined.
    pub continuity_status: Option<String>,
    /// Form of the movie in the series chronology, or null when not determined.
    pub movie_form: Option<String>,
    /// Short explanation of the placement signals, or null when unavailable.
    pub signal_summary: Option<String>,
    /// Explicit operator monitoring choice, or null when title policy manages this link.
    pub monitoring_override: Option<bool>,
    /// Whether current metadata still reports this series-movie relationship.
    pub metadata_active: bool,
    /// Whether acquisition and monitoring are enabled for this link.
    pub monitored: bool,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// Episode identity, schedule metadata, flags, and media readiness fields.
pub struct EpisodePayload {
    /// Stable episode identifier.
    pub id: ID,
    /// Identifier of the title containing this episode.
    pub title_id: ID,
    /// Identifier of the containing collection, or null when not assigned.
    pub collection_id: Option<ID>,
    /// Episode kind, such as regular, special, filler, or recap.
    pub episode_type: EpisodeTypeValue,
    /// Season-relative episode number, or null when unknown.
    pub episode_number: Option<String>,
    /// Season number, or null when the source does not provide one.
    pub season_number: Option<String>,
    /// Display label for the episode number, or null before metadata is available.
    pub episode_label: Option<String>,
    /// Episode title, or null before metadata is available.
    pub title: Option<String>,
    /// Episode synopsis, or null before metadata is available.
    pub overview: Option<String>,
    /// Air date as a calendar date without a time zone, or null when unknown.
    pub air_date: Option<Date>,
    /// Duration in seconds, or null when media or metadata has not supplied it.
    pub duration_seconds: Option<i64>,
    /// Whether more than one audio stream is present in the available media.
    pub has_multi_audio: bool,
    /// Whether at least one subtitle stream is present in the available media.
    pub has_subtitle: bool,
    /// Whether the episode is classified as filler.
    pub is_filler: bool,
    /// Whether the episode is classified as recap.
    pub is_recap: bool,
    /// Absolute episode number, or null when unknown.
    pub absolute_number: Option<String>,
    /// Proxied episode image URL, or null when no image source is available.
    pub image_url: Option<String>,
    /// Whether acquisition and monitoring are enabled for this episode.
    pub monitored: bool,
    /// Time when the episode record was created, in UTC.
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// A title media file with raw scan data and parsed acquisition metadata.
pub struct TitleMediaFilePayload {
    /// Stable media file identifier.
    pub id: ID,
    /// Identifier of the title owning this file.
    pub title_id: ID,
    /// Identifier of the episode represented by this file, or null for title-level media.
    pub episode_id: Option<ID>,
    /// Identifiers of series-movie links represented by this file, or an empty list when none are associated.
    pub series_movie_link_ids: Vec<ID>,
    /// Absolute path to the media file.
    pub file_path: String,
    /// File size in bytes.
    pub size_bytes: Long,
    /// File role, such as primary or additional media.
    pub role: String,
    /// Human-readable quality label, or null before quality is known.
    pub quality_label: Option<String>,
    /// Raw media scan state; analysis fields remain null until scanning completes.
    pub scan_status: String,
    /// Time when the file record was created, in UTC.
    pub created_at: DateTime<Utc>,
    /// Codec discovered by media analysis, or null until a scan supplies it.
    pub video_codec: Option<String>,
    /// Video width in pixels, or null until a scan supplies it.
    pub video_width: Option<i32>,
    /// Video height in pixels, or null until a scan supplies it.
    pub video_height: Option<i32>,
    /// Video bitrate in kilobits per second, or null until a scan supplies it.
    pub video_bitrate_kbps: Option<i32>,
    /// Video bit depth in bits per channel, or null until a scan supplies it.
    pub video_bit_depth: Option<i32>,
    /// HDR format reported by the scan, or null when absent or not scanned.
    pub video_hdr_format: Option<String>,
    /// Video frame rate as a source-formatted string, or null until known.
    pub video_frame_rate: Option<String>,
    /// Codec profile reported by the scan, or null until known.
    pub video_profile: Option<String>,
    /// Audio codec reported by the scan, or null until known.
    pub audio_codec: Option<String>,
    /// Number of audio channels, or null until known.
    pub audio_channels: Option<i32>,
    /// Audio bitrate in kilobits per second, or null until known.
    pub audio_bitrate_kbps: Option<i32>,
    /// Language codes reported for audio streams.
    pub audio_languages: Vec<String>,
    /// Detailed audio streams reported by the scan.
    pub audio_streams: Vec<AudioStreamDetailPayload>,
    /// Language codes reported for subtitle streams.
    pub subtitle_languages: Vec<String>,
    /// Codec names reported for subtitle streams.
    pub subtitle_codecs: Vec<String>,
    /// Detailed subtitle streams reported by the scan.
    pub subtitle_streams: Vec<SubtitleStreamDetailPayload>,
    /// Whether the file contains multiple audio streams.
    pub has_multiaudio: bool,
    /// Media duration in seconds, or null until known.
    pub duration_seconds: Option<i32>,
    /// Number of chapters, or null until known.
    pub num_chapters: Option<i32>,
    /// Container format, or null until known.
    pub container_format: Option<String>,
    /// Parsed scene or release name, or null when import metadata is unavailable.
    pub scene_name: Option<String>,
    /// Parsed release group, or null when unavailable.
    pub release_group: Option<String>,
    /// Parsed source type, or null when unavailable.
    pub source_type: Option<String>,
    /// Parsed resolution label, or null when unavailable.
    pub resolution: Option<String>,
    /// Parsed video codec label, or null when unavailable.
    pub video_codec_parsed: Option<String>,
    /// Parsed audio codec label, or null when unavailable.
    pub audio_codec_parsed: Option<String>,
    /// Acquisition score assigned during release matching, or null when not scored.
    pub acquisition_score: Option<i32>,
    /// Structured scoring explanation, or null when no scoring record exists.
    pub scoring_log: Option<String>,
    /// Indexer or provider that supplied the release, or null when unknown.
    pub indexer_source: Option<String>,
    /// Release title that was grabbed, or null when the file was not acquired from a release.
    pub grabbed_release_title: Option<String>,
    /// Time when the release was grabbed, in UTC, or null when not recorded.
    pub grabbed_at: Option<DateTime<Utc>>,
    /// Parsed edition label, or null when unavailable.
    pub edition: Option<String>,
    /// File path before import moved or renamed the file, or null when not recorded.
    pub original_file_path: Option<String>,
    /// Release identity hash, or null when unavailable.
    pub release_hash: Option<String>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// Library identity, facet, and configured storage roots.
pub struct LibraryPayload {
    /// Stable library identifier.
    pub id: ID,
    /// Media facet managed by the library.
    pub facet: MediaFacetValue,
    /// Display name of the library.
    pub name: String,
    /// Stable URL-friendly library key.
    pub slug: String,
    /// Whether this is the configured default library for its facet.
    pub is_default: bool,
    /// Whether the library was created by bootstrap root-folder setup.
    pub is_bootstrap_default_root_set: bool,
    /// Root folders configured for the library.
    pub roots: Vec<LibraryRootPayload>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// Queue item state with download client, import, and tracked-download information.
pub struct DownloadQueueItemPayload {
    /// Identifier of this queue item.
    pub id: ID,
    /// Identifier of the associated title, or null for an unmatched item.
    pub title_id: Option<ID>,
    /// Identifier of the associated episode, or null for title-level or unmatched items.
    pub episode_id: Option<ID>,
    /// Title name resolved for display, including an unmatched release title when needed.
    pub title_name: String,
    /// Media facet of the matched title, or null when unmatched.
    pub facet: Option<MediaFacetValue>,
    /// Whether the item originated from Scryer's acquisition workflow.
    pub is_scryer_origin: bool,
    /// Provider label associated with the source release, or null when unknown.
    pub source_provider: Option<String>,
    /// Identifier of the download client owning this item.
    pub client_id: ID,
    /// Display name of the download client.
    pub client_name: String,
    /// Download client type identifier.
    pub client_type: String,
    /// State reported by the download client.
    pub state: DownloadQueueStateValue,
    /// Normalized state across the download client and import workflow.
    pub display_state: DownloadDisplayStateValue,
    /// Download progress percentage from 0 through 100.
    pub progress_percent: i32,
    /// Import transfer stage, or null when no import transfer is active.
    pub import_transfer_phase: Option<ImportTransferPhaseValue>,
    /// Bytes transferred during import, or null when transfer accounting is unavailable.
    pub import_transfer_bytes: Option<Long>,
    /// Total bytes expected during import, or null when the total is unknown.
    pub import_transfer_total_bytes: Option<Long>,
    /// Time the import transfer started, in UTC, or null when not started.
    pub import_transfer_started_at: Option<DateTime<Utc>>,
    /// Time the import transfer was last updated, in UTC, or null when not updated.
    pub import_transfer_updated_at: Option<DateTime<Utc>>,
    /// Download size in bytes, or null when the client has not reported it.
    pub size_bytes: Option<Long>,
    /// Estimated remaining time in seconds, or null when unavailable.
    pub remaining_seconds: Option<i32>,
    /// Time the item entered the queue, in UTC, or null when unknown.
    pub queued_at: Option<DateTime<Utc>>,
    /// Time the client last updated the item, in UTC, or null when unknown.
    pub last_updated_at: Option<DateTime<Utc>>,
    /// Whether the item requires attention before the workflow can proceed.
    pub attention_required: bool,
    /// Reason for the attention flag, or null when no attention is required.
    pub attention_reason: Option<String>,
    /// Client-local item identifier; together with `client_id` it identifies the remote item.
    pub download_client_item_id: String,
    /// Application download identifier, or null when the item is not linked to one.
    pub download_id: Option<String>,
    /// Import lifecycle status, or null before import classification.
    pub import_status: Option<ImportStatusValue>,
    /// Import failure code, or null when there is no import failure.
    pub import_error_code: Option<ImportErrorCodeValue>,
    /// Import failure detail, or null when there is no import failure.
    pub import_error_message: Option<String>,
    /// Time the item was imported, in UTC, or null before successful import.
    pub imported_at: Option<DateTime<Utc>>,
    /// Deletion command status, or null when no deletion was requested.
    pub delete_status: Option<DownloadQueueDeleteStatusValue>,
    /// Deletion failure detail, or null when deletion has not failed.
    pub delete_error_message: Option<String>,
    /// Tracked-download lifecycle state, or null when no tracked record matches.
    pub tracked_state: Option<TrackedDownloadStateValue>,
    /// Tracked-download status, or null when no tracked record matches.
    pub tracked_status: Option<TrackedDownloadStatusValue>,
    /// Human-readable tracked-download status messages; empty when none are available.
    pub tracked_status_messages: Vec<String>,
    /// Match classification for the tracked download, or null when unmatched.
    pub tracked_match_type: Option<TitleMatchTypeValue>,
    /// Seeding obligation state, or null when the item carries no torrent seeding information.
    pub seeding_state: Option<DownloadSeedingStateValue>,
    /// Share ratio observed by the download client, or null when it reports none.
    pub seed_ratio: Option<f64>,
    /// Share ratio this download was grabbed under, or null when no profile applied.
    pub seed_ratio_goal: Option<f64>,
    /// Seconds spent seeding as reported by the client, or null when it has no counter.
    pub seed_time_seconds: Option<Long>,
    /// Seeding time in seconds this download was grabbed under, or null when no profile applied.
    pub seed_time_goal_seconds: Option<Long>,
    /// Whether the torrent's metainfo carries the private flag; null when the client cannot say.
    pub is_private: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// A page of completed or failed queue history items and clients available for filtering.
pub struct DownloadHistoryPagePayload {
    /// Items in the requested page in stable history order.
    pub items: Vec<DownloadQueueItemPayload>,
    /// Whether another page exists after this page.
    pub has_more: bool,
    /// Number of matching history items across all pages.
    pub total_count: i32,
    /// Clients represented after activity filters and before the client filter is applied.
    pub available_clients: Vec<DownloadClientFilterOptionPayload>,
}

#[derive(SimpleObject, Clone)]
/// A live download queue page with revision and readiness metadata.
pub struct DownloadQueuePagePayload {
    /// Items in the requested page in stable queue order.
    pub items: Vec<DownloadQueueItemPayload>,
    /// Whether another page exists after this page.
    pub has_more: bool,
    /// Number of matching queue items across all pages.
    pub total_count: i32,
    /// Clients represented after queue, title, and permission filters and before the client filter is applied.
    pub available_clients: Vec<DownloadClientFilterOptionPayload>,
    /// Monotonic queue revision used to detect updates.
    pub revision: Long,
    /// Time the queue data was updated, in UTC, or null before the first update.
    pub updated_at: Option<DateTime<Utc>>,
    /// Whether the queue data is complete enough to represent current state.
    pub ready: bool,
    /// Whether the queue data may be behind its source.
    pub stale: bool,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Runtime phase for an import operation currently awaiting or using a worker lane.
pub enum ActiveImportStreamPhaseValue {
    /// The import is waiting for a worker lane.
    Queued,
    /// Archive extraction is running for the import.
    Extracting,
    /// A fast filesystem placement is in progress.
    Placing,
    /// File content is being copied to its destination.
    Copying,
    /// The destination is being verified and promoted.
    Finalizing,
}

#[derive(SimpleObject, Clone)]
/// A queued or active import operation. This reports real import work only, never worker capacity.
pub struct ActiveImportStreamPayload {
    /// Opaque runtime identity for this active import stream.
    pub id: ID,
    /// Identifier of the persisted import record.
    pub import_id: ID,
    /// Identifier of the library receiving the import.
    pub library_id: ID,
    /// Media category of the import target.
    pub facet: MediaFacetValue,
    /// Filesystem path of the source file or extraction directory.
    pub source_path: String,
    /// Filesystem path where the import is being placed.
    pub destination_path: String,
    /// Current runtime phase of the import.
    pub phase: ActiveImportStreamPhaseValue,
    /// Number of source bytes transferred so far.
    pub bytes: Long,
    /// Total source bytes expected for the transfer, or zero when unavailable.
    pub total_bytes: Long,
    /// Time the import entered the active worker queue.
    pub queued_at: DateTime<Utc>,
    /// Time work began, or null while the import remains queued.
    pub started_at: Option<DateTime<Utc>>,
    /// Time this stream was last updated.
    pub updated_at: DateTime<Utc>,
    /// Whether this stream can still be cancelled.
    pub cancellable: bool,
    /// Whether cancellation has been requested and is being processed.
    pub cancellation_requested: bool,
}

#[derive(SimpleObject, Clone)]
/// Revision metadata for active import stream updates.
pub struct ActiveImportStreamSyncPayload {
    /// Monotonic revision for active import stream changes.
    pub revision: Long,
    /// Time active import streams last changed, or null before the first change.
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Queue revision metadata returned after synchronization.
pub struct DownloadQueueSyncPayload {
    /// New monotonic queue revision.
    pub revision: Long,
    /// Time the queue data was updated, in UTC, or null when no update exists.
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// A page of queue items classified for import.
pub struct DownloadImportPagePayload {
    /// Import items in the requested page in stable order.
    pub items: Vec<DownloadQueueItemPayload>,
    /// Whether another page exists after this page.
    pub has_more: bool,
    /// Number of matching import items across all pages.
    pub total_count: i32,
}

#[derive(SimpleObject, Clone)]
/// Result of adding or reusing a title and optionally queuing its acquisition.
pub struct AddTitleResult {
    /// Title created or reused by the operation.
    pub title: TitlePayload,
    /// Metadata processing outcome, including whether work was accepted or completed.
    pub metadata_hydration_state: AddTitleHydrationStateValue,
    /// Whether an existing title matched the requested identity.
    pub reused_existing_title: bool,
    /// Whether an existing queued download was reused instead of creating another one.
    pub reused_queued_download: bool,
    /// Identifier of the metadata or acquisition job, or null when no job was created.
    pub download_job_id: Option<ID>,
    /// Queued download created or reused by the operation, or null when nothing was queued.
    pub queued_download: Option<QueueDownloadPayload>,
}

#[derive(SimpleObject, Clone)]
/// Result of repairing a title match and any follow-up scan work.
pub struct FixTitleMatchPayload {
    /// Title after the identity repair.
    pub title: TitlePayload,
    /// Whether metadata became available during the repair.
    pub hydrated: bool,
    /// Library scan summary, or null when no scan was accepted or run.
    pub library_scan: Option<LibraryScanSummaryPayload>,
    /// Non-fatal warnings produced while repairing the match.
    pub warnings: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Preview of binding an unmatched imported file to a title and episode.
pub struct PendingImportBindingPreviewPayload {
    /// Title selected for the pending import.
    pub title: TitlePayload,
    /// File identity and parsed metadata available for binding.
    pub file: PendingImportBindingFilePreviewPayload,
    /// Episodes eligible for binding under the selected title.
    pub available_episodes: Vec<EpisodePayload>,
}

#[derive(SimpleObject, Clone)]
/// Result of resolving a pending import to a title.
pub struct ResolvePendingImportPayload {
    /// Title created or reused for the pending import.
    pub title: TitlePayload,
    /// Whether the operation created a new title.
    pub created: bool,
    /// Library scan summary, or null when no scan was accepted or run.
    pub library_scan: Option<LibraryScanSummaryPayload>,
    /// Metadata processing outcome for the resolved title.
    pub metadata_hydration_state: AddTitleHydrationStateValue,
}

#[derive(SimpleObject, Clone)]
/// Result of a queue command and the item state observed after it was issued.
pub struct DownloadQueueActionPayload {
    /// Command action accepted for the queue item.
    pub kind: DownloadQueueActionKindValue,
    /// Client-local item identifier targeted by the command.
    pub download_client_item_id: String,
    /// Identifier of the client targeted by the command, or null when unresolved.
    pub client_id: Option<ID>,
    /// Client type targeted by the command, or null when unresolved.
    pub client_type: Option<String>,
    /// Application import identifier affected by the command, or null when not applicable.
    pub import_id: Option<ID>,
    /// Command identifier for tracking asynchronous work, or null when no command was created.
    pub command_id: Option<ID>,
    /// Whether the item was removed from the queue.
    pub removed: bool,
    /// Updated queue item, or null when the item was removed or is unavailable.
    pub queue_item: Option<DownloadQueueItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Preview of files and targets available for a manual import.
pub struct ManualImportPreviewPayload {
    /// Files discovered for the import request.
    pub files: Vec<ManualImportFilePreviewPayload>,
    /// Episodes eligible as import targets.
    pub available_episodes: Vec<EpisodePayload>,
    /// Series-movie links eligible as import targets.
    pub available_series_movies: Vec<ManualImportSeriesMovieTargetPayload>,
}

#[derive(SimpleObject, Clone)]
/// Persisted manual-import selection that can be submitted for execution.
pub struct ManualImportSelectionPayload {
    /// Identifier of the persisted selection.
    pub selection_id: ID,
    /// The download contains archives and must be explicitly extracted before files can be mapped.
    pub archive_extraction_needed: bool,
    /// Files included in the selection.
    pub files: Vec<ManualImportFilePreviewPayload>,
    /// Episodes eligible as import targets for the selection.
    pub available_episodes: Vec<EpisodePayload>,
    /// Series-movie links eligible as import targets for the selection.
    pub available_series_movies: Vec<ManualImportSeriesMovieTargetPayload>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// Acquisition target for a title scope, including missing and upgrade states.
pub struct WantedItemPayload {
    /// Stable identifier for this scope, or an addressable key such as `episode:<id>`, `title:<id>`, or `series_movie:<id>`.
    pub id: ID,
    /// Identifier of the title targeted for acquisition.
    pub title_id: ID,
    /// Title name, or null when title metadata is unavailable.
    pub title_name: Option<String>,
    /// Stable title slug, or null when unavailable.
    pub title_slug: Option<String>,
    /// Title facet, or null when title context is unavailable.
    pub title_facet: Option<String>,
    /// Identifier of the containing library, or null when the scope has no library context.
    pub library_id: Option<ID>,
    /// Library name, or null when library metadata is unavailable.
    pub library_name: Option<String>,
    /// Stable library slug, or null when library metadata is unavailable.
    pub library_slug: Option<String>,
    /// Identifier of the targeted episode, or null for title and series-movie scopes.
    pub episode_id: Option<ID>,
    /// Identifier of the targeted collection, or null when not applicable.
    pub collection_id: Option<ID>,
    /// Season number for an episodic scope, or null when not applicable or unknown.
    pub season_number: Option<String>,
    /// Episode number for an episodic scope, or null when not applicable or unknown.
    pub episode_number: Option<String>,
    /// Acquisition media type represented by this scope.
    pub media_type: WantedMediaTypeValue,
    /// Time of the latest search in UTC, or null before a search runs.
    pub last_search_at: Option<DateTime<Utc>>,
    /// Acquisition status; missing targets report wanted until a stored status exists.
    pub status: WantedStatusValue,
    /// Serialized grabbed-release metadata, or null when no release has been grabbed.
    pub grabbed_release: Option<String>,
    /// Safe provider label extracted from grabbed-release metadata, with sensitive release details omitted.
    pub source_provider: Option<String>,
    /// The bar this scope's landed file sets, or null when nothing occupies it.
    ///
    /// The re-derived canonical score of the primary media file occupying the
    /// scope: computed on read from the row, never read back from the persisted
    /// `acquisition_score`, which is display history and is only valid while the
    /// profile, persona, rule packs and scoring algorithm that wrote it are all
    /// unchanged. This is the same number the admission gate compares a
    /// candidate against, so the value shown here and the value a grab or import
    /// decision used cannot disagree.
    ///
    /// It used to be a per-scope ledger column that only held a landed score in
    /// one of its five lifecycle states; after a rejected import it held the
    /// score of a release that never landed, which read lower than the
    /// incumbent.
    pub current_score: Option<i32>,
    /// Latest release decision, or null before a candidate decision exists.
    pub latest_release_decision: Option<ReleaseDecisionPayload>,
    /// Number of saved fallback candidates keyed to this scope. Season-pack
    /// candidates are keyed to their season's anchor episode, so sibling
    /// episodes can report zero while the anchor reports the saved candidates.
    pub standby_count: i64,
    /// Whether a changed title match permits a recovery search.
    pub mismatch_recovery_eligible: bool,
    /// Convergence state showing whether indexer coverage is queued, searching, complete, or deferred.
    pub convergence_state: ConvergenceStateValue,
    /// Number of routed indexers already searched under the current fingerprint.
    pub indexers_covered: i32,
    /// Number of routed indexers enabled for this scope.
    pub indexers_routed: i32,
    /// Scheduling lane used to prioritize convergence work.
    pub recency_lane: RecencyLaneValue,
    /// Creation time in UTC, using a fallback timestamp for synthesized scopes.
    pub created_at: DateTime<Utc>,
    /// Update time in UTC, using a fallback timestamp for synthesized scopes.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// A complete or partial page of wanted acquisition scopes.
pub struct WantedItemsListPayload {
    /// Wanted items in stable order.
    pub items: Vec<WantedItemPayload>,
    /// Number of matching scopes across all pages.
    pub total_count: i64,
    /// Whether another page exists after this page.
    pub has_more: bool,
}

#[derive(SimpleObject, Clone)]
/// Wanted items returned without a total-count query.
pub struct WantedItemsPagePayload {
    /// Wanted items in stable order.
    pub items: Vec<WantedItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Acquisition diagnostics for a title, including decisions and state counts.
pub struct TitleAcquisitionDiagnosticsPayload {
    /// Recent release decisions, newest first.
    pub recent_decisions: Vec<ReleaseDecisionPayload>,
    /// Counts grouped by release decision code.
    pub decision_counts: Vec<DecisionCodeCountPayload>,
    /// Counts grouped by wanted status.
    pub wanted_status_counts: Vec<WantedStatusCountPayload>,
    /// Counts grouped by pending-release status.
    pub pending_release_counts: Vec<PendingReleaseStatusCountPayload>,
    /// Number of scopes eligible for mismatch-recovery search.
    pub mismatch_recovery_eligible_count: i64,
    /// Time of the latest release decision in UTC, or null when none exists.
    pub latest_decision_at: Option<DateTime<Utc>>,
    /// Time of the latest wanted search in UTC, or null when none exists.
    pub latest_wanted_search_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// A scored release decision associated with a wanted scope.
pub struct ReleaseDecisionPayload {
    /// Stable release-decision identifier.
    pub id: ID,
    /// Identifier of the wanted scope evaluated by the decision.
    pub wanted_item_id: ID,
    /// Identifier of the title targeted by the decision.
    pub title_id: ID,
    /// Release title evaluated by the matcher.
    pub release_title: String,
    /// Release URL, or null when the provider did not supply one.
    pub release_url: Option<String>,
    /// Release size in bytes, or null when unavailable.
    pub release_size_bytes: Option<Long>,
    /// Machine-readable decision code.
    pub decision_code: String,
    /// Candidate score assigned by the scoring rules.
    pub candidate_score: i32,
    /// The bar this decision was measured against: the re-derived canonical
    /// score of the primary media file occupying the scope at decision time,
    /// never the persisted `acquisition_score`. Null when the decision was
    /// recorded before any comparison ran (a parse or identity refusal), or when
    /// nothing occupied the scope.
    pub current_score: Option<i32>,
    /// Candidate minus current score, or null when no current score exists.
    pub score_delta: Option<i32>,
    /// Structured scoring explanation, or null when no explanation was stored.
    pub explanation_json: Option<Json<serde_json::Value>>,
    /// Time when the decision was recorded, in UTC.
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// A page of release decisions with a total count.
pub struct ReleaseDecisionsPagePayload {
    /// Decisions in stable order.
    pub items: Vec<ReleaseDecisionPayload>,
    /// Number of matching decisions across all pages.
    pub total_count: i64,
    /// Whether another page exists after this page.
    pub has_more: bool,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
/// A release held for delayed acquisition evaluation.
pub struct PendingReleasePayload {
    /// Stable pending-release identifier.
    pub id: ID,
    /// Identifier of the wanted scope awaiting this release.
    pub wanted_item_id: ID,
    /// Identifier of the title targeted by the pending release.
    pub title_id: ID,
    /// Release title held for later evaluation.
    pub release_title: String,
    /// Release URL, or null when the provider did not supply one.
    pub release_url: Option<String>,
    /// Release size in bytes, or null when unavailable.
    pub release_size_bytes: Option<Long>,
    /// Score assigned to the pending release.
    pub release_score: i32,
    /// Structured scoring explanation, or null when unavailable.
    pub scoring_log_json: Option<Json<serde_json::Value>>,
    /// Provider label, or null when unavailable.
    pub indexer_source: Option<String>,
    /// Provider identifier, or null when the source is not linked.
    pub indexer_id: Option<ID>,
    /// RFC3339 publication time reported by the indexer, or null when unavailable.
    pub published_at: Option<DateTime<Utc>>,
    /// Number of torrent seeders reported when this release was saved, or null when unknown.
    pub seeders: Option<i64>,
    /// Time when the release entered the pending set, in UTC.
    pub added_at: DateTime<Utc>,
    /// Time before which the release is held, in UTC.
    pub delay_until: DateTime<Utc>,
    /// Current machine-readable reason this release is held, or null when none was recorded.
    pub last_decision_code: Option<String>,
    /// Arbitration role independent of the release lifecycle state.
    pub role: PendingReleaseRoleValue,
    /// Lifecycle state of the pending release.
    pub status: PendingReleaseStatusValue,
}

#[derive(InputObject, Clone)]
/// Optional predicates for pending-release pages.
pub struct PendingReleaseFilterInput {
    /// Restrict results to a title identifier; null leaves all titles included.
    pub title_id: Option<ID>,
    /// Restrict results to a wanted scope identifier; null leaves all scopes included.
    pub wanted_item_id: Option<ID>,
    /// Restrict results to these pending-release statuses; null leaves all statuses included.
    pub statuses: Option<Vec<PendingReleaseStatusValue>>,
}

#[derive(SimpleObject, Clone)]
/// A page of pending releases with count and continuation metadata.
pub struct PendingReleasesPayload {
    /// Pending releases in stable order.
    pub items: Vec<PendingReleasePayload>,
    /// Whether another page exists after this page.
    pub has_more: bool,
    /// Number of matching pending releases across all pages.
    pub total_count: i32,
}
