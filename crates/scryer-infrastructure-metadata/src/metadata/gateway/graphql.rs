//! Gateway query compatibility contract.
//!
//! Keep every selected operation within SMG's general limits: at most 100 root
//! fields, 100 aggregate requested `series`/`movie`/`metadataBulk` items, and
//! 10,000 default unweighted gqlgen complexity units. The three discovery
//! roots (`discoverPublicFeed`, `titleRecommendations`, and
//! `collectionCompletions`) share a combined limit of 10 root fields.
//!
//! Synchronous discovery subject lists are capped at 5,000. Snapshot and
//! context operations retain their configured 30,000-subject and
//! 250-changed-subject limits. `sectionTypes` is capped at the schema's current
//! raw enum count of 21 before deduplication. `metadataBulk` remains capped at
//! 50 items.
//!
//! These are shape limits, so do not add operation-name or query-hash
//! allowlists. Anonymous GraphQL GET remains supported by SMG, but Scryer
//! discovery uses authenticated POST requests. The artwork builder legitimately
//! produces 100 movie or series aliases (up to 1,200 scalar/object field
//! complexity); checked-in documents peak at 191.

pub const SEARCH_TVDB_QUERY: &str = include_str!("metadata_gateway/search_tvdb.graphql");
pub const SEARCH_TVDB_BATCH_QUERY: &str =
    include_str!("metadata_gateway/search_tvdb_batch.graphql");
pub const SEARCH_TVDB_RICH_QUERY: &str = include_str!("metadata_gateway/search_tvdb_rich.graphql");
pub const SEARCH_TVDB_MULTI_QUERY: &str =
    include_str!("metadata_gateway/search_tvdb_multi.graphql");
pub const GET_MOVIE_QUERY: &str = include_str!("metadata_gateway/get_movie.graphql");
pub const GET_SERIES_QUERY: &str = include_str!("metadata_gateway/get_series.graphql");
pub const METADATA_BULK_QUERY: &str = include_str!("metadata_gateway/metadata_bulk.graphql");
pub const TITLES_QUERY: &str = include_str!("metadata_gateway/titles.graphql");
pub const RESOLVE_TITLES_QUERY: &str = include_str!("metadata_gateway/resolve_titles.graphql");
pub const SEARCH_TITLES_QUERY: &str = include_str!("metadata_gateway/search_titles.graphql");
pub const SEARCH_TITLES_MULTI_QUERY: &str =
    include_str!("metadata_gateway/search_titles_multi.graphql");
pub const SEARCH_TITLES_BATCH_QUERY: &str =
    include_str!("metadata_gateway/search_titles_batch.graphql");
pub const DISCOVER_PUBLIC_FEED_QUERY: &str =
    include_str!("metadata_gateway/discover_public_feed.graphql");
pub const TITLE_RECOMMENDATIONS_QUERY: &str =
    include_str!("metadata_gateway/title_recommendations.graphql");
pub const COLLECTION_COMPLETIONS_QUERY: &str =
    include_str!("metadata_gateway/collection_completions.graphql");
pub const SUBMIT_DISCOVERY_CONTEXT_SNAPSHOT_QUERY: &str =
    include_str!("metadata_gateway/submit_discovery_context_snapshot.graphql");
pub const DISCOVERY_CONTEXT_SNAPSHOT_STATUS_QUERY: &str =
    include_str!("metadata_gateway/discovery_context_snapshot_status.graphql");
pub const DISCOVERY_CONTEXT_SNAPSHOT_PAGE_QUERY: &str =
    include_str!("metadata_gateway/discovery_context_snapshot_page.graphql");
pub const DISCOVERY_CONTEXT_CHANGES_QUERY: &str =
    include_str!("metadata_gateway/discovery_context_changes.graphql");
pub const ACKNOWLEDGE_DISCOVERY_CONTEXT_SNAPSHOT_QUERY: &str =
    include_str!("metadata_gateway/acknowledge_discovery_context_snapshot.graphql");
