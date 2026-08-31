use super::*;
use crate::contracts::{
    ClientJobLocator, DownloadClientBindingRecord, DownloadRecord, ObservationResolution,
    ObservedClientJob, TerminalDownloadHistoryRow,
};
use crate::types::{
    ApiKeyRecord, EpisodeMediaAvailability, IndexerSearchPlanCapability, IndexerSearchPlanRequest,
    IndexerSearchPlanSummary, IndexerSearchStrategyEventSink, LoginVerificationChallengeRecord,
    OAuthClientRegistrationRecord, PendingReleaseObservation, PendingReleaseRole,
    TitleCatalogFilterCounts,
};
use async_trait::async_trait;
use scryer_domain::download_identity::DownloadId;
use scryer_domain::{
    CanonicalMediaTag, ImportTransferPhase, ImportType, IndexerCapsSnapshot,
    PersistedPluginWasmPayload, title_catalog_name_tie_key, title_catalog_sort_key_for_title,
};
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginProcessRequest, ArchivePluginProcessResponse,
    SubtitleSyncAlignResponse, SubtitleSyncAudioCodec, SubtitleSyncMediaMetadataSnapshot,
    SubtitleSyncOptions,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const NOTIFICATION_REQUEST_SCHEMA_VERSION: u32 = 1;
const TITLE_QUALITY_PROFILE_TAG_PREFIX: &str = "scryer:quality-profile:";

/// A field-level title option patch. `None` preserves stored state, `Some(None)`
/// clears an override, and `Some(Some(_))` applies an explicit override.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitleOptionsPatch {
    pub quality_profile_id: Option<Option<String>>,
    pub root_folder_id: Option<Option<String>>,
    pub monitor_type: Option<Option<String>>,
    pub use_season_folders: Option<Option<bool>>,
    pub monitor_specials: Option<Option<bool>>,
    pub inter_season_movies: Option<Option<bool>>,
    pub filler_policy: Option<Option<String>>,
    pub recap_policy: Option<Option<String>>,
}

#[derive(Clone, Debug)]
pub struct TitleArtworkUrlUpdate {
    pub title_id: String,
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleDeletePreviewInfo {
    pub title_id: String,
    pub library_id: String,
    pub title_name: String,
    pub facet: MediaFacet,
    pub folder_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleExternalIdLookup {
    pub lookup_index: usize,
    pub source: String,
    pub external_id: String,
}

#[derive(Clone, Debug)]
pub struct TitleExternalIdLookupMatch {
    pub lookup_index: usize,
    pub title: Title,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesMovieExternalIdLookupMatch {
    pub lookup_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogOwnedTitleRecord {
    pub id: String,
    pub facet: MediaFacet,
    pub imdb_id: Option<String>,
    pub external_ids: Vec<CatalogOwnedExternalIdRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogOwnedExternalIdRecord {
    pub source: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HousekeepingMediaFileRootRow {
    pub media_file_id: String,
    pub title_id: String,
    pub file_path: String,
    pub library_id: String,
    pub root_paths: Vec<String>,
}

pub const DISCOVERY_DEFAULT_SCOPE_KEY: &str = "default";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverySyncStateRecord {
    pub scope_key: String,
    pub last_success_generation_id: Option<String>,
    pub last_public_feed_generation_id: Option<String>,
    pub last_subject_fingerprint: Option<String>,
    pub last_context_snapshot_completed_at: Option<DateTime<Utc>>,
    pub last_incremental_reload_completed_at: Option<DateTime<Utc>>,
    pub last_public_feed_completed_at: Option<DateTime<Utc>>,
    pub dirty_since: Option<DateTime<Utc>>,
    pub dirty_reason_mask: i64,
    pub bootstrap_started_at: Option<DateTime<Utc>>,
    pub bootstrap_quiet_until: Option<DateTime<Utc>>,
    pub next_context_snapshot_eligible_at: Option<DateTime<Utc>>,
    pub next_incremental_reload_eligible_at: Option<DateTime<Utc>>,
    pub next_public_feed_eligible_at: Option<DateTime<Utc>>,
    pub backoff_until: Option<DateTime<Utc>>,
    pub transient_failure_count: i64,
    pub startup_jitter_seconds: i64,
    pub context_jitter_seconds: i64,
    pub incremental_reload_jitter_seconds: i64,
    pub public_feed_jitter_seconds: i64,
    pub last_seen_domain_event_sequence: Option<i64>,
    pub inflight_context_snapshot_run_id: Option<String>,
    pub inflight_subject_fingerprint: Option<String>,
    pub inflight_domain_event_sequence: Option<i64>,
    pub lease_owner_id: Option<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl Default for DiscoverySyncStateRecord {
    fn default() -> Self {
        Self {
            scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            last_success_generation_id: None,
            last_public_feed_generation_id: None,
            last_subject_fingerprint: None,
            last_context_snapshot_completed_at: None,
            last_incremental_reload_completed_at: None,
            last_public_feed_completed_at: None,
            dirty_since: None,
            dirty_reason_mask: 0,
            bootstrap_started_at: None,
            bootstrap_quiet_until: None,
            next_context_snapshot_eligible_at: None,
            next_incremental_reload_eligible_at: None,
            next_public_feed_eligible_at: None,
            backoff_until: None,
            transient_failure_count: 0,
            startup_jitter_seconds: 0,
            context_jitter_seconds: 0,
            incremental_reload_jitter_seconds: 0,
            public_feed_jitter_seconds: 0,
            last_seen_domain_event_sequence: None,
            inflight_context_snapshot_run_id: None,
            inflight_subject_fingerprint: None,
            inflight_domain_event_sequence: None,
            lease_owner_id: None,
            lease_expires_at: None,
            updated_at: Utc::now(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverySyncRunRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub trigger_source: String,
    pub region: String,
    pub language: String,
    pub subject_count: i64,
    pub subject_fingerprint: Option<String>,
    pub previous_subject_fingerprint: Option<String>,
    pub base_generation_id: Option<String>,
    pub changed_subject_count: i64,
    pub affected_target_count: i64,
    pub smg_request_id: Option<String>,
    pub smg_status: Option<String>,
    pub discovery_index_watermark: Option<String>,
    pub page_count: Option<i32>,
    pub item_count: Option<i64>,
    pub facet_count: Option<i64>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub error_text: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoverySyncStatus {
    pub state: DiscoverySyncStateRecord,
    pub recent_runs: Vec<DiscoverySyncRunRecord>,
    pub pending_context_change_count: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveryPruneReport {
    pub runs_deleted: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryHomeQuery {
    pub include_public: bool,
    pub include_personalized: bool,
    pub include_unresolved: bool,
    pub limit_per_section: usize,
    pub filters: DiscoveryHomeFilters,
}

impl Default for DiscoveryHomeQuery {
    fn default() -> Self {
        Self {
            include_public: true,
            include_personalized: true,
            include_unresolved: false,
            limit_per_section: 25,
            filters: DiscoveryHomeFilters::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscoveryHomeFilters {
    pub content_types: Vec<String>,
    pub genre_tag_keys: Vec<String>,
    pub theme_tag_keys: Vec<String>,
    pub studio_slugs: Vec<String>,
    pub minimum_year: Option<i32>,
    pub maximum_year: Option<i32>,
    pub minimum_rating: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryCanonicalTagFilterOption {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveryHomeFilterOptions {
    pub genres: Vec<DiscoveryCanonicalTagFilterOption>,
    pub themes: Vec<DiscoveryCanonicalTagFilterOption>,
    pub studio_slugs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryHomeResult {
    pub status: DiscoverySyncStatus,
    pub hero_item: Option<DiscoveryItemRecord>,
    pub public_sections: Vec<DiscoverySectionResult>,
    pub personalized_sections: Vec<DiscoverySectionResult>,
    pub complete_collection: Option<DiscoverySectionResult>,
    pub facets: Vec<DiscoveryFacetRecord>,
    pub can_view_personalized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogDiscoveryQuery {
    pub facet: MediaFacet,
    pub library_ids: Vec<String>,
    pub include_unresolved: bool,
    pub limit_per_group: usize,
    pub max_groups: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogDiscoveryResult {
    pub groups: Vec<CatalogDiscoveryGroup>,
    pub can_view_personalized: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogDiscoveryGroup {
    pub id: String,
    pub kind: CatalogDiscoveryGroupKind,
    pub surface: CatalogDiscoverySurface,
    pub label_value: Option<String>,
    pub total_count: i64,
    pub items: Vec<DiscoveryItemRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogDiscoveryGroupKind {
    PublicTop,
    PublicSection,
    GenreAffinity,
    ThemeAffinity,
    Acclaimed,
    CompleteCollection,
    Fallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogDiscoverySurface {
    Public,
    Personalized,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveryItemsQuery {
    pub query: Option<String>,
    pub target_keys: Vec<String>,
    pub target_kinds: Vec<String>,
    pub sources: Vec<String>,
    pub relation_types: Vec<String>,
    pub relation_subtypes: Vec<String>,
    pub genres: Vec<String>,
    pub status_tags: Vec<String>,
    pub facet_terms: Vec<String>,
    pub include_owned: bool,
    pub include_unresolved: bool,
    pub include_public: bool,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryItemsResult {
    pub items: Vec<DiscoveryItemRecord>,
    pub total_count: i64,
    pub can_view_personalized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryItemDetailQuery {
    pub target_key: String,
    pub include_unresolved: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryItemsPageRecord {
    pub items: Vec<DiscoveryItemRecord>,
    pub total_count: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryItemsStorageQuery {
    pub context_run_id: Option<String>,
    pub public_run_id: Option<String>,
    pub readable_library_ids: Vec<String>,
    pub allowed_media_kinds: Vec<String>,
    pub filters: DiscoveryItemsQuery,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoverySectionResult {
    pub section_id: String,
    pub section_type: String,
    pub title: String,
    pub surface: String,
    pub total_count: i64,
    pub items: Vec<DiscoveryItemRecord>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogDiscoveryCandidatesRecord {
    pub total_count: i64,
    pub items: Vec<DiscoveryItemRecord>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogDiscoverySectionCandidatesRecord {
    pub section_id: String,
    pub section_type: String,
    pub title: Option<String>,
    pub total_count: i64,
    pub items: Vec<DiscoveryItemRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoverySectionItemsRecord {
    pub section: DiscoverySectionRecord,
    pub total_count: i64,
    pub items: Vec<DiscoveryItemRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryContextSnapshotCommit {
    pub state: DiscoverySyncStateRecord,
    pub run: DiscoverySyncRunRecord,
    pub submitted_subjects: Vec<DiscoverySubmittedSubjectRecord>,
    pub items: Vec<DiscoveryItemRecord>,
    pub facets: Vec<DiscoveryFacetRecord>,
    pub clear_pending_through_sequence: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryContextIncrementalCommit {
    pub state: DiscoverySyncStateRecord,
    pub run: DiscoverySyncRunRecord,
    pub items: Vec<DiscoveryItemRecord>,
    pub tombstone_target_keys: Vec<String>,
    pub clear_pending_through_sequence: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryPublicFeedCommit {
    pub state: DiscoverySyncStateRecord,
    pub run: DiscoverySyncRunRecord,
    pub sections: Vec<DiscoverySectionRecord>,
    pub items: Vec<DiscoveryItemRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverySubmittedSubjectRecord {
    pub run_id: String,
    pub subject_key: String,
    pub title_id: Option<String>,
    pub library_id: Option<String>,
    pub library_facet: Option<String>,
    pub title_kind: Option<String>,
    pub display_title: Option<String>,
    pub external_ids_json: String,
    pub raw_subject_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryPendingContextChangeRecord {
    pub id: String,
    pub scope_key: String,
    pub subject_key: Option<String>,
    pub previous_subject_key: Option<String>,
    pub change_type: String,
    pub title_id: Option<String>,
    pub previous_title_id: Option<String>,
    pub library_facet: Option<String>,
    pub raw_subject_json: Option<String>,
    pub raw_previous_subject_json: Option<String>,
    pub first_seen_sequence: Option<i64>,
    pub last_seen_sequence: Option<i64>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverySectionRecord {
    pub id: String,
    pub run_id: String,
    pub section_id: String,
    pub section_type: String,
    pub surface: String,
    pub title: String,
    pub sort_index: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoverySourceTagRecord {
    pub category: Option<String>,
    pub name: Option<String>,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveryExternalIdRecord {
    pub source: String,
    pub kind: String,
    pub id: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveryRankComponentRecord {
    pub component_index: i32,
    pub component_name: Option<String>,
    pub component_value: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveryItemLibraryProvenanceRecord {
    pub subject_key: String,
    pub title_id: Option<String>,
    pub library_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DiscoveryItemRecord {
    pub id: String,
    pub run_id: String,
    pub base_generation_id: Option<String>,
    pub source_run_kind: String,
    pub section_id: Option<String>,
    pub sort_index: i32,
    pub target_key: String,
    pub target_kind: String,
    pub resolved: bool,
    pub resolved_title_id: Option<String>,
    pub display_title: String,
    pub original_title: Option<String>,
    pub sort_title: Option<String>,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
    pub overview: Option<String>,
    pub content_type: Option<String>,
    pub canonical_tags: Vec<CanonicalMediaTag>,
    pub is_adult: bool,
    pub content_ratings: Vec<DiscoveryContentRating>,
    pub rating: Option<f64>,
    pub rating_sources: Vec<String>,
    pub external_ratings: Vec<TitleExternalRating>,
    pub external_ids: Vec<DiscoveryExternalIdRecord>,
    pub status_tags: Vec<String>,
    pub source_tags: Vec<DiscoverySourceTagRecord>,
    pub sources: Vec<String>,
    pub best_source: Option<String>,
    pub relation_types: Vec<String>,
    pub relation_subtypes: Vec<String>,
    pub chart_signals: Vec<String>,
    pub provider_signals: Vec<String>,
    pub rank_components: Vec<DiscoveryRankComponentRecord>,
    pub source_count: Option<i32>,
    pub edge_count: Option<i32>,
    pub relation_count: Option<i32>,
    pub source_subject_count: Option<i32>,
    pub rank_score: Option<f64>,
    pub matched_subject_keys: Vec<String>,
    pub matched_subject_titles: Vec<String>,
    pub matched_subject_count: i32,
    pub library_provenance: Vec<DiscoveryItemLibraryProvenanceRecord>,
    pub tmdb_collection_id: Option<String>,
    pub tmdb_collection_name: Option<String>,
    pub owned_in_input: bool,
    pub studio_slug: Option<String>,
    pub person_ids: Vec<i32>,
    pub facet_terms: Vec<String>,
    pub context_terms: Vec<String>,
    pub change_subject_keys: Vec<String>,
    pub removed_subject_keys: Vec<String>,
    pub tombstoned_by_run_id: Option<String>,
    pub tombstoned_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The compact representation used while selecting discovery-home cards.
///
/// `item` deliberately contains no title-term, subject-link, or other card
/// children. The separate signals below are the complete set needed to keep
/// home ranking and section construction stable before selected cards are
/// hydrated for output.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryHomeCandidate {
    pub item: DiscoveryItemRecord,
    pub discovery_title_id: String,
    pub matched_subject_keys: Vec<String>,
    pub affinity_terms: Vec<String>,
    pub has_hero_backdrop: bool,
    pub rating_source_count: i32,
    pub best_external_rating: Option<f64>,
    pub best_external_rating_votes: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryHomeSectionCandidatesRecord {
    pub section: DiscoverySectionRecord,
    pub total_count: i64,
    pub items: Vec<DiscoveryHomeCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryFacetRecord {
    pub run_id: String,
    pub facet_name: String,
    pub facet_value: String,
    pub smg_count: Option<i64>,
    pub local_count: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct EpisodeImageUrlUpdate {
    pub episode_id: String,
    pub image_url: Option<String>,
}

#[async_trait]
pub trait DiscoveryRepository: Send + Sync {
    async fn get_discovery_sync_state(
        &self,
        scope_key: &str,
    ) -> AppResult<Option<DiscoverySyncStateRecord>>;
    async fn upsert_discovery_sync_state(&self, state: &DiscoverySyncStateRecord) -> AppResult<()>;
    async fn try_acquire_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool>;
    async fn renew_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool>;
    async fn release_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()>;
    async fn get_discovery_sync_run(&self, id: &str) -> AppResult<Option<DiscoverySyncRunRecord>>;
    async fn list_recent_discovery_sync_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>>;
    async fn list_unacked_discovery_context_snapshot_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>>;
    async fn upsert_discovery_sync_run(&self, run: &DiscoverySyncRunRecord) -> AppResult<()>;
    async fn commit_discovery_context_snapshot(
        &self,
        commit: &DiscoveryContextSnapshotCommit,
    ) -> AppResult<()>;
    async fn commit_discovery_context_incremental(
        &self,
        commit: &DiscoveryContextIncrementalCommit,
    ) -> AppResult<()>;
    async fn commit_discovery_public_feed(
        &self,
        commit: &DiscoveryPublicFeedCommit,
    ) -> AppResult<()>;
    async fn replace_discovery_submitted_subjects(
        &self,
        run_id: &str,
        subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<()>;
    async fn list_discovery_submitted_subjects(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>>;
    async fn upsert_pending_discovery_context_change(
        &self,
        change: &DiscoveryPendingContextChangeRecord,
    ) -> AppResult<()>;
    async fn get_pending_discovery_context_change(
        &self,
        id: &str,
    ) -> AppResult<Option<DiscoveryPendingContextChangeRecord>>;
    async fn delete_pending_discovery_context_change(&self, id: &str) -> AppResult<u64>;
    async fn list_all_pending_discovery_context_changes(
        &self,
        scope_key: &str,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>>;
    async fn list_pending_discovery_context_changes(
        &self,
        scope_key: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>>;
    async fn count_pending_discovery_context_changes(&self, scope_key: &str) -> AppResult<i64>;
    async fn clear_pending_discovery_context_changes_through_sequence(
        &self,
        scope_key: &str,
        last_seen_sequence: i64,
    ) -> AppResult<u64>;
    async fn replace_discovery_sections(
        &self,
        run_id: &str,
        sections: &[DiscoverySectionRecord],
    ) -> AppResult<()>;
    async fn replace_discovery_items(
        &self,
        run_id: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()>;
    async fn replace_discovery_facets(
        &self,
        run_id: &str,
        facets: &[DiscoveryFacetRecord],
    ) -> AppResult<()>;
    async fn list_discovery_sections(
        &self,
        run_id: &str,
        surface: Option<&str>,
    ) -> AppResult<Vec<DiscoverySectionRecord>>;
    async fn list_public_discovery_section_items(
        &self,
        run_id: &str,
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit_per_section: i64,
    ) -> AppResult<Vec<DiscoveryHomeSectionCandidatesRecord>>;
    async fn list_personalized_discovery_home_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>>;
    async fn list_personalized_complete_collection_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>>;
    async fn list_personalized_discovery_facets(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
    ) -> AppResult<Vec<DiscoveryFacetRecord>>;
    #[allow(clippy::too_many_arguments)]
    async fn list_discovery_home_top_rated_items(
        &self,
        public_run_id: Option<&str>,
        context_run_id: Option<&str>,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>>;
    async fn hydrate_discovery_home_candidates(
        &self,
        candidates: &mut [DiscoveryHomeCandidate],
    ) -> AppResult<()>;
    async fn hydrate_discovery_home_hero(
        &self,
        candidate: &mut DiscoveryHomeCandidate,
    ) -> AppResult<()>;
    #[allow(clippy::too_many_arguments)]
    async fn list_discovery_home_filter_options(
        &self,
        public_run_id: Option<&str>,
        context_run_id: Option<&str>,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
    ) -> AppResult<DiscoveryHomeFilterOptions>;
    async fn list_catalog_public_discovery_items(
        &self,
        run_id: &str,
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord>;
    async fn list_catalog_public_discovery_sections(
        &self,
        run_id: &str,
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit_per_section: i64,
    ) -> AppResult<Vec<CatalogDiscoverySectionCandidatesRecord>>;
    async fn list_catalog_personalized_discovery_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord>;
    async fn query_discovery_items(
        &self,
        query: &DiscoveryItemsStorageQuery,
    ) -> AppResult<DiscoveryItemsPageRecord>;
    async fn replace_title_more_like_this_items(
        &self,
        title_id: &str,
        language: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()>;
    async fn list_title_more_like_this_items(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>>;
    async fn list_discovery_items_for_generation(
        &self,
        base_generation_id: &str,
    ) -> AppResult<Vec<DiscoveryItemRecord>>;
    async fn list_discovery_facets(&self, run_id: &str) -> AppResult<Vec<DiscoveryFacetRecord>>;
    async fn prune_discovery_history(
        &self,
        scope_key: &str,
        retain_successful_per_kind: usize,
        diagnostic_cutoff: DateTime<Utc>,
    ) -> AppResult<DiscoveryPruneReport>;
}

#[async_trait]
pub trait TitleRepository: Send + Sync {
    async fn list(&self, facet: Option<MediaFacet>, query: Option<String>)
    -> AppResult<Vec<Title>>;
    /// Counts titles whose quality-profile structured tag resolves to the
    /// supplied profile ID. The profile portion follows resolver semantics:
    /// it is trimmed after the structured prefix is removed.
    async fn count_by_quality_profile_id(&self, profile_id: &str) -> AppResult<u64> {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Ok(0);
        }
        Ok(self
            .list(None, None)
            .await?
            .into_iter()
            .filter(|title| {
                title.tags.iter().any(|tag| {
                    tag.strip_prefix("scryer:quality-profile:")
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case(profile_id))
                })
            })
            .count() as u64)
    }
    async fn list_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list(facet, query).await
    }
    async fn list_delete_preview_info(&self) -> AppResult<Vec<TitleDeletePreviewInfo>> {
        Ok(self
            .list_without_external_ids(None, None)
            .await?
            .into_iter()
            .map(|title| TitleDeletePreviewInfo {
                title_id: title.id,
                library_id: title.library_id,
                title_name: title.name,
                facet: title.facet,
                folder_path: title.folder_path,
            })
            .collect())
    }
    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let titles = self.list(facet, query).await?;
        Ok(titles
            .into_iter()
            .filter(|title| library_ids.iter().any(|id| id == &title.library_id))
            .collect())
    }
    async fn list_catalog_owned_title_records(
        &self,
        library_ids: &[String],
    ) -> AppResult<Vec<CatalogOwnedTitleRecord>> {
        Ok(self
            .list_for_libraries(None, library_ids, None)
            .await?
            .into_iter()
            .map(|title| CatalogOwnedTitleRecord {
                id: title.id,
                facet: title.facet,
                imdb_id: title.imdb_id,
                external_ids: title
                    .external_ids
                    .into_iter()
                    .map(|external_id| CatalogOwnedExternalIdRecord {
                        source: external_id.source,
                        value: external_id.value,
                    })
                    .collect(),
            })
            .collect())
    }
    async fn list_for_libraries_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let titles = self.list_without_external_ids(facet, query).await?;
        Ok(titles
            .into_iter()
            .filter(|title| library_ids.iter().any(|id| id == &title.library_id))
            .collect())
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "title catalog repository queries mirror the user-visible filter, sort, pagination, and projection surface"
    )]
    async fn list_for_libraries_catalog(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
        filter: TitleCatalogFilter,
        sort: TitleCatalogSort,
        limit: usize,
        offset: usize,
        include_external_ids: bool,
        include_catalog_counts: bool,
    ) -> AppResult<TitleCatalogResult> {
        if library_ids.is_empty() {
            return Ok(TitleCatalogResult {
                items: Vec::new(),
                limit,
                offset,
                has_more: false,
                total_count: 0,
                filter_counts: TitleCatalogFilterCounts::default(),
                managed_bytes: 0,
            });
        }

        let mut titles = if include_external_ids {
            self.list_for_libraries(facet, library_ids, query).await?
        } else {
            self.list_for_libraries_without_external_ids(facet, library_ids, query)
                .await?
        };
        let filter_counts = if include_catalog_counts {
            title_catalog_filter_counts(&titles, &filter)
        } else {
            TitleCatalogFilterCounts::default()
        };
        titles.retain(|title| title_matches_catalog_filter(title, &filter));
        sort_titles_for_catalog(&mut titles, sort);

        let total_count = if include_catalog_counts {
            titles.len()
        } else {
            0
        };
        let items = titles
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = include_catalog_counts && offset.saturating_add(items.len()) < total_count;

        Ok(TitleCatalogResult {
            items,
            limit,
            offset,
            has_more,
            total_count,
            filter_counts,
            // Non-SQL test repositories do not expose media-file aggregates.
            managed_bytes: 0,
        })
    }
    async fn title_catalog_filter_options(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        root_folder_ids: &[String],
    ) -> AppResult<TitleCatalogFilterOptions> {
        if library_ids.is_empty() {
            return Ok(TitleCatalogFilterOptions::default());
        }

        let root_folder_ids = root_folder_ids.iter().collect::<BTreeSet<_>>();
        let titles = self.list_for_libraries(facet, library_ids, None).await?;
        let mut genres = BTreeMap::<String, String>::new();
        let mut tags = BTreeMap::<String, String>::new();
        let mut minimum_year = None;
        let mut maximum_year = None;

        for title in titles {
            if !root_folder_ids.is_empty() && !root_folder_ids.contains(&title.root_folder_id) {
                continue;
            }
            if let Some(year) = title.year {
                minimum_year = Some(minimum_year.map_or(year, |current: i32| current.min(year)));
                maximum_year = Some(maximum_year.map_or(year, |current: i32| current.max(year)));
            }
            for tag in title.canonical_tags {
                let target = if tag.category.eq_ignore_ascii_case("genre") {
                    Some(&mut genres)
                } else if tag.category.eq_ignore_ascii_case("theme") {
                    Some(&mut tags)
                } else {
                    None
                };
                if let Some(target) = target {
                    let key = tag.key.trim();
                    let name = tag.name.trim();
                    if !key.is_empty() && !name.is_empty() {
                        target
                            .entry(key.to_string())
                            .or_insert_with(|| name.to_string());
                    }
                }
            }
        }

        let to_options = |values: BTreeMap<String, String>| {
            let mut options = values
                .into_iter()
                .map(|(key, name)| TitleCatalogTagFilterOption { key, name })
                .collect::<Vec<_>>();
            options.sort_by(|left, right| {
                left.name
                    .to_lowercase()
                    .cmp(&right.name.to_lowercase())
                    .then_with(|| left.key.cmp(&right.key))
            });
            options
        };

        Ok(TitleCatalogFilterOptions {
            genres: to_options(genres),
            tags: to_options(tags),
            minimum_year,
            maximum_year,
        })
    }
    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>>;
    async fn list_by_external_id_lookups(
        &self,
        lookups: &[TitleExternalIdLookup],
    ) -> AppResult<Vec<TitleExternalIdLookupMatch>> {
        let mut matches = Vec::new();
        for lookup in lookups {
            for title in self
                .list_by_external_ids(&lookup.source, std::slice::from_ref(&lookup.external_id))
                .await?
            {
                matches.push(TitleExternalIdLookupMatch {
                    lookup_index: lookup.lookup_index,
                    title,
                });
            }
        }
        Ok(matches)
    }

    async fn list_existing_external_ids_in_library_and_facet(
        &self,
        library_id: &str,
        facet: MediaFacet,
        source: &str,
        values: &[String],
    ) -> AppResult<BTreeSet<String>> {
        let library_id = library_id.trim();
        let source = source.trim();
        if library_id.is_empty() || source.is_empty() || values.is_empty() {
            return Ok(BTreeSet::new());
        }

        let requested = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        if requested.is_empty() {
            return Ok(BTreeSet::new());
        }

        let mut existing = BTreeSet::new();
        for title in self
            .list_for_libraries(Some(facet), &[library_id.to_string()], None)
            .await?
        {
            for external_id in title.external_ids {
                let value = external_id.value.trim();
                if external_id.source.eq_ignore_ascii_case(source) && requested.contains(value) {
                    existing.insert(value.to_string());
                }
            }
        }
        Ok(existing)
    }
    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>>;
    async fn get_by_id_without_external_ids(&self, id: &str) -> AppResult<Option<Title>> {
        self.get_by_id(id).await
    }
    /// Batch-load titles by id for dataloaders. Missing ids are simply absent
    /// from the result; order and multiplicity are not guaranteed. The default
    /// fans out to `get_by_id`; SQL stores override with a single `IN` query.
    async fn get_by_ids(&self, ids: &[String]) -> AppResult<Vec<Title>> {
        let mut titles = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(title) = self.get_by_id(id).await? {
                titles.push(title);
            }
        }
        Ok(titles)
    }
    async fn get_title_ratings(&self, _title_id: &str) -> AppResult<TitleRatingSummary> {
        Ok(TitleRatingSummary::default())
    }
    async fn list_title_ratings(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<(String, TitleRatingSummary)>> {
        let mut ratings = Vec::with_capacity(title_ids.len());
        for title_id in title_ids {
            ratings.push((title_id.clone(), self.get_title_ratings(title_id).await?));
        }
        Ok(ratings)
    }
    /// Title-local cache of the credits SMG returned for the title's last successful
    /// hydration, in SMG's order.
    async fn get_title_credits(&self, _title_id: &str) -> AppResult<Vec<TitleCredit>> {
        Ok(Vec::new())
    }
    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>>;
    async fn get_by_facet_libraries_and_slug(
        &self,
        facet: MediaFacet,
        library_ids: &[String],
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let normalized_slug = slug.trim();
        if normalized_slug.is_empty() || library_ids.is_empty() {
            return Ok(None);
        }

        let matches =
            self.list_for_libraries(Some(facet), library_ids, None)
                .await?
                .into_iter()
                .filter(|title| {
                    title.slug.as_deref().is_some_and(|candidate| {
                        candidate.trim().eq_ignore_ascii_case(normalized_slug)
                    })
                })
                .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [title] => Ok(Some(title.clone())),
            _ => Err(AppError::Validation(
                "multiple titles found for slug lookup".into(),
            )),
        }
    }
    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>>;
    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>>;
    async fn find_by_external_id_in_library_and_facet(
        &self,
        library_id: &str,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let library_id = library_id.trim();
        if library_id.is_empty() {
            return Ok(None);
        }

        Ok(self
            .list_for_libraries(Some(facet), &[library_id.to_string()], None)
            .await?
            .into_iter()
            .find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source)
                        && external_id.value.trim() == value.trim()
                })
            }))
    }
    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome>;
    async fn create_or_get_existing_with_options_patch(
        &self,
        title: Title,
        _options_patch: TitleOptionsPatch,
    ) -> AppResult<CreateTitleOutcome> {
        self.create_or_get_existing(title).await
    }
    async fn create_or_get_existing_and_bind_pending_import(
        &self,
        title: Title,
        _pending_import_id: &str,
    ) -> AppResult<CreateTitleOutcome> {
        let _ = title;
        Err(AppError::Repository(
            "transactional pending import title creation is not supported".into(),
        ))
    }
    async fn create(&self, title: Title) -> AppResult<Title>;
    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>>;
    /// Ordered movie titles whose canonical external-id set can be resolved to
    /// an SMG title id, but does not contain one yet.
    async fn list_movie_titles_missing_smg_id_after_id(
        &self,
        _after_id: Option<&str>,
        _limit: usize,
    ) -> AppResult<Vec<Title>> {
        Ok(Vec::new())
    }
    /// Record a failed SMG-identity resolution attempt for a movie title.
    async fn record_movie_smg_identity_backfill_unresolved(
        &self,
        _title_id: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    /// Merge an SMG title id into a title's canonical external-id set.
    async fn persist_smg_id(
        &self,
        _title_id: &str,
        _smg_id: i64,
        _redirected_from: Option<i64>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "persisting SMG title ids is not supported".into(),
        ))
    }
    async fn list_title_ids_with_metadata_hydration_due(
        &self,
        _facet: Option<MediaFacet>,
        _library_ids: &[String],
    ) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()>;
    async fn mark_titles_metadata_hydration_due_now(&self, ids: &[String]) -> AppResult<()> {
        for id in ids {
            self.mark_title_metadata_hydration_due_now(id).await?;
        }
        Ok(())
    }
    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()>;
    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()>;
    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title>;
    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
        root_folder_id: Option<String>,
    ) -> AppResult<Title>;
    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title>;
    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()>;
    async fn clear_folder_path(&self, id: &str) -> AppResult<()>;
    async fn clear_metadata_language_for_all(&self) -> AppResult<u64>;
    async fn list_page_after_id(
        &self,
        after_id: Option<String>,
        limit: usize,
    ) -> AppResult<Vec<Title>> {
        let mut titles = self.list(None, None).await?;
        titles.sort_by(|left, right| left.id.cmp(&right.id));
        let filtered = titles
            .into_iter()
            .filter(|title| {
                after_id
                    .as_ref()
                    .is_none_or(|after_id| title.id.as_str() > after_id.as_str())
            })
            .take(limit)
            .collect();
        Ok(filtered)
    }
    async fn update_title_artwork_urls(
        &self,
        _updates: &[TitleArtworkUrlUpdate],
    ) -> AppResult<u64> {
        Ok(0)
    }
}

fn sort_titles_for_catalog(titles: &mut [Title], sort: TitleCatalogSort) {
    titles.sort_by(|left, right| match sort.key {
        TitleCatalogSortKey::Year => {
            compare_nullable_ord_null_last(left.year, right.year, sort.direction)
                .then_with(|| compare_titles_by_catalog_title(left, right))
        }
        TitleCatalogSortKey::Runtime => compare_nullable_ord_null_last(
            left.runtime_minutes,
            right.runtime_minutes,
            sort.direction,
        )
        .then_with(|| compare_titles_by_catalog_title(left, right)),
        TitleCatalogSortKey::Popularity => {
            compare_nullable_partial_null_last(left.popularity, right.popularity, sort.direction)
                .then_with(|| compare_titles_by_catalog_title(left, right))
        }
        _ => {
            let ordering = match sort.key {
                TitleCatalogSortKey::Title => compare_titles_by_catalog_title(left, right),
                TitleCatalogSortKey::Library => left
                    .library_id
                    .cmp(&right.library_id)
                    .then_with(|| compare_titles_by_catalog_title(left, right)),
                TitleCatalogSortKey::Monitored => left
                    .monitored
                    .cmp(&right.monitored)
                    .then_with(|| compare_titles_by_catalog_title(left, right)),
                TitleCatalogSortKey::Quality => title_catalog_quality_profile_id(left)
                    .cmp(&title_catalog_quality_profile_id(right))
                    .then_with(|| compare_titles_by_catalog_title(left, right)),
                TitleCatalogSortKey::Status => title_catalog_status_sort_value(left)
                    .cmp(&title_catalog_status_sort_value(right))
                    .then_with(|| compare_titles_by_catalog_title(left, right)),
                TitleCatalogSortKey::Episodes
                | TitleCatalogSortKey::Size
                | TitleCatalogSortKey::Root
                | TitleCatalogSortKey::MediaResolution
                | TitleCatalogSortKey::MediaHdr
                | TitleCatalogSortKey::MediaAudioCodec
                | TitleCatalogSortKey::RatingScryer
                | TitleCatalogSortKey::RatingImdb
                | TitleCatalogSortKey::RatingRottenTomatoes
                | TitleCatalogSortKey::RatingPopcornmeter
                | TitleCatalogSortKey::RatingMetacritic
                | TitleCatalogSortKey::RatingMetacriticUser
                | TitleCatalogSortKey::RatingLetterboxd
                | TitleCatalogSortKey::RatingTmdb
                | TitleCatalogSortKey::RatingTvdb
                | TitleCatalogSortKey::RatingTrakt
                | TitleCatalogSortKey::RatingMyanimelist
                | TitleCatalogSortKey::RatingAnilist
                | TitleCatalogSortKey::RatingAnidb
                | TitleCatalogSortKey::RatingMdblist => {
                    compare_titles_by_catalog_title(left, right)
                }
                TitleCatalogSortKey::Added => left
                    .created_at
                    .cmp(&right.created_at)
                    .then_with(|| compare_titles_by_catalog_title(left, right)),
                TitleCatalogSortKey::Year
                | TitleCatalogSortKey::Runtime
                | TitleCatalogSortKey::Popularity => Ordering::Equal,
            };
            match sort.direction {
                SortDirection::Asc => ordering,
                SortDirection::Desc => ordering.reverse(),
            }
        }
    });
}

fn compare_nullable_ord_null_last<T: Ord>(
    left: Option<T>,
    right: Option<T>,
    direction: SortDirection,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => match direction {
            SortDirection::Asc => left.cmp(&right),
            SortDirection::Desc => right.cmp(&left),
        },
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_nullable_partial_null_last(
    left: Option<f64>,
    right: Option<f64>,
    direction: SortDirection,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            let ordering = left.partial_cmp(&right).unwrap_or(Ordering::Equal);
            match direction {
                SortDirection::Asc => ordering,
                SortDirection::Desc => ordering.reverse(),
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_titles_by_catalog_title(left: &Title, right: &Title) -> Ordering {
    title_catalog_sort_value(left)
        .cmp(&title_catalog_sort_value(right))
        .then_with(|| title_catalog_name_tie_value(left).cmp(&title_catalog_name_tie_value(right)))
        .then_with(|| left.year.cmp(&right.year))
        .then_with(|| left.id.cmp(&right.id))
}

fn title_catalog_sort_value(title: &Title) -> String {
    title_catalog_sort_key_for_title(title)
}

fn title_catalog_name_tie_value(title: &Title) -> String {
    title_catalog_name_tie_key(&title.name)
}

fn title_catalog_quality_profile_id(title: &Title) -> String {
    title
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix(TITLE_QUALITY_PROFILE_TAG_PREFIX))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_lowercase()
}

fn title_catalog_status_sort_value(title: &Title) -> String {
    title
        .content_status
        .as_deref()
        .map(normalize_catalog_status)
        .unwrap_or_default()
}

fn title_matches_catalog_filter(title: &Title, filter: &TitleCatalogFilter) -> bool {
    if !filter.root_folder_ids.is_empty()
        && !filter
            .root_folder_ids
            .iter()
            .any(|root_folder_id| root_folder_id == &title.root_folder_id)
    {
        return false;
    }

    if let Some(minimum_year) = filter.minimum_year
        && title.year.is_none_or(|year| year < minimum_year)
    {
        return false;
    }
    if let Some(maximum_year) = filter.maximum_year
        && title.year.is_none_or(|year| year > maximum_year)
    {
        return false;
    }

    if !title_matches_catalog_tag_filter(title, "genre", &filter.genre_tag_keys)
        || !title_matches_catalog_tag_filter(title, "theme", &filter.theme_tag_keys)
    {
        return false;
    }

    // The fallback repository surface has no normalized rating projection.
    // Treat those titles as unrated; the production store applies this in SQL.
    if filter.minimum_rating.is_some() {
        return false;
    }

    if let Some(monitored) = filter.monitored
        && title.monitored != monitored
    {
        return false;
    }

    if !filter.content_statuses.is_empty() {
        let status = title
            .content_status
            .as_deref()
            .map(normalize_catalog_status)
            .unwrap_or_default();
        if !filter
            .content_statuses
            .iter()
            .any(|candidate| title_catalog_status_filter_matches(*candidate, &status))
        {
            return false;
        }
    }

    true
}

fn title_matches_catalog_tag_filter(title: &Title, category: &str, tag_keys: &[String]) -> bool {
    tag_keys.is_empty()
        || title.canonical_tags.iter().any(|tag| {
            tag.category.eq_ignore_ascii_case(category)
                && tag_keys.iter().any(|tag_key| tag_key == &tag.key)
        })
}

fn title_catalog_filter_counts(
    titles: &[Title],
    active_filter: &TitleCatalogFilter,
) -> TitleCatalogFilterCounts {
    let all_filter = TitleCatalogFilter {
        monitored: None,
        content_statuses: Vec::new(),
        ..active_filter.clone()
    };
    let monitored_filter = TitleCatalogFilter {
        monitored: Some(true),
        content_statuses: active_filter.content_statuses.clone(),
        ..active_filter.clone()
    };
    let unmonitored_filter = TitleCatalogFilter {
        monitored: Some(false),
        content_statuses: active_filter.content_statuses.clone(),
        ..active_filter.clone()
    };
    let continuing_filter = TitleCatalogFilter {
        monitored: active_filter.monitored,
        content_statuses: vec![TitleCatalogContentStatus::Continuing],
        ..active_filter.clone()
    };
    let ended_filter = TitleCatalogFilter {
        monitored: active_filter.monitored,
        content_statuses: vec![TitleCatalogContentStatus::Ended],
        ..active_filter.clone()
    };

    TitleCatalogFilterCounts {
        all: titles
            .iter()
            .filter(|title| title_matches_catalog_filter(title, &all_filter))
            .count(),
        monitored: titles
            .iter()
            .filter(|title| title_matches_catalog_filter(title, &monitored_filter))
            .count(),
        unmonitored: titles
            .iter()
            .filter(|title| title_matches_catalog_filter(title, &unmonitored_filter))
            .count(),
        continuing: titles
            .iter()
            .filter(|title| title_matches_catalog_filter(title, &continuing_filter))
            .count(),
        ended: titles
            .iter()
            .filter(|title| title_matches_catalog_filter(title, &ended_filter))
            .count(),
    }
}

fn title_catalog_status_filter_matches(
    candidate: TitleCatalogContentStatus,
    normalized_status: &str,
) -> bool {
    match candidate {
        TitleCatalogContentStatus::Continuing => normalized_status == "continuing",
        TitleCatalogContentStatus::Ended => normalized_status == "ended",
    }
}

fn normalize_catalog_status(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "returning" => "continuing".to_string(),
        "finished" => "ended".to_string(),
        other => other.to_string(),
    }
}

#[async_trait]
pub trait LibraryRepository: Send + Sync {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>>;
    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>>;
    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library>;
    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library>;
    async fn delete_library(&self, library_id: &str) -> AppResult<bool>;
    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask>;
    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()>;
    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>>;
    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()>;
    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>>;
}

#[derive(Clone, Debug)]
pub struct NewMediaRequest {
    pub id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    pub identity_fingerprint: String,
    pub title: String,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub poster_url: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
    pub requested_quality_profile_id: Option<String>,
    pub requested_quality_profile_name: Option<String>,
    pub requested_monitor_type: Option<String>,
    pub external_ids: Vec<ExternalId>,
    pub created_by_user_id: String,
}

#[derive(Clone, Debug)]
pub struct MediaRequestResolution {
    pub status: scryer_domain::MediaRequestStatus,
    pub resolved_by_user_id: String,
    pub resolved_at: chrono::DateTime<chrono::Utc>,
    pub created_title_id: Option<String>,
    pub approved_quality_profile_id: Option<String>,
    pub approved_quality_profile_name: Option<String>,
    pub event: NewDomainEvent,
}

#[derive(Clone, Debug)]
pub struct MediaRequestSubmissionResult {
    pub request: MediaRequest,
    pub event: DomainEvent,
}

#[derive(Clone, Debug)]
pub struct MediaRequestResolutionResult {
    pub updated: u64,
    pub event: Option<DomainEvent>,
}

#[derive(Clone, Debug)]
pub struct MediaRequestUpdateResult {
    pub request: MediaRequest,
    pub event: DomainEvent,
}

#[derive(Clone, Debug, Default)]
pub struct MediaRequestQuery {
    pub facet: Option<MediaFacet>,
    pub library_ids: Option<Vec<String>>,
    pub status: Option<scryer_domain::MediaRequestStatus>,
    pub requester_user_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaRequestQualityProfileReferenceCounts {
    /// Only pending requests can still consume a requested profile. Approval
    /// creates the title before writing the terminal status, so approved IDs
    /// are protected by the title reference instead of request history.
    pub pending_requested: u64,
}

#[async_trait]
pub trait MediaRequestRepository: Send + Sync {
    async fn submit(
        &self,
        request: NewMediaRequest,
        requester: &User,
        submitted_event: NewDomainEvent,
    ) -> AppResult<MediaRequestSubmissionResult>;

    async fn get(&self, request_id: &str) -> AppResult<Option<MediaRequest>>;

    async fn resolve_pending_overlapping(
        &self,
        request: &MediaRequest,
        resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult>;

    async fn resolve_pending(
        &self,
        request_id: &str,
        resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult>;

    async fn update_pending_request_preferences(
        &self,
        request_id: &str,
        requested_quality_profile_id: String,
        requested_quality_profile_name: String,
        requested_monitor_type: Option<String>,
        updated_event: NewDomainEvent,
    ) -> AppResult<MediaRequestUpdateResult>;

    async fn count_pending_by_facet(&self, library_ids: &[String])
    -> AppResult<MediaRequestCounts>;

    async fn count_quality_profile_references(
        &self,
        profile_id: &str,
    ) -> AppResult<MediaRequestQualityProfileReferenceCounts> {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Ok(MediaRequestQualityProfileReferenceCounts::default());
        }
        let requests = self.list(MediaRequestQuery::default()).await?;
        Ok(MediaRequestQualityProfileReferenceCounts {
            pending_requested: requests
                .iter()
                .filter(|request| {
                    request.status == scryer_domain::MediaRequestStatus::Pending
                        && request
                            .requested_quality_profile_id
                            .as_deref()
                            .is_some_and(|value| value.eq_ignore_ascii_case(profile_id))
                })
                .count() as u64,
        })
    }

    async fn list(&self, query: MediaRequestQuery) -> AppResult<Vec<MediaRequest>>;
}

#[async_trait]
pub trait TitleImageRepository: Send + Sync {
    async fn list_title_image_refresh_work(
        &self,
        limit: usize,
        skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>>;

    async fn clear_title_image_cache(&self) -> AppResult<()>;

    async fn upsert_title_image_source_result(
        &self,
        title_id: &str,
        result: TitleImageSourceResult,
        event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>>;

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageProxyKind {
    Poster,
    Fanart,
    EpisodeStill,
    Person,
}

impl ImageProxyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Poster => "poster",
            Self::Fanart => "fanart",
            Self::EpisodeStill => "episode_still",
            Self::Person => "person",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProxyRegistration {
    pub upstream_url: Option<String>,
    pub owner_type: Option<String>,
    pub owner_id: Option<String>,
    pub image_kind: ImageProxyKind,
    pub fallback_class: String,
    pub default_variant: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProxySourceRecord {
    pub token: String,
    pub upstream_url: Option<String>,
    pub owner_type: Option<String>,
    pub owner_id: Option<String>,
    pub image_kind: String,
    pub fallback_class: String,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProxyCacheEntryRecord {
    pub token: String,
    pub variant: String,
    pub content_type: String,
    pub byte_size: i64,
    pub upstream_etag: Option<String>,
    pub upstream_last_modified: Option<String>,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
    pub last_accessed_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub trait ImageProxyRepository: Send + Sync {
    /// Registers a server-approved image source and returns its Scryer-owned URL.
    /// Implementations must never accept registrations from the HTTP image route.
    fn register_image_source(&self, registration: ImageProxyRegistration) -> String;
    async fn flush_image_proxy_sources(&self) -> AppResult<()>;
    fn clear_image_proxy_memory(&self);

    async fn get_image_proxy_source(
        &self,
        token: &str,
    ) -> AppResult<Option<ImageProxySourceRecord>>;

    async fn get_image_proxy_cache_entry(
        &self,
        token: &str,
        variant: &str,
    ) -> AppResult<Option<ImageProxyCacheEntryRecord>>;

    async fn upsert_image_proxy_cache_entry(
        &self,
        entry: &ImageProxyCacheEntryRecord,
    ) -> AppResult<()>;

    async fn touch_image_proxy_cache_entry(
        &self,
        token: &str,
        variant: &str,
        observed_fetched_at: chrono::DateTime<chrono::Utc>,
        last_accessed_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()>;

    async fn delete_image_proxy_cache_entry(&self, token: &str, variant: &str) -> AppResult<()>;

    async fn list_image_proxy_cache_entries_lru(
        &self,
    ) -> AppResult<Vec<ImageProxyCacheEntryRecord>>;

    async fn clear_image_proxy_cache_entries(&self) -> AppResult<()>;

    async fn prune_image_proxy_sources_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<u64>;
}

#[async_trait]
pub trait ImageProxyCacheControl: Send + Sync {
    async fn clear_cache(&self) -> AppResult<()>;
    async fn set_configured_max_bytes(&self, value: u64) -> AppResult<()>;
}

#[async_trait]
pub trait TitleImageProcessor: Send + Sync {
    async fn fetch_and_process_image(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        variants: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult>;
}

#[async_trait]
pub trait ShowRepository: Send + Sync {
    async fn list_series_movie_links_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SeriesMovieLink>>;
    /// Batch-load series-movie links for many series titles. Returns a flat list
    /// (each link carries `series_title_id`); callers group by that field. The
    /// default fans out to the singular query; SQL stores override with `IN`.
    async fn list_series_movie_links_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_domain::SeriesMovieLink>> {
        let mut links = Vec::new();
        for title_id in title_ids {
            links.extend(self.list_series_movie_links_for_title(title_id).await?);
        }
        Ok(links)
    }
    async fn list_series_movie_external_id_lookup_matches(
        &self,
        library_ids: &[String],
        lookups: &[TitleExternalIdLookup],
    ) -> AppResult<Vec<SeriesMovieExternalIdLookupMatch>>;
    async fn get_series_movie_link_by_id(
        &self,
        link_id: &str,
    ) -> AppResult<Option<scryer_domain::SeriesMovieLink>>;
    async fn list_movie_entity_credits(
        &self,
        movie_entity_id: &str,
    ) -> AppResult<Vec<crate::TitleCredit>> {
        let _ = movie_entity_id;
        Ok(Vec::new())
    }
    async fn find_series_movie_link_by_legacy_collection_id(
        &self,
        collection_id: &str,
    ) -> AppResult<Option<scryer_domain::SeriesMovieLink>>;
    async fn upsert_series_movie_link(
        &self,
        link: scryer_domain::SeriesMovieLink,
    ) -> AppResult<scryer_domain::SeriesMovieLink>;
    async fn delete_stale_series_movie_links(
        &self,
        title_id: &str,
        retained_link_ids: &[String],
    ) -> AppResult<()>;
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>>;
    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>>;
    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>>;
    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>>;
    /// Batch-load collections by id for dataloaders. Missing ids are absent from
    /// the result. The default fans out to `get_collection_by_id`; SQL stores
    /// override with a single `IN` query.
    async fn get_collections_by_ids(&self, ids: &[String]) -> AppResult<Vec<Collection>> {
        let mut collections = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(collection) = self.get_collection_by_id(id).await? {
                collections.push(collection);
            }
        }
        Ok(collections)
    }
    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>>;
    /// Batch-load collections by ordered path. Paths without a collection are
    /// absent from the result. The default fans out to
    /// `get_collection_by_ordered_path`; SQL stores override with one query.
    async fn list_collections_by_ordered_paths(
        &self,
        ordered_paths: &[String],
    ) -> AppResult<Vec<Collection>> {
        let mut collections = Vec::with_capacity(ordered_paths.len());
        for ordered_path in ordered_paths {
            if let Some(collection) = self.get_collection_by_ordered_path(ordered_path).await? {
                collections.push(collection);
            }
        }
        Ok(collections)
    }
    async fn create_collection(&self, collection: Collection) -> AppResult<Collection>;
    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection>;
    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()>;
    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()>;
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()>;
    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>>;
    /// Batch-load episodes for many collections. Returns a flat list (each
    /// episode carries `collection_id`); callers group by that field. The
    /// default fans out to `list_episodes_for_collection`; SQL stores override
    /// with a single `IN` query.
    async fn list_episodes_for_collections(
        &self,
        collection_ids: &[String],
    ) -> AppResult<Vec<Episode>> {
        let mut episodes = Vec::new();
        for collection_id in collection_ids {
            episodes.extend(self.list_episodes_for_collection(collection_id).await?);
        }
        Ok(episodes)
    }
    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>>;
    async fn list_episode_external_ids(&self, episode_id: &str)
    -> AppResult<Vec<ScopedExternalId>>;
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>>;
    /// Batch-load episodes by id for dataloaders. Missing ids are absent from
    /// the result. The default fans out to `get_episode_by_id`; SQL stores
    /// override with a single `IN` query.
    async fn get_episodes_by_ids(&self, ids: &[String]) -> AppResult<Vec<Episode>> {
        let mut episodes = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(episode) = self.get_episode_by_id(id).await? {
                episodes.push(episode);
            }
        }
        Ok(episodes)
    }
    async fn create_episode(&self, episode: Episode) -> AppResult<Episode>;
    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode>;
    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()>;
    async fn delete_episode(&self, episode_id: &str) -> AppResult<()>;
    async fn update_episode_image_urls(
        &self,
        _updates: &[EpisodeImageUrlUpdate],
    ) -> AppResult<u64> {
        Ok(0)
    }
    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>>;
    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>>;
    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>>;
    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>>;
    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>>;
    async fn get_login_snapshot_by_username(
        &self,
        username: &str,
    ) -> AppResult<Option<crate::types::UserLoginSnapshot>> {
        let Some(user) = self.get_by_username(username).await? else {
            return Ok(None);
        };
        let auth_session_version = self.auth_session_version(&user.id).await?;
        Ok(Some(crate::types::UserLoginSnapshot {
            user,
            auth_session_version,
        }))
    }
    async fn create(&self, user: User) -> AppResult<User>;
    async fn list_all(&self) -> AppResult<Vec<User>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>>;
    async fn auth_session_version(&self, user_id: &str) -> AppResult<Option<String>>;
    async fn rotate_auth_session_version(
        &self,
        _user_id: &str,
        _auth_session_version: &str,
    ) -> AppResult<User> {
        Err(AppError::Repository(
            "authentication-session rotation is not configured".into(),
        ))
    }
    async fn reset_authentication_factors_and_invalidate_sessions(
        &self,
        _user_id: &str,
        _auth_session_version: &str,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "authentication-factor recovery is not configured".into(),
        ))
    }
    async fn update_password_and_invalidate_sessions(
        &self,
        id: &str,
        password_hash: String,
        password_change_required: bool,
        auth_session_version: &str,
    ) -> AppResult<User>;
    async fn complete_required_password_change(
        &self,
        _id: &str,
        _password_hash: String,
        _expected_auth_session_version: &Option<String>,
        _auth_session_version: &str,
    ) -> AppResult<User> {
        Err(AppError::Repository(
            "temporary-password replacement is not configured".into(),
        ))
    }
    async fn update_login_status_and_rotate_session(
        &self,
        id: &str,
        status: scryer_domain::UserLoginStatus,
        auth_session_version: &str,
    ) -> AppResult<User>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait UserUiSettingsRepository: Send + Sync {
    async fn get_by_user_id(&self, user_id: &str) -> AppResult<Option<UiSettings>>;
    async fn upsert(&self, user_id: &str, settings: UiSettingsUpdate) -> AppResult<UiSettings>;
}

#[async_trait]
pub trait OAuthRepository: Send + Sync {
    async fn create_api_key(&self, record: ApiKeyRecord) -> AppResult<ApiKeyRecord>;
    async fn get_api_key_by_lookup_id(&self, lookup_id: &str) -> AppResult<Option<ApiKeyRecord>>;
    async fn list_api_keys(&self, user_id: &str) -> AppResult<Vec<ApiKeyRecord>>;
    async fn list_environment_api_keys(&self) -> AppResult<Vec<ApiKeyRecord>>;
    async fn upsert_environment_api_key(&self, record: ApiKeyRecord) -> AppResult<ApiKeyRecord>;
    async fn revoke_api_key(
        &self,
        id: &str,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool>;
    async fn touch_api_key_last_used(
        &self,
        id: &str,
        used_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool>;
    async fn create_client_registration(
        &self,
        record: OAuthClientRegistrationRecord,
    ) -> AppResult<OAuthClientRegistrationRecord>;
    async fn get_client_registration(
        &self,
        client_id: &str,
    ) -> AppResult<Option<OAuthClientRegistrationRecord>>;
    async fn list_client_registrations(&self) -> AppResult<Vec<OAuthClientRegistrationRecord>>;
    async fn update_client_registration(
        &self,
        record: OAuthClientRegistrationRecord,
        revoke_grants: bool,
        revoked_at: chrono::DateTime<chrono::Utc>,
        revoke_reason: &str,
    ) -> AppResult<Option<OAuthClientRegistrationRecord>>;
    async fn delete_client_registration(
        &self,
        client_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        revoke_reason: &str,
    ) -> AppResult<bool>;
    async fn is_refresh_grant_active(&self, grant_id: &str, client_id: &str) -> AppResult<bool>;
    async fn create_authorization_code(
        &self,
        record: OAuthAuthorizationCodeRecord,
    ) -> AppResult<OAuthAuthorizationCodeRecord>;
    async fn get_authorization_code(
        &self,
        id: &str,
    ) -> AppResult<Option<OAuthAuthorizationCodeRecord>>;
    async fn consume_authorization_code(
        &self,
        id: &str,
        consumed_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool>;
    async fn consume_authorization_code_and_create_refresh_grant(
        &self,
        code: OAuthAuthorizationCodeRecord,
        consumed_at: chrono::DateTime<chrono::Utc>,
        grant: OAuthRefreshGrantRecord,
        token: OAuthRefreshTokenRecord,
        require_active_client_registration: bool,
    ) -> AppResult<Option<OAuthRefreshGrantRecord>>;
    async fn create_refresh_grant(
        &self,
        grant: OAuthRefreshGrantRecord,
        token: OAuthRefreshTokenRecord,
        require_active_client_registration: bool,
    ) -> AppResult<OAuthRefreshGrantRecord>;
    async fn get_refresh_token(
        &self,
        id: &str,
    ) -> AppResult<Option<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)>>;
    async fn rotate_refresh_token(
        &self,
        token_id: &str,
        consumed_at: chrono::DateTime<chrono::Utc>,
        next_token: OAuthRefreshTokenRecord,
    ) -> AppResult<OAuthRefreshRotationOutcome>;
    async fn revoke_refresh_grant(
        &self,
        grant_id: &str,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<bool>;
    async fn revoke_refresh_family(
        &self,
        family_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64>;
    async fn revoke_user_refresh_grants(
        &self,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64>;
    async fn revoke_authless_refresh_grants(
        &self,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64>;
    async fn touch_refresh_grant_last_used(
        &self,
        grant_id: &str,
        client_id: &str,
        used_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool>;
    async fn list_connected_apps(&self, user_id: &str) -> AppResult<Vec<OAuthConnectedAppRecord>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedExternalIdentity {
    pub provider: scryer_domain::ExternalAccountProvider,
    pub connection_id: String,
    pub external_user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Whether the remote account has a password configured, when the provider
    /// reports it. `None` means the provider does not expose the fact (Plex
    /// verifies through a PIN exchange, not a password), so callers must not
    /// treat unknown as "no password".
    pub remote_password_configured: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlexServerDiscovery {
    pub id: String,
    pub name: String,
}

#[async_trait]
pub trait UserExternalAccountRepository: Send + Sync {
    async fn create(
        &self,
        account: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount>;
    async fn list_by_user_id(
        &self,
        user_id: &str,
    ) -> AppResult<Vec<scryer_domain::UserExternalAccount>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::UserExternalAccount>>;
    async fn get_by_provider_identity(
        &self,
        provider: scryer_domain::ExternalAccountProvider,
        connection_id: &str,
        external_user_id: &str,
    ) -> AppResult<Option<scryer_domain::UserExternalAccount>>;
    async fn get_pending_claim_by_provider_username(
        &self,
        provider: scryer_domain::ExternalAccountProvider,
        connection_id: &str,
        username: &str,
    ) -> AppResult<Option<scryer_domain::UserExternalAccount>>;
    async fn update(
        &self,
        account: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount>;
    async fn create_auto_added_user_with_account(
        &self,
        user: scryer_domain::User,
        app_permissions: scryer_domain::AppPermissionMask,
        library_grants: Vec<scryer_domain::LibraryGrant>,
        account: scryer_domain::UserExternalAccount,
    ) -> AppResult<(scryer_domain::User, scryer_domain::UserExternalAccount)>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait MediaServerConnectionRepository: Send + Sync {
    async fn list(
        &self,
        provider: Option<scryer_domain::MediaServerProvider>,
    ) -> AppResult<Vec<scryer_domain::MediaServerConnection>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::MediaServerConnection>>;
    async fn create(
        &self,
        connection: scryer_domain::MediaServerConnection,
    ) -> AppResult<scryer_domain::MediaServerConnection>;
    async fn update(
        &self,
        connection: scryer_domain::MediaServerConnection,
    ) -> AppResult<scryer_domain::MediaServerConnection>;
    async fn list_playback_items_for_entity(
        &self,
        entity_kind: scryer_domain::MediaServerPlaybackEntityKind,
        entity_id: &str,
    ) -> AppResult<Vec<scryer_domain::MediaServerPlaybackItem>>;
    async fn list_playback_items_for_entities(
        &self,
        entities: &[(scryer_domain::MediaServerPlaybackEntityKind, String)],
    ) -> AppResult<Vec<scryer_domain::MediaServerPlaybackItem>> {
        let mut items = Vec::new();
        for (entity_kind, entity_id) in entities {
            items.extend(
                self.list_playback_items_for_entity(*entity_kind, entity_id)
                    .await?,
            );
        }
        Ok(items)
    }
    async fn upsert_playback_items_for_connection(
        &self,
        _connection_id: &str,
        _items: Vec<scryer_domain::MediaServerPlaybackItem>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "incremental media server playback mapping updates are not configured".into(),
        ))
    }
    /// Atomically replace every discovered playback mapping for one connection.
    ///
    /// Callers use this after a successful full catalog scan so stale provider
    /// item IDs are removed without exposing a partially scanned catalog.
    async fn replace_playback_items_for_connection(
        &self,
        connection_id: &str,
        items: Vec<scryer_domain::MediaServerPlaybackItem>,
    ) -> AppResult<()>;
    async fn compare_and_set_emby_base_url(
        &self,
        _connection_id: &str,
        _expected_base_url: &str,
        _expected_server_id: &str,
        _new_base_url: &str,
    ) -> AppResult<bool> {
        Ok(false)
    }
    async fn delete(&self, id: &str) -> AppResult<()>;
    async fn has_external_accounts(&self, id: &str) -> AppResult<bool>;
    async fn has_notification_channels(&self, id: &str) -> AppResult<bool>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JellyfinServerUser {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbyServerIdentity {
    pub api_base_url: String,
    pub server_id: String,
    pub server_name: String,
    pub version: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbyApiKeyExchange {
    pub api_key: String,
    pub server_identity: EmbyServerIdentity,
    pub created_new_key: bool,
    pub cleanup: Option<EmbyApiKeyExchangeCleanup>,
}

impl std::fmt::Debug for EmbyApiKeyExchange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmbyApiKeyExchange")
            .field("api_key", &"[REDACTED]")
            .field("server_identity", &self.server_identity)
            .field("created_new_key", &self.created_new_key)
            .field("cleanup", &self.cleanup.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbyApiKeyExchangeCleanup {
    api_base_url: String,
    local_user_id: String,
    session_access_token: String,
    created_api_key: Option<String>,
}

impl EmbyApiKeyExchangeCleanup {
    pub fn new(
        api_base_url: String,
        local_user_id: String,
        session_access_token: String,
        created_api_key: Option<String>,
    ) -> Self {
        Self {
            api_base_url,
            local_user_id,
            session_access_token,
            created_api_key,
        }
    }

    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }

    pub fn local_user_id(&self) -> &str {
        &self.local_user_id
    }

    pub fn session_access_token(&self) -> &str {
        &self.session_access_token
    }

    pub fn created_api_key(&self) -> Option<&str> {
        self.created_api_key.as_deref()
    }
}

impl std::fmt::Debug for EmbyApiKeyExchangeCleanup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmbyApiKeyExchangeCleanup")
            .field("api_base_url", &self.api_base_url)
            .field("local_user_id", &"[REDACTED]")
            .field("session_access_token", &"[REDACTED]")
            .field(
                "created_api_key",
                &self.created_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbyConnectIdentityVerification {
    pub identity: VerifiedExternalIdentity,
    pub resolved_api_base_url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbyConnectAddressStatus {
    Reachable,
    Unreachable,
    InvalidUrl,
    ServerIdMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbyConnectUserType {
    LinkedUser,
    Guest,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbyConnectServer {
    pub server_id: String,
    pub name: String,
    pub user_type: EmbyConnectUserType,
    pub local_address: Option<String>,
    pub remote_address: Option<String>,
    pub local_api_base_url: Option<String>,
    pub remote_api_base_url: Option<String>,
    pub local_status: EmbyConnectAddressStatus,
    pub remote_status: EmbyConnectAddressStatus,
    pub suggested_base_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbyServerUser {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbyAvatar {
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlexServerUser {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaServerUser {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

impl From<JellyfinServerUser> for MediaServerUser {
    fn from(user: JellyfinServerUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            avatar_url: user.avatar_url,
        }
    }
}

impl From<PlexServerUser> for MediaServerUser {
    fn from(user: PlexServerUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            avatar_url: user.avatar_url,
        }
    }
}

impl From<EmbyServerUser> for MediaServerUser {
    fn from(user: EmbyServerUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            avatar_url: user.avatar_url,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaServerUserGroupStatus {
    Ready,
    MissingCredentials,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaServerUserGroup {
    pub connection_id: String,
    pub connection_name: String,
    pub provider: scryer_domain::ExternalAccountProvider,
    pub status: MediaServerUserGroupStatus,
    pub error_message: Option<String>,
    pub users: Vec<MediaServerUser>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaServerCatalogItemKind {
    Movie,
    Series,
    Episode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaServerCatalogItem {
    pub kind: MediaServerCatalogItemKind,
    pub provider_item_id: String,
    pub external_ids: Vec<scryer_domain::ExternalId>,
    pub series_provider_item_id: Option<String>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub episode_number_end: Option<i32>,
}

#[async_trait]
pub trait ExternalIdentityVerifier: Send + Sync {
    /// List visible provider items for exact playback-link matching.
    async fn scan_media_server_catalog(
        &self,
        _connection: &scryer_domain::MediaServerConnection,
    ) -> AppResult<Vec<MediaServerCatalogItem>> {
        Err(AppError::Repository(
            "media server catalog scanning is not configured".into(),
        ))
    }

    /// List recently added or changed provider items without clearing stale mappings.
    async fn scan_media_server_catalog_incremental(
        &self,
        connection: &scryer_domain::MediaServerConnection,
    ) -> AppResult<Vec<MediaServerCatalogItem>> {
        self.scan_media_server_catalog(connection).await
    }

    async fn verify_plex(
        &self,
        connection_id: &str,
        machine_id: Option<&str>,
        plex_auth_token: &str,
    ) -> AppResult<VerifiedExternalIdentity>;

    async fn discover_plex_servers(
        &self,
        plex_auth_token: &str,
    ) -> AppResult<Vec<PlexServerDiscovery>>;

    async fn verify_jellyfin(
        &self,
        connection_id: &str,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> AppResult<VerifiedExternalIdentity>;

    async fn test_jellyfin_connection(&self, base_url: &str) -> AppResult<()>;
    async fn test_jellyfin_api_key(&self, base_url: &str, api_key: &str) -> AppResult<()>;
    async fn exchange_jellyfin_admin_api_key(
        &self,
        connection_id: &str,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> AppResult<String>;
    async fn list_jellyfin_users(
        &self,
        base_url: &str,
        api_key: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>>;
    async fn resolve_emby_api_base(
        &self,
        _connection_id: &str,
        _base_url: &str,
    ) -> AppResult<EmbyServerIdentity> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn test_emby_api_key(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _api_key: &str,
        _expected_server_id: Option<&str>,
    ) -> AppResult<EmbyServerIdentity> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn exchange_emby_local_admin_api_key(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _username: &str,
        _password: &str,
    ) -> AppResult<EmbyApiKeyExchange> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn discover_emby_connect_servers(
        &self,
        _username_or_email: &str,
        _password: &str,
    ) -> AppResult<Vec<EmbyConnectServer>> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn exchange_emby_connect_admin_api_key(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _server_id: &str,
        _username_or_email: &str,
        _password: &str,
    ) -> AppResult<EmbyApiKeyExchange> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn finish_emby_api_key_exchange(
        &self,
        _connection_id: &str,
        _cleanup: EmbyApiKeyExchangeCleanup,
        _compensate_created_key: bool,
    ) {
    }
    async fn verify_emby_local_identity(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _expected_server_id: &str,
        _username: &str,
        _password: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn verify_emby_connect_identity(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _expected_server_id: &str,
        _username_or_email: &str,
        _password: &str,
    ) -> AppResult<EmbyConnectIdentityVerification> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn test_emby_connect_identity(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _expected_server_id: &str,
        _username_or_email: &str,
        _password: &str,
    ) -> AppResult<EmbyConnectIdentityVerification> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn list_emby_users(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _api_key: &str,
        _search: Option<&str>,
    ) -> AppResult<Vec<EmbyServerUser>> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn fetch_emby_user_avatar(
        &self,
        _connection_id: &str,
        _base_url: &str,
        _api_key: &str,
        _user_id: &str,
        _image_tag: &str,
    ) -> AppResult<Option<EmbyAvatar>> {
        Err(AppError::Repository(
            "Emby integration is not configured".into(),
        ))
    }
    async fn list_plex_users(
        &self,
        plex_auth_token: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<PlexServerUser>>;
}

#[async_trait]
pub trait WebauthnRepository: Send + Sync {
    async fn list_credentials_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Vec<WebauthnCredentialRecord>>;
    async fn get_credential_by_id_for_user(
        &self,
        credential_record_id: &str,
        user_id: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>>;
    async fn get_credential_by_credential_id(
        &self,
        credential_id: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>>;
    async fn create_credential(
        &self,
        credential: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord>;
    /// Installs a passkey only while the observed authentication session remains current.
    async fn create_credential_for_current_session(
        &self,
        _credential: WebauthnCredentialRecord,
        _expected_auth_session_version: Option<&str>,
    ) -> AppResult<WebauthnCredentialRecord> {
        Err(AppError::Repository(
            "atomic passkey creation for the current session is not configured".into(),
        ))
    }
    async fn update_credential(
        &self,
        credential: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord>;
    async fn update_credential_if_current(
        &self,
        credential: WebauthnCredentialRecord,
        expected_credential_json: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>>;
    async fn delete_credential_for_user(
        &self,
        credential_record_id: &str,
        user_id: &str,
    ) -> AppResult<()>;
    /// Deletes a passkey only when another sign-in route and the current session remain valid.
    async fn delete_credential_preserving_login_route_for_current_session(
        &self,
        _credential_record_id: &str,
        _user_id: &str,
        _expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "atomic passkey deletion for the current session is not configured".into(),
        ))
    }
    async fn create_challenge(
        &self,
        challenge: WebauthnChallengeRecord,
    ) -> AppResult<WebauthnChallengeRecord>;
    async fn get_challenge(&self, id: &str) -> AppResult<Option<WebauthnChallengeRecord>>;
    async fn take_challenge(&self, id: &str) -> AppResult<Option<WebauthnChallengeRecord>>;
    async fn delete_challenge(&self, id: &str) -> AppResult<()>;
    async fn delete_expired_challenges(&self, now: &str) -> AppResult<u64>;
    async fn create_login_verification_challenge(
        &self,
        _challenge: LoginVerificationChallengeRecord,
        _expected_auth_session_version: &Option<String>,
    ) -> AppResult<LoginVerificationChallengeRecord> {
        Err(AppError::Repository(
            "login verification challenges are not configured".into(),
        ))
    }
    async fn get_login_verification_challenge(
        &self,
        _id: &str,
    ) -> AppResult<Option<LoginVerificationChallengeRecord>> {
        Ok(None)
    }
    async fn take_login_verification_challenge(
        &self,
        _id: &str,
    ) -> AppResult<Option<LoginVerificationChallengeRecord>> {
        Ok(None)
    }
    async fn delete_login_verification_challenges_for_user(
        &self,
        _user_id: &str,
    ) -> AppResult<u64> {
        Ok(0)
    }
    async fn delete_expired_login_verification_challenges(&self, _now: &str) -> AppResult<u64> {
        Ok(0)
    }
}

#[async_trait]
pub trait TotpRepository: Send + Sync {
    async fn get_credential_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Option<TotpCredentialRecord>>;
    async fn upsert_credential(
        &self,
        credential: TotpCredentialRecord,
    ) -> AppResult<TotpCredentialRecord>;
    async fn delete_credential_for_user(&self, user_id: &str) -> AppResult<()>;
    async fn create_enrollment_challenge(
        &self,
        challenge: TotpEnrollmentChallengeRecord,
    ) -> AppResult<TotpEnrollmentChallengeRecord>;
    async fn get_enrollment_challenge(
        &self,
        id: &str,
        user_id: &str,
    ) -> AppResult<Option<TotpEnrollmentChallengeRecord>>;
    async fn delete_enrollment_challenge(&self, id: &str, user_id: &str) -> AppResult<()>;
    async fn delete_enrollment_challenges_for_user(&self, user_id: &str) -> AppResult<u64>;
    async fn delete_expired_enrollment_challenges(&self, now: &str) -> AppResult<u64>;
    async fn reset_user_mfa_and_invalidate_sessions(
        &self,
        user_id: &str,
        auth_session_version: &str,
    ) -> AppResult<()>;
    /// Atomically installs a newly verified TOTP factor while the observed session remains current.
    async fn complete_enrollment_for_current_session(
        &self,
        _credential: TotpCredentialRecord,
        _challenge_id: &str,
        _recovery_codes: Vec<TotpRecoveryCodeRecord>,
        _expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "atomic TOTP enrollment completion for the current session is not configured".into(),
        ))
    }
    /// Atomically removes a TOTP factor and its related secrets for the current session.
    async fn disable_for_current_session(
        &self,
        _user_id: &str,
        _expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "atomic TOTP disablement for the current session is not configured".into(),
        ))
    }
    /// Atomically replaces recovery codes for the current session.
    async fn replace_recovery_codes_for_current_session(
        &self,
        _user_id: &str,
        _codes: Vec<TotpRecoveryCodeRecord>,
        _expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "atomic recovery-code replacement for the current session is not configured".into(),
        ))
    }
    async fn replace_recovery_codes(
        &self,
        user_id: &str,
        codes: Vec<TotpRecoveryCodeRecord>,
    ) -> AppResult<()>;
    async fn list_recovery_codes_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Vec<TotpRecoveryCodeRecord>>;
    async fn mark_recovery_code_used(
        &self,
        id: &str,
        user_id: &str,
        used_at: &str,
    ) -> AppResult<()>;
    /// Atomically reserves one verification attempt for the credential's rolling window.
    async fn reserve_totp_attempt(
        &self,
        user_id: &str,
        attempted_at: &str,
        window_started_after: &str,
        limit: i32,
    ) -> AppResult<bool> {
        let _ = (user_id, attempted_at, window_started_after, limit);
        Err(AppError::Repository(
            "atomic TOTP attempt reservations are not configured".into(),
        ))
    }
    /// Clears the rolling attempt reservation after a successful verification.
    async fn clear_totp_attempt_reservations(&self, user_id: &str) -> AppResult<()> {
        let _ = user_id;
        Err(AppError::Repository(
            "atomic TOTP attempt reservations are not configured".into(),
        ))
    }
    /// Atomically accepts one previously unused TOTP time step.
    async fn claim_totp_step(&self, user_id: &str, step: i64, used_at: &str) -> AppResult<bool> {
        let _ = (user_id, step, used_at);
        Err(AppError::Repository(
            "atomic TOTP step claims are not configured".into(),
        ))
    }
    async fn record_failed_attempt(&self, attempt: TotpFailedAttemptRecord) -> AppResult<()>;
    async fn count_failed_attempts_since(&self, user_id: &str, since: &str) -> AppResult<i64>;
    async fn clear_failed_attempts(&self, user_id: &str) -> AppResult<u64>;
}

#[async_trait]
pub trait DomainEventRepository: Send + Sync {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent>;
    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>>;
    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>>;
    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64>;
    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>>;
    /// Aggregate dashboard activity counts for two adjacent time windows in one
    /// grouped SQL query. Events are restricted to titles in `library_ids`, the
    /// current window is `[current_start, current_end)`, and the previous window
    /// is `[previous_start, current_start)`. An empty `library_ids` counts nothing.
    async fn count_dashboard_activity_events(
        &self,
        library_ids: &[String],
        previous_start: chrono::DateTime<chrono::Utc>,
        current_start: chrono::DateTime<chrono::Utc>,
        current_end: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<DashboardActivityStats>;
    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>>;
    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64>;
    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerSystemBackoff {
    pub disabled_until: chrono::DateTime<chrono::Utc>,
    pub escalation_level: usize,
}

#[async_trait]
pub trait IndexerConfigRepository: Send + Sync {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>>;
    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig>;
    async fn touch_last_error(&self, id: &str) -> AppResult<()>;
    async fn record_last_error(&self, id: &str, _message: Option<String>) -> AppResult<()> {
        self.touch_last_error(id).await
    }
    async fn clear_last_error(&self, _id: &str) -> AppResult<()> {
        Ok(())
    }
    async fn list_system_backoffs(
        &self,
    ) -> AppResult<std::collections::HashMap<String, IndexerSystemBackoff>> {
        Ok(std::collections::HashMap::new())
    }
    async fn set_system_backoff(&self, _id: &str, _backoff: IndexerSystemBackoff) -> AppResult<()> {
        Ok(())
    }
    async fn clear_system_backoff(&self, _id: &str) -> AppResult<()> {
        Ok(())
    }
    async fn update(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig>;
    async fn set_download_client_mapping(
        &self,
        indexer_id: &str,
        download_client_id: Option<String>,
    ) -> AppResult<IndexerConfig> {
        self.update(IndexerConfigUpdate {
            id: indexer_id.to_string(),
            download_client_id: Some(download_client_id),
            ..IndexerConfigUpdate::default()
        })
        .await
    }
    async fn set_seeding_profile_mapping(
        &self,
        indexer_id: &str,
        seeding_profile_id: Option<String>,
    ) -> AppResult<IndexerConfig> {
        self.update(IndexerConfigUpdate {
            id: indexer_id.to_string(),
            seeding_profile_id: Some(seeding_profile_id),
            ..IndexerConfigUpdate::default()
        })
        .await
    }
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait IndexerProxyConfigRepository: Send + Sync {
    async fn list(
        &self,
        provider_type: Option<scryer_domain::IndexerProxyProviderType>,
    ) -> AppResult<Vec<scryer_domain::IndexerProxyConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::IndexerProxyConfig>>;
    async fn create(
        &self,
        config: scryer_domain::IndexerProxyConfig,
    ) -> AppResult<scryer_domain::IndexerProxyConfig>;
    async fn update(
        &self,
        config: scryer_domain::IndexerProxyConfig,
    ) -> AppResult<scryer_domain::IndexerProxyConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    /// Persist a health observation without bumping `updated_at`, which
    /// doubles as the plugin client cache revision for proxied indexers.
    async fn record_health(
        &self,
        id: &str,
        status: scryer_domain::IndexerProxyHealthStatus,
        error_message: Option<String>,
        error_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait IndexerCapsSnapshotRefresher: Send + Sync {
    async fn fetch_for_config(
        &self,
        config: &IndexerConfig,
    ) -> AppResult<Option<IndexerCapsSnapshot>>;
}

/// One coverage-ledger row: the raw `(scope_key, indexer_id)`
/// coverage a batched page fetch returns, so the wanted views can compute
/// covered/routed counts in memory for a whole page in one round-trip instead of
/// a per-row lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeCoverageRow {
    pub scope_key: String,
    pub indexer_id: String,
    pub fingerprint: String,
    pub searched_at: String,
}

/// Convergence ledger: which indexers an acquisition scope's catalog has
/// been actively searched against, under which search-criteria fingerprint. A
/// scope is "converged" (RSS-only) once every routed indexer has a
/// current-fingerprint coverage row; a fingerprint change or a new indexer
/// re-opens convergence. Coverage is recorded only for a search that actually
/// queried the indexer (results incl. empty) — never a deferral/error.
#[async_trait]
pub trait ScopeIndexerCoverageRepository: Send + Sync {
    /// Upsert coverage for `(scope_key, facet, indexer_id)`, overwriting any
    /// prior fingerprint/timestamp (a re-search under a new fingerprint replaces
    /// the old row).
    async fn record_coverage(
        &self,
        scope_key: &str,
        facet: &str,
        indexer_id: &str,
        fingerprint: &str,
    ) -> AppResult<()>;

    /// Indexer ids that currently have coverage for the scope matching
    /// `fingerprint` (rows with a stale fingerprint are excluded, i.e. treated as
    /// uncovered). `stale_before`, when set, additionally excludes rows searched
    /// before it (the optional slow re-converge backstop).
    async fn covered_indexers(
        &self,
        scope_key: &str,
        facet: &str,
        fingerprint: &str,
        stale_before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<Vec<String>>;

    /// Delete every coverage row for `scope_key`, forcing all routed indexers
    /// to be searched again on the next convergence pass.
    async fn prune_scope(&self, scope_key: &str) -> AppResult<()>;

    /// Delete coverage rows for one indexer within `scope_key`, forcing only
    /// that indexer to be searched again on the next convergence pass.
    async fn prune_scope_indexer(&self, scope_key: &str, indexer_id: &str) -> AppResult<()>;

    /// Delete coverage for one indexer across every scope. Used when the
    /// indexer's search contract or health is no longer trustworthy.
    async fn prune_indexer(&self, _indexer_id: &str) -> AppResult<()> {
        Ok(())
    }

    /// All coverage rows for the given scope keys, fetched in one round-trip
    ///. The wanted views group these by scope key and compare
    /// each row's `fingerprint` to the live one in memory, so a full page's
    /// convergence progress costs a single query — never a per-row lookup. Rows
    /// for scope keys with no coverage are simply absent.
    async fn list_coverage_for_scope_keys(
        &self,
        scope_keys: &[String],
    ) -> AppResult<Vec<ScopeCoverageRow>>;
}

#[async_trait]
pub trait DownloadClientConfigRepository: Send + Sync {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>>;
    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig>;
    async fn update(&self, update: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    async fn delete_with_cleared_indexer_mapping_count(&self, id: &str) -> AppResult<u64> {
        self.delete(id).await?;
        Ok(0)
    }
    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()>;
}

#[async_trait]
pub trait SeedingProfileRepository: Send + Sync {
    async fn list(&self) -> AppResult<Vec<scryer_domain::SeedingProfile>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::SeedingProfile>>;
    async fn create(
        &self,
        profile: scryer_domain::SeedingProfile,
    ) -> AppResult<scryer_domain::SeedingProfile>;
    async fn update(
        &self,
        profile: scryer_domain::SeedingProfile,
    ) -> AppResult<scryer_domain::SeedingProfile>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait SubtitleProviderConfigRepository: Send + Sync {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<SubtitleProviderConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<SubtitleProviderConfig>>;
    async fn create(&self, config: SubtitleProviderConfig) -> AppResult<SubtitleProviderConfig>;
    async fn update(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>>;
    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.get_setting_json(scope, key_name, scope_id).await
    }

    /// Batch-load explicit (non-default) setting values for many scope ids under
    /// one scope + key. Returns `(scope_id, effective_value_json)` for scope ids
    /// that have an explicit row; scope ids without one are absent. The default
    /// fans out to `get_setting_json_explicit`; SQL stores override with `IN`.
    async fn list_setting_json_explicit_for_scope_ids(
        &self,
        scope: &str,
        key_name: &str,
        scope_ids: &[String],
    ) -> AppResult<Vec<(String, String)>> {
        let mut values = Vec::with_capacity(scope_ids.len());
        for scope_id in scope_ids {
            if let Some(value) = self
                .get_setting_json_explicit(scope, key_name, Some(scope_id.clone()))
                .await?
            {
                values.push((scope_id.clone(), value));
            }
        }
        Ok(values)
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()>;

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()>;

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32>;
}

#[async_trait]
pub trait SystemInfoProvider: Send + Sync {
    async fn datastore_info(&self) -> AppResult<DatastoreInfo>;
    async fn current_migration_version(&self) -> AppResult<Option<String>>;
    async fn current_encryption_key_base64(&self) -> AppResult<Option<String>>;
}

pub trait PluginHttpTrustConfigRuntime: Send + Sync {
    fn set_plugin_http_ca_bundle_pem(&self, bundle_pem: String) -> AppResult<()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatastoreInfo {
    pub engine: String,
    pub current_migration_key: Option<String>,
}

#[async_trait]
pub trait LogicalBackupExporter: Send + Sync {
    async fn export_backup_bundle(
        &self,
        request: crate::BackupBundleExportRequest,
    ) -> AppResult<crate::BackupExportOutcome>;
}

#[async_trait]
pub trait HousekeepingRepository: Send + Sync {
    async fn delete_stale_workflow_operations(
        &self,
        completed_days: i64,
        warning_failed_days: i64,
    ) -> AppResult<u32>;
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[DomainEventType],
    ) -> AppResult<u32>;
    async fn delete_title_history_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_terminal_download_queue_commands_older_than(&self, days: i64)
    -> AppResult<u32>;
    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32>;
    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>>;
    async fn list_media_files_with_roots(&self) -> AppResult<Vec<HousekeepingMediaFileRootRow>>;
    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32>;
    async fn prune_unreferenced_title_image_blobs(&self, limit: u32) -> AppResult<u32>;
    async fn run_database_maintenance(&self) -> AppResult<()> {
        Ok(())
    }
}

pub trait IndexerStatsTracker: Send + Sync {
    fn record_query(&self, indexer_id: &str, indexer_name: &str, success: bool);
    /// Record one release grabbed through this indexer and accepted by a
    /// download client.
    ///
    /// Shares the in-memory rolling 24-hour window that backs `record_query`,
    /// with the same lifetime: it is not persisted and resets on restart.
    fn record_grab(&self, indexer_id: &str, indexer_name: &str);
    fn record_api_limits(
        &self,
        indexer_id: &str,
        api_current: Option<u32>,
        api_max: Option<u32>,
        grab_current: Option<u32>,
        grab_max: Option<u32>,
    );
    fn all_stats(&self) -> Vec<IndexerQueryStats>;

    fn is_at_quota(&self, indexer_id: &str) -> bool {
        self.all_stats()
            .iter()
            .find(|s| s.indexer_id == indexer_id)
            .map(|s| match (s.api_current, s.api_max) {
                (Some(c), Some(m)) if m > 0 => c >= m * 95 / 100,
                _ => false,
            })
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IndexerSearchLearningKey {
    pub indexer_id: String,
    pub title_id: String,
    pub facet: String,
    pub strategy_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerSearchLearningRecord {
    pub key: IndexerSearchLearningKey,
    pub attempts: u32,
    pub empty_successes: u32,
    pub usable_successes: u32,
    pub last_attempt_at: Option<String>,
    pub last_usable_at: Option<String>,
    pub suppressed: bool,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexerSearchLearningContext {
    pub title_id: String,
    pub facet: String,
    pub subject_kind: ReleaseSearchSubjectKind,
    /// Correlates every persisted candidate page produced by one search pass.
    pub search_session_id: String,
    /// Background convergence value hint for this scope: the
    /// convergence cursor sets it from the target's recency lane (hot → high,
    /// cold → low). Rides the Auto-only background search path into the
    /// scheduler candidate's `ExpectedValueHint`; RSS and interactive searches
    /// leave it `None`, which resolves to the neutral value. Plan 112 owns how
    /// the scheduler acts on the resulting value under quota pressure.
    pub background_value: Option<f64>,
    /// Whether this pass may be served from the persisted search-candidate
    /// corpus instead of firing the indexer.
    ///
    /// Reuse is a **background-lane** economy: convergence cycles walk the same
    /// scopes repeatedly and a candidate set persisted hours ago is as good as
    /// a fresh one for them. An operator-triggered search is the opposite — the
    /// user is asking "what is on the indexer *now*", usually seconds after a
    /// release they expect to see appeared. Serving that from a corpus snapshot
    /// taken before the release existed reports "nothing new" for up to the
    /// whole reuse window, which is how an explicit upgrade search stopped
    /// finding a PROPER registered moments earlier.
    pub candidate_reuse_allowed: bool,
}

#[derive(Debug, Clone)]
pub struct IndexerSearchRunWrite {
    pub id: String,
    pub indexer_id: String,
    pub provider_type: String,
    pub search_session_id: String,
    pub scope_key: String,
    pub query_signature: String,
    pub branch: String,
    pub page: Option<u32>,
    pub range_min_size: Option<i64>,
    pub range_max_size: Option<i64>,
    pub result_count: u32,
    pub completion_state: String,
    pub retry_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error_summary: Option<String>,
    pub indexer_fingerprint: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct NormalizedIndexerSearchCandidate {
    pub provider_ref: Option<String>,
    pub source: String,
    pub title: String,
    pub download_url: Option<String>,
    pub download_url_credential_keys: Vec<String>,
    pub link_url: Option<String>,
    pub link_url_credential_keys: Vec<String>,
    pub size_bytes: Option<i64>,
    pub published_at: Option<String>,
    pub source_kind: Option<String>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub grabs: Option<i64>,
    pub grab_current: Option<i64>,
    pub grab_max: Option<i64>,
    pub languages: Vec<String>,
    pub subtitles: Vec<String>,
    pub response_tvdb_id: Option<String>,
    pub response_tmdb_id: Option<String>,
    pub response_imdb_id: Option<String>,
    pub response_categories: Vec<String>,
    pub extra_categories: Vec<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub absolute_episode: Option<i64>,
    pub series_names: Vec<String>,
    pub release_group: Option<String>,
    pub provider_source: Option<String>,
    pub info_hash: Option<String>,
    pub seeders: Option<i64>,
    pub peers: Option<i64>,
    pub download_volume_factor: Option<f64>,
    pub upload_volume_factor: Option<f64>,
    pub protected: Option<bool>,
    pub tags: Vec<String>,
    pub provider_categories: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct IndexerSearchCandidateWrite {
    pub id: String,
    pub run_id: String,
    pub search_session_id: String,
    pub indexer_id: String,
    pub scope_key: String,
    pub query_signature: String,
    /// The release's cross-indexer content identity
    /// ([`crate::release_candidate_fingerprint`]): the durable candidate key
    /// the store dedups by across runs, sessions, and indexers — no longer
    /// session-scoped despite the field name.
    pub session_identity_hash: String,
    pub normalized: NormalizedIndexerSearchCandidate,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub reusable_until: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct ReusableIndexerSearchCandidate {
    pub normalized: NormalizedIndexerSearchCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReusableIndexerSearchStrategy {
    pub run_id: String,
    pub candidate_run_id: Option<String>,
    pub query_signature: String,
    pub branch: String,
    pub completion_state: String,
    pub retry_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[async_trait]
pub trait IndexerSearchLearningRepository: Send + Sync {
    async fn list_for_title(
        &self,
        indexer_id: &str,
        title_id: &str,
        facet: &str,
    ) -> AppResult<Vec<IndexerSearchLearningRecord>>;

    async fn record_outcome(
        &self,
        key: &IndexerSearchLearningKey,
        usable_hits: u32,
    ) -> AppResult<IndexerSearchLearningRecord>;

    async fn record_search_diagnostics(
        &self,
        _run: &IndexerSearchRunWrite,
        _candidates: &[IndexerSearchCandidateWrite],
    ) -> AppResult<()> {
        Ok(())
    }

    /// Makes the evaluated subset from one search pass reusable and discards
    /// every staged payload that automatic acquisition rejected.
    async fn finalize_search_session(
        &self,
        _search_session_id: &str,
        _admissible_fingerprints: &[String],
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_reusable_search_candidates(
        &self,
        _indexer_id: &str,
        _scope_key: &str,
        _indexer_fingerprint: &str,
        _now: chrono::DateTime<chrono::Utc>,
        _limit: u32,
    ) -> AppResult<Vec<ReusableIndexerSearchCandidate>> {
        Ok(Vec::new())
    }

    async fn list_search_run_candidates(
        &self,
        _run_id: &str,
    ) -> AppResult<Vec<ReusableIndexerSearchCandidate>> {
        Ok(Vec::new())
    }

    async fn list_reusable_search_strategies(
        &self,
        _indexer_id: &str,
        _scope_key: &str,
        _indexer_fingerprint: &str,
        _created_after: chrono::DateTime<chrono::Utc>,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<ReusableIndexerSearchStrategy>> {
        Ok(Vec::new())
    }

    async fn cleanup_search_diagnostics(
        &self,
        _candidate_cutoff: chrono::DateTime<chrono::Utc>,
        _run_cutoff: chrono::DateTime<chrono::Utc>,
        _limit: u32,
    ) -> AppResult<u32> {
        Ok(0)
    }

    async fn prune_indexer(&self, _indexer_id: &str) -> AppResult<()> {
        Ok(())
    }

    async fn set_suppressed(
        &self,
        key: &IndexerSearchLearningKey,
        suppressed: bool,
    ) -> AppResult<()>;

    async fn try_claim_suppressed_reprobe(
        &self,
        key: &IndexerSearchLearningKey,
        stale_before: DateTime<Utc>,
    ) -> AppResult<bool>;
}

#[async_trait]
pub trait QualityProfileRepository: Send + Sync {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>>;
    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait ReleaseAttemptRepository: Send + Sync {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()>;

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>>;

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<crate::ReleaseDownloadFailureRecord>>;

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait AcquisitionStateRepository: Send + Sync {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadRegistryRepository: Send + Sync {
    /// Resolve a client observation to its canonical identity in one transaction.
    async fn resolve_observation(
        &self,
        observation: &ObservedClientJob,
    ) -> AppResult<ObservationResolution>;

    /// Load a canonical download row by its durable identifier.
    async fn load_download(&self, id: &DownloadId) -> AppResult<Option<DownloadRecord>>;

    /// Load the current client binding for a canonical download.
    async fn load_binding(&self, id: &DownloadId)
    -> AppResult<Option<DownloadClientBindingRecord>>;

    /// Find a non-ended binding with an exact configured-client/type/native-item locator.
    async fn find_active_binding_by_locator(
        &self,
        locator: &ClientJobLocator,
    ) -> AppResult<Option<DownloadClientBindingRecord>>;

    /// List old active bindings for one configured client/type so an
    /// authoritative snapshot can reconcile jobs that disappeared while the
    /// tracker was not running. Implementations must bound the result.
    async fn list_active_bindings_for_client_before(
        &self,
        _client_config_id: &str,
        _client_type: &str,
        _observed_before: DateTime<Utc>,
        _limit: usize,
    ) -> AppResult<Vec<DownloadClientBindingRecord>> {
        Ok(Vec::new())
    }

    /// End an active binding; ending an already-ended or absent binding is a no-op.
    async fn end_binding(&self, id: &DownloadId) -> AppResult<()>;
}

/// The canonical row and compatibility values carried by a tracked-state update.
/// Keeping these together prevents identity state APIs from growing positional
/// argument lists as the legacy compatibility columns are retired.
pub struct IdentityTrackedStateTarget<'a> {
    pub canonical_download_id: Option<&'a DownloadId>,
    pub identity: &'a DownloadSubmissionIdentity,
    pub source_identity: Option<&'a ClientJobLocator>,
}

#[async_trait]
pub trait DownloadSubmissionRepository: Send + Sync {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()>;

    /// The most recent downloads whose durable tracked state is terminal
    /// (imported / failed / ignored), newest first, capped at `limit`.
    ///
    /// Download history is projected from the live client snapshot, which is
    /// only as durable as the client's own list: rTorrent (among others) evicts
    /// finished jobs, and an imported download then disappeared from history
    /// entirely. These rows are merged into that projection so a finished grab
    /// stays visible once the client forgets it.
    ///
    /// Terminality is read the same way `bound_download_is_terminal_tx` reads
    /// it — the canonical identity state first, then the submission's
    /// `tracked_state` — so this cannot drift into a parallel notion of "done".
    /// Defaults to empty so a store without the query simply contributes no
    /// durable rows.
    async fn list_terminal_download_history_rows(
        &self,
        _limit: usize,
    ) -> AppResult<Vec<TerminalDownloadHistoryRow>> {
        Ok(Vec::new())
    }

    /// The grab-time infohash of a submission for this title whose release
    /// title normalizes to `normalized_release_name`, when one was recorded.
    ///
    /// The import-rejection path keys its blocklist row on this: a release
    /// rejected on content is equally bad under any name, and the infohash is
    /// the content identity. Name normalization happens in Rust (never SQL) so
    /// the two engines cannot drift; matching any submission suffices because
    /// one release name means one release means one hash. Defaults to `None`
    /// so a store without the lookup degrades to a name-only block.
    async fn find_info_hash_for_title_release(
        &self,
        _title_id: &str,
        _normalized_release_name: &str,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// Persist a submit whose client may have accepted the mutation but did
    /// not return a native item identifier.
    async fn record_ambiguous_submission(&self, submission: DownloadSubmission) -> AppResult<()>;

    async fn record_submission_identity(
        &self,
        _identity: &ClientJobLocator,
        _submission_identity: &DownloadSubmissionIdentity,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn record_submission_with_identity(
        &self,
        submission: DownloadSubmission,
        submission_identity: DownloadSubmissionIdentity,
        seed_goals: Option<PersistedSeedGoals>,
    ) -> AppResult<CanonicalDownloadIdentityDisposition>;

    async fn record_submission_actor_snapshot(
        &self,
        _identity: &ClientJobLocator,
        _actor: DownloadSubmissionActorSnapshot,
    ) -> AppResult<()> {
        Ok(())
    }

    /// Read the goals a torrent was grabbed under, by download-client identity.
    async fn get_seed_goals(
        &self,
        _identity: &ClientJobLocator,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        Ok(None)
    }

    /// Prefer the canonical submission row when available, then retain the
    /// legacy source-identity lookup as a fallback.
    async fn get_seed_goals_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        self.get_seed_goals(identity).await
    }

    /// Read the goals a torrent was grabbed under, by observed info hash — the
    /// only key a Tier-B evaluator has when it is walking client items.
    async fn find_seed_goals_by_info_hash(
        &self,
        _info_hash: &str,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        Ok(None)
    }

    /// Batch form of `get_seed_goals`, for the queue projection: it needs the
    /// goals behind every visible torrent on every refresh, and one query per
    /// row would put the poll cadence on a per-item round trip. The default
    /// falls back to the single-row read so no implementor is forced to
    /// reimplement it.
    async fn list_seed_goals_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<(ClientJobLocator, PersistedSeedGoals)>> {
        let mut out = Vec::new();
        for identity in client_items {
            if let Some(goals) = self.get_seed_goals(identity).await? {
                out.push((identity.clone(), goals));
            }
        }
        Ok(out)
    }

    async fn get_submission_actor_snapshot(
        &self,
        _identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmissionActorSnapshot>> {
        Ok(None)
    }

    async fn find_by_client_item_id(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>>;

    /// Prefer the canonical submission row when its download id is already
    /// known, then retain the legacy source-identity lookup as a fallback.
    async fn find_by_client_item_id_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>> {
        self.find_by_client_item_id(identity).await
    }

    async fn find_by_canonical_download_id(
        &self,
        _download_id: &DownloadId,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }

    async fn find_by_download_id(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_id: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(self
            .list_by_download_id(client_id, client_type, download_id)
            .await?
            .into_iter()
            .next())
    }

    async fn list_by_download_id(
        &self,
        _client_id: Option<&str>,
        _client_type: &str,
        _download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        Ok(Vec::new())
    }

    /// Prefer the canonical submission row when available, then retain the
    /// legacy client-and-download-id query as a fallback.
    async fn list_by_download_id_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        client_id: Option<&str>,
        client_type: &str,
        download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        self.list_by_download_id(client_id, client_type, download_id)
            .await
    }

    async fn get_submission_identity(
        &self,
        _identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmissionIdentity>> {
        Ok(None)
    }

    async fn record_identity_tracked_state(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
        _tracked_state: &str,
        _reason: Option<&str>,
        _detail: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }

    /// Record durable state for an identity, carrying the canonical download
    /// id when the caller already resolved the observation.
    async fn record_identity_tracked_state_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
        tracked_state: &str,
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<()> {
        self.record_identity_tracked_state(identity, source_identity, tracked_state, reason, detail)
            .await
    }

    /// Upsert the durable identity tracked state, returning the previous
    /// state. When the previous state is listed in `preserve_previous` the
    /// row is left untouched and that state is returned — this is how a
    /// terminal outcome (imported/failed) survives a later ignore attempt.
    async fn upsert_identity_tracked_state_returning_previous(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
        tracked_state: &str,
        preserve_previous: &[&str],
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<Option<String>> {
        let previous = self
            .get_identity_tracked_state(identity, source_identity)
            .await?;
        if let Some(previous) = previous
            .as_deref()
            .filter(|previous| preserve_previous.contains(previous))
        {
            return Ok(Some(previous.to_string()));
        }
        self.record_identity_tracked_state(
            identity,
            source_identity,
            tracked_state,
            reason,
            detail,
        )
        .await?;
        Ok(previous)
    }

    /// Canonical-aware variant of the durable tracked-state upsert.
    async fn upsert_identity_tracked_state_for_download_returning_previous(
        &self,
        target: IdentityTrackedStateTarget<'_>,
        tracked_state: &str,
        preserve_previous: &[&str],
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.upsert_identity_tracked_state_returning_previous(
            target.identity,
            target.source_identity,
            tracked_state,
            preserve_previous,
            reason,
            detail,
        )
        .await
    }

    async fn get_identity_tracked_state(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// Read by canonical id when it is available; implementations retain the
    /// legacy identity-key lookup for callers that do not have one.
    async fn get_identity_tracked_state_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        self.get_identity_tracked_state(identity, source_identity)
            .await
    }

    async fn get_identity_tracked_state_reason(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_reason_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        self.get_identity_tracked_state_reason(identity, source_identity)
            .await
    }

    async fn get_identity_tracked_state_detail(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_detail_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        self.get_identity_tracked_state_detail(identity, source_identity)
            .await
    }

    async fn list_identity_tracked_states_for_client_items(
        &self,
        _client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<(ClientJobLocator, String)>> {
        Ok(Vec::new())
    }

    /// Batch lookup that uses canonical identity for entries that have one and
    /// keeps the legacy client-item lookup for entries that do not.
    async fn list_identity_tracked_states_for_client_items_with_download_ids(
        &self,
        client_items: &[(ClientJobLocator, Option<DownloadId>)],
    ) -> AppResult<Vec<(ClientJobLocator, String)>> {
        let mut states = Vec::new();
        for (source_identity, canonical_download_id) in client_items {
            let identity = self
                .get_submission_identity(source_identity)
                .await?
                .unwrap_or_default();
            if let Some(state) = self
                .get_identity_tracked_state_for_download(
                    canonical_download_id.as_ref(),
                    &identity,
                    Some(source_identity),
                )
                .await?
            {
                states.push((source_identity.clone(), state));
            }
        }
        Ok(states)
    }

    async fn list_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<DownloadSubmission>>;

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>>;

    /// List Scryer submissions for a title whose canonical client binding is
    /// still active but has not acquired a native client item identifier.
    async fn list_active_unbound_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        Ok(Vec::new())
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
        purpose: DownloadSubmissionPurpose,
        scope: &SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;

    async fn delete_by_client_item_id(&self, identity: &ClientJobLocator) -> AppResult<()>;

    async fn update_tracked_state(
        &self,
        identity: &ClientJobLocator,
        tracked_state: &str,
    ) -> AppResult<()>;

    async fn get_tracked_state(&self, identity: &ClientJobLocator) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait ImportArtifactRepository: Send + Sync {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()>;

    /// Canonical-aware artifact writer for a completed download already
    /// resolved by the caller. Implementations that have not adopted
    /// canonical storage retain the legacy write path.
    async fn insert_artifact_for_download(
        &self,
        artifact: ImportArtifact,
        _canonical_download_id: Option<&DownloadId>,
    ) -> AppResult<()> {
        self.insert_artifact(artifact).await
    }

    async fn insert_artifacts_for_download(
        &self,
        artifacts: Vec<ImportArtifact>,
        canonical_download_id: Option<&DownloadId>,
    ) -> AppResult<()> {
        for artifact in artifacts {
            self.insert_artifact_for_download(artifact, canonical_download_id)
                .await?;
        }
        Ok(())
    }

    async fn list_by_source_identity(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Vec<ImportArtifact>>;

    /// Canonical-first artifact history lookup with a legacy tuple fallback.
    async fn list_by_source_identity_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Vec<ImportArtifact>> {
        self.list_by_source_identity(identity).await
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &ClientJobLocator,
        result: &str,
    ) -> AppResult<u64>;

    /// Canonical-first artifact outcome count with a legacy tuple fallback.
    async fn count_by_result_for_source_identity_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
        result: &str,
    ) -> AppResult<u64> {
        self.count_by_result_for_source_identity(identity, result)
            .await
    }
}

#[async_trait]
pub trait JobRunRepository: Send + Sync {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord>;

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord>;

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>>;

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>>;

    async fn list_job_runs_for_actor(
        &self,
        job_key: Option<JobKey>,
        actor_user_id: &str,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>>;

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>>;

    /// Fail every persisted run still in a non-terminal state and return the
    /// count. Runs named in `excluded_run_ids` remain running because an
    /// operating-system-owned completion step is still pending.
    async fn reconcile_interrupted_job_runs(&self, excluded_run_ids: &[String]) -> AppResult<u64>;
}

#[async_trait]
pub trait LibraryProbeRepository: Send + Sync {
    async fn get_probe_signature(&self, title_id: &str)
    -> AppResult<Option<LibraryProbeSignature>>;

    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()>;
    async fn delete_probe_signatures_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32>;
}

#[async_trait]
pub trait LibraryScanUnmatchedItemRepository: Send + Sync {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String>;

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>>;

    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()>;
    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32>;

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>>;

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64>;
}

#[async_trait]
pub trait StagedNzbStore: Send + Sync {
    async fn create_pending_staged_nzb(
        &self,
        source_url: &str,
        title_id: Option<&str>,
    ) -> AppResult<PendingStagedNzb>;

    async fn finalize_pending_staged_nzb(
        &self,
        pending: PendingStagedNzb,
        raw_size_bytes: u64,
    ) -> AppResult<StagedNzbRef>;

    async fn delete_staged_nzb(&self, staged_nzb: &StagedNzbRef) -> AppResult<bool>;

    async fn prune_staged_nzbs_older_than(&self, older_than: DateTime<Utc>) -> AppResult<u32>;

    fn mark_artifact_active(&self, path: &Path) -> AppResult<()>;

    fn mark_artifact_inactive(&self, path: &Path) -> AppResult<()>;
}

#[async_trait]
pub trait ImportRepository: Send + Sync {
    async fn queue_import_request(
        &self,
        source_identity: ClientJobLocator,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String>;

    async fn queue_import_request_with_identity(
        &self,
        source_identity: ClientJobLocator,
        import_type: String,
        payload_json: String,
        _submission_identity: Option<DownloadSubmissionIdentity>,
    ) -> AppResult<String> {
        self.queue_import_request(source_identity, import_type, payload_json)
            .await
    }

    /// Canonical-aware import request writer for callers that already resolved
    /// the observed download. Implementations that have not adopted canonical
    /// storage retain the legacy write path.
    async fn queue_import_request_with_identity_for_download(
        &self,
        source_identity: ClientJobLocator,
        import_type: String,
        payload_json: String,
        submission_identity: Option<DownloadSubmissionIdentity>,
        _canonical_download_id: Option<&DownloadId>,
    ) -> AppResult<String> {
        self.queue_import_request_with_identity(
            source_identity,
            import_type,
            payload_json,
            submission_identity,
        )
        .await
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>>;

    /// Returns the canonical id durably attached to an import request, when
    /// one was available while it was queued.
    async fn canonical_download_id_for_import(&self, _id: &str) -> AppResult<Option<DownloadId>> {
        Ok(None)
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()>;

    async fn update_import_transfer_progress(
        &self,
        import_id: &str,
        phase: ImportTransferPhase,
        bytes: i64,
        total_bytes: i64,
    ) -> AppResult<()>;

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64>;

    async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64>;

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>>;

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>>;

    async fn list_imports_for_identities(
        &self,
        identities: &[ClientJobLocator],
    ) -> AppResult<Vec<ImportRecord>>;

    /// Returns completed manual imports updated at or after `updated_after`
    /// (newest first, at most `limit`) for bounded tracked-download recovery.
    /// The time bound keeps the recovery sweep from re-reading the whole
    /// manual-import history every tick and from matching a fresh download
    /// that merely reuses an old client item id against a stale record.
    async fn list_completed_manual_imports(
        &self,
        _updated_after: DateTime<Utc>,
        _limit: usize,
    ) -> AppResult<Vec<ImportRecord>> {
        Ok(Vec::new())
    }

    /// Replaces the caller's previous unconsumed selection for the same source and title.
    async fn replace_manual_import_selection(
        &self,
        _selection: ManualImportSelection,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "manual import selection persistence is not configured".to_string(),
        ))
    }

    /// Canonical-aware selection replacement for a completed download already
    /// resolved by the caller.
    async fn replace_manual_import_selection_for_download(
        &self,
        mut selection: ManualImportSelection,
        canonical_download_id: Option<&DownloadId>,
    ) -> AppResult<()> {
        selection.canonical_download_id = canonical_download_id.copied();
        self.replace_manual_import_selection(selection).await
    }

    /// Returns the caller's current unconsumed selection for a source and title, if any.
    async fn find_manual_import_selection(
        &self,
        _actor_user_id: &str,
        _title_id: &str,
        _source_identity: &ClientJobLocator,
    ) -> AppResult<Option<ManualImportSelection>> {
        Ok(None)
    }

    /// Canonical-first selection lookup with a legacy source-tuple fallback.
    async fn find_manual_import_selection_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        actor_user_id: &str,
        title_id: &str,
        source_identity: &ClientJobLocator,
    ) -> AppResult<Option<ManualImportSelection>> {
        self.find_manual_import_selection(actor_user_id, title_id, source_identity)
            .await
    }

    async fn get_manual_import_selection(
        &self,
        _selection_id: &str,
        _actor_user_id: &str,
    ) -> AppResult<Option<ManualImportSelection>> {
        Ok(None)
    }

    /// Atomically consumes a selection and returns only the requested server-owned candidates.
    async fn consume_manual_import_selection(
        &self,
        _selection_id: &str,
        _actor_user_id: &str,
        _candidate_ids: &[String],
    ) -> AppResult<Option<ManualImportSelection>> {
        Ok(None)
    }

    async fn delete_manual_import_selections_for_source(
        &self,
        _source_identity: &ClientJobLocator,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>>;
}

#[async_trait]
pub trait ExternalImportMonitorSnapshotRepository: Send + Sync {
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &crate::ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()>;

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        session_id: &str,
        facet: MediaFacet,
        entry_kind: crate::ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<crate::ExternalImportMonitorSnapshotChunk>>;

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        session_id: &str,
        facet: MediaFacet,
    ) -> AppResult<()>;

    async fn delete_external_import_monitor_snapshot_chunks_for_session_prefix(
        &self,
        session_prefix: &str,
        facet: MediaFacet,
    ) -> AppResult<()>;

    async fn delete_external_import_monitor_snapshot_chunks_except_session_prefix(
        &self,
        preserved_session_prefix: &str,
    ) -> AppResult<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalImportSetupSecretInstanceKind {
    Sonarr,
    Radarr,
    Prowlarr,
}

impl ExternalImportSetupSecretInstanceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sonarr => "sonarr",
            Self::Radarr => "radarr",
            Self::Prowlarr => "prowlarr",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sonarr" => Some(Self::Sonarr),
            "radarr" => Some(Self::Radarr),
            "prowlarr" => Some(Self::Prowlarr),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalImportSetupInstanceApiKeyDraft {
    pub instance_id: String,
    pub kind: ExternalImportSetupSecretInstanceKind,
    pub api_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalImportSetupSecretOverrideDraft {
    pub dedup_key: String,
    pub secret: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExternalImportSetupSecretDraftInput {
    pub instance_api_keys: Vec<ExternalImportSetupInstanceApiKeyDraft>,
    pub download_client_api_key_overrides: Vec<ExternalImportSetupSecretOverrideDraft>,
    pub download_client_password_overrides: Vec<ExternalImportSetupSecretOverrideDraft>,
    pub indexer_api_key_overrides: Vec<ExternalImportSetupSecretOverrideDraft>,
}

impl ExternalImportSetupSecretDraftInput {
    pub fn is_empty(&self) -> bool {
        self.instance_api_keys.is_empty()
            && self.download_client_api_key_overrides.is_empty()
            && self.download_client_password_overrides.is_empty()
            && self.indexer_api_key_overrides.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalImportSetupSecretDraft {
    pub owner_user_id: String,
    pub updated_at: DateTime<Utc>,
    pub secrets: ExternalImportSetupSecretDraftInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalImportSetupSecretDraftStatus {
    pub has_draft: bool,
    pub owned_by_current_user: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalImportSetupSecretDraftSaveResult {
    pub saved: bool,
    pub overwrote_another_user_draft: bool,
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait ExternalImportSetupSecretDraftRepository: Send + Sync {
    async fn get_for_owner(
        &self,
        owner_user_id: &str,
    ) -> AppResult<Option<ExternalImportSetupSecretDraft>>;

    async fn status_for_actor(
        &self,
        actor_user_id: &str,
    ) -> AppResult<ExternalImportSetupSecretDraftStatus>;

    async fn save_for_owner(
        &self,
        owner_user_id: &str,
        draft: ExternalImportSetupSecretDraftInput,
    ) -> AppResult<ExternalImportSetupSecretDraftSaveResult>;

    async fn clear_for_owner(&self, owner_user_id: &str) -> AppResult<bool>;
}

#[async_trait]
pub trait DownloadQueueCommandRepository: Send + Sync {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<crate::DownloadQueueCommandRecord>;

    /// Persist the already-resolved canonical identity alongside the legacy
    /// queue-command tuple. Implementations that do not yet store it retain
    /// the legacy queue behavior.
    async fn queue_delete_command_for_download(
        &self,
        _canonical_download_id: Option<&DownloadId>,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<crate::DownloadQueueCommandRecord> {
        self.queue_delete_command(
            client_id,
            client_type,
            download_client_item_id,
            is_history,
            requested_by_user_id,
        )
        .await
    }

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64>;

    async fn list_pending_delete_commands(
        &self,
    ) -> AppResult<Vec<crate::DownloadQueueCommandRecord>>;

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()>;

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()>;

    async fn mark_delete_command_failed(&self, id: &str, error_text: Option<&str>)
    -> AppResult<()>;

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
        completed_only: bool,
    ) -> AppResult<Vec<crate::DownloadQueueCommandRecord>>;

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32>;
}

#[derive(Debug, Clone)]
pub struct WorkflowOperationInfo {
    pub id: String,
    pub operation_type: String,
    pub status: String,
    pub actor_user_id: Option<String>,
    pub progress_json: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[async_trait]
pub trait WorkflowOperationRepository: Send + Sync {
    async fn create_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo>;
}

#[derive(Clone, Debug)]
pub struct ImportFileTransferProgress {
    pub phase: ImportTransferPhase,
    pub bytes: u64,
    pub total_bytes: u64,
}

pub type ImportFileTransferProgressSender =
    tokio::sync::mpsc::UnboundedSender<ImportFileTransferProgress>;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ImportFilePermissions {
    pub set_permissions_linux: bool,
    pub file_chmod: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
}

#[derive(Clone)]
pub struct ImportFileExecutionContext {
    client_lane_key: String,
    active_import_stream: Option<crate::ActiveImportStreamHandle>,
}

impl ImportFileExecutionContext {
    pub fn new(client_id: &str, client_type: &str) -> Self {
        let client_lane_key = [client_id, client_type]
            .into_iter()
            .map(str::trim)
            .find(|value| !value.is_empty())
            .unwrap_or("unknown-client")
            .to_ascii_lowercase();
        Self {
            client_lane_key,
            active_import_stream: None,
        }
    }

    pub fn client_lane_key(&self) -> &str {
        &self.client_lane_key
    }

    pub fn with_active_import_stream(
        mut self,
        active_import_stream: crate::ActiveImportStreamHandle,
    ) -> Self {
        self.active_import_stream = Some(active_import_stream);
        self
    }

    pub fn cancellation_token(&self) -> Option<crate::ImportCancellation> {
        self.active_import_stream
            .as_ref()
            .map(crate::ActiveImportStreamHandle::cancellation_token)
    }

    pub async fn mark_active_import_placing(&self) {
        if let Some(stream) = &self.active_import_stream {
            stream.mark_placing().await;
        }
    }

    pub async fn mark_active_import_copying(&self) {
        if let Some(stream) = &self.active_import_stream {
            stream.mark_copying().await;
        }
    }

    pub async fn mark_active_import_finalizing(&self) {
        if let Some(stream) = &self.active_import_stream {
            stream.mark_finalizing().await;
        }
    }
}

#[cfg(test)]
mod import_file_execution_context_tests {
    use super::ImportFileExecutionContext;

    #[test]
    fn normalizes_client_id_before_falling_back_to_type() {
        assert_eq!(
            ImportFileExecutionContext::new(" Client-A ", "SABnzbd").client_lane_key(),
            "client-a"
        );
        assert_eq!(
            ImportFileExecutionContext::new("", " qBittorrent ").client_lane_key(),
            "qbittorrent"
        );
        assert_eq!(
            ImportFileExecutionContext::new(" ", " ").client_lane_key(),
            "unknown-client"
        );
    }
}

#[async_trait]
pub trait FileImporter: Send + Sync {
    async fn snapshot_import_source(
        &self,
        source: &Path,
    ) -> AppResult<scryer_domain::ImportSourceSnapshot>;

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    ) -> AppResult<ImportFileResult>;

    async fn import_file_with_progress(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
    ) -> AppResult<ImportFileResult> {
        let _ = progress;
        self.import_file(source, dest, mode, expected_source).await
    }

    async fn import_file_with_progress_and_permissions(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
        permissions: &ImportFilePermissions,
    ) -> AppResult<ImportFileResult> {
        let _ = permissions;
        self.import_file_with_progress(source, dest, mode, expected_source, progress)
            .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "file placement keeps transfer, permission, source-snapshot, and lane context explicit"
    )]
    async fn import_file_with_execution_context(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
        permissions: &ImportFilePermissions,
        context: &ImportFileExecutionContext,
    ) -> AppResult<ImportFileResult> {
        let _ = context;
        self.import_file_with_progress_and_permissions(
            source,
            dest,
            mode,
            expected_source,
            progress,
            permissions,
        )
        .await
    }

    async fn remove_import_source_after_verified_import(
        &self,
        guard: scryer_domain::ImportSourceCleanupGuard,
        final_dest_path: &Path,
    ) -> AppResult<()>;

    async fn remove_import_source_after_verified_import_with_context(
        &self,
        guard: scryer_domain::ImportSourceCleanupGuard,
        final_dest_path: &Path,
        context: &ImportFileExecutionContext,
    ) -> AppResult<()> {
        let _ = context;
        self.remove_import_source_after_verified_import(guard, final_dest_path)
            .await
    }
}

#[async_trait]
pub trait MediaAnalyzer: Send + Sync {
    async fn analyze_file(&self, path: PathBuf) -> AppResult<MediaAnalysisOutcome>;
}

#[async_trait]
pub trait MediaFileRepository: Send + Sync {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String>;

    async fn claim_import_destination(
        &self,
        input: &InsertMediaFileInput,
        associations: &MediaFileAssociations,
    ) -> AppResult<ClaimedMediaFile>;

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()>;

    async fn link_file_to_series_movie(
        &self,
        file_id: &str,
        series_movie_link_id: &str,
    ) -> AppResult<()>;

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>>;

    /// Batch-load media files for many titles. Returns a flat list (each file
    /// carries `title_id`); callers group by that field. The default fans out to
    /// `list_media_files_for_title`; SQL stores override with a single `IN`
    /// query.
    async fn list_media_files_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaFile>> {
        let mut files = Vec::new();
        for title_id in title_ids {
            files.extend(self.list_media_files_for_title(title_id).await?);
        }
        Ok(files)
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>>;

    async fn list_series_movie_link_ids_with_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<String>>;

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>>;

    /// Total byte size of the live media file(s) backing a collection, matched by
    /// the collection's `ordered_path` against `media_files.file_path`. `None`
    /// when nothing is indexed at that path (mirrors the previous filesystem
    /// stat returning no size).
    async fn collection_media_size_bytes(
        &self,
        title_id: &str,
        ordered_path: &str,
    ) -> AppResult<Option<i64>>;

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>>;

    async fn list_title_movie_media_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>>;

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>>;

    /// One sweep over library state returning every monitored, fileless scope —
    /// the raw candidates the derived missing-target set is computed from
    ///. Facet shape, availability windows, and filler opt-ins are
    /// application-layer policy applied on top of these rows.
    async fn list_missing_scope_candidates(&self) -> AppResult<MissingScopeCandidates> {
        Ok(MissingScopeCandidates::default())
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>>;

    /// Compact row data for episode availability. Implementations must not
    /// hydrate full media-file records for this projection.
    async fn list_episode_media_availability(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<EpisodeMediaAvailability>> {
        Ok(Vec::new())
    }

    async fn list_collection_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>>;

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()>;

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()>;

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()>;

    async fn set_media_file_roles_for_title(
        &self,
        title_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()>;

    async fn set_media_file_roles_for_episode(
        &self,
        title_id: &str,
        episode_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()>;

    async fn replace_media_file_for_upgrade(
        &self,
        old_file_id: &str,
        replacement_file_id: &str,
        replacement_file_path: &str,
    ) -> AppResult<()> {
        self.delete_media_file(old_file_id).await?;
        self.update_media_file_path(replacement_file_id, replacement_file_path)
            .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()>;

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>>;

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>>;

    /// Batch-load media files by path, pairing each hit with the path that was
    /// asked for. Callers key results by the requested path because a stored
    /// path may differ from it (Windows matches case- and separator-insensitively),
    /// so the row's own `file_path` is not a reliable lookup key. Paths without a
    /// tracked file are absent. The default fans out to `get_media_file_by_path`;
    /// SQL stores override with one query.
    async fn list_media_files_by_paths(
        &self,
        file_paths: &[String],
    ) -> AppResult<Vec<(String, TitleMediaFile)>> {
        let mut media_files = Vec::with_capacity(file_paths.len());
        for file_path in file_paths {
            if let Some(media_file) = self.get_media_file_by_path(file_path).await? {
                media_files.push((file_path.clone(), media_file));
            }
        }
        Ok(media_files)
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait AcquisitionScopeStateRepository: Send + Sync {
    async fn upsert_acquisition_scope_state(
        &self,
        item: &AcquisitionScopeState,
    ) -> AppResult<String>;

    /// Get-or-create the acquisition-state row for `item`'s scope. An existing
    /// row is returned untouched — state rows are written only by the events
    /// that change them (grabs, pauses, searches), never re-seeded.
    async fn ensure_acquisition_scope_state(
        &self,
        item: &AcquisitionScopeState,
    ) -> AppResult<String> {
        if let Some(existing) = find_existing_acquisition_scope_state(self, item).await? {
            return Ok(existing.id);
        }
        self.upsert_acquisition_scope_state(item).await?;
        Ok(item.id.clone())
    }

    async fn update_acquisition_scope_status(
        &self,
        id: &str,
        status: &str,
        last_search_at: Option<&str>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()>;

    /// Stamp the scope's last active-search time — cooldown state read by the
    /// upgrade policy and failed-grab staleness checks.
    async fn record_acquisition_scope_search_attempt(
        &self,
        id: &str,
        last_search_at: &str,
    ) -> AppResult<()>;

    async fn transition_acquisition_scope_to_grabbed(
        &self,
        transition: &AcquisitionScopeGrabTransition,
    ) -> AppResult<()> {
        self.update_acquisition_scope_status(
            &transition.id,
            AcquisitionScopeStatus::Grabbed.as_str(),
            transition.last_search_at.as_deref(),
            Some(&transition.grabbed_release),
        )
        .await
    }

    async fn transition_acquisition_scope_to_completed(
        &self,
        transition: &AcquisitionScopeCompleteTransition,
    ) -> AppResult<()> {
        self.update_acquisition_scope_status(
            &transition.id,
            AcquisitionScopeStatus::Completed.as_str(),
            transition.last_search_at.as_deref(),
            transition.grabbed_release.as_deref(),
        )
        .await
    }

    /// Mark a scope completed.
    ///
    /// `landed_import` says whether a file actually landed for this scope, which
    /// is the only thing the old `current_score` argument was ever used for
    /// here: a landed import clears the in-flight grab, while a passive scan or
    /// manual completion leaves it alone. It used to be inferred from
    /// `current_score.is_some()`, which read a score as a flag.
    async fn complete_acquisition_scope_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        landed_import: bool,
    ) -> AppResult<bool> {
        let Some(wanted) = self
            .get_acquisition_scope_state_for_title(title_id, episode_id)
            .await?
        else {
            return Ok(false);
        };

        self.transition_acquisition_scope_to_completed(&AcquisitionScopeCompleteTransition {
            id: wanted.id,
            last_search_at: last_search_at.map(str::to_string),
            grabbed_release: if landed_import {
                None
            } else {
                wanted.grabbed_release
            },
        })
        .await?;

        Ok(true)
    }

    async fn transition_acquisition_scope_to_paused(
        &self,
        transition: &AcquisitionScopePauseTransition,
    ) -> AppResult<()> {
        self.update_acquisition_scope_status(
            &transition.id,
            AcquisitionScopeStatus::Paused.as_str(),
            transition.last_search_at.as_deref(),
            transition.grabbed_release.as_deref(),
        )
        .await
    }

    /// Re-open a scope for acquisition after a failed grab, a rejected import,
    /// or an operator replacement: status back to `wanted`, the in-flight grab
    /// cleared, the upgrade-baseline score and search cooldown preserved. The
    /// convergence re-open (coverage invalidation) is the caller's second half — this
    /// only resets the state row.
    async fn transition_acquisition_scope_to_reopened(&self, id: &str) -> AppResult<()> {
        let Some(existing) = self.get_acquisition_scope_state_by_id(id).await? else {
            return Ok(());
        };
        self.update_acquisition_scope_status(
            id,
            AcquisitionScopeStatus::Wanted.as_str(),
            existing.last_search_at.as_deref(),
            None,
        )
        .await
    }

    async fn get_acquisition_scope_state_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<AcquisitionScopeState>>;

    async fn delete_acquisition_scope_states_for_title(&self, title_id: &str) -> AppResult<()>;

    async fn delete_acquisition_scope_states_for_collection(
        &self,
        collection_id: &str,
    ) -> AppResult<()>;

    async fn delete_acquisition_scope_states_for_series_movie_link(
        &self,
        series_movie_link_id: &str,
    ) -> AppResult<()>;

    async fn delete_acquisition_scope_states_for_episode(&self, episode_id: &str) -> AppResult<()>;

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String>;

    async fn get_acquisition_scope_state_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<AcquisitionScopeState>>;

    /// Batch-load acquisition scope states (wanted items) by id for dataloaders.
    /// Missing ids are absent from the result. The default fans out to
    /// `get_acquisition_scope_state_by_id`; SQL stores override with `IN`.
    async fn list_acquisition_scope_states_by_ids(
        &self,
        ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let mut states = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(state) = self.get_acquisition_scope_state_by_id(id).await? {
                states.push(state);
            }
        }
        Ok(states)
    }

    async fn list_acquisition_scope_states(
        &self,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<Vec<AcquisitionScopeState>>;

    /// Batch-load every acquisition scope state for many titles. Returns a flat
    /// list (each state carries `title_id`); callers group by that field. The
    /// default fans out via `list_acquisition_scope_states`; SQL stores override
    /// with a single `IN` query over `title_id`.
    async fn list_acquisition_scope_states_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let mut states = Vec::new();
        for title_id in title_ids {
            states.extend(
                self.list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                    title_id: Some(title_id.clone()),
                    limit: i64::MAX,
                    ..AcquisitionScopeStatesQuery::default()
                })
                .await?,
            );
        }
        Ok(states)
    }

    async fn count_acquisition_scope_states(
        &self,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<i64>;

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>>;

    async fn list_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>>;

    async fn count_release_decisions_for_title(&self, title_id: &str) -> AppResult<i64>;

    async fn count_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<i64>;
}

/// Locate the acquisition-state row matching `item`'s scope (episode /
/// series-movie link / collection / bare title), if one exists.
pub(crate) async fn find_existing_acquisition_scope_state<
    R: AcquisitionScopeStateRepository + ?Sized,
>(
    repo: &R,
    item: &AcquisitionScopeState,
) -> AppResult<Option<AcquisitionScopeState>> {
    if let Some(series_movie_link_id) = item.series_movie_link_id.as_deref() {
        return Ok(repo
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                title_id: Some(item.title_id.clone()),
                limit: 500,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?
            .into_iter()
            .find(|existing| {
                existing.series_movie_link_id.as_deref() == Some(series_movie_link_id)
            }));
    }

    // Episode scope before collection scope: an episode state row carries both
    // its `episode_id` and its owning `collection_id`, so it must be matched on
    // the more specific episode identity first — otherwise every episode of a
    // season would resolve to the same collection-matched sibling row.
    if let Some(episode_id) = item.episode_id.as_deref() {
        return repo
            .get_acquisition_scope_state_for_title(&item.title_id, Some(episode_id))
            .await;
    }

    if let Some(collection_id) = item.collection_id.as_deref() {
        return Ok(repo
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                title_id: Some(item.title_id.clone()),
                limit: 500,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?
            .into_iter()
            .find(|existing| {
                existing.episode_id.is_none()
                    && existing.collection_id.as_deref() == Some(collection_id)
            }));
    }

    Ok(repo
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_id: Some(item.title_id.clone()),
            limit: 500,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await?
        .into_iter()
        .find(|existing| {
            existing.episode_id.is_none()
                && existing.collection_id.is_none()
                && existing.series_movie_link_id.is_none()
        }))
}

#[async_trait]
pub trait PendingReleaseRepository: Send + Sync {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String>;
    /// Record another observation without refreshing its original delay clock.
    async fn insert_pending_release_with_role(
        &self,
        release: &PendingRelease,
        role: PendingReleaseRole,
    ) -> AppResult<String>;
    async fn insert_pending_release_observation(
        &self,
        release: &PendingRelease,
        observation: &PendingReleaseObservation,
    ) -> AppResult<String>;
    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>>;
    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>>;
    /// Active rows whose indexer observation did not include a publish time.
    /// Callers use `added_at` plus the current policy to decide review timing.
    async fn list_active_release_age_unknown_pending_releases(
        &self,
    ) -> AppResult<Vec<PendingRelease>>;
    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>>;
    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    /// Return one page of `waiting` pending releases matching `query` plus the
    /// total number of matching rows. Filtering, ordering, limit/offset, and the
    /// count are all computed in storage.
    async fn list_pending_releases_page(
        &self,
        query: PendingReleasesPageQuery,
    ) -> AppResult<(Vec<PendingRelease>, i64)>;
    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()>;
    /// Terminal policy rejection is retained as audit history, not deleted.
    async fn expire_pending_release(&self, id: &str, decision_code: &str) -> AppResult<()>;
    async fn mark_release_age_unknown_pending_release_needs_review(
        &self,
        id: &str,
        decision_code: &str,
    ) -> AppResult<()>;
    async fn update_pending_release_delay_until(
        &self,
        id: &str,
        delay_until: &str,
    ) -> AppResult<()>;
    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    /// Standby rows for a title, ordered best-first for cross-episode pack
    /// recovery.
    async fn list_standby_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    /// `standby` row counts grouped by wanted item, for the given items only —
    /// one query, so a Wanted page never reads the whole standby table.
    async fn count_standby_pending_releases_for_wanted_items(
        &self,
        wanted_item_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, i64>>;
    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()>;
    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>>;
    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool>;
    /// Retire the caller's freshly judged lower-or-equal pending candidates.
    /// The caller computes overlap and excludes a pending winner, if any.
    async fn retire_lower_or_equal_overlapping_pending_releases(
        &self,
        lower_or_equal_ids: &[String],
    ) -> AppResult<()>;
    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait BlocklistRepository: Send + Sync {
    /// Records the block, or does nothing when it is already recorded.
    ///
    /// `Ok(true)` means a new row was written. Idempotence is the schema's job
    /// -- two unique indexes, one per key shape -- not a read-then-write in
    /// application code, so concurrent writers cannot both insert.
    async fn block(&self, entry: &NewBlocklistEntry) -> AppResult<bool>;

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>>;

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)>;

    /// One entry by id. The clear path resolves the entry this way for its
    /// permission check — a paged scan over `list_all` would make entries past
    /// the page silently unclearable.
    async fn get(&self, id: &str) -> AppResult<Option<BlocklistEntry>>;

    /// Whether this release is already blocked for the title, using the same
    /// key [`BlocklistRepository::block`] writes.
    async fn is_blocked(
        &self,
        title_id: &str,
        indexer_id: &str,
        release_name: &str,
        info_hash: Option<&str>,
    ) -> AppResult<bool>;

    async fn remove(&self, id: &str) -> AppResult<()>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;

    /// Drops every block recorded against one indexer. Called when the indexer
    /// is deleted: its rows can never match again, but they would still render.
    async fn delete_for_indexer(&self, indexer_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait RuleSetRepository: Send + Sync {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>>;
    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()>;
    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()>;
    async fn delete_rule_set(&self, id: &str) -> AppResult<()>;
    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()>;
    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>>;
    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()>;
    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>>;
}

#[async_trait]
pub trait PostProcessingScriptRepository: Send + Sync {
    async fn list_scripts(&self) -> AppResult<Vec<scryer_domain::PostProcessingScript>>;
    async fn get_script(&self, id: &str) -> AppResult<Option<scryer_domain::PostProcessingScript>>;
    async fn create_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript>;
    async fn update_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript>;
    async fn delete_script(&self, id: &str) -> AppResult<()>;
    async fn list_enabled_for_facet(
        &self,
        facet: &str,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScript>>;
    async fn record_run(&self, run: scryer_domain::PostProcessingScriptRun) -> AppResult<()>;
    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>>;
    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>>;
}

#[async_trait]
pub trait PluginInstallationRepository: Send + Sync {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>>;
    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>>;
    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()>;
    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>>;
    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>>;
    #[expect(
        clippy::too_many_arguments,
        reason = "builtin plugin seeding persists the full published plugin contract explicitly"
    )]
    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        sdk_version: &str,
        sdk_constraint: &str,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()>;
    async fn upsert_plugin_catalog_source(&self, source: &PluginCatalogSource) -> AppResult<()>;
    async fn delete_plugin_catalog_source(&self, source_key: &str) -> AppResult<()>;
    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>>;
    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>>;
    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()>;
    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<PluginCatalogStatusRecord>>;
}

pub trait PluginDescriptorLoader: Send + Sync {
    fn load_descriptor_from_wasm_bytes(
        &self,
        wasm_bytes: &[u8],
    ) -> AppResult<scryer_plugin_sdk::PluginDescriptor>;
}

#[async_trait]
pub trait IndexerClient: Send + Sync {
    async fn finalize_search_session(
        &self,
        _search_session_id: &str,
        _admissible_fingerprints: &[String],
    ) -> AppResult<()> {
        Ok(())
    }

    fn search_plan_capability(&self) -> Option<IndexerSearchPlanCapability> {
        None
    }

    async fn search_plan(
        &self,
        _request: IndexerSearchPlanRequest,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        _cancel_token: tokio_util::sync::CancellationToken,
        _event_sink: IndexerSearchStrategyEventSink,
    ) -> AppResult<IndexerSearchPlanSummary> {
        Err(AppError::Repository(
            "indexer does not support strategy-plan search".to_string(),
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "indexer search forwards the full caller-controlled search envelope to plugins"
    )]
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
        learning_context: Option<IndexerSearchLearningContext>,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let (page_tx, mut page_rx) = tokio::sync::mpsc::channel(2);
        let page_sink = crate::IndexerSearchPageSink::new(page_tx, 2);
        let producer = self.search_stream(
            query,
            ids,
            category,
            facet,
            id_search_facet,
            newznab_categories,
            indexer_routing,
            mode,
            operation,
            season,
            episode,
            absolute_episode,
            tagged_aliases,
            learning_context,
            cancel_token,
            page_sink,
        );
        tokio::pin!(producer);

        let mut results = Vec::new();
        let mut page_source_open = true;
        let mut response = loop {
            tokio::select! {
                response = &mut producer => break response?,
                page = page_rx.recv(), if page_source_open => match page {
                    Some(page) => results.extend(page.results),
                    None => page_source_open = false,
                },
            }
        };
        while let Some(page) = page_rx.recv().await {
            results.extend(page.results);
        }
        if results.is_empty() {
            results = std::mem::take(&mut response.results);
        }
        response.results = results;
        Ok(response)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the streaming adapter preserves the complete search envelope"
    )]
    async fn search_queries_stream(
        &self,
        queries: Vec<String>,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
        learning_context: Option<IndexerSearchLearningContext>,
        cancel_token: tokio_util::sync::CancellationToken,
        page_sink: crate::IndexerSearchPageSink,
    ) -> AppResult<IndexerSearchResponse> {
        let mut combined: Option<IndexerSearchResponse> = None;
        for query in queries {
            let mut response = self
                .search_stream(
                    query,
                    ids.clone(),
                    category.clone(),
                    facet.clone(),
                    id_search_facet.clone(),
                    newznab_categories.clone(),
                    indexer_routing.clone(),
                    mode,
                    operation,
                    season,
                    episode,
                    absolute_episode,
                    tagged_aliases.clone(),
                    learning_context.clone(),
                    cancel_token.child_token(),
                    page_sink.clone(),
                )
                .await?;
            if let Some(existing) = combined.as_mut() {
                existing.results.append(&mut response.results);
                existing
                    .indexer_outcomes
                    .append(&mut response.indexer_outcomes);
                if response.completion != IndexerSearchCompletion::Complete {
                    existing.completion = response.completion;
                }
                existing.api_current = response.api_current.or(existing.api_current);
                existing.api_max = response.api_max.or(existing.api_max);
                existing.grab_current = response.grab_current.or(existing.grab_current);
                existing.grab_max = response.grab_max.or(existing.grab_max);
            } else {
                combined = Some(response);
            }
        }
        Ok(combined.unwrap_or(IndexerSearchResponse {
            results: Vec::new(),
            completion: IndexerSearchCompletion::Complete,
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
            indexer_outcomes: Vec::new(),
        }))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the streaming adapter preserves the complete search envelope"
    )]
    async fn search_stream(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
        learning_context: Option<IndexerSearchLearningContext>,
        cancel_token: tokio_util::sync::CancellationToken,
        page_sink: crate::IndexerSearchPageSink,
    ) -> AppResult<IndexerSearchResponse> {
        let mut response = self
            .search(
                query,
                ids,
                category,
                facet,
                id_search_facet,
                newznab_categories,
                indexer_routing,
                mode,
                operation,
                season,
                episode,
                absolute_episode,
                tagged_aliases,
                learning_context,
                cancel_token,
            )
            .await?;
        if !response.results.is_empty() {
            page_sink
                .send(std::mem::take(&mut response.results))
                .await
                .map_err(|_| AppError::canceled("indexer scoring pipeline closed"))?;
        }
        Ok(response)
    }

    async fn prune_search_learning(&self, _indexer_id: &str) -> AppResult<()> {
        Ok(())
    }
}

pub trait IndexerPluginProvider: Send + Sync {
    /// Validate normalized instance configuration using provider-owned
    /// descriptor metadata. Providers without such metadata accept it.
    fn validate_config_for_provider(
        &self,
        _provider_type: &str,
        _config_json: &str,
    ) -> AppResult<()> {
        Ok(())
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>>;
    fn client_for_provider_with_proxy(
        &self,
        config: &IndexerConfig,
        _proxy_config: Option<&scryer_domain::IndexerProxyConfig>,
    ) -> Option<Arc<dyn IndexerClient>> {
        self.client_for_provider(config)
    }
    fn management_client_for_provider(
        &self,
        _config: &IndexerConfig,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        None
    }
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn search_semantics_version_for_provider(&self, _provider_type: &str) -> Option<u32> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_type_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy>;
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn prepare_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime preparation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }
    fn plugin_name_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn rate_limit_seconds_for_provider(&self, _provider_type: &str) -> Option<i64> {
        None
    }
    fn management_capabilities_for_provider(
        &self,
        _provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        scryer_domain::IndexerManagementCapabilities::default()
    }
    fn capabilities_for_provider(
        &self,
        _provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        scryer_domain::IndexerProviderCapabilities {
            rss: true,
            supported_ids: std::collections::HashMap::from([
                ("movie".into(), vec!["imdb_id".into()]),
                ("series".into(), vec!["tvdb_id".into()]),
            ]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: true,
            tvdb_search: true,
            anidb_search: false,
            ..Default::default()
        }
    }
}

#[async_trait]
pub trait IndexerManagementClient: Send + Sync {
    async fn validate_connection(&self) -> AppResult<IndexerValidationResult>;
    async fn preview_sync_plan(&self, parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        self.plan_sync(parent_config_id).await
    }
    async fn plan_sync(&self, _parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        Err(AppError::Repository(
            "managed child sync is not supported for this provider".to_string(),
        ))
    }
    async fn enrichment_sync_plan(
        &self,
        _parent_config_id: &str,
    ) -> AppResult<Option<IndexerSyncPlan>> {
        Ok(None)
    }
    fn name(&self) -> &str;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExternalPluginWasm<'a> {
    pub bytes: &'a [u8],
    pub first_party: bool,
}

#[derive(Clone, Debug)]
pub struct RuntimePluginLoad {
    pub descriptor: scryer_plugin_sdk::PluginDescriptor,
    pub wasm_bytes: Vec<u8>,
    pub first_party: bool,
}

pub trait DownloadClientPluginProvider: Send + Sync {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }
    fn plugin_name_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
        vec![]
    }
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationAppPayload {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationExternalIdsPayload {
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub anidb_id: Option<String>,
    pub tvmaze_id: Option<String>,
    pub anilist_ids: Vec<String>,
    pub mal_ids: Vec<String>,
    pub kitsu_ids: Vec<String>,
    pub by_source: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationActorPayload {
    pub user_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotificationSeverityPayload {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationTitlePayload {
    pub id: Option<String>,
    pub name: String,
    pub facet: String,
    pub year: Option<i32>,
    pub slug: Option<String>,
    pub path: Option<String>,
    pub overview: Option<String>,
    pub sort_title: Option<String>,
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub original_language: Option<String>,
    pub original_country: Option<String>,
    pub external_ids: NotificationExternalIdsPayload,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationEpisodePayload {
    pub id: Option<String>,
    pub episode_ids: Vec<String>,
    pub media_file_id: Option<String>,
    pub media_file_path: Option<String>,
    pub display: Option<String>,
    pub collection_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub absolute_number: Option<String>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub air_date_utc: Option<String>,
    pub episode_type: Option<String>,
    pub finale_type: Option<String>,
    pub tvdb_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationReleasePayload {
    pub source_title: Option<String>,
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub provider: Option<String>,
    pub language: Option<String>,
    pub release_group: Option<String>,
    pub protocol: Option<String>,
    pub indexer: Option<String>,
    pub languages: Vec<String>,
    pub custom_scores: BTreeMap<String, i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationDownloadPayload {
    pub download_id: Option<String>,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    pub client_type: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub status_message: Option<String>,
    pub size_bytes: Option<i64>,
    pub progress_percent: Option<i32>,
    pub output_path: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationImportPayload {
    pub import_id: Option<String>,
    pub source_system: Option<String>,
    pub source_ref: Option<String>,
    pub source_title: Option<String>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub imported_count: Option<i32>,
    pub status: Option<String>,
    pub skipped_count: Option<i32>,
    pub rejected_count: Option<i32>,
    pub upgrade: bool,
    pub deleted_paths: Vec<String>,
    pub replaced_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationHealthPayload {
    pub status: Option<String>,
    pub message: Option<String>,
    pub severity: Option<String>,
    pub code: Option<String>,
    pub details: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationMediaUpdateTypePayload {
    Created,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationMediaUpdatePayload {
    pub path: String,
    pub update_type: NotificationMediaUpdateTypePayload,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationFilePayload {
    pub primary_path: Option<String>,
    pub media_updates: Vec<NotificationMediaUpdatePayload>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationMediaFilePayload {
    pub id: Option<String>,
    pub path: String,
    pub previous_path: Option<String>,
    pub recycle_bin_path: Option<String>,
    pub size_bytes: Option<i64>,
    pub quality: Option<String>,
    pub release_group: Option<String>,
    pub scene_name: Option<String>,
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub video_frame_rate: Option<String>,
    pub container_format: Option<String>,
    pub edition: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationApplicationUpdatePayload {
    pub current_version: Option<String>,
    pub target_version: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationManualInteractionPayload {
    pub kind: Option<String>,
    pub reason: Option<String>,
    pub link: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationMediaRequestPayload {
    pub request_id: Option<String>,
    pub library_id: Option<String>,
    pub status: Option<String>,
    pub facet: Option<String>,
    pub requested_quality_profile_id: Option<String>,
    pub requested_quality_profile_name: Option<String>,
    pub requested_monitor_type: Option<String>,
    pub approved_quality_profile_id: Option<String>,
    pub approved_quality_profile_name: Option<String>,
    pub created_title_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationPayload {
    pub schema_version: u32,
    pub event_type: scryer_domain::NotificationEventType,
    pub event_id: Option<String>,
    pub occurred_at: Option<String>,
    pub correlation_id: Option<String>,
    pub actor: Option<NotificationActorPayload>,
    pub severity: Option<NotificationSeverityPayload>,
    pub is_test: bool,
    pub summary_title: String,
    pub summary_message: String,
    pub app: NotificationAppPayload,
    pub title: Option<NotificationTitlePayload>,
    pub episode: Option<NotificationEpisodePayload>,
    pub episodes: Vec<NotificationEpisodePayload>,
    pub release: Option<NotificationReleasePayload>,
    pub download: Option<NotificationDownloadPayload>,
    pub import: Option<NotificationImportPayload>,
    pub health: Option<NotificationHealthPayload>,
    pub file: Option<NotificationFilePayload>,
    pub media_files: Vec<NotificationMediaFilePayload>,
    pub application_update: Option<NotificationApplicationUpdatePayload>,
    pub manual_interaction: Option<NotificationManualInteractionPayload>,
    pub media_request: Option<NotificationMediaRequestPayload>,
}

#[async_trait]
pub trait NotificationClient: Send + Sync {
    async fn send_notification(&self, payload: &NotificationPayload) -> AppResult<()>;
}

#[async_trait]
pub trait SubtitleProviderClient: Send + Sync {
    async fn search(
        &self,
        query: &crate::subtitles::SubtitleQuery,
    ) -> AppResult<Vec<crate::subtitles::SubtitleMatch>>;
    async fn download(&self, provider_file_id: &str) -> AppResult<crate::subtitles::SubtitleFile>;
    async fn validate_connection(&self) -> AppResult<SubtitleProviderValidationResult>;
    async fn generate(
        &self,
        _request: &SubtitleGenerationInput,
    ) -> AppResult<crate::subtitles::SubtitleFile> {
        Err(AppError::Repository(
            "subtitle generation is not supported for this provider".to_string(),
        ))
    }
    fn name(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct SubtitleSyncReferenceSubtitle {
    pub content: Vec<u8>,
    pub format: String,
    pub file_name: Option<String>,
    pub encoding_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubtitleSyncJob {
    pub input_path: PathBuf,
    pub subtitle_content: Vec<u8>,
    pub subtitle_format: String,
    pub subtitle_file_name: Option<String>,
    pub subtitle_encoding_hint: Option<String>,
    pub reference_subtitle: Option<SubtitleSyncReferenceSubtitle>,
    pub max_offset_seconds: i64,
    pub sync_options: SubtitleSyncOptions,
    pub expected_codec: Option<SubtitleSyncAudioCodec>,
    pub media_metadata: Option<SubtitleSyncMediaMetadataSnapshot>,
}

#[async_trait]
pub trait SubtitleSyncClient: Send + Sync {
    async fn align_subtitle(&self, job: SubtitleSyncJob) -> AppResult<SubtitleSyncAlignResponse>;
}

pub trait NotificationPluginProvider: Send + Sync {
    fn client_for_channel(
        &self,
        config: &scryer_domain::NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn supported_events_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::NotificationEventType> {
        vec![]
    }
    fn supports_test_for_provider(&self, _provider_type: &str) -> bool {
        false
    }
    fn config_fields_for_provider(&self, provider_type: &str)
    -> Vec<scryer_domain::ConfigFieldDef>;
    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String>;
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
}

pub trait SubtitlePluginProvider: Send + Sync {
    fn client_for_config(
        &self,
        config: &scryer_domain::SubtitleProviderConfig,
        host_bindings: &std::collections::HashMap<scryer_domain::PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>>;
    fn subtitle_sync_client(&self) -> Option<Arc<dyn SubtitleSyncClient>> {
        None
    }
    fn available_provider_types(&self) -> Vec<String>;
    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }
    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool;
    fn recommended_facets_for_provider(&self, provider_type: &str) -> Vec<String>;
    fn config_fields_for_provider(&self, provider_type: &str)
    -> Vec<scryer_domain::ConfigFieldDef>;
    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String>;
    fn plugin_description_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support single-plugin runtime mutation".to_string())
    }
    fn prepare_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime preparation".to_string())
    }
    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support builtin runtime restoration".to_string())
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
}

#[async_trait]
pub trait ArchiveExtractorClient: Send + Sync {
    async fn process(
        &self,
        request: ArchivePluginProcessRequest,
    ) -> AppResult<ArchivePluginProcessResponse>;
}

pub trait ArchiveExtractorPluginProvider: Send + Sync {
    fn client_for_format(
        &self,
        format: ArchivePluginFormat,
    ) -> Option<Arc<dyn ArchiveExtractorClient>>;

    fn available_provider_types(&self) -> Vec<String>;

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support runtime-load upsert".to_string())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support runtime-load removal".to_string())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
}

#[async_trait]
pub trait NotificationChannelRepository: Send + Sync {
    async fn list_channels(&self) -> AppResult<Vec<scryer_domain::NotificationChannelConfig>>;
    async fn get_channel(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_domain::NotificationChannelConfig>>;
    async fn create_channel(
        &self,
        config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig>;
    async fn update_channel(
        &self,
        config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig>;
    async fn delete_channel(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait NotificationSubscriptionRepository: Send + Sync {
    async fn list_subscriptions(&self) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn list_subscriptions_for_channel(
        &self,
        channel_id: &str,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn list_subscriptions_for_target(
        &self,
        target_kind: scryer_domain::NotificationTargetKind,
        target_id: &str,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn list_subscriptions_for_event(
        &self,
        event_type: scryer_domain::NotificationEventType,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn create_subscription(
        &self,
        sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription>;
    async fn update_subscription(
        &self,
        sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription>;
    async fn delete_subscription(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait BuiltinDownloadClientConnectionTester: Send + Sync {
    async fn test_connection(&self, client_type: &str, config_json: &str) -> AppResult<()>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadClientFeedbackScope {
    pub categories: Vec<String>,
}

/// A lenient queue/activity snapshot with the subset of clients for which
/// absence is authoritative.
#[derive(Clone, Debug, Default)]
pub struct DownloadClientSnapshotOutcome {
    pub items: Vec<DownloadQueueItem>,
    pub authoritative_client_ids: std::collections::HashSet<String>,
    pub any_client_read_succeeded: bool,
}

#[async_trait]
pub trait DownloadClient: Send + Sync {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult>;

    async fn submit_to_download_queue(
        &self,
        title: &Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> AppResult<DownloadGrabResult> {
        let request = DownloadClientAddRequest::from_legacy(
            title,
            source_hint,
            source_kind,
            source_title,
            source_password,
            category,
        );
        self.submit_download(&request).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Err(AppError::Repository(
            "download queue listing is not supported for this client".to_string(),
        ))
    }

    async fn list_queue_with_feedback_scope(
        &self,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue().await
    }

    async fn list_queue_excluding_client_types(
        &self,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if !excluded_client_types.is_empty() {
            return Err(AppError::Repository(
                "download client type exclusion requires a router-backed download client"
                    .to_string(),
            ));
        }
        let mut items = self.list_queue().await?;
        items.retain(|item| {
            !excluded_client_types
                .iter()
                .any(|client_type| item.client_type.eq_ignore_ascii_case(client_type.trim()))
        });
        Ok(items)
    }

    async fn list_queue_for_title(&self, _title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue().await
    }

    async fn list_queue_for_title_with_feedback_scope(
        &self,
        title_id: &str,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue_for_title(title_id).await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Err(AppError::Repository(
            "download history listing is not supported for this client".to_string(),
        ))
    }

    async fn list_history_with_feedback_scope(
        &self,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_history().await
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let items = self.list_history().await?;
        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn list_history_page_with_feedback_scope(
        &self,
        offset: usize,
        limit: usize,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_history_page(offset, limit).await
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_history_page(0, limit).await
    }

    async fn list_recent_activity_with_feedback_scope(
        &self,
        limit: usize,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_recent_activity(limit).await
    }

    async fn list_recent_activity_excluding_client_types(
        &self,
        limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if !excluded_client_types.is_empty() {
            return Err(AppError::Repository(
                "download client type exclusion requires a router-backed download client"
                    .to_string(),
            ));
        }
        let mut items = self.list_recent_activity(limit).await?;
        items.retain(|item| {
            !excluded_client_types
                .iter()
                .any(|client_type| item.client_type.eq_ignore_ascii_case(client_type.trim()))
        });
        Ok(items)
    }

    /// Read queue and recent activity leniently, identifying clients whose
    /// complete snapshot is safe to use for absence reconciliation.
    ///
    /// Implementations without per-client feedback can retain tracking
    /// liveness through the default while conservatively authorizing no prune.
    async fn list_snapshot_outcome_excluding_client_types(
        &self,
        recent_activity_limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<DownloadClientSnapshotOutcome> {
        let queue = self
            .list_queue_excluding_client_types(excluded_client_types)
            .await;
        let activity = self
            .list_recent_activity_excluding_client_types(
                recent_activity_limit,
                excluded_client_types,
            )
            .await;

        match (queue, activity) {
            (Ok(mut queue_items), Ok(activity_items)) => {
                queue_items.extend(activity_items);
                Ok(DownloadClientSnapshotOutcome {
                    items: queue_items,
                    any_client_read_succeeded: true,
                    ..Default::default()
                })
            }
            (Ok(items), Err(_)) | (Err(_), Ok(items)) => Ok(DownloadClientSnapshotOutcome {
                items,
                any_client_read_succeeded: true,
                ..Default::default()
            }),
            (Err(error), Err(_)) => Err(error),
        }
    }

    async fn list_recent_activity_for_title(
        &self,
        _title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_recent_activity(limit).await
    }

    async fn list_recent_activity_for_title_with_feedback_scope(
        &self,
        title_id: &str,
        limit: usize,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_recent_activity_for_title(title_id, limit).await
    }

    /// Recent activity restricted to the given client types.
    ///
    /// Used to reconcile clients that are excluded from generic polling
    /// because a realtime bridge owns their live queue: the bridge can miss
    /// terminal events, so history still needs a bounded sweep.
    async fn list_recent_activity_for_client_types(
        &self,
        limit: usize,
        client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 || client_types.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = self.list_recent_activity(limit).await?;
        items.retain(|item| {
            client_types
                .iter()
                .any(|client_type| item.client_type.eq_ignore_ascii_case(client_type.trim()))
        });
        Ok(items)
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        Err(AppError::Repository(
            "completed download listing is not supported for this client".to_string(),
        ))
    }

    async fn list_completed_downloads_with_feedback_scope(
        &self,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<CompletedDownload>> {
        self.list_completed_downloads().await
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut items = self.list_completed_downloads().await?;
        items.sort_by_key(|item| std::cmp::Reverse(item.completed_at));
        items.truncate(limit);
        Ok(items)
    }

    async fn list_recent_completed_downloads_with_feedback_scope(
        &self,
        limit: usize,
        _scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<CompletedDownload>> {
        self.list_recent_completed_downloads(limit).await
    }

    async fn list_recent_completed_downloads_for_client_scope(
        &self,
        limit: usize,
        client_ids: &[String],
        client_types: &[String],
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut items = self.list_recent_completed_downloads(limit).await?;
        items.retain(|item| {
            let item_type = item.client_type.trim();
            if excluded_client_types
                .iter()
                .any(|client_type| item_type.eq_ignore_ascii_case(client_type.trim()))
            {
                return false;
            }

            let has_scope = !client_ids.is_empty() || !client_types.is_empty();
            if !has_scope {
                return true;
            }

            let item_client_id = item.client_id.trim();
            let id_matches = !item_client_id.is_empty()
                && client_ids
                    .iter()
                    .any(|client_id| item_client_id == client_id.trim());
            let type_matches = !item_type.is_empty()
                && client_types
                    .iter()
                    .any(|client_type| item_type.eq_ignore_ascii_case(client_type.trim()));

            id_matches || type_matches
        });
        Ok(items)
    }

    async fn list_recent_completed_downloads_excluding_client_types(
        &self,
        limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<CompletedDownload>> {
        self.list_recent_completed_downloads_for_client_scope(
            limit,
            &[],
            &[],
            excluded_client_types,
        )
        .await
    }

    /// Fetch a single completed download by its client-scoped source
    /// reference.
    ///
    /// The default scans the bounded recent window, so an item that has aged
    /// out of that window is not found. Clients whose backend supports a
    /// direct per-item history lookup should override this — callers rely on
    /// it to recover long-stuck items past the recent listing.
    async fn get_completed_download_for_source(
        &self,
        client_id: &str,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        let reference = download_client_item_id.trim();
        if reference.is_empty() {
            return Ok(None);
        }

        let client_ids = Some(client_id.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .into_iter()
            .collect::<Vec<_>>();
        let client_types = Some(client_type.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .into_iter()
            .collect::<Vec<_>>();
        let items = self
            .list_recent_completed_downloads_for_client_scope(
                crate::DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
                &client_ids,
                &client_types,
                &[],
            )
            .await?;
        Ok(items
            .into_iter()
            .find(|item| item.download_client_item_id == reference))
    }

    async fn pause_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "pause is not supported for this download client".to_string(),
        ))
    }

    async fn pause_queue_item_for_client(&self, _client_id: &str, id: &str) -> AppResult<()> {
        self.pause_queue_item(id).await
    }

    async fn resume_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "resume is not supported for this download client".to_string(),
        ))
    }

    async fn resume_queue_item_for_client(&self, _client_id: &str, id: &str) -> AppResult<()> {
        self.resume_queue_item(id).await
    }

    /// Remove one item from the client.
    ///
    /// `remove_data` asks the client to delete the payload it downloaded along
    /// with the entry — Sonarr's `RemoveItem(item, deleteData: true)`, which it
    /// uses for both post-import and failed-download cleanup. Callers own the
    /// policy: the terminal-cleanup executor asks for it only where the data is
    /// Scryer's to reclaim (see
    /// `import::workflow::results::reconcile_terminal_download_cleanup`), and
    /// everything else passes `false`. A client that has no way to keep the
    /// data, or no way to delete it, honors what it can and documents the rest.
    async fn delete_queue_item(
        &self,
        _id: &str,
        _is_history: bool,
        _remove_data: bool,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "delete is not supported for this download client".to_string(),
        ))
    }

    async fn delete_queue_item_for_client_id(
        &self,
        _client_id: &str,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.delete_queue_item(id, is_history, remove_data).await
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        let _ = client_type;
        self.delete_queue_item(id, is_history, remove_data).await
    }

    async fn mark_imported(&self, _request: &DownloadClientMarkImportedRequest) -> AppResult<()> {
        Err(AppError::Repository(
            "mark_imported is not supported for this download client".to_string(),
        ))
    }

    async fn mark_imported_non_destructive(
        &self,
        _request: &DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn mark_imported_non_destructive_for_client_id(
        &self,
        _client_id: &str,
        request: &DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        self.mark_imported_non_destructive(request).await
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        Err(AppError::Repository(
            "client status is not supported for this download client".to_string(),
        ))
    }

    async fn get_client_status_for_client_id(
        &self,
        _client_id: &str,
    ) -> AppResult<DownloadClientStatus> {
        self.get_client_status().await
    }

    async fn test_connection(&self) -> AppResult<String> {
        Err(AppError::Repository(
            "test connection is not supported for this download client".to_string(),
        ))
    }
}

#[async_trait]
pub trait SubtitleDownloadRepository: Send + Sync {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>>;
    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>>;
    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>>;
    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<crate::subtitles::ExternalSubtitleProbeCacheEntry>>;
    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>>;
    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()>;
    async fn upsert_probe_cache_entry(
        &self,
        entry: &crate::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()>;
    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()>;
    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>>;
    async fn delete_probe_cache_entry(&self, media_file_id: &str, file_path: &str)
    -> AppResult<()>;
    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool>;
    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()>;
}
