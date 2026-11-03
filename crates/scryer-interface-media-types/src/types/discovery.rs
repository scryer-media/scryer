use super::{Long, MediaFacetValue};
use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

#[derive(SimpleObject, Clone)]
/// Discovery synchronization state and the number of context changes awaiting processing.
pub struct DiscoverySyncStatusPayload {
    /// Current synchronization lifecycle state and next eligible work times.
    pub state: DiscoverySyncStatePayload,
    /// Number of discovery context changes not yet incorporated into a completed snapshot.
    pub pending_context_change_count: Long,
}

#[derive(SimpleObject, Clone)]
/// Generation markers, completion timestamps, and eligibility timestamps for discovery synchronization.
pub struct DiscoverySyncStatePayload {
    /// Generation ID of the last successfully completed context snapshot, or null before the first success.
    pub last_success_generation_id: Option<ID>,
    /// Generation ID of the last completed public-feed build, or null when none has completed.
    pub last_public_feed_generation_id: Option<ID>,
    /// UTC completion time of the last context snapshot, or null when none has completed.
    pub last_context_snapshot_completed_at: Option<DateTime<Utc>>,
    /// UTC completion time of the last incremental reload, or null when none has completed.
    pub last_incremental_reload_completed_at: Option<DateTime<Utc>>,
    /// UTC completion time of the last public-feed build, or null when none has completed.
    pub last_public_feed_completed_at: Option<DateTime<Utc>>,
    /// UTC time when another context snapshot becomes eligible, or null when no schedule is set.
    pub next_context_snapshot_eligible_at: Option<DateTime<Utc>>,
    /// UTC time when another incremental reload becomes eligible, or null when no schedule is set.
    pub next_incremental_reload_eligible_at: Option<DateTime<Utc>>,
    /// UTC time when another public-feed build becomes eligible, or null when no schedule is set.
    pub next_public_feed_eligible_at: Option<DateTime<Utc>>,
    /// UTC time when this synchronization state was last updated.
    pub updated_at: DateTime<Utc>,
}

#[derive(InputObject, Clone, Default)]
/// Controls which discovery home surfaces are included and how many items each section may contain.
pub struct DiscoveryHomeInput {
    /// Include public discovery sections; defaults to true when omitted.
    pub include_public: Option<bool>,
    /// Include personalized sections when the caller is authorized; defaults to true when omitted.
    pub include_personalized: Option<bool>,
    /// Include unresolved external discovery items; defaults to false when omitted.
    pub include_unresolved: Option<bool>,
    /// Maximum items per section; defaults to 25 and must be between 1 and 100.
    pub limit_per_section: Option<i32>,
    /// Optional content, tag, studio, year, and rating filters.
    pub filters: Option<DiscoveryHomeFiltersInput>,
}

#[derive(InputObject, Clone, Default)]
/// Filters for discovery home items; omitted lists are treated as empty filters.
pub struct DiscoveryHomeFiltersInput {
    /// Restrict results to these media facets; omitted includes all facets.
    pub content_types: Option<Vec<MediaFacetValue>>,
    /// Canonical genre tag keys to include; blank keys are invalid.
    pub genre_tag_keys: Option<Vec<String>>,
    /// Canonical theme tag keys to include; blank keys are invalid.
    pub theme_tag_keys: Option<Vec<String>>,
    /// Studio slugs to include; omitted applies no studio restriction.
    pub studio_slugs: Option<Vec<String>>,
    /// Inclusive minimum release year, if supplied.
    pub minimum_year: Option<i32>,
    /// Inclusive maximum release year, if supplied.
    pub maximum_year: Option<i32>,
    /// Inclusive rating floor on a finite 0 through 10 scale.
    pub minimum_rating: Option<f64>,
}

#[derive(InputObject, Clone, Default)]
/// Selects public, personalized, and unresolved discovery sources for filter-option lookup.
pub struct DiscoveryHomeFilterOptionsInput {
    /// Include public filter options; defaults to true when omitted.
    pub include_public: Option<bool>,
    /// Include personalized filter options when authorized; defaults to true when omitted.
    pub include_personalized: Option<bool>,
    /// Include unresolved items in option generation; defaults to false when omitted.
    pub include_unresolved: Option<bool>,
}

#[derive(InputObject, Clone, Default)]
/// Filters and paginates the general discovery-item listing.
pub struct DiscoveryItemsInput {
    /// Free-text query applied to discovery titles and searchable metadata.
    pub query: Option<String>,
    /// Target-kind values to include; omitted uses no target-kind filter.
    pub target_kinds: Option<Vec<String>>,
    /// Source identifiers to include; omitted uses all available sources.
    pub sources: Option<Vec<String>>,
    /// Relation types to include; omitted applies no relation-type filter.
    pub relation_types: Option<Vec<String>>,
    /// Relation subtypes to include; omitted applies no relation-subtype filter.
    pub relation_subtypes: Option<Vec<String>>,
    /// Genre values to include; omitted applies no genre filter.
    pub genres: Option<Vec<String>>,
    /// Status tags to include; omitted applies no status-tag filter.
    pub status_tags: Option<Vec<String>>,
    /// Facet terms to include; omitted applies no facet-term filter.
    pub facet_terms: Option<Vec<String>>,
    /// Include items already owned in the caller's libraries; defaults to false.
    pub include_owned: Option<bool>,
    /// Include unresolved items; defaults to false.
    pub include_unresolved: Option<bool>,
    /// Include public-source items; defaults to false.
    pub include_public: Option<bool>,
    /// Page size; defaults to 50 and values below 1 become 1.
    pub limit: Option<i32>,
    /// Zero-based offset; defaults to 0 and negative values become 0.
    pub offset: Option<i32>,
    /// Page one discovery home section in full instead of listing items.
    /// With the same `sectionRailLimit`, home filters and `includeOwned` as
    /// the home and the default sort, the first `sectionRailLimit` items are
    /// the rail as the dashboard renders it, after titles earlier rails
    /// showed. When set, only `includeOwned`, `includeUnresolved`, `limit`,
    /// `offset` and the section fields below apply. A blank id is refused;
    /// an unknown or empty section returns no items and no section.
    pub section_id: Option<String>,
    /// The home's `limitPerSection` the section was opened from; defaults to
    /// 25 and must be between 1 and 100. It must match the home for the
    /// first page to match the rail.
    pub section_rail_limit: Option<i32>,
    /// Home filters applied to the section, validated as on the home.
    pub home_filters: Option<DiscoveryHomeFiltersInput>,
    /// Section ordering; defaults to `RELEVANCE`, the rail's own order.
    pub sort: Option<DiscoverySectionSortValue>,
    /// Drop titles with a pending media request in a readable library;
    /// defaults to false.
    pub hide_requested: Option<bool>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Ordering of a paged discovery section. Every order other than relevance
/// falls back to relevance on ties.
pub enum DiscoverySectionSortValue {
    /// The section's own order, as the home rail shows it.
    Relevance,
    /// Most popular first.
    Popularity,
    /// Highest rated first.
    Rating,
    /// Newest release year first; titles without a year last.
    ReleaseDate,
    /// Alphabetical by sort title.
    Title,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Label family a browse card lists titles under.
pub enum DiscoveryBrowseLabelFamilyValue {
    /// A canonical genre.
    Genre,
    /// A canonical theme.
    Theme,
}

#[derive(SimpleObject, Clone)]
/// One "browse by" card: an allowlisted genre or theme the caller's
/// personalized pool holds enough titles of to open as a section.
pub struct DiscoveryBrowseLabelPayload {
    /// Whether the card lists a genre or a theme.
    pub family: DiscoveryBrowseLabelFamilyValue,
    /// Canonical hyphenated label key, such as `coming-of-age`. Clients
    /// translate the key and fall back to `title` for a key they do not know.
    pub label_key: String,
    /// Section id to pass to `discoveryItems.sectionId`.
    pub section_id: String,
    /// English card title.
    pub title: String,
    /// Titles under the label in the home's candidate pool. The section reads
    /// a deeper pool, so it can list more than this.
    pub item_count: Long,
}

#[derive(InputObject, Clone)]
/// Identifies one discovery item and controls whether unresolved records may be returned.
pub struct DiscoveryItemDetailInput {
    /// Stable discovery target key, not a local title ID.
    pub target_key: String,
    /// Include the unresolved record when no local title is resolved; defaults to true.
    pub include_unresolved: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// Discovery home response with synchronization status, selected sections, facets, and authorization state.
pub struct DiscoveryHomePayload {
    /// Synchronization readiness and pending-context information used to interpret the result.
    pub status: DiscoverySyncStatusPayload,
    /// Selected hero item, or null when no eligible item exists.
    pub hero_item: Option<DiscoveryItemPayload>,
    /// Public sections; an empty list means no public section was selected.
    pub public_sections: Vec<DiscoverySectionPayload>,
    /// Personalized sections; empty when unavailable or no items qualify.
    pub personalized_sections: Vec<DiscoverySectionPayload>,
    /// Complete-collection section, or null when none is available.
    pub complete_collection: Option<DiscoverySectionPayload>,
    /// Facet summaries returned for the selected discovery scope.
    pub facets: Vec<DiscoveryFacetPayload>,
    /// "Browse by" genre and theme cards, in allowlist order; empty when
    /// personalized results are unavailable.
    pub browse_labels: Vec<DiscoveryBrowseLabelPayload>,
    /// Whether the caller may view personalized results.
    pub can_view_personalized: bool,
}

#[derive(SimpleObject, Clone)]
/// Card-oriented discovery home response with synchronization status and optional hero and sections.
pub struct DiscoveryHomeCardsPayload {
    /// Synchronization readiness and pending-context information.
    pub status: DiscoverySyncStatusPayload,
    /// Selected hero card, or null when no eligible card exists.
    pub hero_item: Option<DiscoveryHomeHeroPayload>,
    /// Public card sections; empty means no public section was selected.
    pub public_sections: Vec<DiscoveryHomeSectionPayload>,
    /// Personalized card sections; empty when unavailable or no items qualify.
    pub personalized_sections: Vec<DiscoveryHomeSectionPayload>,
    /// Complete-collection card section, or null when none is available.
    pub complete_collection: Option<DiscoveryHomeSectionPayload>,
    /// "Browse by" genre and theme cards, in allowlist order; empty when
    /// personalized results are unavailable.
    pub browse_labels: Vec<DiscoveryBrowseLabelPayload>,
    /// Whether the caller may view personalized cards.
    pub can_view_personalized: bool,
}

#[derive(SimpleObject, Clone)]
/// A titled discovery home section with its source surface, total count, and bounded items.
pub struct DiscoveryHomeSectionPayload {
    /// Stable section identifier.
    pub section_id: String,
    /// Section classification value.
    pub section_type: String,
    /// Human-readable section title.
    pub title: String,
    /// Whether this section is public, personalized, or mixed.
    pub surface: DiscoverySurfaceValue,
    /// Rail-label key a personalized "because you like" section is named after,
    /// in canonical hyphenated form such as `coming-of-age`; null for every other
    /// section. Clients translate the key and fall back to `title` for a key they
    /// do not know.
    pub label_key: Option<String>,
    /// Total matching items before the returned item page is applied.
    pub total_count: Long,
    /// Items returned for this section, possibly fewer than the total.
    pub items: Vec<DiscoveryHomeCardPayload>,
    /// Discovery generations a section page was composed from. Empty on the
    /// home. A client paging a section restarts from the first page when
    /// these change between pages.
    pub generation_ids: Vec<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Origin surface of a discovery home section.
pub enum DiscoverySurfaceValue {
    /// Publicly available discovery content.
    Public,
    /// Personalized content based on caller-visible context.
    Personalized,
    /// A section combining public and personalized content.
    Mixed,
}

#[derive(SimpleObject, Clone)]
/// Compact discovery card identity and display metadata.
pub struct DiscoveryHomeCardPayload {
    /// Stable local or discovery-card ID for this result.
    pub id: ID,
    /// Stable discovery target key used for detail lookup.
    pub target_key: String,
    /// Kind of target represented by the card.
    pub target_kind: MediaFacetValue,
    /// Display title selected for this card.
    pub display_title: String,
    /// Original source title when it differs from the display title, or null.
    pub original_title: Option<String>,
    /// Normalized sort title when available, or null.
    pub sort_title: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Poster URL, or null when no poster is available.
    pub poster_url: Option<String>,
    /// Content facet represented by the card.
    pub content_type: MediaFacetValue,
    /// Whether the source metadata marks the content as adult.
    pub is_adult: bool,
    /// Whether the target is already owned in the input scope.
    pub owned_in_input: bool,
}

#[derive(SimpleObject, Clone)]
/// Expanded discovery hero metadata, ratings, tags, and ownership state.
pub struct DiscoveryHomeHeroPayload {
    /// Stable local or discovery-card ID for this result.
    pub id: ID,
    /// Stable discovery target key used for detail lookup.
    pub target_key: String,
    /// Kind of target represented by the hero.
    pub target_kind: MediaFacetValue,
    /// Display title selected for the hero.
    pub display_title: String,
    /// Original source title when it differs from the display title, or null.
    pub original_title: Option<String>,
    /// Normalized sort title when available, or null.
    pub sort_title: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Poster URL, or null when no poster is available.
    pub poster_url: Option<String>,
    /// Background artwork URL, or null when unavailable.
    pub background_url: Option<String>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Content facet represented by the hero.
    pub content_type: MediaFacetValue,
    /// Whether the source metadata marks the content as adult.
    pub is_adult: bool,
    /// Primary rating on the source's normalized scale, or null when unavailable.
    pub rating: Option<f64>,
    /// Names of rating sources contributing to the primary rating.
    pub rating_sources: Vec<String>,
    /// External ratings with source-specific values and vote counts.
    pub external_ratings: Vec<DiscoveryExternalRatingPayload>,
    /// Canonical genre and theme tags associated with the hero.
    pub genre_tags: Vec<CanonicalMediaTagPayload>,
    /// Number of subject matches contributing to this hero selection.
    pub matched_subject_count: i32,
    /// Whether the target is already owned in the input scope.
    pub owned_in_input: bool,
}

#[derive(SimpleObject, Clone)]
/// Available discovery-home filter values for genres, themes, and studios.
pub struct DiscoveryHomeFilterOptionsPayload {
    /// Canonical genre options; empty means no matching genre option exists.
    pub genres: Vec<CanonicalTagFilterOptionPayload>,
    /// Canonical theme options; empty means no matching theme option exists.
    pub themes: Vec<CanonicalTagFilterOptionPayload>,
    /// Studio slugs available in the selected scope.
    pub studio_slugs: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Canonical tag key and display name used by discovery filters.
pub struct CanonicalTagFilterOptionPayload {
    /// Stable canonical tag key accepted by discovery filters.
    pub key: String,
    /// Display name associated with the canonical key.
    pub name: String,
}

#[derive(SimpleObject, Clone)]
/// Paged discovery-item results with a total count and personalization capability.
pub struct DiscoveryItemsPayload {
    /// Items in the requested page; empty means no items matched.
    pub items: Vec<DiscoveryItemPayload>,
    /// Total number of matching items before pagination.
    pub total_count: Long,
    /// Whether the caller may view personalized results.
    pub can_view_personalized: bool,
    /// The paged section when `sectionId` was requested and found: its
    /// identity, total and this page's items as cards. Null otherwise.
    pub section: Option<DiscoveryHomeSectionPayload>,
}

#[derive(InputObject, Clone)]
/// Controls catalog discovery for one required media facet and optional library scope.
pub struct CatalogDiscoveryInput {
    /// Required media facet whose catalog is searched.
    pub facet: MediaFacetValue,
    /// Library IDs defining the owned scope; omitted or empty means no explicit library restriction.
    pub library_ids: Option<Vec<ID>>,
    /// Include unresolved discovery records; defaults to true.
    pub include_unresolved: Option<bool>,
    /// Maximum items per result group; defaults to 12 and values below 1 become 1.
    pub limit_per_group: Option<i32>,
    /// Maximum number of groups; defaults to 6 and values below 1 become 1.
    pub max_groups: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Catalog discovery response containing authorization state and grouped results.
pub struct CatalogDiscoveryPayload {
    /// Whether the caller may view personalized groups.
    pub can_view_personalized: bool,
    /// Discovery groups returned in selection order; empty means no group qualified.
    pub groups: Vec<CatalogDiscoveryGroupPayload>,
}

#[derive(SimpleObject, Clone)]
/// One catalog discovery group with surface, count, and bounded items.
pub struct CatalogDiscoveryGroupPayload {
    /// Stable group identifier.
    pub id: String,
    /// Reason this group was selected.
    pub kind: CatalogDiscoveryGroupKindValue,
    /// Public or personalized origin of the group.
    pub surface: CatalogDiscoverySurfaceValue,
    /// Optional label value, such as a genre or theme key.
    pub label_value: Option<String>,
    /// Rail-label key a genre or theme affinity group is named after, in
    /// canonical hyphenated form such as `coming-of-age`; null for every other
    /// group. Clients translate the key and fall back to `labelValue` for a key
    /// they do not know.
    pub label_key: Option<String>,
    /// Total matching items before the returned item page is applied.
    pub total_count: Long,
    /// Items returned for this group.
    pub items: Vec<DiscoveryItemPayload>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Classification of a catalog discovery group.
pub enum CatalogDiscoveryGroupKindValue {
    /// Top public recommendations.
    PublicTop,
    /// A named public section.
    PublicSection,
    /// Personalized genre affinity.
    GenreAffinity,
    /// Personalized theme affinity.
    ThemeAffinity,
    /// Acclaimed content group.
    Acclaimed,
    /// Complete-collection group.
    CompleteCollection,
    /// Fallback group used when a more specific group is unavailable.
    Fallback,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Origin surface of a catalog discovery group.
pub enum CatalogDiscoverySurfaceValue {
    /// Publicly available group.
    Public,
    /// Personalized group based on caller-visible context.
    Personalized,
}

#[derive(SimpleObject, Clone)]
/// A discovery section containing a title, origin surface, count, and items.
pub struct DiscoverySectionPayload {
    /// Stable section identifier.
    pub section_id: String,
    /// Section classification value.
    pub section_type: String,
    /// Human-readable section title.
    pub title: String,
    /// Surface label for this section.
    pub surface: String,
    /// Rail-label key a personalized "because you like" section is named after,
    /// in canonical hyphenated form such as `coming-of-age`; null for every other
    /// section. Clients translate the key and fall back to `title` for a key they
    /// do not know.
    pub label_key: Option<String>,
    /// Total matching items before the returned item page is applied.
    pub total_count: Long,
    /// Items returned for this section.
    pub items: Vec<DiscoveryItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Rating from an external source, including normalized score and optional vote count.
pub struct DiscoveryExternalRatingPayload {
    /// External rating source name.
    pub source: String,
    /// Source-specific raw value, or null when unavailable.
    pub value: Option<f64>,
    /// Source-specific score, or null when unavailable.
    pub score: Option<f64>,
    /// Normalized score used for cross-source comparison.
    pub normalized: f64,
    /// Number of votes reported by the source, or null when unavailable.
    pub votes: Option<i32>,
    /// Source URL for the rating record.
    pub url: String,
}

#[derive(SimpleObject, Clone)]
/// External identifier associated with a discovery item.
pub struct DiscoveryExternalIdPayload {
    /// External source name.
    pub source: String,
    /// External identifier kind.
    pub kind: String,
    /// Provider-issued identifier value.
    pub id: String,
    /// Canonical composite key for the external identity.
    pub key: String,
}

#[derive(SimpleObject, Clone)]
/// Content certification for one country and release type.
pub struct DiscoveryContentCertificationPayload {
    /// Certification value, such as an age category.
    pub value: String,
    /// Authority or source that supplied the certification.
    pub source: String,
    /// Release-type code, or null when the source did not specify one.
    pub release_type: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Country-specific content ratings and their certifications.
pub struct DiscoveryContentRatingPayload {
    /// ISO-like country code supplied by the source.
    pub country: String,
    /// Certifications reported for this country.
    pub certifications: Vec<DiscoveryContentCertificationPayload>,
    /// Numeric age rating, or null when unavailable.
    pub age_rating: Option<i32>,
    /// Source of the numeric age rating, or null when unavailable.
    pub age_rating_source: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Canonical metadata tag with provenance and content-safety flags.
pub struct CanonicalMediaTagPayload {
    /// Stable canonical tag key.
    pub key: String,
    /// Canonical tag category.
    pub category: String,
    /// Display name for the tag.
    pub name: String,
    /// Confidence score from 0 through 1 when supplied by the source.
    pub confidence: Option<f64>,
    /// Source names contributing this tag.
    pub sources: Vec<String>,
    /// Original provider tag keys that mapped to this canonical tag.
    pub source_tag_keys: Vec<String>,
    /// Whether the tag is marked adult.
    pub is_adult: bool,
    /// Whether the tag is marked spoiler-sensitive.
    pub is_spoiler: bool,
}

#[derive(SimpleObject, Clone)]
/// Detailed discovery item with resolution, metadata, relationships, provenance, and ownership state.
pub struct DiscoveryItemPayload {
    /// Stable local or discovery-item ID.
    pub id: ID,
    /// Stable discovery target key used for detail lookup.
    pub target_key: String,
    /// Target kind string supplied by the discovery source.
    pub target_kind: String,
    /// Whether the target resolved to a local title.
    pub resolved: bool,
    /// Local title ID when resolved, or null when unresolved.
    pub resolved_title_id: Option<ID>,
    /// Display title selected from available metadata.
    pub display_title: String,
    /// Original source title when available.
    pub original_title: Option<String>,
    /// Normalized sort title when available.
    pub sort_title: Option<String>,
    /// Release year, or null when unavailable.
    pub year: Option<i32>,
    /// Poster URL, or null when unavailable.
    pub poster_url: Option<String>,
    /// Background artwork URL, or null when unavailable.
    pub background_url: Option<String>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Content facet string, or null when the source did not classify it.
    pub content_type: Option<String>,
    /// Canonical tags associated with the item.
    pub canonical_tags: Vec<CanonicalMediaTagPayload>,
    /// Whether the source metadata marks the content as adult.
    pub is_adult: bool,
    /// Country-specific content ratings.
    pub content_ratings: Vec<DiscoveryContentRatingPayload>,
    /// Primary normalized rating, or null when unavailable.
    pub rating: Option<f64>,
    /// Names of sources contributing to the primary rating.
    pub rating_sources: Vec<String>,
    /// External ratings from individual providers.
    pub external_ratings: Vec<DiscoveryExternalRatingPayload>,
    /// External IDs associated with the item.
    pub external_ids: Vec<DiscoveryExternalIdPayload>,
    /// Source-derived status tags.
    pub status_tags: Vec<String>,
    /// Source-derived tags not mapped to canonical tags.
    pub source_tags: Vec<String>,
    /// Source identifiers contributing this item.
    pub sources: Vec<String>,
    /// Highest-priority source, or null when no source is preferred.
    pub best_source: Option<String>,
    /// Relation types connecting this item to other discovery subjects.
    pub relation_types: Vec<String>,
    /// Relation subtypes connecting this item to other discovery subjects.
    pub relation_subtypes: Vec<String>,
    /// Number of contributing sources, or null when not computed.
    pub source_count: Option<i32>,
    /// Number of discovery edges, or null when not computed.
    pub edge_count: Option<i32>,
    /// Number of relations, or null when not computed.
    pub relation_count: Option<i32>,
    /// Number of matched source subjects, or null when not computed.
    pub source_subject_count: Option<i32>,
    /// Ranking score used for ordering, or null when not computed.
    pub rank_score: Option<f64>,
    /// Titles of matched subjects contributing to this item.
    pub matched_subject_titles: Vec<String>,
    /// Number of matched subjects contributing to this item.
    pub matched_subject_count: i32,
    /// TMDB collection ID, or null when unavailable.
    pub tmdb_collection_id: Option<String>,
    /// TMDB collection name, or null when unavailable.
    pub tmdb_collection_name: Option<String>,
    /// Whether the target is already owned in the input scope.
    pub owned_in_input: bool,
    /// Studio slug, or null when unavailable.
    pub studio_slug: Option<String>,
    /// Numeric person IDs associated with the item.
    pub person_ids: Vec<i32>,
    /// Facet terms extracted from the source.
    pub facet_terms: Vec<String>,
    /// Context terms extracted from the source.
    pub context_terms: Vec<String>,
    /// The `DiscoveryNeighborGroup` this related title belongs to; set only on `discoveryNeighbors` items, and null there when the related titles were not grouped.
    pub group_id: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Discovery facet counts for external and local catalog matches.
pub struct DiscoveryFacetPayload {
    /// Human-readable facet name.
    pub name: String,
    /// Stable facet value.
    pub value: String,
    /// External discovery count, or null when unavailable.
    pub smg_count: Option<Long>,
    /// Local owned-title count, or null when unavailable.
    pub local_count: Option<Long>,
}

#[derive(InputObject, Clone)]
/// One anchor whose related titles are returned, owned or not.
pub struct DiscoveryNeighborsInput {
    /// The discovery target key of the anchor, in the server's own vocabulary (tvdb:series:<id>, tmdb:movie:<id>, tmdb:series:<id>, imdb:title:<tt>, mal:anime:<id>, anidb:anime:<id>, anilist:anime:<id>).
    pub target_key: String,
    /// Media facet of the anchor. A movie key fits MOVIE or ANIME, a series key SERIES or ANIME.
    pub facet: MediaFacetValue,
    /// Maximum related titles to return; defaults to 24, which is also the most ever returned. Values outside 0 through 24 are clamped.
    #[graphql(default = 24)]
    pub limit: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// One neighbourhood among an anchor's related titles, found by the metadata gateway from the semantic and relation links between them.
pub struct DiscoveryNeighborGroup {
    /// Group ID, matching `DiscoveryItemPayload.groupId`.
    pub id: i32,
    /// Label key of the genre or theme that names the group, or null when the group is named after its medoid.
    pub label_key: Option<String>,
    /// Display label of the group.
    pub label: String,
    /// Target key of the related title that best represents the group.
    pub medoid: String,
    /// Number of returned related titles in the group.
    pub size: i32,
}

#[derive(SimpleObject, Clone)]
/// An anchor's related titles and the neighbourhoods they fall into.
pub struct DiscoveryNeighborsPayload {
    /// Related titles, best first.
    pub items: Vec<DiscoveryItemPayload>,
    /// Groups holding at least one returned item, in the order the metadata gateway ranked them (largest first before the returned items were chosen); empty when the related titles were not grouped.
    pub groups: Vec<DiscoveryNeighborGroup>,
}

#[derive(InputObject, Clone)]
/// Selects one facet's library graph and, optionally, the libraries it covers.
pub struct LibraryGraphInput {
    /// Media facet whose library graph is returned.
    pub facet: MediaFacetValue,
    /// Library IDs to cover; omitted or empty covers every library of the facet the viewer can see. IDs the viewer cannot see are ignored.
    pub library_ids: Option<Vec<ID>>,
}

#[derive(SimpleObject, Clone)]
/// One owned title in a library graph.
pub struct LibraryGraphNode {
    /// Discovery subject key of the owned title.
    pub subject_key: String,
    /// Discovery target key the title resolved to; edges and medoids use this key.
    pub target_key: String,
    /// Cluster the node belongs to, or -1 when it was not placed in a cluster.
    pub cluster_id: i32,
}

#[derive(SimpleObject, Clone)]
/// An undirected similarity link between two owned titles, by target key.
pub struct LibraryGraphEdge {
    /// Target key of one endpoint; always sorts before `to`.
    pub from: String,
    /// Target key of the other endpoint.
    pub to: String,
    /// Link strength from 0 to 1.
    pub weight: f64,
    /// Evidence for the link: `semantic`, `relation:<type>`, or `affinity:<family>:<label>`.
    pub kinds: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// A cluster of related owned titles.
pub struct LibraryGraphCluster {
    /// Cluster ID, matching `LibraryGraphNode.clusterId`.
    pub id: i32,
    /// Label key of the genre or theme that names the cluster, or null when the cluster is named after its medoid.
    pub label_key: Option<String>,
    /// Display label of the cluster.
    pub label: String,
    /// Target key of the visible title that best represents the cluster.
    pub medoid: String,
    /// Number of visible titles in the cluster.
    pub size: i32,
}

#[derive(SimpleObject, Clone)]
/// A facet's library graph, limited to the titles the viewer can see. Layout coordinates are not included.
pub struct LibraryGraph {
    /// When the metadata gateway built the graph.
    pub generated_at: String,
    /// Owned titles in the graph.
    pub nodes: Vec<LibraryGraphNode>,
    /// Similarity links between visible titles.
    pub edges: Vec<LibraryGraphEdge>,
    /// Clusters with at least one visible title.
    pub clusters: Vec<LibraryGraphCluster>,
    /// Owned titles of the facet in the covered libraries, counted by discovery subject.
    pub subject_count: i32,
    /// True when the facet had more titles than one graph covers, so the newest were graphed and the rest left out.
    pub truncated: bool,
}
