use async_trait::async_trait;
use scryer_domain::{CanonicalMediaTag, ExternalId, MediaFacet, Title};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::{AppError, AppResult, TitleExternalRating};

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

impl LibraryScanSummary {
    pub fn absorb(&mut self, delta: &LibraryScanSummary) {
        self.scanned = self.scanned.saturating_add(delta.scanned);
        self.matched = self.matched.saturating_add(delta.matched);
        self.imported = self.imported.saturating_add(delta.imported);
        self.skipped = self.skipped.saturating_add(delta.skipped);
        self.unmatched = self.unmatched.saturating_add(delta.unmatched);
    }
}

#[derive(Debug, Clone)]
pub struct MetadataSearchItem {
    pub tvdb_id: String,
    pub smg_id: Option<i64>,
    pub primary_source: Option<String>,
    pub external_ids: Vec<ExternalId>,
    pub name: String,
    pub year: Option<i32>,
    pub auto_match_safe: bool,
    pub auto_match_signals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetadataSearchQuery {
    pub query: String,
    pub type_hint: String,
    pub year: Option<i32>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub tvdb_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RichMetadataSearchItem {
    pub tvdb_id: String,
    pub smg_id: Option<i64>,
    pub primary_source: Option<String>,
    pub external_ids: Vec<ExternalId>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovieTitleRef {
    pub smg_id: Option<i64>,
    pub tvdb_id: Option<i64>,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
}

impl MovieTitleRef {
    pub fn from_title(title: &Title) -> Option<Self> {
        if title.facet != MediaFacet::Movie {
            return None;
        }

        let external_id = |source: &str| {
            title
                .external_ids
                .iter()
                .find(|external_id| external_id.source.trim().eq_ignore_ascii_case(source))
                .map(|external_id| external_id.value.trim())
                .filter(|value| !value.is_empty())
        };
        let reference = Self {
            smg_id: external_id("smg").and_then(|value| value.parse().ok()),
            tvdb_id: external_id("tvdb").and_then(|value| value.parse().ok()),
            tmdb_id: external_id("tmdb").and_then(|value| value.parse().ok()),
            imdb_id: external_id("imdb").map(str::to_string).or_else(|| {
                title
                    .imdb_id
                    .clone()
                    .filter(|value| !value.trim().is_empty())
            }),
        };

        (reference.smg_id.is_some()
            || reference.tvdb_id.is_some()
            || reference.tmdb_id.is_some()
            || reference.imdb_id.is_some())
        .then_some(reference)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MultiMetadataSearchResult {
    pub movies: Vec<RichMetadataSearchItem>,
    pub series: Vec<RichMetadataSearchItem>,
    pub anime: Vec<RichMetadataSearchItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryExternalIdInput {
    pub source: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverySubjectInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvdb_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmdb_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mal_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anidb_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_ids: Vec<DiscoveryExternalIdInput>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPublicFeedInput {
    pub region: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_types: Vec<String>,
    pub limit_per_section: i32,
    pub include_unresolved: bool,
    /// Opt-in to un-deduplicated sections (the client owns cross-rail
    /// presentation). Serialized even when false so the submit-json audit trail
    /// records the choice; old gateways reject unknown fields, so this ships
    /// only after the gateway supports it (server-before-client, SaaS order).
    #[serde(default)]
    pub full_sections: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCollectionCompletionInput {
    #[serde(default)]
    pub library_subjects: Vec<DiscoverySubjectInput>,
    pub limit: i32,
    pub include_future: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryContextSnapshotSubmitInput {
    #[serde(default)]
    pub subjects: Vec<DiscoverySubjectInput>,
    pub region: String,
    pub language: String,
    pub max_items: i32,
    pub include_owned: bool,
    pub include_unresolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiscoveryContextChangeType {
    Added,
    Updated,
    Removed,
    Rematched,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryContextChangedSubjectInput {
    pub subject: DiscoverySubjectInput,
    pub change_type: DiscoveryContextChangeType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_subject: Option<DiscoverySubjectInput>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryContextChangesInput {
    #[serde(default)]
    pub context_subject_keys: Vec<String>,
    #[serde(default)]
    pub changed_subjects: Vec<DiscoveryContextChangedSubjectInput>,
    pub region: String,
    pub language: String,
    pub max_items: i32,
    pub include_owned: bool,
    pub include_unresolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_context_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryDashboardResult {
    #[serde(default)]
    pub subject_keys: Vec<String>,
    pub generated_at: String,
    #[serde(default)]
    pub sections: Vec<DiscoveryDashboardSection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TitleRecommendationsInput {
    pub subject: DiscoverySubjectInput,
    #[serde(default)]
    pub query: String,
    pub limit: i32,
    pub language: String,
    pub include_unresolved: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryRelatedResult {
    pub subject_key: String,
    pub query: String,
    pub generated_at: String,
    #[serde(default)]
    pub results: Vec<DiscoveryTitle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryDashboardSection {
    pub section_id: String,
    pub section_type: String,
    pub title: String,
    #[serde(default)]
    pub source_signals: Vec<String>,
    #[serde(default)]
    pub facets: Vec<DiscoveryFacet>,
    #[serde(default)]
    pub items: Vec<DiscoveryTitle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryFacet {
    pub name: String,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoverySnapshotFacetGroup {
    pub name: String,
    #[serde(default)]
    pub values: Vec<DiscoverySnapshotFacetValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoverySnapshotFacetValue {
    pub value: String,
    pub count: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryCollectionCompletionResult {
    #[serde(default)]
    pub subject_keys: Vec<String>,
    pub generated_at: String,
    #[serde(default)]
    pub results: Vec<DiscoveryTitle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryContextSnapshotSubmitResult {
    pub request_id: Option<String>,
    pub status: String,
    pub subject_count: i32,
    pub retry_after_seconds: i32,
    pub expires_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryContextSnapshotStatusResult {
    pub request_id: String,
    pub status: String,
    pub phase: String,
    pub subject_count: i32,
    pub item_count: i32,
    pub page_count: i32,
    pub facet_count: i32,
    pub lazy_hydration_queued_count: i32,
    #[serde(default)]
    pub lazy_hydration_sources: Vec<String>,
    pub discovery_index_watermark: String,
    pub retry_after_seconds: i32,
    pub created_at: String,
    pub started_at: String,
    pub completed_at: String,
    pub expires_at: String,
    pub last_error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryContextSnapshotPageResult {
    pub request_id: String,
    pub page: i32,
    pub page_count: i32,
    pub generated_at: String,
    pub discovery_index_watermark: String,
    #[serde(default)]
    pub facets: Vec<DiscoverySnapshotFacetGroup>,
    #[serde(default)]
    pub items: Vec<DiscoveryTitle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryContextSnapshotAckResult {
    pub request_id: String,
    pub status: String,
    pub acknowledged_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryContextChangesResult {
    pub status: String,
    pub retry_after_seconds: i32,
    pub generated_at: String,
    pub context_fingerprint: String,
    pub previous_context_fingerprint: String,
    pub discovery_index_watermark: String,
    pub context_subject_count: i32,
    pub changed_subject_count: i32,
    #[serde(default)]
    pub resolved_changed_subject_keys: Vec<String>,
    #[serde(default)]
    pub removed_subject_keys: Vec<String>,
    #[serde(default)]
    pub affected_target_keys: Vec<String>,
    #[serde(default)]
    pub items: Vec<DiscoveryTitle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryExternalId {
    pub source: String,
    pub kind: String,
    pub id: String,
    pub key: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryContentCertification {
    pub value: String,
    pub source: String,
    pub release_type: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryContentRating {
    pub country: String,
    #[serde(default)]
    pub certifications: Vec<DiscoveryContentCertification>,
    pub age_rating: Option<i32>,
    pub age_rating_source: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryTitle {
    pub target_key: String,
    pub target_kind: String,
    pub resolved: bool,
    pub resolved_title_id: String,
    pub display_title: String,
    pub original_title: String,
    pub year: Option<i32>,
    pub poster_path: String,
    pub poster_url: String,
    pub overview: String,
    pub content_type: String,
    pub rating: Option<f64>,
    #[serde(default)]
    pub rating_sources: Vec<String>,
    #[serde(default)]
    pub external_ratings: Vec<TitleExternalRating>,
    #[serde(default)]
    pub external_ids: Vec<DiscoveryExternalId>,
    #[serde(default, skip_serializing)]
    pub rating_provenance: Vec<DiscoveryRatingProvenance>,
    #[serde(default)]
    pub status_tags: Vec<String>,
    pub background_url: String,
    #[serde(default)]
    pub source_tags: Vec<serde_json::Value>,
    #[serde(default)]
    pub canonical_tags: Vec<serde_json::Value>,
    #[serde(default)]
    pub is_adult: bool,
    #[serde(default)]
    pub content_ratings: Vec<DiscoveryContentRating>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub relation_types: Vec<String>,
    #[serde(default)]
    pub relation_subtypes: Vec<String>,
    #[serde(default)]
    pub chart_signals: Vec<serde_json::Value>,
    #[serde(default)]
    pub provider_signals: Vec<serde_json::Value>,
    #[serde(default)]
    pub rank_components: Vec<serde_json::Value>,
    pub source_count: i32,
    pub edge_count: i32,
    pub relation_count: i32,
    pub source_subject_count: i32,
    pub rank_score: f64,
    pub best_source: String,
    #[serde(default)]
    pub matched_subject_keys: Vec<String>,
    #[serde(default)]
    pub matched_subject_titles: Vec<String>,
    #[serde(default)]
    pub matched_subject_count: i32,
    pub tmdb_collection_id: Option<i32>,
    #[serde(default)]
    pub tmdb_collection_name: String,
    #[serde(default)]
    pub owned_in_input: bool,
    #[serde(default)]
    pub studio_slug: Option<String>,
    #[serde(default)]
    pub person_ids: Vec<i32>,
    #[serde(default)]
    pub facet_terms: Vec<String>,
    #[serde(default)]
    pub context_terms: Vec<String>,
    #[serde(default)]
    pub change_subject_keys: Vec<String>,
    #[serde(default)]
    pub removed_subject_keys: Vec<String>,
}

impl DiscoveryTitle {
    pub fn apply_rating_provenance(&mut self) {
        self.external_ratings = self
            .rating_provenance
            .iter()
            .filter_map(DiscoveryRatingProvenance::to_external_rating)
            .collect();
        self.rating_provenance.clear();
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryRatingProvenance {
    #[serde(default)]
    pub metadata_source: String,
    #[serde(default)]
    pub rating_source: String,
    pub value: Option<f64>,
    pub score: Option<f64>,
    #[serde(default)]
    pub normalized: f64,
    pub votes: Option<i32>,
    #[serde(default)]
    pub url: String,
}

impl DiscoveryRatingProvenance {
    fn to_external_rating(&self) -> Option<TitleExternalRating> {
        let source = self.rating_source.trim();
        let source = if source.is_empty() {
            self.metadata_source.trim()
        } else {
            source
        };
        if source.is_empty() {
            return None;
        }
        Some(TitleExternalRating {
            source: source.to_string(),
            value: self.value,
            score: self.score,
            normalized: self.normalized,
            votes: self.votes,
            url: self.url.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct MovieMetadata {
    pub target_key: Option<String>,
    pub smg_id: Option<i64>,
    pub primary_source: String,
    pub tvdb_id: Option<i64>,
    pub name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub content_status: String,
    pub overview: String,
    pub poster_url: String,
    pub background_url: Option<String>,
    pub language: String,
    pub original_language: Option<String>,
    pub runtime_minutes: i32,
    pub sort_title: String,
    pub imdb_id: String,
    pub tmdb_id: Option<i64>,
    pub popularity: Option<f64>,
    pub anidb_id: Option<i64>,
    pub canonical_tags: Vec<CanonicalMediaTag>,
    pub studio: String,
    pub tmdb_release_date: Option<String>,
    pub ratings: crate::TitleRatingSummary,
    pub credits: Vec<crate::TitleCredit>,
}

#[derive(Debug, Clone)]
pub struct TitleResolution {
    pub ref_index: usize,
    pub resolved: bool,
    pub smg_id: Option<i64>,
    pub kind: String,
    pub primary_source: String,
    pub redirected_from: Option<i64>,
    pub created: bool,
    pub external_ids: Vec<ExternalId>,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct MovieTitleBulkResult {
    pub by_ref_index: HashMap<usize, MovieMetadata>,
    pub redirects: Vec<(i64, i64)>,
    pub missing_ref_indexes: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct SeriesMetadata {
    pub target_key: Option<String>,
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
    pub background_url: Option<String>,
    pub original_language: Option<String>,
    pub country: String,
    pub canonical_tags: Vec<CanonicalMediaTag>,
    pub aliases: Vec<String>,
    pub tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    pub seasons: Vec<SeasonMetadata>,
    pub episodes: Vec<EpisodeMetadata>,
    pub anime_mappings: Vec<AnimeMapping>,
    pub anime_movies: Vec<AnimeMovie>,
    /// Community (AniDB/AniList/MAL) season layout for this series, when SMG
    /// could build one. `None` for non-anime, for an SMG that predates the
    /// field, and for anime whose community numbering matches TVDB's.
    pub anime_numbering_bridge: Option<scryer_domain::AnimeNumberingBridge>,
    pub ratings: crate::TitleRatingSummary,
    pub credits: Vec<crate::TitleCredit>,
}

#[derive(Debug, Clone)]
pub struct AnimeMapping {
    pub mal_id: Option<i64>,
    pub mal_dub_id: Option<i64>,
    pub anilist_id: Option<i64>,
    pub anidb_id: Option<i64>,
    pub kitsu_id: Option<i64>,
    pub simkl_id: Option<i64>,
    pub thetvdb_id: Option<i64>,
    pub themoviedb_id: Option<i64>,
    pub imdb_id: Option<i64>,
    pub trakt_id: Option<i64>,
    pub alt_tvdb_id: Option<i64>,
    pub thetvdb_season: Option<i32>,
    pub thetvdb_part: Option<i32>,
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
    pub image_url: String,
}

#[async_trait]
pub trait MetadataGateway: Send + Sync {
    async fn search_tvdb(
        &self,
        query: &str,
        type_hint: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>>;

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>>;

    async fn search_tvdb_rich(
        &self,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
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

    async fn get_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        language: &str,
    ) -> AppResult<MovieTitleBulkResult> {
        let _ = (refs, language);
        Err(AppError::Repository(
            "metadata gateway does not support title-id queries".into(),
        ))
    }

    async fn resolve_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        create_missing: bool,
    ) -> AppResult<Vec<TitleResolution>> {
        let _ = (refs, create_missing);
        Err(AppError::Repository(
            "metadata gateway does not support title-id queries".into(),
        ))
    }

    async fn search_titles(
        &self,
        query: &str,
        kind: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        let _ = (query, kind, limit, language, year);
        Err(AppError::Repository(
            "metadata gateway does not support title-id queries".into(),
        ))
    }

    async fn search_titles_batch(
        &self,
        queries: &[MetadataSearchQuery],
        kind: &str,
        language: &str,
        create_missing: bool,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        let _ = (queries, kind, language, create_missing);
        Err(AppError::Repository(
            "metadata gateway does not support title-id queries".into(),
        ))
    }

    async fn title_recommendations(
        &self,
        input: &TitleRecommendationsInput,
    ) -> AppResult<DiscoveryRelatedResult> {
        let _ = input;
        Err(AppError::Repository(
            "metadata gateway titleRecommendations is not implemented".into(),
        ))
    }

    async fn discover_public_feed(
        &self,
        input: &DiscoveryPublicFeedInput,
    ) -> AppResult<DiscoveryDashboardResult> {
        let _ = input;
        Err(AppError::Repository(
            "metadata gateway discoverPublicFeed is not implemented".into(),
        ))
    }

    async fn collection_completions(
        &self,
        input: &DiscoveryCollectionCompletionInput,
    ) -> AppResult<DiscoveryCollectionCompletionResult> {
        let _ = input;
        Err(AppError::Repository(
            "metadata gateway collectionCompletions is not implemented".into(),
        ))
    }

    async fn submit_discovery_context_snapshot(
        &self,
        input: &DiscoveryContextSnapshotSubmitInput,
    ) -> AppResult<DiscoveryContextSnapshotSubmitResult> {
        let _ = input;
        Err(AppError::Repository(
            "metadata gateway submitDiscoveryContextSnapshot is not implemented".into(),
        ))
    }

    async fn discovery_context_snapshot_status(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotStatusResult> {
        let _ = request_id;
        Err(AppError::Repository(
            "metadata gateway discoveryContextSnapshotStatus is not implemented".into(),
        ))
    }

    async fn discovery_context_snapshot_page(
        &self,
        request_id: &str,
        page: i32,
    ) -> AppResult<DiscoveryContextSnapshotPageResult> {
        let _ = (request_id, page);
        Err(AppError::Repository(
            "metadata gateway discoveryContextSnapshotPage is not implemented".into(),
        ))
    }

    async fn discovery_context_changes(
        &self,
        input: &DiscoveryContextChangesInput,
    ) -> AppResult<DiscoveryContextChangesResult> {
        let _ = input;
        Err(AppError::Repository(
            "metadata gateway discoveryContextChanges is not implemented".into(),
        ))
    }

    async fn acknowledge_discovery_context_snapshot(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotAckResult> {
        let _ = request_id;
        Err(AppError::Repository(
            "metadata gateway acknowledgeDiscoveryContextSnapshot is not implemented".into(),
        ))
    }

    async fn get_artwork_urls_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<BulkArtworkUrlResult> {
        let _ = (movie_tvdb_ids, series_tvdb_ids, language);
        Ok(BulkArtworkUrlResult::default())
    }
}

#[derive(Debug, Clone, Default)]
pub struct BulkMetadataResult {
    pub movies: std::collections::HashMap<i64, MovieMetadata>,
    pub series: std::collections::HashMap<i64, SeriesMetadata>,
}

#[derive(Debug, Clone, Default)]
pub struct BulkArtworkUrlResult {
    pub movies: std::collections::HashMap<i64, TitleArtworkUrls>,
    pub series: std::collections::HashMap<i64, SeriesArtworkUrls>,
}

#[derive(Debug, Clone)]
pub struct TitleArtworkUrls {
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SeriesArtworkUrls {
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
    pub episodes: Vec<EpisodeArtworkUrls>,
}

#[derive(Debug, Clone)]
pub struct EpisodeArtworkUrls {
    pub tvdb_id: i64,
    pub season_number: i32,
    pub episode_number: i32,
    pub image_url: Option<String>,
}

#[async_trait]
pub trait LibraryScanner: Send + Sync {
    async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>>;

    async fn scan_directory(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        self.scan_library(root).await
    }

    /// Direct-child media files of `root` only — the shallow evidence pass
    /// used by the streaming scan pipeline for movie title candidates. The
    /// default derives from `scan_directory` so existing implementations and
    /// test doubles keep working; the filesystem scanner overrides this with
    /// a true single-readdir listing.
    async fn scan_directory_children(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        let root_path = std::path::Path::new(root).to_path_buf();
        let mut files = self.scan_directory(root).await?;
        files.retain(|file| {
            std::path::Path::new(&file.path)
                .parent()
                .is_some_and(|parent| parent == root_path.as_path())
        });
        Ok(files)
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
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository(
            "metadata gateway is not configured".into(),
        ))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
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
        _year: Option<i32>,
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
}
