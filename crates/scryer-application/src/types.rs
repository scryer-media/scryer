use std::collections::{BTreeMap, HashMap};

use scryer_domain::{CanonicalMediaTag, DownloadQueueCommandAction, ExternalId, Title};
use serde::{Deserialize, Serialize};

use crate::SubmissionScope;
use crate::library_scan::LibraryScanSummary;
use crate::quality_profile::QualityProfileDecision;
use crate::release_parser::{ParsedReleaseMetadata, VideoCodec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryRootDraft {
    pub path: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TitleExternalRating {
    pub source: String,
    pub value: Option<f64>,
    pub score: Option<f64>,
    pub normalized: f64,
    pub votes: Option<i32>,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TitleRatingSummary {
    pub rating: Option<f64>,
    pub rating_sources: Vec<String>,
    pub external_ratings: Vec<TitleExternalRating>,
}

#[derive(Clone, Debug, Default)]
pub struct TitleMetadataUpdate {
    pub name: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub imdb_id: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub popularity: Option<f64>,
    pub canonical_tags: Vec<CanonicalMediaTag>,
    pub content_status: Option<String>,
    pub language: Option<String>,
    pub first_aired: Option<String>,
    pub network: Option<String>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub aliases: Vec<String>,
    pub tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    pub metadata_language: Option<String>,
    pub metadata_fetched_at: Option<String>,
    pub digital_release_date: Option<String>,
    /// Ratings returned by SMG/MDBList. `Some(default)` clears stale stored ratings.
    pub ratings: Option<TitleRatingSummary>,
    /// Additional external IDs to merge onto the title (e.g. MAL, AniList from anime mappings).
    pub extra_external_ids: Vec<ExternalId>,
    /// Additional tags to merge onto the title (e.g. MAL score, anime media type).
    pub extra_tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedExternalId {
    pub scope_id: String,
    pub source: String,
    pub external_id: String,
    pub provenance: String,
    pub source_scope: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AddTitleHydrationState {
    Pending,
    Complete,
    NotRequired,
}

#[derive(Clone, Debug)]
pub struct CreateTitleOutcome {
    pub title: Title,
    pub reused_existing: bool,
}

#[derive(Clone, Debug)]
pub struct AddTitleOutcome {
    pub title: Title,
    pub metadata_hydration_state: AddTitleHydrationState,
    pub reused_existing_title: bool,
}

#[derive(Clone, Debug)]
pub struct AddTitleAndQueueDownloadOutcome {
    pub title: Title,
    pub metadata_hydration_state: AddTitleHydrationState,
    pub reused_existing_title: bool,
    pub download_job_id: String,
    pub reused_queued_download: bool,
}

#[derive(Clone, Debug)]
pub struct CutoffUnmetItem {
    pub title_id: String,
    pub title_name: String,
    pub title_slug: Option<String>,
    pub title_facet: scryer_domain::MediaFacet,
    pub library_id: String,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    pub episode_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub current_tier: String,
    pub target_tier: String,
}

/// One bounded page of cutoff-unmet targets. `total` is
/// the full unmet count for the query so the UI can paginate without loading the
/// whole set into the browser.
#[derive(Clone, Debug)]
pub struct CutoffUnmetPage {
    pub items: Vec<CutoffUnmetItem>,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycleBinSettings {
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateRecycleBinSettings {
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycledItem {
    pub id: String,
    pub original_path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub title_id: Option<String>,
    pub reason: String,
    pub recycled_at: String,
    pub media_root: String,
    pub library_id: String,
    pub library_name: String,
}

#[derive(Clone, Debug)]
pub struct PendingTitleHydration {
    /// Title queued for background metadata hydration.
    ///
    /// The queue only contains titles whose metadata is still incomplete and whose
    /// persistence-layer retry marker says they are due now. Once hydration succeeds
    /// or retry state is cleared, the title falls out of this queue.
    pub title: Title,
    pub attempt_count: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalImportMonitorMovieEntry {
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub monitored: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalImportMonitorSeasonEntry {
    pub season_number: i32,
    pub monitored: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalImportMonitorEpisodeEntry {
    pub tvdb_id: Option<String>,
    pub season_number: i32,
    pub episode_number: i32,
    pub monitored: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalImportMonitorSeriesEntry {
    pub tvdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub monitored: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seasons: Vec<ExternalImportMonitorSeasonEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<ExternalImportMonitorEpisodeEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalImportMonitorSnapshotEntryKind {
    Movie,
    Series,
}

impl ExternalImportMonitorSnapshotEntryKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalImportMonitorSnapshotChunk {
    pub session_id: String,
    pub facet: scryer_domain::MediaFacet,
    pub entry_kind: ExternalImportMonitorSnapshotEntryKind,
    pub chunk_index: i32,
    pub payload_ndjson: String,
    pub created_at: String,
}

pub const EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID: &str = "external-import-monitor-apply";
pub const EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_PREFIX: &str = "external-import-monitor-apply:";

pub fn external_import_monitor_apply_session_id_for_library(library_id: &str) -> String {
    format!(
        "{EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_PREFIX}{}",
        library_id.trim()
    )
}

pub fn is_external_import_monitor_apply_session_id(session_id: &str) -> bool {
    session_id == EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID
        || session_id.starts_with(EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_PREFIX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibraryScanHintSource {
    ExternalImportRadarr,
    ExternalImportSonarr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibraryScanHintFacet {
    Movie,
    Series,
}

impl LibraryScanHintFacet {
    pub const fn from_media_facet(facet: scryer_domain::MediaFacet) -> Option<Self> {
        match facet {
            scryer_domain::MediaFacet::Movie => Some(Self::Movie),
            scryer_domain::MediaFacet::Series => Some(Self::Series),
            scryer_domain::MediaFacet::Anime => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalIdProvider {
    Imdb,
    Tmdb,
    Tvdb,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalIdHint {
    pub provider: ExternalIdProvider,
    pub value: String,
}

impl ExternalIdHint {
    pub fn normalized(provider: ExternalIdProvider, raw: &str) -> Option<Self> {
        let value = match provider {
            ExternalIdProvider::Imdb => normalize_strict_imdb_id(raw)?,
            ExternalIdProvider::Tmdb | ExternalIdProvider::Tvdb => {
                normalize_numeric_external_id(raw)?
            }
        };
        Some(Self { provider, value })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryScanHint {
    pub source: LibraryScanHintSource,
    pub facet: LibraryScanHintFacet,
    pub path_key: String,
    pub full_path_key: Option<String>,
    pub ids: Vec<ExternalIdHint>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryScanHintSet {
    hints: Vec<LibraryScanHint>,
}

impl LibraryScanHintSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, hint: LibraryScanHint) {
        if hint.ids.is_empty() || hint.path_key.trim().is_empty() {
            return;
        }
        if let Some(existing) = self.hints.iter_mut().find(|existing| {
            existing.facet == hint.facet
                && stored_path_keys_match(&existing.path_key, &hint.path_key)
                && optional_stored_path_keys_match(
                    existing.full_path_key.as_deref(),
                    hint.full_path_key.as_deref(),
                )
        }) {
            if existing.ids.is_empty() || !external_ids_overlap(&existing.ids, &hint.ids) {
                existing.ids.clear();
                return;
            }
            for id in hint.ids {
                if !existing.ids.iter().any(|existing_id| existing_id == &id) {
                    existing.ids.push(id);
                }
            }
            return;
        }
        self.hints.push(hint);
    }

    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }

    pub fn hint_for_external_ids(
        &self,
        facet: LibraryScanHintFacet,
        candidate_ids: &[ExternalIdHint],
    ) -> Option<&LibraryScanHint> {
        if candidate_ids.is_empty() {
            return None;
        }
        self.hints
            .iter()
            .find(|hint| hint.facet == facet && external_ids_overlap(&hint.ids, candidate_ids))
    }

    pub fn hint_for_stored_path(
        &self,
        facet: LibraryScanHintFacet,
        candidate_path_key: &str,
    ) -> Option<&LibraryScanHint> {
        self.hint_for_scan_path(facet, candidate_path_key, None)
    }

    pub fn hint_for_scan_path(
        &self,
        facet: LibraryScanHintFacet,
        candidate_path_key: &str,
        candidate_full_path_key: Option<&str>,
    ) -> Option<&LibraryScanHint> {
        let leaf_matches = self
            .hints
            .iter()
            .filter(|hint| {
                hint.facet == facet
                    && !hint.ids.is_empty()
                    && stored_path_keys_match(&hint.path_key, candidate_path_key)
            })
            .collect::<Vec<_>>();
        let first = leaf_matches.first().copied()?;
        if leaf_matches
            .iter()
            .all(|hint| external_ids_overlap(&first.ids, &hint.ids))
        {
            return Some(first);
        }

        let full_path_key = candidate_full_path_key?;
        let full_matches = leaf_matches
            .into_iter()
            .filter(|hint| {
                hint.full_path_key
                    .as_deref()
                    .is_some_and(|hint_key| stored_path_keys_match(hint_key, full_path_key))
            })
            .collect::<Vec<_>>();
        let first = full_matches.first().copied()?;
        full_matches
            .iter()
            .all(|hint| external_ids_overlap(&first.ids, &hint.ids))
            .then_some(first)
    }
}

fn optional_stored_path_keys_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => stored_path_keys_match(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn stored_path_keys_match(left: &str, right: &str) -> bool {
    let left = normalize_stored_path_key(left);
    let right = normalize_stored_path_key(right);
    !left.is_empty() && left == right
}

fn normalize_stored_path_key(value: &str) -> String {
    value.trim().trim_end_matches(['/', '\\']).to_lowercase()
}

pub fn library_scan_file_leaf_key(path: &str) -> Option<String> {
    let components = leaf_path_components(path);
    let file = components.last()?;
    let parent = components.iter().rev().nth(1)?;
    Some(format!(
        "file:{}/{}",
        normalize_leaf_path_component(parent),
        normalize_leaf_path_component(file)
    ))
}

pub fn library_scan_folder_leaf_key(path: &str) -> Option<String> {
    let components = leaf_path_components(path);
    let folder = components.last()?;
    Some(format!("folder:{}", normalize_leaf_path_component(folder)))
}

pub fn library_scan_file_full_path_key(path: &str) -> Option<String> {
    full_path_key("file-path", path)
}

pub fn library_scan_folder_full_path_key(path: &str) -> Option<String> {
    full_path_key("folder-path", path)
}

fn full_path_key(prefix: &str, path: &str) -> Option<String> {
    let components = leaf_path_components(path);
    (!components.is_empty()).then(|| {
        format!(
            "{prefix}:{}",
            components
                .into_iter()
                .map(normalize_leaf_path_component)
                .collect::<Vec<_>>()
                .join("/")
        )
    })
}

fn leaf_path_components(path: &str) -> Vec<&str> {
    path.trim()
        .trim_end_matches(['/', '\\'])
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|component| !component.is_empty())
        .collect()
}

fn normalize_leaf_path_component(component: &str) -> String {
    component.trim().to_lowercase()
}

fn external_ids_overlap(left: &[ExternalIdHint], right: &[ExternalIdHint]) -> bool {
    left.iter().any(|left_id| {
        right.iter().any(|right_id| {
            left_id.provider == right_id.provider && left_id.value == right_id.value
        })
    })
}

fn normalize_strict_imdb_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    let lower = value.to_ascii_lowercase();
    let digits = lower.strip_prefix("tt")?;
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(format!("tt{digits}"))
}

fn normalize_numeric_external_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())).then(|| value.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadQueueCommandRecord {
    pub id: String,
    pub action: DownloadQueueCommandAction,
    pub client_id: Option<String>,
    pub client_type: String,
    pub download_client_item_id: String,
    pub is_history: bool,
    pub status: scryer_domain::DownloadQueueDeleteStatus,
    pub error_text: Option<String>,
    pub requested_by_user_id: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleImageKind {
    Poster,
    Fanart,
}

impl TitleImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Poster => "poster",
            Self::Fanart => "fanart",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "poster" => Some(Self::Poster),
            "fanart" => Some(Self::Fanart),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TitleImageVariantRecord {
    pub variant_key: String,
    pub format: String,
    pub width: i32,
    pub height: i32,
    pub bytes: Vec<u8>,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleImageVariantSpec {
    pub variant_key: String,
    pub width: u32,
}

#[derive(Clone, Debug)]
pub struct TitleImageSourceResult {
    pub kind: TitleImageKind,
    pub requested_source_url: String,
    pub source_url: String,
    pub source_etag: Option<String>,
    pub source_last_modified: Option<String>,
    pub source_format: String,
    pub source_width: i32,
    pub source_height: i32,
    pub variants: Vec<TitleImageVariantRecord>,
}

#[derive(Clone, Debug)]
pub struct TitleImageSyncTask {
    pub title_id: String,
    pub kind: TitleImageKind,
    pub source_url: String,
    pub variants: Vec<TitleImageVariantSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleImageBlob {
    pub content_type: String,
    pub etag: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct TitleMediaFile {
    pub id: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub series_movie_link_ids: Vec<String>,
    pub file_path: String,
    pub size_bytes: i64,
    pub role: crate::MediaFileRole,
    pub source_signature_scheme: Option<String>,
    pub source_signature_value: Option<String>,
    pub quality_label: Option<String>,
    pub scan_status: String,
    pub created_at: String,
    // Media analysis fields (populated after media scan; None until scan_status='scanned')
    pub video_codec: Option<VideoCodec>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub video_frame_rate: Option<String>,
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_profile: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    pub audio_languages: Vec<String>,
    pub audio_streams: Vec<crate::AudioStreamDetail>,
    pub subtitle_languages: Vec<String>,
    pub subtitle_codecs: Vec<String>,
    pub subtitle_streams: Vec<crate::SubtitleStreamDetail>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub num_chapters: Option<i32>,
    pub container_format: Option<String>,
    // Rich schema fields (populated during import from parsed release metadata)
    pub scene_name: Option<String>,
    pub release_group: Option<String>,
    pub source_type: Option<String>,
    pub resolution: Option<String>,
    pub video_codec_parsed: Option<VideoCodec>,
    pub audio_codec_parsed: Option<String>,
    pub audio_channels_parsed: Option<String>,
    pub acquisition_score: Option<i32>,
    pub scoring_log: Option<String>,
    pub indexer_source: Option<String>,
    pub grabbed_release_title: Option<String>,
    pub grabbed_at: Option<String>,
    pub edition: Option<String>,
    pub original_file_path: Option<String>,
    pub release_hash: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EpisodeScopedMediaFile {
    pub media_file: TitleMediaFile,
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DownloadHistoryPage {
    pub items: Vec<scryer_domain::DownloadQueueItem>,
    pub has_more: bool,
    pub total_count: usize,
    pub available_clients: Vec<DownloadClientFilterOption>,
}

#[derive(Clone, Debug)]
pub struct DownloadImportPage {
    pub items: Vec<scryer_domain::DownloadQueueItem>,
    pub has_more: bool,
    pub total_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadClientFilterOption {
    pub client_id: String,
    pub client_name: String,
    pub client_type: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadDisplayState {
    Queued,
    Downloading,
    Paused,
    PostProcessing,
    Completed,
    Failed,
    Importing,
    ImportPending,
    ImportBlocked,
    ImportFailed,
    Ignored,
    Removing,
    RemoveFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadActivityFilter {
    All,
    Downloading,
    Queued,
    Paused,
    PostProcessing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadImportFilter {
    All,
    Importing,
    Pending,
    Blocked,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadHistoryFilter {
    All,
    Success,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadHistorySortKey {
    Title,
    Client,
    Status,
    Progress,
    Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleCatalogSortKey {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleCatalogSort {
    pub key: TitleCatalogSortKey,
    pub direction: SortDirection,
}

impl Default for TitleCatalogSort {
    fn default() -> Self {
        Self {
            key: TitleCatalogSortKey::Title,
            direction: SortDirection::Asc,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleCatalogFilter {
    pub monitored: Option<bool>,
    pub content_statuses: Vec<TitleCatalogContentStatus>,
    pub root_folder_ids: Vec<String>,
    pub genre_tag_keys: Vec<String>,
    pub theme_tag_keys: Vec<String>,
    pub minimum_year: Option<i32>,
    pub maximum_year: Option<i32>,
    pub minimum_rating: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitleCatalogTagFilterOption {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitleCatalogFilterOptions {
    pub genres: Vec<TitleCatalogTagFilterOption>,
    pub tags: Vec<TitleCatalogTagFilterOption>,
    pub minimum_year: Option<i32>,
    pub maximum_year: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleCatalogContentStatus {
    Continuing,
    Ended,
}

#[derive(Clone, Debug)]
pub struct TitleCatalogResult {
    pub items: Vec<Title>,
    pub limit: usize,
    pub offset: usize,
    pub has_more: bool,
    pub total_count: usize,
    pub filter_counts: TitleCatalogFilterCounts,
    /// Exact managed media bytes for the authorized facet/library scope.
    /// This deliberately ignores catalog search and filter state.
    pub managed_bytes: i64,
}

#[derive(Clone, Debug, Default)]
pub struct TitleCatalogFilterCounts {
    pub all: usize,
    pub monitored: usize,
    pub unmonitored: usize,
    pub continuing: usize,
    pub ended: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadHistorySort {
    pub key: DownloadHistorySortKey,
    pub direction: SortDirection,
}

#[derive(Clone, Debug)]
pub struct FixTitleMatchResult {
    pub title: scryer_domain::Title,
    pub hydrated: bool,
    pub library_scan: Option<LibraryScanSummary>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PendingImportCounts {
    pub movie: i64,
    pub series: i64,
    pub anime: i64,
}

#[derive(Clone, Debug, Default)]
pub struct MediaRequestCounts {
    pub movie: i64,
    pub series: i64,
    pub anime: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingImportStatus {
    #[default]
    Pending,
    Ignored,
}

impl PendingImportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ignored => "ignored",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "ignored" => Some(Self::Ignored),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingImportSearchAttempt {
    pub query: String,
    pub result_count: usize,
    pub top_results: Vec<String>,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingImportItem {
    pub id: String,
    pub library_id: String,
    pub library_slug: Option<String>,
    pub facet: scryer_domain::MediaFacet,
    pub status: PendingImportStatus,
    pub title_id: Option<String>,
    pub title_name: Option<String>,
    pub title_slug: Option<String>,
    pub display_name: String,
    pub path: String,
    pub folder_path: Option<String>,
    pub query: String,
    pub year_hint: Option<i32>,
    pub reason: String,
    pub search_attempts: Vec<PendingImportSearchAttempt>,
}

#[derive(Clone, Debug, Default)]
pub struct PendingImportConnection {
    pub total: i64,
    pub items: Vec<PendingImportItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingImportBindingFilePreview {
    pub file_path: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub parsed_season: Option<u32>,
    pub parsed_episodes: Vec<u32>,
    pub parsed_absolute_numbers: Vec<u32>,
    pub suggested_episode_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PendingImportBindingPreview {
    pub title: scryer_domain::Title,
    pub file: PendingImportBindingFilePreview,
    pub available_episodes: Vec<scryer_domain::Episode>,
}

#[derive(Clone, Debug)]
pub struct ResolvePendingImportResult {
    pub title: scryer_domain::Title,
    pub created: bool,
    pub library_scan: Option<LibraryScanSummary>,
    pub metadata_hydration_state: AddTitleHydrationState,
}

#[derive(Clone, Debug)]
pub struct IgnorePendingImportResult {
    pub id: String,
    pub status: PendingImportStatus,
}

#[derive(Clone, Debug)]
pub struct CancelLibraryScanResult {
    pub session_id: String,
    pub accepted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionScopeStatus {
    #[default]
    Wanted,
    Grabbed,
    Paused,
    Completed,
}

impl AcquisitionScopeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wanted => "wanted",
            Self::Grabbed => "grabbed",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "wanted" => Some(Self::Wanted),
            "grabbed" => Some(Self::Grabbed),
            "paused" => Some(Self::Paused),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AcquisitionScopeGrabTransition {
    pub id: String,
    pub last_search_at: Option<String>,
    pub current_score: Option<i32>,
    pub grabbed_release: String,
}

#[derive(Clone, Debug)]
pub struct AcquisitionScopeCompleteTransition {
    pub id: String,
    pub last_search_at: Option<String>,
    pub current_score: Option<i32>,
    pub grabbed_release: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AcquisitionScopePauseTransition {
    pub id: String,
    pub last_search_at: Option<String>,
    pub current_score: Option<i32>,
    pub grabbed_release: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryScanUnmatchedSearchAttempt {
    pub query: String,
    pub result_count: usize,
    pub top_results: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryScanUnmatchedItem {
    pub id: String,
    pub library_id: String,
    pub facet: scryer_domain::MediaFacet,
    pub status: PendingImportStatus,
    pub title_id: Option<String>,
    pub scan_session_id: String,
    pub scan_root: String,
    pub item_path: String,
    pub display_name: String,
    pub query: String,
    pub year_hint: Option<i32>,
    pub reason_code: String,
    pub error_message: Option<String>,
    pub search_attempts: Vec<LibraryScanUnmatchedSearchAttempt>,
    pub created_at: String,
    pub updated_at: String,
}

/// Per-scope acquisition state. A row exists because something
/// *happened* to the scope — a search recorded decisions, a release was
/// grabbed or went pending, the user paused it — never because a sweep
/// materialized it. What to search is the derived target set
/// (`AcquisitionTarget`); this is the ledger of grabs, scores, and user
/// intent layered on top of it. `last_search_at` is state, not cadence: it
/// feeds the upgrade cooldown and failed-grab staleness checks. The persisted
/// table may still be named `wanted_items`; do not treat that legacy storage
/// name as permission to seed target-truth rows.
#[derive(Clone, Debug)]
pub struct AcquisitionScopeState {
    pub id: String,
    pub title_id: String,
    pub title_name: Option<String>,
    pub title_slug: Option<String>,
    pub title_facet: Option<String>,
    pub library_id: Option<String>,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub series_movie_link_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub media_type: String,
    pub last_search_at: Option<String>,
    pub status: AcquisitionScopeStatus,
    pub grabbed_release: Option<String>,
    pub current_score: Option<i32>,
    pub latest_release_decision: Option<ReleaseDecision>,
    pub mismatch_recovery_eligible: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct ReleaseDecision {
    pub id: String,
    pub wanted_item_id: String,
    pub title_id: String,
    pub release_title: String,
    pub release_url: Option<String>,
    pub release_size_bytes: Option<i64>,
    pub decision_code: String,
    pub candidate_score: i32,
    pub current_score: Option<i32>,
    pub score_delta: Option<i32>,
    pub explanation_json: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct DecisionCodeCount {
    pub code: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct WantedStatusCount {
    pub status: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct PendingReleaseStatusCount {
    pub status: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct TitleAcquisitionDiagnostics {
    pub recent_decisions: Vec<ReleaseDecision>,
    pub decision_counts: Vec<DecisionCodeCount>,
    pub wanted_status_counts: Vec<WantedStatusCount>,
    pub pending_release_counts: Vec<PendingReleaseStatusCount>,
    pub mismatch_recovery_eligible_count: i64,
    pub latest_decision_at: Option<String>,
    pub latest_wanted_search_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PendingRelease {
    pub id: String,
    pub wanted_item_id: String,
    pub title_id: String,
    pub release_title: String,
    pub release_url: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub release_size_bytes: Option<i64>,
    pub release_score: i32,
    pub scoring_log_json: Option<String>,
    pub indexer_source: Option<String>,
    pub release_guid: Option<String>,
    pub added_at: String,
    pub delay_until: String,
    pub status: PendingReleaseStatus,
    pub grabbed_at: Option<String>,
    /// Password hint for protected NZBs (e.g. NZBGeek password field).
    pub source_password: Option<String>,
    /// RFC3339 publish date — used for `is_recent` queue priority calculation.
    pub published_at: Option<String>,
    /// Torrent info hash — passed to download client for magnet resolution.
    pub info_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReleaseStatus {
    Waiting,
    Standby,
    Processing,
    Grabbed,
    Superseded,
    Expired,
    Dismissed,
    /// Parked for a human decision (Pillar A3): the best candidate for the scope
    /// was rejected as `ambiguous_identity`. Carries no delay-timer semantics and
    /// is never auto-promoted — only an explicit grab-now or dismiss resolves it.
    NeedsReview,
}

impl PendingReleaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Standby => "standby",
            Self::Processing => "processing",
            Self::Grabbed => "grabbed",
            Self::Superseded => "superseded",
            Self::Expired => "expired",
            Self::Dismissed => "dismissed",
            Self::NeedsReview => "needs_review",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "waiting" => Some(Self::Waiting),
            "standby" => Some(Self::Standby),
            "processing" => Some(Self::Processing),
            "grabbed" => Some(Self::Grabbed),
            "superseded" => Some(Self::Superseded),
            "expired" => Some(Self::Expired),
            "dismissed" => Some(Self::Dismissed),
            "needs_review" => Some(Self::NeedsReview),
            _ => None,
        }
    }

    /// True for statuses the pending-releases view still lists as open work.
    /// `needs_review` joins `waiting` in the listing base set so a parked row is
    /// visible; it deliberately stays out of the delay-expiry processor.
    pub fn is_open_for_review(self) -> bool {
        matches!(self, Self::Waiting | Self::NeedsReview)
    }
}

#[cfg(test)]
mod pending_release_status_tests {
    use super::PendingReleaseStatus;

    #[test]
    fn pending_release_status_round_trips() {
        let statuses = [
            PendingReleaseStatus::Waiting,
            PendingReleaseStatus::Standby,
            PendingReleaseStatus::Processing,
            PendingReleaseStatus::Grabbed,
            PendingReleaseStatus::Superseded,
            PendingReleaseStatus::Expired,
            PendingReleaseStatus::Dismissed,
            PendingReleaseStatus::NeedsReview,
        ];

        for status in statuses {
            assert_eq!(PendingReleaseStatus::parse(status.as_str()), Some(status));
        }
    }

    #[test]
    fn pending_release_status_rejects_unknown_values() {
        assert_eq!(PendingReleaseStatus::parse("unknown"), None);
    }
}

#[derive(Clone, Debug)]
pub struct DownloadGrabResult {
    pub job_id: String,
    pub client_id: Option<String>,
    pub client_type: String,
    pub info_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadSourceKind {
    /// Direct NZB file upload (binary content sent to the download client).
    NzbFile,
    /// URL to an NZB that the download client fetches itself.
    NzbUrl,
    TorrentFile,
    MagnetUri,
}

impl DownloadSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NzbFile => "nzb",
            Self::NzbUrl => "nzb_url",
            Self::TorrentFile => "torrent_file",
            Self::MagnetUri => "magnet_uri",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "nzb" | "nzb_file" => Some(Self::NzbFile),
            "nzb_url" => Some(Self::NzbUrl),
            "torrent" | "torrent_file" | "torrent_url" | "torrent_bytes" => Some(Self::TorrentFile),
            "magnet" | "magnet_uri" => Some(Self::MagnetUri),
            _ => None,
        }
    }

    pub fn infer_from_hint(value: Option<&str>) -> Option<Self> {
        let raw = value?.trim();
        if raw.is_empty() {
            return None;
        }
        if raw.starts_with("magnet:") {
            return Some(Self::MagnetUri);
        }

        let normalized = raw.to_ascii_lowercase();
        if normalized.ends_with(".torrent") {
            return Some(Self::TorrentFile);
        }
        if normalized.ends_with(".nzb") {
            return Some(Self::NzbUrl);
        }

        reqwest::Url::parse(raw).ok().and_then(|url| {
            let path = url.path().to_ascii_lowercase();
            if path.ends_with(".torrent") {
                return Some(Self::TorrentFile);
            }
            if path.ends_with(".nzb") {
                return Some(Self::NzbUrl);
            }

            url.query_pairs().find_map(|(key, value)| {
                let value = value.trim();
                match key.as_ref() {
                    "magnet" | "magnet_uri" if value.starts_with("magnet:") => {
                        Some(Self::MagnetUri)
                    }
                    "torrent" | "torrent_url" | "file" | "url" if value.ends_with(".torrent") => {
                        Some(Self::TorrentFile)
                    }
                    "nzb" | "nzb_url" | "url" if value.ends_with(".nzb") => Some(Self::NzbUrl),
                    _ => None,
                }
            })
        })
    }

    pub fn infer_from_indexer_result(
        plugin_type: Option<&str>,
        download_url: Option<&str>,
        link: Option<&str>,
        extra: &HashMap<String, serde_json::Value>,
    ) -> Option<Self> {
        if let Some(kind) = extra
            .get("download_type")
            .and_then(|value| value.as_str())
            .and_then(Self::parse)
        {
            return Some(kind);
        }
        if extra.contains_key("magnet_uri") {
            return Some(Self::MagnetUri);
        }
        if extra.contains_key("info_hash") {
            return Some(Self::TorrentFile);
        }
        if let Some(kind) = Self::infer_from_hint(download_url.or(link)) {
            return Some(kind);
        }

        match plugin_type.map(|value| value.trim().to_ascii_lowercase()) {
            Some(plugin_type) if plugin_type == "torrent_indexer" => Some(Self::TorrentFile),
            Some(plugin_type) if plugin_type == "usenet_indexer" => Some(Self::NzbUrl),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleaseDownloadAttemptOutcome {
    Success,
    Failed,
    Pending,
}

impl ReleaseDownloadAttemptOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Pending => "pending",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseDownloadFailureSignature {
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleReleaseBlocklistEntry {
    pub id: String,
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
    pub error_message: Option<String>,
    pub attempted_at: String,
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseSearchSubjectKind {
    Title,
    Episode,
    Season,
    Freetext,
    Rss,
}

impl ReleaseSearchSubjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Episode => "episode",
            Self::Season => "season",
            Self::Freetext => "freetext",
            Self::Rss => "rss",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseStrategyKind {
    IdBacked,
    Freetext,
    Fallback,
    RssFeed,
}

impl ReleaseStrategyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IdBacked => "id_backed",
            Self::Freetext => "freetext",
            Self::Fallback => "fallback",
            Self::RssFeed => "rss_feed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseCandidateProvenance {
    pub search_subject_kind: ReleaseSearchSubjectKind,
    pub strategy_kind: ReleaseStrategyKind,
    pub title_validated_upstream: bool,
}

/// Indexer-asserted attributes read off a newznab/torznab response item: the
/// `tvdbid`/`tmdbid`/`imdbid` attrs and the item's categories (raw numeric
/// newznab ids and/or names, in indexer order). Both are *indexer* claims of the
/// same trust tier — they feed the identity disambiguator (A2(2)) and the
/// category veto (D2), never a title match on their own. Empty whenever the
/// indexer asserted nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexerResponseAttributes {
    pub tvdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub categories: Vec<String>,
}

impl IndexerResponseAttributes {
    /// Whether the indexer asserted any external id on this result.
    pub fn has_external_ids(&self) -> bool {
        self.tvdb_id.is_some() || self.tmdb_id.is_some() || self.imdb_id.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexerSearchResult {
    pub indexer_id: Option<String>,
    pub source: String,
    pub title: String,
    pub link: Option<String>,
    pub download_url: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub size_bytes: Option<i64>,
    pub published_at: Option<String>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub indexer_languages: Option<Vec<String>>,
    pub indexer_subtitles: Option<Vec<String>>,
    pub indexer_grabs: Option<i64>,
    pub password_hint: Option<String>,
    pub parsed_release_metadata: Option<ParsedReleaseMetadata>,
    pub quality_profile_decision: Option<QualityProfileDecision>,
    /// Arbitrary indexer-specific metadata from WASM plugins.
    /// Passed through to OPA scoring as `input.release.extra`.
    pub extra: HashMap<String, serde_json::Value>,
    /// Typed counterpart to `extra` for the response attrs the auto evaluator
    /// consumes directly.
    pub response_attributes: IndexerResponseAttributes,
    pub guid: Option<String>,
    pub info_url: Option<String>,
    pub provenance: Option<ReleaseCandidateProvenance>,
    pub candidate_token: Option<String>,
    pub queue_scope: Option<SubmissionScope>,
    pub auto_eligible: Option<bool>,
    pub auto_decision_code: Option<String>,
    pub auto_decision_summary: Option<String>,
}

/// Per-indexer outcome of a single search query. Determines
/// which routed indexers may be recorded as coverage: only an indexer that actually
/// fired a query and returned a response (empty included) counts — never one the
/// scheduler deferred/skipped, and never one whose query errored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexerSearchOutcome {
    /// The query executed and returned a response — `empty` distinguishes a
    /// zero-result response (still coverage) from a populated one.
    Fired { empty: bool },
    /// The scheduler declined to query this indexer this cycle (deferred/skipped:
    /// destination cooldown, host-RPS, account quota, disabled) — not queried.
    Skipped,
    /// The query was attempted but failed (rate-limited / transport / provider).
    Errored,
}

impl IndexerSearchOutcome {
    /// Whether this indexer actually executed a query and returned a response.
    /// Only a fired indexer may be recorded as convergence coverage.
    pub fn fired(&self) -> bool {
        matches!(self, Self::Fired { .. })
    }
}

/// A single indexer's outcome within an [`IndexerSearchResponse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerQueryOutcome {
    pub indexer_id: String,
    pub outcome: IndexerSearchOutcome,
}

/// Wrapper around search results that also carries API limit metadata
/// from the indexer response.
#[derive(Clone, Debug)]
pub struct IndexerSearchResponse {
    pub results: Vec<IndexerSearchResult>,
    pub api_current: Option<u32>,
    pub api_max: Option<u32>,
    pub grab_current: Option<u32>,
    pub grab_max: Option<u32>,
    /// Per-indexer outcomes for this query: which routed indexers fired
    /// (empty or not), were skipped/deferred, or errored. Empty for synthetic or
    /// no-eligible-indexer responses.
    pub indexer_outcomes: Vec<IndexerQueryOutcome>,
}

#[derive(Clone, Debug)]
pub struct JwtAuthConfig {
    pub issuer: String,
    pub access_ttl_seconds: usize,
    pub jwt_signing_salt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebauthnChallengeType {
    Registration,
    Authentication,
}

impl WebauthnChallengeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Registration => "registration",
            Self::Authentication => "authentication",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "registration" => Some(Self::Registration),
            "authentication" => Some(Self::Authentication),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebauthnCredentialRecord {
    pub id: String,
    pub user_id: String,
    pub credential_id: String,
    pub credential_json: String,
    pub friendly_name: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebauthnChallengeRecord {
    pub id: String,
    pub user_id: Option<String>,
    pub challenge_type: WebauthnChallengeType,
    pub state_json: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebauthnChallengeStart {
    pub challenge_id: String,
    pub options_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasskeySummary {
    pub id: String,
    pub friendly_name: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotpCredentialRecord {
    pub id: String,
    pub user_id: String,
    pub secret_base32: String,
    pub algorithm: String,
    pub digits: i32,
    pub period_seconds: i32,
    pub last_accepted_step: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotpEnrollmentChallengeRecord {
    pub id: String,
    pub user_id: String,
    pub secret_base32: String,
    pub algorithm: String,
    pub digits: i32,
    pub period_seconds: i32,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotpRecoveryCodeRecord {
    pub id: String,
    pub user_id: String,
    pub code_hash: String,
    pub created_at: String,
    pub used_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotpFailedAttemptRecord {
    pub id: String,
    pub user_id: String,
    pub attempted_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserAuthFactorStatus {
    pub has_mfa: bool,
    pub has_passkey: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotpStatus {
    pub enabled: bool,
    pub created_at: Option<String>,
    pub last_used_at: Option<String>,
    pub recovery_codes_remaining: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotpEnrollmentStart {
    pub challenge_id: String,
    pub otpauth_url: String,
    pub secret_base32: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotpEnrollmentComplete {
    pub status: TotpStatus,
    pub recovery_codes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginFailureTimingClass {
    PasswordBackedLocal,
    FastMasked,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JwtSessionScope {
    #[default]
    Full,
    MfaEnrollment,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthenticatedTokenClaims {
    pub mfa_verified_until: Option<i64>,
    pub mfa_step_up_verified_until: Option<i64>,
    pub session_scope: JwtSessionScope,
    pub oauth_client_id: Option<String>,
    pub oauth_grant_id: Option<String>,
    pub oauth_authorization_source: OAuthAuthorizationSource,
    pub actor_capabilities: scryer_domain::ActorCapabilityMask,
}

impl AuthenticatedTokenClaims {
    pub fn is_oauth_access_token(&self) -> bool {
        self.oauth_client_id.is_some() && self.oauth_grant_id.is_some()
    }

    pub fn has_partial_oauth_marker(&self) -> bool {
        self.oauth_client_id.is_some() ^ self.oauth_grant_id.is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthAuthorizationSource {
    #[default]
    Authenticated,
    Authless,
}

impl OAuthAuthorizationSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authenticated => "authenticated",
            Self::Authless => "authless",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "authless" => Self::Authless,
            _ => Self::Authenticated,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        matches!(self, Self::Authenticated)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthAuthorizationCodeRecord {
    pub id: String,
    pub code_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub authorization_source: OAuthAuthorizationSource,
    pub redirect_uri: String,
    pub scope: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub consumed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthRefreshGrantRecord {
    pub id: String,
    pub family_id: String,
    pub user_id: String,
    pub authorization_source: OAuthAuthorizationSource,
    pub client_id: String,
    pub scope: String,
    pub auth_session_version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthRefreshTokenRecord {
    pub id: String,
    pub grant_id: String,
    pub family_id: String,
    pub token_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub consumed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthRefreshRotation {
    pub grant: OAuthRefreshGrantRecord,
    pub previous_token: OAuthRefreshTokenRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OAuthRefreshRotationOutcome {
    Rotated(Box<OAuthRefreshRotation>),
    Unavailable,
    Reused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthConnectedAppRecord {
    pub grant_id: String,
    pub client_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct JwtClaims {
    pub sub: String,
    pub exp: i64,
    pub iat: i64,
    pub iss: String,
    #[serde(default)]
    pub username: String,
    #[serde(default, rename = "appPermissions")]
    pub app_permissions: Vec<String>,
    #[serde(default, rename = "libraryPermissions")]
    pub library_permissions: Vec<JwtLibraryPermissionClaim>,
    #[serde(default, rename = "mfaVerifiedUntil")]
    pub mfa_verified_until: Option<i64>,
    #[serde(default, rename = "mfaStepUpVerifiedUntil")]
    pub mfa_step_up_verified_until: Option<i64>,
    #[serde(default, rename = "authScope")]
    pub auth_scope: JwtSessionScope,
    #[serde(default, rename = "oauthClientId")]
    pub oauth_client_id: Option<String>,
    #[serde(default, rename = "oauthGrantId")]
    pub oauth_grant_id: Option<String>,
    #[serde(
        default,
        rename = "oauthAuthorizationSource",
        skip_serializing_if = "OAuthAuthorizationSource::is_authenticated"
    )]
    pub oauth_authorization_source: OAuthAuthorizationSource,
    #[serde(default, rename = "actorCapabilities")]
    pub actor_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JwtLibraryPermissionClaim {
    #[serde(rename = "libraryId")]
    pub library_id: String,
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ReleaseCandidateTokenClaims {
    pub sub: String,
    pub exp: i64,
    pub iat: i64,
    pub iss: String,
    pub kind: String,
    pub title_id: String,
    pub scope_kind: String,
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexer_id: Option<String>,
    pub source_hint: String,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BackupDownloadTokenClaims {
    pub sub: String,
    pub exp: i64,
    pub iat: i64,
    pub iss: String,
    pub kind: String,
    pub filename: String,
}

/// Lightweight summary of the collection that should represent a title in list
/// views, used to avoid N+1 queries when listing titles with their current
/// collection label.
#[derive(Clone, Debug)]
pub struct PrimaryCollectionSummary {
    pub title_id: String,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
}

/// Aggregated media-file byte totals per title, used by title list views.
#[derive(Clone, Debug)]
pub struct TitleMediaSizeSummary {
    pub title_id: String,
    pub total_size_bytes: i64,
}

/// Aggregated current quality tier per title, based on the lowest-quality live
/// media file linked to the title.
#[derive(Clone, Debug)]
pub struct TitleQualitySummary {
    pub title_id: String,
    pub quality_tier: String,
}

/// Primary movie media technical summary for title list projections.
#[derive(Clone, Debug)]
pub struct TitleMovieMediaSummary {
    pub title_id: String,
    pub resolution: Option<String>,
    pub hdr_format: Option<String>,
    pub audio_codec: Option<String>,
}

/// Aggregated current quality tier per movie title or per episodic item, based
/// on the lowest-quality live media file linked to that item.
#[derive(Clone, Debug)]
pub struct CutoffUnmetQualitySummary {
    pub title_id: String,
    pub episode_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub quality_tier: String,
}

/// The two derived acquisition-target kinds. `Missing` scopes have
/// no primary file; `CutoffUpgrade` scopes have a file whose quality is strictly
/// below the effective profile cutoff. Both converge the same way — they differ
/// only in which derived query produces them and in recency lane (upgrades are
/// always cold: the file already plays).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WantedKind {
    Missing,
    CutoffUpgrade,
}

impl WantedKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::CutoffUpgrade => "cutoff_upgrade",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "missing" => Some(Self::Missing),
            "cutoff_upgrade" => Some(Self::CutoffUpgrade),
            _ => None,
        }
    }
}

/// A monitored episode with no live primary media file — a raw candidate for the
/// derived missing-target set. Policy gates (air-date window, recency lane) are
/// applied by the application layer.
#[derive(Clone, Debug)]
pub struct MissingEpisodeCandidate {
    pub episode_id: String,
    pub title_id: String,
    pub library_id: String,
    pub title_facet: String,
    pub collection_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub air_date: Option<String>,
    pub title_created_at: String,
}

/// A monitored title with no live primary media file at all. The application
/// layer keeps only movie-shaped facets (episodic facets are covered per
/// episode) and applies the minimum-availability gate.
#[derive(Clone, Debug)]
pub struct MissingTitleCandidate {
    pub title_id: String,
    pub library_id: String,
    pub title_facet: String,
    pub min_availability: Option<String>,
    pub first_aired: Option<String>,
    pub digital_release_date: Option<String>,
    pub created_at: String,
}

/// A monitored series-movie link with no linked live media file. The
/// application layer applies the filler opt-in gate.
#[derive(Clone, Debug)]
pub struct MissingSeriesMovieLinkCandidate {
    pub series_movie_link_id: String,
    pub title_id: String,
    pub library_id: String,
    pub title_facet: String,
    pub continuity_status: Option<String>,
    pub movie_digital_release_date: Option<String>,
    pub link_created_at: String,
}

/// Raw candidates for the derived missing-target set: monitored, fileless
/// scopes straight from library state, in one sweep.
#[derive(Clone, Debug, Default)]
pub struct MissingScopeCandidates {
    pub episodes: Vec<MissingEpisodeCandidate>,
    pub titles: Vec<MissingTitleCandidate>,
    pub series_movie_links: Vec<MissingSeriesMovieLinkCandidate>,
}

/// Aggregated episode progress counts per title, excluding specials.
#[derive(Clone, Debug)]
pub struct TitleEpisodeProgressSummary {
    pub title_id: String,
    pub owned_episodes: i64,
    pub monitored_episodes: i64,
    pub total_episodes: i64,
}

/// Aggregated episode progress counts per collection.
#[derive(Clone, Debug)]
pub struct CollectionEpisodeProgressSummary {
    pub collection_id: String,
    pub owned_episodes: i64,
    pub monitored_episodes: i64,
    pub total_episodes: i64,
}

#[derive(Clone, Debug)]
pub struct DiskSpaceInfo {
    pub path: String,
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePathStyle {
    Unix,
    Windows,
}

impl RuntimePathStyle {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

#[cfg(test)]
mod runtime_path_style_tests {
    use super::RuntimePathStyle;

    #[test]
    #[cfg(unix)]
    fn current_is_unix_on_unix() {
        assert_eq!(RuntimePathStyle::current(), RuntimePathStyle::Unix);
    }

    #[test]
    #[cfg(windows)]
    fn current_is_windows_on_windows() {
        assert_eq!(RuntimePathStyle::current(), RuntimePathStyle::Windows);
    }
}

#[derive(Clone, Debug)]
pub struct SystemHealth {
    pub service_ready: bool,
    pub db_path: String,
    pub datastore_engine: String,
    pub datastore_migration_key: Option<String>,
    pub runtime_path_style: RuntimePathStyle,
    pub total_titles: usize,
    pub monitored_titles: usize,
    pub total_users: usize,
    pub titles_movie: usize,
    pub titles_series: usize,
    pub titles_anime: usize,
    pub titles_other: usize,
    pub recent_events: usize,
    pub recent_event_preview: Vec<String>,
    pub db_migration_version: Option<String>,
    pub indexer_stats: Vec<IndexerQueryStats>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SmgVersionCompatibilityNotice {
    pub status: String,
    pub minimum_version: String,
    pub your_version: String,
    pub message: String,
    pub upgrade_deadline: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SmgScryerUpdateNotice {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub latest_tag: String,
    pub release_url: Option<String>,
    pub published_at: Option<String>,
    pub checked_at: String,
}

#[derive(Clone, Debug)]
pub struct IndexerQueryStats {
    pub indexer_id: String,
    pub indexer_name: String,
    pub queries_last_24h: u32,
    pub successful_last_24h: u32,
    pub failed_last_24h: u32,
    pub last_query_at: Option<String>,
    pub api_current: Option<u32>,
    pub api_max: Option<u32>,
    pub grab_current: Option<u32>,
    pub grab_max: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    Creating,
    Ready,
    Invalid,
    Failed,
}

impl BackupStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Invalid => "invalid",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupTrigger {
    #[default]
    Manual,
    Auto,
}

impl BackupTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupInfo {
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String,
    pub format_version: String,
    #[serde(default)]
    pub source_scryer_version: String,
    pub source_engine: String,
    pub source_migration_key: Option<String>,
    pub encrypted: bool,
    pub row_counts: BTreeMap<String, u64>,
    #[serde(default)]
    pub trigger: BackupTrigger,
    pub status: BackupStatus,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupDownloadTicket {
    pub token: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthCheckStatus {
    Ok,
    Warning,
    Error,
}

impl HealthCheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HealthCheckResult {
    pub source: String,
    pub status: HealthCheckStatus,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct HousekeepingReport {
    pub orphaned_media_files: u32,
    pub stale_release_decisions: u32,
    pub stale_release_attempts: u32,
    pub stale_history_events: u32,
    pub stale_history_records: u32,
    pub staged_nzb_artifacts_pruned: u32,
    pub recycled_purged: u32,
    pub discovery_pruned_runs: u32,
    pub ran_at: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiTheme {
    Light,
    #[default]
    Dark,
    Pride,
    System,
}

impl UiTheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Pride => "pride",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            "pride" => Some(Self::Pride),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiDateTimeFormat {
    #[default]
    Locale,
    Iso24h,
}

impl UiDateTimeFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Locale => "locale",
            Self::Iso24h => "iso24h",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "locale" => Some(Self::Locale),
            "iso24h" => Some(Self::Iso24h),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiDensity {
    Compact,
    #[default]
    Comfortable,
}

impl UiDensity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "compact" => Some(Self::Compact),
            "comfortable" => Some(Self::Comfortable),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiSidebarMode {
    Collapsed,
    #[default]
    Expanded,
}

impl UiSidebarMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collapsed => "collapsed",
            Self::Expanded => "expanded",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "collapsed" => Some(Self::Collapsed),
            "expanded" => Some(Self::Expanded),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiDefaultLandingView {
    #[default]
    Movies,
    Series,
    Anime,
    Activity,
    Calendar,
    Wanted,
    History,
    Settings,
    System,
}

impl UiDefaultLandingView {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Series => "series",
            Self::Anime => "anime",
            Self::Activity => "activity",
            Self::Calendar => "calendar",
            Self::Wanted => "wanted",
            Self::History => "history",
            Self::Settings => "settings",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "movies" => Some(Self::Movies),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            "activity" => Some(Self::Activity),
            "calendar" => Some(Self::Calendar),
            "wanted" => Some(Self::Wanted),
            "history" => Some(Self::History),
            "settings" => Some(Self::Settings),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiSettingsFacet {
    Movies,
    Series,
    Anime,
}

impl UiSettingsFacet {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "movies" | "movie" => Some(Self::Movies),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiTableViewMode {
    Compact,
    PosterTable,
}

impl UiTableViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::PosterTable => "poster-table",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "compact" => Some(Self::Compact),
            "poster-table" => Some(Self::PosterTable),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiTableColumnSetting {
    pub facet: UiSettingsFacet,
    pub table_view_mode: UiTableViewMode,
    pub column_id: String,
    pub column_order: i32,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiSettings {
    pub user_id: String,
    pub theme: UiTheme,
    pub date_time_format: UiDateTimeFormat,
    pub highlight_color: Option<String>,
    pub secondary_color: Option<String>,
    pub high_contrast_mode: bool,
    pub reduce_motion: bool,
    pub hide_sponsor_button: bool,
    pub density: UiDensity,
    pub sidebar_mode: UiSidebarMode,
    pub default_landing_view: UiDefaultLandingView,
    pub table_columns: Vec<UiTableColumnSetting>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiSettingsUpdate {
    pub theme: UiTheme,
    pub date_time_format: UiDateTimeFormat,
    pub highlight_color: Option<String>,
    pub secondary_color: Option<String>,
    pub high_contrast_mode: bool,
    pub reduce_motion: bool,
    pub hide_sponsor_button: bool,
    pub density: UiDensity,
    pub sidebar_mode: UiSidebarMode,
    pub default_landing_view: UiDefaultLandingView,
    pub table_columns: Vec<UiTableColumnSetting>,
}

impl UiSettings {
    pub fn defaults_for_user(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            theme: UiTheme::default(),
            date_time_format: UiDateTimeFormat::default(),
            highlight_color: None,
            secondary_color: None,
            high_contrast_mode: false,
            reduce_motion: false,
            hide_sponsor_button: false,
            density: UiDensity::default(),
            sidebar_mode: UiSidebarMode::default(),
            default_landing_view: UiDefaultLandingView::default(),
            table_columns: Vec::new(),
            created_at: None,
            updated_at: None,
        }
    }
}

/// A canonical, server-owned root for a completed download that may need manual import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualImportSourceRegistration {
    pub source_identity: crate::DownloadSourceIdentity,
    pub trusted_root: String,
}

/// A server-owned file candidate. `canonical_path` never crosses the public API boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualImportSelectionCandidate {
    pub id: String,
    pub canonical_path: String,
    pub quality: Option<String>,
}

/// A durable selection of files rooted in a tracked completed download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualImportSelection {
    pub id: String,
    pub actor_user_id: String,
    pub title_id: String,
    pub source: ManualImportSourceRegistration,
    pub candidates: Vec<ManualImportSelectionCandidate>,
}

#[cfg(test)]
mod tests {
    use super::{
        ExternalIdHint, ExternalIdProvider, LibraryScanHint, LibraryScanHintFacet,
        LibraryScanHintSet, LibraryScanHintSource, library_scan_file_full_path_key,
        library_scan_file_leaf_key, library_scan_folder_leaf_key,
    };

    #[test]
    fn library_scan_leaf_keys_ignore_root_and_separator_style() {
        let unix = library_scan_file_leaf_key(
            "/mnt/media/Foundation (2021)/Season 01/Foundation.S01E01.mkv",
        );
        let windows = library_scan_file_leaf_key(
            r"D:\Series\Foundation (2021)\Season 01\Foundation.S01E01.mkv",
        );
        assert_eq!(unix, windows);

        assert_eq!(
            library_scan_folder_leaf_key("/mnt/media/Foundation (2021)"),
            library_scan_folder_leaf_key(r"D:\Series\Foundation (2021)")
        );
    }

    #[test]
    fn library_scan_hint_set_resolves_conflicting_leaf_key_by_full_path() {
        let first_path = "/mnt/media/Foundation (2021)/Season 01/Foundation.S01E01.mkv";
        let second_path = "/other/Foundation (2021)/Season 01/Foundation.S01E01.mkv";
        let path_key = library_scan_file_leaf_key(first_path).expect("leaf key");
        let mut hints = LibraryScanHintSet::new();
        hints.push(LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportSonarr,
            facet: LibraryScanHintFacet::Series,
            path_key: path_key.clone(),
            full_path_key: library_scan_file_full_path_key(first_path),
            ids: vec![ExternalIdHint {
                provider: ExternalIdProvider::Tvdb,
                value: "366972".to_string(),
            }],
        });
        hints.push(LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportSonarr,
            facet: LibraryScanHintFacet::Series,
            path_key: path_key.clone(),
            full_path_key: library_scan_file_full_path_key(second_path),
            ids: vec![ExternalIdHint {
                provider: ExternalIdProvider::Tvdb,
                value: "999999".to_string(),
            }],
        });

        assert!(
            hints
                .hint_for_stored_path(LibraryScanHintFacet::Series, &path_key)
                .is_none()
        );
        let hint = hints
            .hint_for_scan_path(
                LibraryScanHintFacet::Series,
                &path_key,
                library_scan_file_full_path_key(first_path).as_deref(),
            )
            .expect("full path resolves conflict");
        assert_eq!(hint.ids[0].value, "366972");
    }
}
