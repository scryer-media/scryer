use std::collections::HashSet;

use async_trait::async_trait;
use scryer_domain::{Collection, MediaFacet, Title};

use crate::{
    ActivityKind, AnimeMapping, AppResult, EpisodeMetadata, MetadataGateway, RenameCollisionPolicy,
    RenameMissingMetadataPolicy, RenamePlanItem, SeasonMetadata, TitleMetadataUpdate,
};

/// Result of hydrating a title's metadata from a metadata gateway.
/// Movies return empty seasons/episodes. Series include full season/episode data.
pub struct HydrationResult {
    pub metadata_update: TitleMetadataUpdate,
    pub seasons: Vec<SeasonMetadata>,
    pub episodes: Vec<EpisodeMetadata>,
    pub anime_mappings: Vec<AnimeMapping>,
}

/// Configuration and strategies for a specific media facet.
/// Each facet (movie, tv, anime) implements this trait to define
/// its metadata hydration, rename strategy, import routing, and
/// acquisition behavior.
#[async_trait]
pub trait FacetHandler: Send + Sync {
    /// The domain enum variant this handler covers.
    fn facet(&self) -> MediaFacet;

    /// String ID used in settings keys, database columns, audit logs.
    /// e.g. "movie", "tv", "anime"
    fn facet_id(&self) -> &str;

    /// Download client category string.
    fn download_category(&self) -> &str;

    /// Settings key for the library root path (e.g. "movies.path").
    fn library_path_key(&self) -> &str;

    /// Settings key for the global rename template (e.g. "rename.template.movie.global").
    fn rename_template_key(&self) -> &str;

    /// Settings key for the global collision policy (e.g. "rename.collision_policy.movie.global").
    fn collision_policy_key(&self) -> &str;

    /// Settings key for the global missing metadata policy.
    fn missing_metadata_policy_key(&self) -> &str;

    /// Default rename template when no setting is configured.
    fn default_rename_template(&self) -> &str;

    /// Default library root path.
    fn default_library_path(&self) -> &str;

    /// Whether this facet has episode-level structure.
    fn has_episodes(&self) -> bool;

    /// Optional activity kind emitted when a title of this facet is added.
    fn title_added_activity_kind(&self) -> Option<ActivityKind>;

    /// Indexer search category (e.g. "movie", "series", "anime").
    fn search_category(&self) -> &str;

    /// Scope ID used for scoped rename settings lookups (e.g. "movie", "series").
    fn rename_scope_id(&self) -> &str;

    /// Hydrate a title's metadata by calling the metadata gateway.
    async fn hydrate_metadata(
        &self,
        gateway: &dyn MetadataGateway,
        tvdb_id: i64,
        language: &str,
    ) -> AppResult<HydrationResult>;

    /// Build a rename plan item for a single title+collection.
    fn build_rename_plan_item(
        &self,
        title: &Title,
        collection: &Collection,
        template: &str,
        collision_policy: &RenameCollisionPolicy,
        missing_metadata_policy: &RenameMissingMetadataPolicy,
        planned_targets: &mut HashSet<String>,
    ) -> RenamePlanItem;
}
