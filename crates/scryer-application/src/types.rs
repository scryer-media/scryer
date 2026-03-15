use std::collections::HashMap;

use scryer_domain::ExternalId;
use serde::{Deserialize, Serialize};

use crate::quality_profile::QualityProfileDecision;
use crate::release_parser::ParsedReleaseMetadata;

#[derive(Clone, Debug, Default)]
pub struct TitleMetadataUpdate {
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub banner_url: Option<String>,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub imdb_id: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub genres: Vec<String>,
    pub content_status: Option<String>,
    pub language: Option<String>,
    pub first_aired: Option<String>,
    pub network: Option<String>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub aliases: Vec<String>,
    pub metadata_language: Option<String>,
    pub metadata_fetched_at: Option<String>,
    pub digital_release_date: Option<String>,
    /// Additional external IDs to merge onto the title (e.g. MAL, AniList from anime mappings).
    pub extra_external_ids: Vec<ExternalId>,
    /// Additional tags to merge onto the title (e.g. MAL score, anime media type).
    pub extra_tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleImageKind {
    Poster,
    Banner,
    Fanart,
}

impl TitleImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Poster => "poster",
            Self::Banner => "banner",
            Self::Fanart => "fanart",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "poster" => Some(Self::Poster),
            "banner" => Some(Self::Banner),
            "fanart" => Some(Self::Fanart),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleImageStorageMode {
    Original,
    AvifMaster,
}

impl TitleImageStorageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::AvifMaster => "avif_master",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "original" => Some(Self::Original),
            "avif_master" => Some(Self::AvifMaster),
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
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct TitleImageReplacement {
    pub kind: TitleImageKind,
    pub source_url: String,
    pub source_etag: Option<String>,
    pub source_last_modified: Option<String>,
    pub source_format: String,
    pub source_width: i32,
    pub source_height: i32,
    pub storage_mode: TitleImageStorageMode,
    pub master_format: String,
    pub master_sha256: String,
    pub master_width: i32,
    pub master_height: i32,
    pub master_bytes: Vec<u8>,
    pub variants: Vec<TitleImageVariantRecord>,
}

#[derive(Clone, Debug)]
pub struct TitleImageSyncTask {
    pub title_id: String,
    pub source_url: String,
    pub cached_source_url: Option<String>,
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
    pub file_path: String,
    pub size_bytes: i64,
    pub quality_label: Option<String>,
    pub scan_status: String,
    pub created_at: String,
    // Media analysis fields (populated after media scan; None until scan_status='scanned')
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
    pub video_codec_parsed: Option<String>,
    pub audio_codec_parsed: Option<String>,
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
pub struct WantedItem {
    pub id: String,
    pub title_id: String,
    pub title_name: Option<String>,
    pub episode_id: Option<String>,
    pub season_number: Option<String>,
    pub media_type: String,
    pub search_phase: String,
    pub next_search_at: Option<String>,
    pub last_search_at: Option<String>,
    pub search_count: i64,
    pub baseline_date: Option<String>,
    pub status: String,
    pub grabbed_release: Option<String>,
    pub current_score: Option<i32>,
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
    pub status: String,
    pub grabbed_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DownloadGrabResult {
    pub job_id: String,
    pub client_type: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadSourceKind {
    NzbUrl,
    TorrentFile,
    MagnetUri,
}

impl DownloadSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NzbUrl => "nzb_url",
            Self::TorrentFile => "torrent_file",
            Self::MagnetUri => "magnet_uri",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "nzb" | "nzb_url" => Some(Self::NzbUrl),
            "torrent" | "torrent_file" => Some(Self::TorrentFile),
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
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
    pub error_message: Option<String>,
    pub attempted_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexerSearchResult {
    pub source: String,
    pub title: String,
    pub link: Option<String>,
    pub download_url: Option<String>,
    pub source_kind: Option<DownloadSourceKind>,
    pub size_bytes: Option<i64>,
    pub published_at: Option<String>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub nzbgeek_languages: Option<Vec<String>>,
    pub nzbgeek_subtitles: Option<Vec<String>>,
    pub nzbgeek_grabs: Option<i64>,
    pub nzbgeek_password_protected: Option<String>,
    pub parsed_release_metadata: Option<ParsedReleaseMetadata>,
    pub quality_profile_decision: Option<QualityProfileDecision>,
    /// Arbitrary indexer-specific metadata from WASM plugins.
    /// Passed through to OPA scoring as `input.release.extra`.
    pub extra: HashMap<String, serde_json::Value>,
    pub guid: Option<String>,
    pub info_url: Option<String>,
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
}

#[derive(Clone, Debug)]
pub struct JwtAuthConfig {
    pub issuer: String,
    pub access_ttl_seconds: usize,
    pub jwt_hmac_secret: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct JwtClaims {
    pub sub: String,
    pub exp: i64,
    pub iat: i64,
    pub iss: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub entitlements: Vec<String>,
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

#[derive(Clone, Debug)]
pub struct DiskSpaceInfo {
    pub path: String,
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct SystemHealth {
    pub service_ready: bool,
    pub db_path: String,
    pub total_titles: usize,
    pub monitored_titles: usize,
    pub total_users: usize,
    pub titles_movie: usize,
    pub titles_tv: usize,
    pub titles_anime: usize,
    pub titles_other: usize,
    pub recent_events: usize,
    pub recent_event_preview: Vec<String>,
    pub db_migration_version: Option<String>,
    pub db_pending_migrations: usize,
    pub smg_cert_expires_at: Option<String>,
    pub smg_cert_days_remaining: Option<i64>,
    pub indexer_stats: Vec<IndexerQueryStats>,
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

#[derive(Clone, Debug)]
pub struct BackupInfo {
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String,
}

#[derive(Clone, Debug)]
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
    pub expired_event_outboxes: u32,
    pub stale_history_events: u32,
    pub recycled_purged: u32,
    pub ran_at: String,
}
