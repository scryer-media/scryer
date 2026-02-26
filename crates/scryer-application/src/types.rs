use serde::{Deserialize, Serialize};
use scryer_domain::ExternalId;

use crate::quality_profile::QualityProfileDecision;
use crate::release_parser::ParsedReleaseMetadata;

#[derive(Clone, Debug, Default)]
pub struct TitleMetadataUpdate {
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
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
    /// Additional external IDs to merge onto the title (e.g. MAL, AniList from anime mappings).
    pub extra_external_ids: Vec<ExternalId>,
    /// Additional tags to merge onto the title (e.g. MAL score, anime media type).
    pub extra_tags: Vec<String>,
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
}

#[derive(Clone, Debug)]
pub struct WantedItem {
    pub id: String,
    pub title_id: String,
    pub title_name: Option<String>,
    pub episode_id: Option<String>,
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
}

#[derive(Clone, Debug)]
pub struct JwtAuthConfig {
    pub issuer: String,
    pub access_ttl_seconds: usize,
    pub jwt_ec_private_pem: String,
    pub jwt_ec_public_pem: String,
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

/// Lightweight summary of a title's primary (index=0) collection, used to
/// avoid N+1 queries when listing titles with their quality tier and file size.
#[derive(Clone, Debug)]
pub struct PrimaryCollectionSummary {
    pub title_id: String,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
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
}
