use std::collections::{BTreeMap, HashMap};

use chrono::DateTime;
use scryer_domain::{
    CanonicalMediaTag, DownloadQueueCommandAction, ExternalId, IndexerConfig, TaggedAlias, Title,
    download_identity::DownloadId,
};
pub use scryer_domain::{TitleCredit, TitleExternalRating, TitleRatingSummary};
use serde::{Deserialize, Serialize};

use crate::acquisition::seed_goals::ReleaseSeedMinimums;
use crate::library_scan::LibraryScanSummary;
use crate::quality_profile::QualityProfileDecision;
use crate::release_parser::{ParsedReleaseMetadata, VideoCodec};
use crate::{AppResult, SubmissionScope};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryRootDraft {
    pub path: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MetadataFieldUpdate<T> {
    #[default]
    Unchanged,
    Set(T),
    Clear,
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
    /// Authoritative original-language update from the metadata provider.
    pub language: MetadataFieldUpdate<String>,
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
    /// Complete credit set returned by SMG. `Some(vec![])` clears the stale cache;
    /// `None` leaves the existing rows untouched for callers that do not hydrate.
    pub credits: Option<Vec<TitleCredit>>,
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

/// How thoroughly a download-client completed-download copy is proven before
/// its source may be touched (FR-042).
///
/// Location operations do not read it: `LOCATION_OPERATION_VERIFICATION_DEPTH`
/// forces full depth for every library move, root change, and consolidation,
/// because those relocate the user's only copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationSettings {
    pub depth: crate::location::model::VerificationDepth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateVerificationSettings {
    pub depth: crate::location::model::VerificationDepth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycledItem {
    pub id: String,
    pub original_path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub title_id: Option<String>,
    pub title_name: Option<String>,
    pub reason: String,
    pub recycled_at: String,
    pub scheduled_deletion_at: String,
    pub media_root: String,
    pub library_id: String,
    pub library_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecycleRestoreConflictPolicy {
    KeepBoth,
    ReplaceExisting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycleRestorePreviewItem {
    pub id: String,
    pub original_path: String,
    pub destination_occupied: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycleRestorePreview {
    pub fingerprint: String,
    pub items: Vec<RecycleRestorePreviewItem>,
}

#[derive(Clone, Debug)]
pub struct RecycleBinBatchJobAccepted {
    pub entry_ids: Vec<String>,
    pub job_run: crate::JobRun,
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
    pub canonical_download_id: Option<scryer_domain::download_identity::DownloadId>,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaFileAssociations {
    pub episode_ids: Vec<String>,
    pub series_movie_link_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TitleMediaFile {
    pub id: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub series_movie_link_ids: Vec<String>,
    pub file_path: String,
    pub size_bytes: i64,
    /// The announced (indexer-advertised) size this row was scored on, kept only
    /// when the import actually scored on it: the file landed within the normal
    /// overhead band of what its release advertised
    /// (`canonical_scoring::size_basis_bytes`). `None` means the row is scored
    /// on its real size — every row written before the column existed, scanned
    /// files, adopted downloads, and files that landed short of the band.
    pub announced_size_bytes: Option<i64>,
    pub role: crate::MediaFileRole,
    pub source_signature_scheme: Option<String>,
    pub source_signature_value: Option<String>,
    /// Persisted full-file hashes (migration 0205, FR-041/046/047), separate
    /// from the sampled head+tail proof above. `None` when nothing has read the
    /// file end to end yet, or when a scan invalidated the stored values.
    pub content_hashes: Option<crate::location::model::PersistedContentHashes>,
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
    pub dovi_profile: Option<u8>,
    pub dovi_bl_compat_id: Option<u8>,
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
    pub title_role: crate::MediaFileRole,
    pub episode_ids: Vec<String>,
    pub primary_episode_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DownloadHistoryPage {
    pub items: Vec<scryer_domain::DownloadQueueItem>,
    pub has_more: bool,
    pub total_count: usize,
    pub available_clients: Vec<DownloadClientFilterOption>,
}

#[derive(Clone, Debug)]
pub struct DownloadQueuePage {
    pub items: Vec<scryer_domain::DownloadQueueItem>,
    pub has_more: bool,
    pub total_count: usize,
    pub available_clients: Vec<DownloadClientFilterOption>,
    pub revision: u64,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub ready: bool,
    pub stale: bool,
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
    ImportedSeeding,
    Failed,
    /// The client reports a recoverable problem; the row stays in the activity
    /// list with its message instead of being presented as a dead grab.
    Warning,
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
    Seeding,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadImportFilter {
    All,
    Attention,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DownloadHistorySortKey {
    Title,
    Client,
    Status,
    Progress,
    Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// Coarse bucket derived from `reason`, for UI grouping and filtering.
    pub reason_class: PendingImportReasonClass,
    pub search_attempts: Vec<PendingImportSearchAttempt>,
    /// Size of the pending file: the size the scanner persisted, else a
    /// filesystem stat taken while assembling the page. `None` when the item is
    /// a folder or the file is no longer readable.
    pub size_bytes: Option<i64>,
    pub created_at: String,
}

/// Coarse classification of why an item is awaiting import resolution.
///
/// The free-text `reason` stays the authoritative scanner code; this is the
/// stable bucket the dashboard groups by, so new scanner codes land in
/// [`PendingImportReasonClass::Other`] instead of breaking the API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PendingImportReasonClass {
    /// Metadata lookup returned no candidates at all.
    Unmatched,
    /// Candidates were returned but none could be accepted automatically.
    Ambiguous,
    /// The file's media metadata could not be read, so quality is unknown.
    QualityUnknown,
    /// Any other scanner reason code.
    #[default]
    Other,
}

impl PendingImportReasonClass {
    /// Bucket a scanner `reason_code`. Unknown codes classify as
    /// [`PendingImportReasonClass::Other`] rather than erroring, so the scanner
    /// can add codes without a coordinated API change.
    pub fn from_reason_code(reason_code: &str) -> Self {
        match reason_code.trim() {
            // A lookup ran and produced nothing to choose from.
            "no_metadata_search_results" | "no_metadata_match" | "episode_lookup_failed" => {
                Self::Unmatched
            }
            // A lookup produced candidates but none could be accepted.
            "no_acceptable_metadata_match" => Self::Ambiguous,
            // Media analysis failed, so the file's quality is unknown.
            "skipped_file_metadata_unreadable" => Self::QualityUnknown,
            // `episode_identity_missing`, `skipped_unusable_title_evidence`, and
            // `title_already_owns_another_folder` are parse/ownership problems
            // rather than match outcomes, so they fall through to `Other`.
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unmatched => "unmatched",
            Self::Ambiguous => "ambiguous",
            Self::QualityUnknown => "quality_unknown",
            Self::Other => "other",
        }
    }
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
    pub grabbed_release: String,
}

#[derive(Clone, Debug)]
pub struct AcquisitionScopeCompleteTransition {
    pub id: String,
    pub last_search_at: Option<String>,
    pub grabbed_release: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AcquisitionScopePauseTransition {
    pub id: String,
    pub last_search_at: Option<String>,
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
    /// Size of the unmatched file when the scanner knew it. `None` for
    /// folder-shaped items and for rows recorded before the column existed;
    /// readers fall back to a filesystem stat rather than backfilling.
    pub size_bytes: Option<i64>,
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
    /// The bar this scope's landed file sets, resolved from the library rather
    /// than stored. `None` when nothing occupies the scope.
    pub landed_bar: Option<i32>,
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

/// Arbitration is independent of lifecycle: a fallback can remain active while
/// a primary waits for its delay to elapse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingReleaseRole {
    Primary,
    Fallback,
}

impl PendingReleaseRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Fallback => "fallback",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "primary" => Some(Self::Primary),
            "fallback" => Some(Self::Fallback),
            _ => None,
        }
    }
}

/// Immutable facts from the current indexer observation. They are deliberately
/// separate from lifecycle fields on `PendingRelease` so rediscovery cannot
/// restart the original delay clock.
#[derive(Clone, Debug)]
pub struct PendingReleaseObservation {
    pub eligible_at: String,
    pub last_observed_at: String,
    pub latest_decision_code: Option<String>,
    pub release_identity: String,
    pub coverage_identity: String,
    pub role: PendingReleaseRole,
    pub release_age_unknown: bool,
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
    /// Stable indexer provenance used to resolve the current download-client mapping at submission time.
    pub indexer_id: Option<String>,
    pub release_guid: Option<String>,
    pub added_at: String,
    pub last_observed_at: String,
    pub delay_until: String,
    pub status: PendingReleaseStatus,
    pub grabbed_at: Option<String>,
    /// Password hint for protected NZBs (e.g. NZBGeek password field).
    pub source_password: Option<String>,
    /// RFC3339 publish date — used for `is_recent` queue priority calculation.
    pub published_at: Option<String>,
    /// Torrent info hash — passed to download client for magnet resolution.
    pub info_hash: Option<String>,
    /// Tracker-declared seeding minimums lifted off the release `extra` map at
    /// park time. The `extra` map itself is not persisted, so without these the
    /// delayed grab would reach the client with profile goals but no tracker
    /// clamp — the immediate-grab paths read them straight off the release.
    /// Rows parked before migration 0165 read back as all-`None`.
    pub seed_minimums: ReleaseSeedMinimums,
    /// Seeders the indexer reported when the row was parked (migration 0169).
    ///
    /// Kept so automatic promotion can re-judge the swarm against the threshold
    /// in force *now* rather than the one that applied at park time. Sonarr does
    /// the same: `RssSyncService` re-runs every specification over the pending
    /// list using the seeders stored on the original release. `None` is unknown
    /// — for a row parked before this column existed, or an indexer that reports
    /// nothing — and unknown always stays eligible.
    pub seeders: Option<i64>,
    pub release_identity: String,
    pub coverage_identity: String,
    pub role: PendingReleaseRole,
    pub last_decision_code: Option<String>,
    pub release_age_unknown: bool,
}

impl PendingReleaseObservation {
    pub fn derived(release: &PendingRelease, role: PendingReleaseRole) -> Self {
        let indexer = release
            .indexer_id
            .as_deref()
            .or(release.indexer_source.as_deref())
            .map(normalize_pending_identity_part)
            .unwrap_or_else(|| "unknown".to_string());
        let title = normalize_pending_identity_part(&release.release_title);
        let coverage_identity = if release.coverage_identity.trim().is_empty() {
            format!(
                "scope:{}",
                normalize_pending_identity_part(&release.wanted_item_id)
            )
        } else {
            release.coverage_identity.clone()
        };
        let release_identity = if !release.release_identity.trim().is_empty() {
            release.release_identity.clone()
        } else if let Some(guid) = release
            .release_guid
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            format!("guid:{indexer}:{}", normalize_pending_identity_part(guid))
        } else if let Some(info_hash) = release
            .info_hash
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            format!("hash:{}", normalize_pending_identity_part(info_hash))
        } else if let Some(source) = release
            .release_url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            format!("source:{}", source.trim())
        } else {
            let published_at = release
                .published_at
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(normalize_pending_identity_part)
                .unwrap_or_else(|| "unknown".to_string());
            format!("listing:{indexer}:{title}:{published_at}")
        };
        Self {
            eligible_at: release.delay_until.clone(),
            last_observed_at: if release.last_observed_at.trim().is_empty() {
                release.added_at.clone()
            } else {
                release.last_observed_at.clone()
            },
            latest_decision_code: release.last_decision_code.clone(),
            release_identity,
            coverage_identity,
            role,
            release_age_unknown: release.release_age_unknown
                || (release
                    .published_at
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                    && pending_release_delay_is_active(release)),
        }
    }
}

fn pending_release_delay_is_active(release: &PendingRelease) -> bool {
    let Ok(added_at) = DateTime::parse_from_rfc3339(&release.added_at) else {
        return false;
    };
    DateTime::parse_from_rfc3339(&release.delay_until)
        .is_ok_and(|eligible_at| eligible_at > added_at)
}

fn normalize_pending_identity_part(value: &str) -> String {
    value.trim().to_ascii_lowercase()
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
    /// The pre-allocated identity used for the successful client mutation.
    pub download_id: Option<DownloadId>,
    /// Torrent seed goals resolved by the selected client route. The canonical
    /// submission coordinator freezes them with the accepted identity.
    pub seed_goals: Option<crate::PersistedSeedGoals>,
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
        if ["magnet_uri", "magnet_url"].into_iter().any(|key| {
            extra
                .get(key)
                .and_then(serde_json::Value::as_str)
                .is_some_and(is_valid_magnet_uri)
        }) {
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

/// One recorded failed download attempt for a title.
///
/// History and audit only: `release_download_attempts` never gates acquisition.
/// The per-title blocklist is the single exclusion source, and this listing used
/// to borrow its entry type, which read as though the two were the same thing.
#[derive(Clone, Debug)]
pub struct ReleaseDownloadFailureRecord {
    pub id: String,
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
    pub error_message: Option<String>,
    pub attempted_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleReleaseBlocklistEntry {
    pub id: String,
    pub release_name: String,
    pub error_message: Option<String>,
    pub attempted_at: String,
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
    /// What this release actually covers, as resolved against the catalog while
    /// it was scored.
    ///
    /// Deliberately not `queue_scope`, which means something else — the scope a
    /// *queued grab* will use — and is only populated on the interactive
    /// annotation path. This one is the release's own coverage, and it exists so
    /// the auto evaluator can refuse a multi-episode release that reaches into
    /// an episode nobody is monitoring (D21). `resolve_release_coverage` already
    /// computed it during scoring; before this it was dropped on the floor and
    /// only its `coverage_distance` survived into the search rank.
    pub coverage_scope: Option<SubmissionScope>,
    pub auto_eligible: Option<bool>,
    pub auto_decision_code: Option<String>,
    pub auto_decision_summary: Option<String>,
}

/// Returns whether a plugin-provided magnet contains a usable BitTorrent
/// identifier. Keeping this validation in the application layer prevents an
/// HTTP download URL from being mislabeled as a magnet merely because a
/// plugin populated an optional metadata key.
pub fn extract_magnet_info_hash(value: &str) -> Option<String> {
    let value = value.trim();
    let (scheme, query) = value.split_once("?")?;
    if !scheme.eq_ignore_ascii_case("magnet:") {
        return None;
    }
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find_map(|(key, raw)| {
            if !key.eq_ignore_ascii_case("xt") {
                return None;
            }
            let (urn, remainder) = raw.trim().split_once(':')?;
            if !urn.eq_ignore_ascii_case("urn") {
                return None;
            }
            let (kind, hash) = remainder.split_once(':')?;
            match kind.to_ascii_lowercase().as_str() {
                "btih"
                    if (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                        || (hash.len() == 32
                            && hash
                                .bytes()
                                .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'2'..=b'7'))) =>
                {
                    Some(hash.to_ascii_lowercase())
                }
                "btmh"
                    if hash.len() == 68
                        && hash[..4].eq_ignore_ascii_case("1220")
                        && hash[4..].bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    Some(hash[4..].to_ascii_lowercase())
                }
                _ => None,
            }
        })
}

pub fn is_valid_magnet_uri(value: &str) -> bool {
    extract_magnet_info_hash(value).is_some()
}

impl IndexerSearchResult {
    /// Selects the source that should be submitted to a download client.
    /// Explicit NZB results retain their HTTP source; torrent results prefer
    /// a validated magnet emitted by the plugin.
    /// The BitTorrent v1 infohash the indexer announced, if any.
    ///
    /// Indexers report it inside `extra` rather than as a typed field, so this
    /// is the single place that key is read.
    pub fn info_hash(&self) -> Option<&str> {
        self.extra.get("info_hash").and_then(|value| value.as_str())
    }

    pub fn canonical_download_source(&self) -> Option<(String, DownloadSourceKind)> {
        let explicit_nzb = matches!(
            self.source_kind,
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl)
        );
        if !explicit_nzb {
            for key in ["magnet_uri", "magnet_url"] {
                if let Some(value) = self.extra.get(key).and_then(serde_json::Value::as_str)
                    && is_valid_magnet_uri(value)
                {
                    return Some((value.trim().to_string(), DownloadSourceKind::MagnetUri));
                }
            }
        }
        self.download_url
            .as_deref()
            .or(self.link.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let source_kind = match self.source_kind {
                    Some(DownloadSourceKind::MagnetUri) => {
                        DownloadSourceKind::infer_from_hint(Some(value))
                            .unwrap_or(DownloadSourceKind::TorrentFile)
                    }
                    Some(kind) => kind,
                    None => DownloadSourceKind::infer_from_hint(Some(value))
                        .unwrap_or(DownloadSourceKind::TorrentFile),
                };
                (value.to_string(), source_kind)
            })
    }
}

#[cfg(test)]
mod canonical_download_source_tests {
    use super::{
        DownloadSourceKind, IndexerSearchResult, extract_magnet_info_hash, is_valid_magnet_uri,
    };
    use std::collections::HashMap;

    fn result(magnet: Option<&str>) -> IndexerSearchResult {
        let mut extra = HashMap::new();
        if let Some(magnet) = magnet {
            extra.insert(
                "magnet_url".to_string(),
                serde_json::Value::String(magnet.to_string()),
            );
        }
        IndexerSearchResult {
            indexer_id: None,
            source: "test".to_string(),
            title: "release".to_string(),
            link: Some("https://example.test/link".to_string()),
            download_url: Some("https://example.test/download".to_string()),
            source_kind: Some(DownloadSourceKind::TorrentFile),
            size_bytes: None,
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: None,
            quality_profile_decision: None,
            extra,
            response_attributes: Default::default(),
            guid: None,
            info_url: None,
            provenance: None,
            candidate_token: None,
            queue_scope: None,
            coverage_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
        }
    }

    #[test]
    fn validates_btih_and_btmh_magnets() {
        let uppercase_btih = "MAGNET:?XT=URN:BTIH:0123456789ABCDEF0123456789ABCDEF01234567";
        assert!(is_valid_magnet_uri(uppercase_btih));
        assert_eq!(
            extract_magnet_info_hash(uppercase_btih).as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        let multihash = format!("magnet:?xt=urn:btmh:1220{}", "ab".repeat(32));
        assert!(is_valid_magnet_uri(&multihash));
        assert_eq!(extract_magnet_info_hash(&multihash), Some("ab".repeat(32)));
        assert!(!is_valid_magnet_uri("magnet:?xt=urn:btih:not-a-hash"));
    }

    #[test]
    fn prefers_valid_magnet_and_keeps_http_aliases() {
        let result = result(Some(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        ));
        assert_eq!(
            result.canonical_download_source().unwrap().1,
            DownloadSourceKind::MagnetUri
        );
    }

    #[test]
    fn invalid_magnet_falls_back_to_download_url() {
        let mut result = result(Some("magnet:?xt=urn:btih:invalid"));
        result.source_kind = Some(DownloadSourceKind::MagnetUri);
        assert_eq!(
            result.canonical_download_source().unwrap(),
            (
                "https://example.test/download".to_string(),
                DownloadSourceKind::TorrentFile,
            )
        );
    }

    #[test]
    fn explicit_nzb_ignores_stray_magnet_metadata() {
        let mut result = result(Some(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        ));
        result.source_kind = Some(DownloadSourceKind::NzbUrl);

        assert_eq!(
            result.canonical_download_source().unwrap(),
            (
                "https://example.test/download".to_string(),
                DownloadSourceKind::NzbUrl,
            )
        );
    }

    #[test]
    fn indexer_inference_requires_a_valid_magnet_value() {
        let invalid = HashMap::from([(
            "magnet_uri".to_string(),
            serde_json::Value::String("magnet:?xt=urn:btih:invalid".to_string()),
        )]);
        assert_eq!(
            DownloadSourceKind::infer_from_indexer_result(
                Some("torrent_indexer"),
                Some("https://example.test/download"),
                None,
                &invalid,
            ),
            Some(DownloadSourceKind::TorrentFile)
        );
    }
}

/// Prefix shared by caps-health persistence and automatic-search suppression.
pub const INDEXER_CAPS_REFRESH_ERROR_PREFIX: &str = "caps refresh failed:";

/// Project an indexer's caps snapshot down to fields that can change search
/// dispatch or the returned corpus. Health and display-only metadata must not
/// reopen convergence.
pub fn search_relevant_indexer_caps(raw: Option<&str>) -> serde_json::Value {
    let value = raw.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    project_search_relevant_caps(value.as_ref())
}

fn project_search_relevant_caps(value: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(object) = value.and_then(serde_json::Value::as_object) else {
        return serde_json::Value::Null;
    };
    let mut projected = serde_json::Map::new();
    for key in [
        "search",
        "tv_search",
        "movie_search",
        "categories",
        "limits_default",
        "limits_max",
    ] {
        if let Some(value) = object.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(projected)
}

/// Project managed-indexer metadata down to fields that affect automatic
/// search routing. Embedded caps and cosmetic manager metadata are represented
/// elsewhere (or are intentionally irrelevant) in the search fingerprint.
pub fn search_relevant_managed_indexer_metadata(raw: Option<&str>) -> serde_json::Value {
    let Some(object) = raw
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.as_object().cloned())
    else {
        return serde_json::Value::Null;
    };
    let mut projected = serde_json::Map::new();
    for key in ["enable_automatic_search"] {
        if let Some(value) = object.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(projected)
}

/// Canonical search identity shared by coverage, reusable-candidate diagnostics,
/// and learning invalidation. Keeping this projection in one place prevents
/// those lifecycle decisions from drifting apart.
pub fn indexer_search_identity(
    config: &IndexerConfig,
    search_semantics_version: Option<u32>,
) -> serde_json::Value {
    let config_json = config
        .config_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .unwrap_or(serde_json::Value::Null);
    let secret_fingerprint = config.api_key_encrypted.as_deref().map(|secret| {
        crate::helpers::blake3_identity_hex(crate::helpers::HashDomain::IndexerSecret, secret)
    });
    let direct_caps = config
        .caps_snapshot_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    let managed_caps = config
        .managed_metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.get("caps_snapshot").cloned());
    let caps = project_search_relevant_caps(direct_caps.as_ref().or(managed_caps.as_ref()));
    serde_json::json!({
        "version": 2,
        "provider": config.provider_type.trim().to_ascii_lowercase(),
        "endpoint": config.base_url.trim().trim_end_matches('/'),
        "config": config_json,
        "secret_fingerprint": secret_fingerprint,
        "proxy": config.proxy_config_id,
        "routing": search_relevant_managed_indexer_metadata(config.managed_metadata_json.as_deref()),
        "caps": caps,
        "search_semantics": search_semantics_version,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexerSearchIncompleteReason {
    UpstreamFailure,
    RateLimited,
    MalformedContent,
    PageCeilingReached,
    FanoutBranchFailed,
    SaturatedPartition,
    Unattested,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndexerSearchCompletion {
    #[default]
    Complete,
    Partial {
        reason: Option<IndexerSearchIncompleteReason>,
        retry_after: Option<std::time::Duration>,
    },
}

impl IndexerSearchCompletion {
    pub fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// Per-indexer outcome of a single search query. Only a validated, complete
/// response is eligible for convergence coverage. Partial responses may still
/// contribute candidates, but must be retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexerSearchOutcome {
    Complete {
        empty: bool,
    },
    Partial {
        empty: bool,
        reason: Option<IndexerSearchIncompleteReason>,
        retry_after: Option<std::time::Duration>,
    },
    Deferred {
        retry_after: Option<std::time::Duration>,
    },
    Skipped {
        retry_after: Option<std::time::Duration>,
    },
    Errored,
}

impl IndexerSearchOutcome {
    pub fn coverage_eligible(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }
}

/// A single indexer's outcome within an [`IndexerSearchResponse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerQueryOutcome {
    pub indexer_id: String,
    pub outcome: IndexerSearchOutcome,
}

#[derive(Clone, Debug)]
pub struct IndexerSearchResponse {
    pub results: Vec<IndexerSearchResult>,
    pub completion: IndexerSearchCompletion,
    pub api_current: Option<u32>,
    pub api_max: Option<u32>,
    pub grab_current: Option<u32>,
    pub grab_max: Option<u32>,
    /// Per-indexer outcomes for this query: which routed indexers fired
    /// (empty or not), were skipped/deferred, or errored. Empty for synthetic or
    /// no-eligible-indexer responses.
    pub indexer_outcomes: Vec<IndexerQueryOutcome>,
}

/// One complete effective search strategy submitted to a plan-capable indexer.
#[derive(Clone, Debug)]
pub struct IndexerSearchStrategyRequest {
    pub strategy_id: String,
    pub labels: Vec<String>,
    pub query: String,
    pub ids: std::collections::HashMap<String, String>,
    pub category: Option<String>,
    pub facet: Option<String>,
    pub id_search_facet: Option<String>,
    pub newznab_categories: Option<Vec<String>>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub absolute_episode: Option<u32>,
    /// The subject's known release year, or `None` when the search has no
    /// year the host can vouch for.
    pub year: Option<i32>,
    pub tagged_aliases: Vec<TaggedAlias>,
}

/// A tier of strategies that one indexer component may execute concurrently.
#[derive(Clone, Debug)]
pub struct IndexerSearchPlanRequest {
    pub plan_id: String,
    pub strategies: Vec<IndexerSearchStrategyRequest>,
}

/// A strategy result emitted while an indexer plan is still running.
#[derive(Debug)]
pub struct IndexerSearchStrategyEvent {
    pub strategy_id: String,
    pub response: AppResult<IndexerSearchResponse>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerSearchPlanSummary {
    pub plan_id: String,
    pub emitted_strategy_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexerSearchPlanCapability {
    pub version: u32,
    pub max_parallel_strategies: u32,
}

#[derive(Clone, Debug)]
pub struct IndexerSearchStrategyEventSink {
    sender: tokio::sync::mpsc::Sender<IndexerSearchStrategyEvent>,
}

impl IndexerSearchStrategyEventSink {
    pub fn new(sender: tokio::sync::mpsc::Sender<IndexerSearchStrategyEvent>) -> Self {
        Self { sender }
    }

    pub async fn send(&self, event: IndexerSearchStrategyEvent) -> Result<(), ()> {
        self.sender.send(event).await.map_err(|_| ())
    }
}

/// A persisted page made available to the scoring pipeline.
#[derive(Debug)]
pub struct IndexerSearchPage {
    pub results: Vec<IndexerSearchResult>,
    _reservation: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// The bounded hand-off between indexer retrieval and release scoring.
#[derive(Clone, Debug)]
pub struct IndexerSearchPageSink {
    sender: tokio::sync::mpsc::Sender<IndexerSearchPage>,
    reservations: std::sync::Arc<tokio::sync::Semaphore>,
}

#[derive(Debug)]
pub struct IndexerSearchPageReservation {
    sender: tokio::sync::mpsc::Sender<IndexerSearchPage>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl IndexerSearchPageSink {
    pub fn new(sender: tokio::sync::mpsc::Sender<IndexerSearchPage>, max_pages: usize) -> Self {
        Self {
            sender,
            reservations: std::sync::Arc::new(tokio::sync::Semaphore::new(max_pages)),
        }
    }

    pub async fn reserve(&self) -> Option<IndexerSearchPageReservation> {
        let permit = self.reservations.clone().acquire_owned().await.ok()?;
        Some(IndexerSearchPageReservation {
            sender: self.sender.clone(),
            permit: Some(permit),
        })
    }

    pub async fn send(&self, results: Vec<IndexerSearchResult>) -> Result<(), ()> {
        let Some(reservation) = self.reserve().await else {
            return Err(());
        };
        reservation.send(results).await
    }
}

impl IndexerSearchPageReservation {
    pub async fn send(mut self, results: Vec<IndexerSearchResult>) -> Result<(), ()> {
        self.sender
            .send(IndexerSearchPage {
                results,
                _reservation: self.permit.take(),
            })
            .await
            .map_err(|_| ())
    }
}

/// Why an indexer HTTP request was issued. This is diagnostic metadata only;
/// search routing continues to use [`SearchMode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexerErrorOperation {
    ConnectionTest,
    InteractiveSearch,
    AutomaticSearch,
    RssSync,
    IndexerAction,
    ManagementSync,
    CapsRefresh,
}

impl IndexerErrorOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionTest => "connection_test",
            Self::InteractiveSearch => "interactive_search",
            Self::AutomaticSearch => "automatic_search",
            Self::RssSync => "rss_sync",
            Self::IndexerAction => "indexer_action",
            Self::ManagementSync => "management_sync",
            Self::CapsRefresh => "caps_refresh",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "connection_test" => Some(Self::ConnectionTest),
            "interactive_search" => Some(Self::InteractiveSearch),
            "automatic_search" => Some(Self::AutomaticSearch),
            "rss_sync" => Some(Self::RssSync),
            "indexer_action" => Some(Self::IndexerAction),
            "management_sync" => Some(Self::ManagementSync),
            "caps_refresh" => Some(Self::CapsRefresh),
            _ => None,
        }
    }
}

/// Stable, operator-facing classification for a persisted indexer error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexerErrorClassification {
    NewznabInvalidApiKey,
    NewznabAccountSuspended,
    NewznabInsufficientPrivileges,
    NewznabRegistrationDenied,
    NewznabRegistrationsClosed,
    NewznabInvalidRegistration,
    NewznabInvalidRegistrationEmail,
    NewznabRegistrationFailed,
    NewznabMissingParameter,
    NewznabIncorrectParameter,
    NewznabNoSuchFunction,
    NewznabFunctionNotAvailable,
    NewznabNoSuchItem,
    NewznabRequestLimitReached,
    NewznabDownloadLimitReached,
    NewznabUnknownError,
    NewznabApiDisabled,
    HttpBadRequest,
    HttpUnauthorized,
    HttpForbidden,
    HttpNotFound,
    HttpRequestTimeout,
    HttpRateLimited,
    HttpServerError,
    Unknown,
}

impl IndexerErrorClassification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NewznabInvalidApiKey => "newznab_invalid_api_key",
            Self::NewznabAccountSuspended => "newznab_account_suspended",
            Self::NewznabInsufficientPrivileges => "newznab_insufficient_privileges",
            Self::NewznabRegistrationDenied => "newznab_registration_denied",
            Self::NewznabRegistrationsClosed => "newznab_registrations_closed",
            Self::NewznabInvalidRegistration => "newznab_invalid_registration",
            Self::NewznabInvalidRegistrationEmail => "newznab_invalid_registration_email",
            Self::NewznabRegistrationFailed => "newznab_registration_failed",
            Self::NewznabMissingParameter => "newznab_missing_parameter",
            Self::NewznabIncorrectParameter => "newznab_incorrect_parameter",
            Self::NewznabNoSuchFunction => "newznab_no_such_function",
            Self::NewznabFunctionNotAvailable => "newznab_function_not_available",
            Self::NewznabNoSuchItem => "newznab_no_such_item",
            Self::NewznabRequestLimitReached => "newznab_request_limit_reached",
            Self::NewznabDownloadLimitReached => "newznab_download_limit_reached",
            Self::NewznabUnknownError => "newznab_unknown_error",
            Self::NewznabApiDisabled => "newznab_api_disabled",
            Self::HttpBadRequest => "http_bad_request",
            Self::HttpUnauthorized => "http_unauthorized",
            Self::HttpForbidden => "http_forbidden",
            Self::HttpNotFound => "http_not_found",
            Self::HttpRequestTimeout => "http_request_timeout",
            Self::HttpRateLimited => "http_rate_limited",
            Self::HttpServerError => "http_server_error",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "newznab_invalid_api_key" => Self::NewznabInvalidApiKey,
            "newznab_account_suspended" => Self::NewznabAccountSuspended,
            "newznab_insufficient_privileges" => Self::NewznabInsufficientPrivileges,
            "newznab_registration_denied" => Self::NewznabRegistrationDenied,
            "newznab_registrations_closed" => Self::NewznabRegistrationsClosed,
            "newznab_invalid_registration" => Self::NewznabInvalidRegistration,
            "newznab_invalid_registration_email" => Self::NewznabInvalidRegistrationEmail,
            "newznab_registration_failed" => Self::NewznabRegistrationFailed,
            "newznab_missing_parameter" => Self::NewznabMissingParameter,
            "newznab_incorrect_parameter" => Self::NewznabIncorrectParameter,
            "newznab_no_such_function" => Self::NewznabNoSuchFunction,
            "newznab_function_not_available" => Self::NewznabFunctionNotAvailable,
            "newznab_no_such_item" => Self::NewznabNoSuchItem,
            "newznab_request_limit_reached" => Self::NewznabRequestLimitReached,
            "newznab_download_limit_reached" => Self::NewznabDownloadLimitReached,
            "newznab_unknown_error" => Self::NewznabUnknownError,
            "newznab_api_disabled" => Self::NewznabApiDisabled,
            "http_bad_request" => Self::HttpBadRequest,
            "http_unauthorized" => Self::HttpUnauthorized,
            "http_forbidden" => Self::HttpForbidden,
            "http_not_found" => Self::HttpNotFound,
            "http_request_timeout" => Self::HttpRequestTimeout,
            "http_rate_limited" => Self::HttpRateLimited,
            "http_server_error" => Self::HttpServerError,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedIndexerHttpHeader {
    pub name: String,
    pub value: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedIndexerHttpResponse {
    pub status: u16,
    pub headers: Vec<CapturedIndexerHttpHeader>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct NewIndexerError {
    pub id: String,
    pub indexer_id: String,
    pub indexer_name: String,
    pub operation: IndexerErrorOperation,
    pub classification: IndexerErrorClassification,
    pub provider_error_code: Option<u16>,
    pub message: String,
    pub content_type: Option<String>,
    /// Present only when the upstream returned an HTTP response.
    pub response: Option<CapturedIndexerHttpResponse>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct IndexerErrorSummary {
    pub id: String,
    pub indexer_id: String,
    pub indexer_name: String,
    pub operation: IndexerErrorOperation,
    /// Present only when the upstream returned an HTTP response.
    pub http_status: Option<u16>,
    pub classification: IndexerErrorClassification,
    pub provider_error_code: Option<u16>,
    pub message: String,
    pub content_type: Option<String>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct IndexerErrorDetail {
    pub summary: IndexerErrorSummary,
    /// Present only when the upstream returned an HTTP response.
    pub response: Option<CapturedIndexerHttpResponse>,
}

#[derive(Clone, Debug)]
pub struct IndexerErrorPage {
    pub items: Vec<IndexerErrorSummary>,
    pub next_cursor: Option<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebauthnChallengePurpose {
    StandaloneAuthentication,
    LoginVerification,
    AccountSecurityReauthentication,
    AccountRegistration,
    LoginEnrollment,
}

impl WebauthnChallengePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StandaloneAuthentication => "standalone_authentication",
            Self::LoginVerification => "login_verification",
            Self::AccountSecurityReauthentication => "account_security_reauthentication",
            Self::AccountRegistration => "account_registration",
            Self::LoginEnrollment => "login_enrollment",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "standalone_authentication" => Some(Self::StandaloneAuthentication),
            "login_verification" => Some(Self::LoginVerification),
            "account_security_reauthentication" => Some(Self::AccountSecurityReauthentication),
            "account_registration" => Some(Self::AccountRegistration),
            "login_enrollment" => Some(Self::LoginEnrollment),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginVerificationMethod {
    LocalPassword,
    Jellyfin,
    Emby,
}

impl LoginVerificationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalPassword => "local_password",
            Self::Jellyfin => "jellyfin",
            Self::Emby => "emby",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local_password" => Some(Self::LocalPassword),
            "jellyfin" => Some(Self::Jellyfin),
            "emby" => Some(Self::Emby),
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
    pub purpose: WebauthnChallengePurpose,
    pub login_verification_challenge_id: Option<String>,
    /// Version of the interactive session that created this challenge.
    pub auth_session_version: Option<String>,
    pub state_json: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebauthnChallengeStart {
    pub challenge_id: String,
    pub options_json: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginVerificationChallengeRecord {
    pub id: String,
    pub user_id: String,
    pub login_method: LoginVerificationMethod,
    pub persist_session: bool,
    pub allow_passkey: bool,
    pub allow_totp: bool,
    pub auth_session_version: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginVerificationSatisfied {
    pub mfa_verified_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Session version observed before a factor was verified. When
    /// `mfa_verified_until` is set, callers must bind the issued token to this
    /// version so an administrator reset cannot race token issuance.
    pub auth_session_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginVerificationRequirement {
    Satisfied(LoginVerificationSatisfied),
    EnrollmentRequired {
        auth_session_version: Option<String>,
    },
    Challenge(LoginVerificationChallengeRecord),
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
    /// Version of the interactive session that created this challenge.
    pub auth_session_version: Option<String>,
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

#[derive(Clone, Debug)]
pub struct UserLoginSnapshot {
    pub user: scryer_domain::User,
    pub auth_session_version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct VerifiedLocalCredentials {
    pub user: scryer_domain::User,
    pub auth_session_version: Option<String>,
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
    PasswordChangeRequired,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthenticatedTokenClaims {
    pub mfa_verified_until: Option<i64>,
    pub mfa_step_up_verified_until: Option<i64>,
    pub security_action_verified_until: Option<i64>,
    pub session_scope: JwtSessionScope,
    pub persist_session: bool,
    pub auth_session_version: Option<String>,
    pub password_change_required_after_enrollment: bool,
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
pub struct OAuthClientRegistrationRecord {
    pub client_id: String,
    pub display_name: String,
    pub redirect_uris: Vec<String>,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiKeyProvisioningSource {
    User,
    Environment,
}

impl ApiKeyProvisioningSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Environment => "environment",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "environment" => Some(Self::Environment),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub id: String,
    pub user_id: String,
    pub lookup_id: String,
    pub secret_hash: String,
    pub label: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub provisioning_source: ApiKeyProvisioningSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthAuthorizationCodeRecord {
    pub id: String,
    pub code_hash: String,
    pub client_id: String,
    pub user_id: String,
    /// Exact session epoch that approved this short-lived authorization code.
    /// The empty string represents an intentional database NULL epoch.
    pub auth_session_version: String,
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
    #[serde(default, rename = "securityActionVerifiedUntil")]
    pub security_action_verified_until: Option<i64>,
    #[serde(default, rename = "authScope")]
    pub auth_scope: JwtSessionScope,
    #[serde(default, rename = "persistSession")]
    pub persist_session: bool,
    #[serde(default, rename = "authSessionVersion")]
    pub auth_session_version: Option<String>,
    #[serde(default, rename = "passwordChangeRequiredAfterEnrollment")]
    pub password_change_required_after_enrollment: bool,
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
    /// Absent on tokens minted before torrent info-hash handoff existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    /// Absent on tokens minted before minimum-seeder admission existed, which
    /// reads as an unknown count and therefore stays eligible for the rest of
    /// that token's short TTL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeders: Option<i64>,
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

/// Compact availability information for an episode row. This intentionally
/// excludes the full media-file payload used by the expanded episode panel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpisodeMediaAvailabilityState {
    Available,
    PendingScan,
    ScanFailed,
    Missing,
    Unmonitored,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeMediaAvailability {
    pub title_id: String,
    pub episode_id: String,
    pub state: EpisodeMediaAvailabilityState,
    pub primary_quality_label: Option<String>,
}

pub fn derive_primary_quality_label(
    video_width: Option<i32>,
    video_height: Option<i32>,
    quality_label: Option<&str>,
    resolution: Option<&str>,
) -> Option<String> {
    match video_width.filter(|width| *width > 0) {
        Some(width) if width >= 3840 => return Some("4K".to_string()),
        Some(width) if width >= 1920 => return Some("1080p".to_string()),
        Some(width) if width >= 1280 => return Some("720p".to_string()),
        _ => {}
    }
    if let Some(height) = video_height.filter(|height| *height > 0) {
        return Some(format!("{height}p"));
    }
    quality_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .or_else(|| resolution.map(str::trim).filter(|label| !label.is_empty()))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod primary_quality_label_tests {
    use super::derive_primary_quality_label;

    #[test]
    fn prefers_dimensions_then_stored_quality_metadata() {
        assert_eq!(
            derive_primary_quality_label(Some(3840), Some(1080), Some("1080p"), None),
            Some("4K".to_string())
        );
        assert_eq!(
            derive_primary_quality_label(Some(1920), Some(720), Some("720p"), None),
            Some("1080p".to_string())
        );
        assert_eq!(
            derive_primary_quality_label(Some(1280), Some(1080), Some("1080p"), None),
            Some("720p".to_string())
        );
        assert_eq!(
            derive_primary_quality_label(None, Some(576), Some("1080p"), None),
            Some("576p".to_string())
        );
        assert_eq!(
            derive_primary_quality_label(None, None, Some("  WEB  "), Some("1080p")),
            Some("WEB".to_string())
        );
        assert_eq!(
            derive_primary_quality_label(None, None, Some("  "), Some("  480p  ")),
            Some("480p".to_string())
        );
    }
}

/// Aggregated episode progress counts per collection.
#[derive(Clone, Debug)]
pub struct CollectionEpisodeProgressSummary {
    pub collection_id: String,
    pub owned_episodes: i64,
    pub monitored_episodes: i64,
    pub total_episodes: i64,
    pub episode_records_total: i64,
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
    /// Releases grabbed through this indexer in the trailing 24 hours.
    ///
    /// Scryer's own count of accepted download-client submissions, not a
    /// provider-reported quota counter like `grab_current`. It shares the
    /// in-memory rolling window that backs `queries_last_24h`, so it resets
    /// when the process restarts.
    pub grabs_last_24h: u32,
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
    pub stale_indexer_errors: u32,
    pub stale_history_events: u32,
    pub stale_history_records: u32,
    pub staged_nzb_artifacts_pruned: u32,
    pub recycled_purged: u32,
    /// Stale pending recycle entries committed so they become visible and expirable.
    pub recycled_pending_reconciled: u32,
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

/// A server-owned file candidate. `canonical_path` never crosses the public API boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualImportSelectionCandidate {
    pub id: String,
    pub canonical_path: String,
}

/// A durable selection of files rooted in a tracked completed download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualImportSelection {
    pub id: String,
    pub actor_user_id: String,
    pub title_id: String,
    pub source_identity: crate::ClientJobLocator,
    pub canonical_download_id: Option<scryer_domain::download_identity::DownloadId>,
    pub release_evidence_json: Option<String>,
    /// Server-selected root that every candidate must remain beneath.
    pub trusted_source_root: String,
    /// Temporary archive workspace retained until the queued import completes.
    pub archive_workspace_root: Option<String>,
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
            "/mnt/media/Fathomline (2021)/Season 01/Fathomline.S01E01.mkv",
        );
        let windows = library_scan_file_leaf_key(
            r"D:\Series\Fathomline (2021)\Season 01\Fathomline.S01E01.mkv",
        );
        assert_eq!(unix, windows);

        assert_eq!(
            library_scan_folder_leaf_key("/mnt/media/Fathomline (2021)"),
            library_scan_folder_leaf_key(r"D:\Series\Fathomline (2021)")
        );
    }

    #[test]
    fn library_scan_hint_set_resolves_conflicting_leaf_key_by_full_path() {
        let first_path = "/mnt/media/Fathomline (2021)/Season 01/Fathomline.S01E01.mkv";
        let second_path = "/other/Fathomline (2021)/Season 01/Fathomline.S01E01.mkv";
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

#[cfg(test)]
mod pending_import_reason_class_tests {
    use super::PendingImportReasonClass;

    /// Pins the scanner reason codes this repo actually emits to their buckets.
    /// If a scanner code is renamed, this fails rather than silently
    /// reclassifying those rows as `Other`.
    #[test]
    fn scanner_reason_codes_map_to_expected_classes() {
        for (code, expected) in [
            (
                "no_metadata_search_results",
                PendingImportReasonClass::Unmatched,
            ),
            ("episode_lookup_failed", PendingImportReasonClass::Unmatched),
            ("no_metadata_match", PendingImportReasonClass::Unmatched),
            (
                "no_acceptable_metadata_match",
                PendingImportReasonClass::Ambiguous,
            ),
            (
                "skipped_file_metadata_unreadable",
                PendingImportReasonClass::QualityUnknown,
            ),
            ("episode_identity_missing", PendingImportReasonClass::Other),
            (
                "skipped_unusable_title_evidence",
                PendingImportReasonClass::Other,
            ),
            (
                "title_already_owns_another_folder",
                PendingImportReasonClass::Other,
            ),
        ] {
            assert_eq!(
                PendingImportReasonClass::from_reason_code(code),
                expected,
                "reason code {code} should classify as {expected:?}"
            );
        }
    }

    #[test]
    fn unknown_and_blank_reason_codes_fall_back_to_other() {
        assert_eq!(
            PendingImportReasonClass::from_reason_code("some_future_scanner_code"),
            PendingImportReasonClass::Other
        );
        assert_eq!(
            PendingImportReasonClass::from_reason_code(""),
            PendingImportReasonClass::Other
        );
    }

    #[test]
    fn surrounding_whitespace_does_not_defeat_classification() {
        assert_eq!(
            PendingImportReasonClass::from_reason_code("  no_metadata_search_results  "),
            PendingImportReasonClass::Unmatched
        );
    }
}

#[cfg(test)]
mod indexer_search_identity_tests {
    use super::indexer_search_identity;
    use chrono::Utc;
    use scryer_domain::IndexerConfig;

    fn config() -> IndexerConfig {
        IndexerConfig {
            id: "idx-1".into(),
            name: "Synthetic Indexer".into(),
            provider_type: "newznab".into(),
            base_url: "https://indexer.example.test".into(),
            api_key_encrypted: Some("secret-a".into()),
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: Some(
                serde_json::json!({
                    "enable_rss": true,
                    "enable_automatic_search": true,
                    "display_name": "First",
                    "caps_snapshot": {"cosmetic": "embedded"},
                })
                .to_string(),
            ),
            caps_snapshot_json: Some(
                serde_json::json!({
                    "search": true,
                    "categories": [5000],
                    "display_name": "First",
                })
                .to_string(),
            ),
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(serde_json::json!({"api_path": "/api"}).to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn cosmetic_managed_metadata_and_caps_do_not_change_search_identity() {
        let original = config();
        let mut cosmetic = original.clone();
        cosmetic.managed_metadata_json = Some(
            serde_json::json!({
                "enable_rss": true,
                "enable_automatic_search": true,
                "display_name": "Second",
                "caps_snapshot": {"cosmetic": "changed"},
            })
            .to_string(),
        );
        cosmetic.caps_snapshot_json = Some(
            serde_json::json!({
                "search": true,
                "categories": [5000],
                "display_name": "Second",
            })
            .to_string(),
        );

        assert_eq!(
            indexer_search_identity(&original, Some(7)),
            indexer_search_identity(&cosmetic, Some(7))
        );
    }

    #[test]
    fn every_search_relevant_indexer_change_changes_identity() {
        let original = config();
        let original_identity = indexer_search_identity(&original, Some(7));
        let variants = [
            {
                let mut value = original.clone();
                value.base_url = "https://other.example.test".into();
                value
            },
            {
                let mut value = original.clone();
                value.api_key_encrypted = Some("secret-b".into());
                value
            },
            {
                let mut value = original.clone();
                value.proxy_config_id = Some("proxy-1".into());
                value
            },
            {
                let mut value = original.clone();
                value.managed_metadata_json = Some(
                    serde_json::json!({
                        "enable_rss": true,
                        "enable_automatic_search": false,
                    })
                    .to_string(),
                );
                value
            },
            {
                let mut value = original.clone();
                value.caps_snapshot_json = Some(
                    serde_json::json!({
                        "search": true,
                        "categories": [5000, 5070],
                    })
                    .to_string(),
                );
                value
            },
            {
                let mut value = original.clone();
                value.caps_snapshot_json = Some(
                    serde_json::json!({
                        "search": true,
                        "categories": [5000],
                        "limits_default": 100,
                        "limits_max": 200,
                    })
                    .to_string(),
                );
                value
            },
        ];

        for variant in variants {
            assert_ne!(
                original_identity,
                indexer_search_identity(&variant, Some(7))
            );
        }
    }

    #[test]
    fn managed_embedded_caps_are_used_when_no_direct_snapshot_exists() {
        let mut original = config();
        original.caps_snapshot_json = None;
        original.managed_metadata_json = Some(
            serde_json::json!({
                "enable_rss": true,
                "enable_automatic_search": true,
                "caps_snapshot": {"search": true, "categories": [5000]},
            })
            .to_string(),
        );
        let mut changed = original.clone();
        changed.managed_metadata_json = Some(
            serde_json::json!({
                "enable_rss": true,
                "enable_automatic_search": true,
                "caps_snapshot": {"search": true, "categories": [5000, 5070]},
            })
            .to_string(),
        );

        assert_ne!(
            indexer_search_identity(&original, Some(7)),
            indexer_search_identity(&changed, Some(7))
        );
    }
}
