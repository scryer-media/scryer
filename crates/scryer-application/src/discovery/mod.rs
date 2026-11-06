use crate::library_scan::{
    DiscoveryContextChangeType, DiscoveryContextChangedSubjectInput, DiscoveryContextChangesInput,
    DiscoveryContextMediumMixInput, DiscoveryContextSnapshotPageResult,
    DiscoveryContextSnapshotSubmitInput, DiscoveryDashboardResult, DiscoveryDashboardSection,
    DiscoveryExternalIdInput, DiscoveryPublicFeedInput, DiscoverySubjectInput, DiscoveryTitle,
};
use crate::ports::{
    CatalogDiscoveryGroup, CatalogDiscoveryGroupKind, CatalogDiscoveryQuery,
    CatalogDiscoveryResult, CatalogDiscoverySectionCandidatesRecord, CatalogDiscoverySurface,
    CatalogOwnedProbe, CatalogOwnedSeriesMovieRecord, CatalogOwnedTitleRecord,
    DISCOVERY_DEFAULT_SCOPE_KEY, DiscoveryExternalIdRecord, DiscoveryFacetRecord,
    DiscoveryHomeCandidate, DiscoveryHomeFilterOptions, DiscoveryHomeQuery, DiscoveryHomeResult,
    DiscoveryHomeSectionCandidatesRecord, DiscoveryItemDetailQuery,
    DiscoveryItemLibraryProvenanceRecord, DiscoveryItemRecord, DiscoveryItemTargetRecord,
    DiscoveryItemsQuery, DiscoveryItemsResult, DiscoveryItemsStorageQuery,
    DiscoveryPendingContextChangeRecord, DiscoveryRankComponentRecord, DiscoverySectionItemsRecord,
    DiscoverySectionRecord, DiscoverySectionResult, DiscoverySourceTagRecord,
    DiscoverySubmittedSubjectRecord, DiscoverySyncStatus, TitleExternalIdLookup,
};
use crate::settings::keys::{
    DISCOVERY_GENRE_RAIL_CORROBORATION_KEY, DISCOVERY_GENRE_RAIL_CREDIBILITY_GATE_KEY,
    DISCOVERY_MEDIUM_AFFINITY_GATE_KEY, DISCOVERY_RAIL_ORDER_KEY, DISCOVERY_RAIL_ORDER_LEGACY,
    SETTINGS_SCOPE_SYSTEM,
};
use crate::{AppError, AppResult, AppUseCase};
use chrono::{DateTime, Utc};
use scryer_domain::{
    CanonicalMediaTag, DomainEvent, DomainEventPayload, DomainExternalIds, ExternalId,
    LibraryPermission, MediaFacet, Title, TitleContextSnapshot, User, title_catalog_sort_input,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;
use tracing::{debug, warn};

pub(crate) const DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS: usize = 250;
const DISCOVERY_HOME_MIN_CANDIDATES: usize = 500;
const DISCOVERY_HOME_MAX_CANDIDATES: usize = 2_000;
/// The personalized pool a section page reads, and so how deep a personalized
/// section can go. The home reads a prefix of the same ordered pool
/// (`personalized_home_candidate_limit`), which is what a section rebuilds
/// the home's rails and evidence median from.
const DISCOVERY_PERSONALIZED_SECTION_MAX_CANDIDATES: usize = 2_500;
/// How deep a public section page can go.
const DISCOVERY_PUBLIC_SECTION_MAX_ITEMS: usize = 2_500;
const DISCOVERY_FOR_YOU_SECTION_ID: &str = "for_you";
const DISCOVERY_COMPLETE_COLLECTION_MIN_CANDIDATES: usize = 100;
const DISCOVERY_COMPLETE_COLLECTION_MAX_CANDIDATES: usize = 500;
const CATALOG_ANIME_WEEKLY_SECTION_ID: &str = "anime_this_week";
const CATALOG_ANIME_SUPPRESSED_PUBLIC_SECTION_IDS: &[&str] = &["trending_now", "popular_series"];
const CATALOG_ANIME_PRIORITY_SECTION_IDS: &[&str] = &["new_on_streaming", "most_anticipated_anime"];
/// Minimum number of distinct rating providers required before an item's rating is
/// treated as corroborated. A lone source (for example a single trakt vote)
/// can spike to 10.0 without corroboration, so such fossils must lose to
/// corroborated evidence whenever ratings decide ordering.
pub const DISCOVERY_MIN_CREDIBLE_RATING_SOURCE_COUNT: usize = 2;
/// A single-provider rating still counts as credible when it is backed by at
/// least this many votes: an aggregator score built from thousands of votes
/// (a MAL-only anime, for instance) is stronger evidence than two thinly
/// sourced provider entries, and must not be demoted below them. Kept in
/// lockstep with SMG's display-evidence bar (CanonicalRatingDisplayMinVotes)
/// so a rating SMG displays is never ranked non-credible here for votes alone.
pub const DISCOVERY_CREDIBLE_RATING_MIN_VOTES: i32 = 25;
/// A rail's share of one medium may not exceed this multiple of the library's
/// share of that medium. Shared across the three mediums by design: anime and
/// Western animation are separate mediums under one rule.
const DISCOVERY_MEDIUM_SHARE_MULTIPLIER: f64 = 2.0;
/// Floor under the proportional cap, so a library that owns a single anime can
/// still be shown a couple rather than none.
const DISCOVERY_MEDIUM_SHARE_MIN_SLOTS: usize = 2;
/// A label earns a rail only when at least this many owned titles carry it...
const DISCOVERY_RAIL_LABEL_SUPPORT_MIN_CARRIERS: usize = 3;
/// ...or this share of the library, whichever is larger. Three crime titles in
/// a ten-title library must not earn a Crime rail with the same authority as
/// forty in three hundred.
const DISCOVERY_RAIL_LABEL_SUPPORT_SHARE: f64 = 0.10;
/// Growth in submitted subjects since the last snapshot run that, on its own,
/// justifies re-snapshotting ahead of the normal cadence. A library that has
/// grown by a quarter is a different library.
const DISCOVERY_LIBRARY_GROWTH_RESNAPSHOT_SHARE: f64 = 0.25;
/// A personalized rail is emitted with however many items pass the evidence
/// floor, and dropped entirely below this many. Short rails are a feature: they
/// say the system knows little yet. Padding to `limit` from the evidence-less
/// tail is what produced the alphabetical junk this floor exists to stop.
const DISCOVERY_RAIL_MIN_ITEMS: usize = 8;
/// `rank_score` at or below this is the evidence-less band: SMG's semantic
/// floor (~1.0) and the affiliation-key-only 0. Anything above it was reached
/// by a real relation edge.
const DISCOVERY_EVIDENCE_LESS_RANK_SCORE_CEILING: f64 = 1.5;
/// Below this many owned titles the library gets FOR_YOU and nothing else.
const DISCOVERY_RAIL_BUDGET_MIN_OWNED_TITLES: usize = 10;
/// At or above this many owned titles the full label ladder composes; between
/// the two tiers the ladder is one theme rail and one genre rail.
const DISCOVERY_RAIL_BUDGET_FULL_LADDER_OWNED_TITLES: usize = 50;
/// Width of the label ladder per family (genres, themes) at the full tier.
const DISCOVERY_RAIL_LADDER_LABELS: usize = 2;
/// SMG scores a provider's own genre list at 0.95, caps name-alias matches at
/// 0.9 and phrase-sequence guesses at 0.85. This line keeps the first two and
/// drops the guesses.
const DISCOVERY_CORROBORATED_GENRE_MIN_CONFIDENCE: f64 = 0.9;
/// Distinct sources that make a confident genre corroborated on their own,
/// without a provider genre key.
const DISCOVERY_CORROBORATED_GENRE_MIN_SOURCES: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryContextDefaults {
    pub(crate) region: String,
    pub(crate) language: String,
    pub(crate) max_items: usize,
    pub(crate) include_owned: bool,
    pub(crate) include_unresolved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryLibraryProvenance {
    pub(crate) subject_key: String,
    pub(crate) title_id: Option<String>,
    pub(crate) library_id: String,
}

impl Default for DiscoveryContextDefaults {
    fn default() -> Self {
        Self {
            region: "US".to_string(),
            language: "eng".to_string(),
            max_items: 5_000,
            include_owned: true,
            include_unresolved: true,
        }
    }
}

impl DiscoveryContextDefaults {
    pub(crate) fn public_feed_input(&self) -> DiscoveryPublicFeedInput {
        DiscoveryPublicFeedInput {
            region: self.region.clone(),
            language: self.language.clone(),
            section_types: Vec::new(),
            limit_per_section: 25,
            include_unresolved: self.include_unresolved,
            full_sections: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveryLibraryContext {
    pub(crate) subjects: Vec<DiscoveryLibrarySubject>,
    pub(crate) subject_provenance: Vec<DiscoveryLibrarySubject>,
    /// Owned titles per medium, computed from the same classifier that gates
    /// the rails. Sent to SMG on every submission and folded into the
    /// fingerprint, so a library that gains its first anime invalidates the
    /// cached run rather than serving a pool built without anime.
    pub(crate) medium_mix: DiscoveryLibraryMediumMix,
    pub(crate) fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveryLibrarySubject {
    pub(crate) title_id: String,
    pub(crate) library_id: String,
    pub(crate) title_name: String,
    pub(crate) facet: String,
    pub(crate) subject_key: String,
    pub(crate) subject: DiscoverySubjectInput,
    canonical: CanonicalSubject,
}

#[derive(Default)]
struct DiscoveryVisibility {
    /// Libraries the viewer holds `View` on.
    owned_library_ids: HashSet<String>,
    /// Libraries the viewer holds `Request` on.
    requestable_library_ids: HashSet<String>,
    facets_by_library_id: HashMap<String, MediaFacet>,
    readable_library_ids: HashSet<String>,
    allowed_media_kinds: HashSet<&'static str>,
}

impl DiscoveryVisibility {
    fn allows_facet(&self, facet: &MediaFacet) -> bool {
        self.allowed_media_kinds
            .contains(discovery_media_kind_for_facet(facet.clone()))
    }

    fn allows_item(&self, item: &DiscoveryItemRecord) -> bool {
        discovery_item_media_kind(item)
            .is_some_and(|media_kind| self.allowed_media_kinds.contains(media_kind))
    }

    fn sorted_allowed_media_kinds(&self) -> Vec<String> {
        let mut media_kinds = self
            .allowed_media_kinds
            .iter()
            .map(|media_kind| (*media_kind).to_string())
            .collect::<Vec<_>>();
        media_kinds.sort();
        media_kinds
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalSubject {
    subject_key: String,
    key: Option<String>,
    kind: String,
    facet: String,
    external_ids: Vec<CanonicalExternalId>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalExternalId {
    source: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalContext<'a> {
    schema_version: u8,
    defaults: &'a DiscoveryContextDefaults,
    medium_mix: DiscoveryContextMediumMixInput,
    subjects: &'a [CanonicalSubject],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoverySubjectParts {
    facet: String,
    subject_key: String,
    subject: DiscoverySubjectInput,
    canonical: CanonicalSubject,
}

mod affinity_cache;
mod catalog;
mod context;
mod home;
mod library_graph;
mod medium;
mod neighbors;
mod rail_labels;
mod records;
mod section;
mod watchlist;

use catalog::*;
use context::*;
use home::*;
use medium::*;
use rail_labels::*;
use records::*;
pub(crate) use section::DiscoverySectionCache;
#[cfg(test)]
use section::*;
use watchlist::*;

pub(crate) use affinity_cache::DiscoveryAffinityCache;
pub(crate) use catalog::discovery_library_growth_warrants_snapshot;
pub use catalog::{
    CatalogOwnershipDecisions, catalog_owned_probe_for_targets, catalog_ownership_decisions,
};
#[cfg(test)]
pub(crate) use context::build_discovery_library_context;
pub(crate) use context::{
    build_projected_discovery_library_context, coalesce_pending_context_change,
    incremental_item_records, pending_context_change_from_domain_event, public_feed_item_records,
    public_feed_section_records, snapshot_item_records,
};
#[cfg(test)]
pub(crate) use library_graph::{
    LIBRARY_GRAPH_AXIS, LIBRARY_GRAPH_MIN_COSINE, LIBRARY_GRAPH_NEIGHBORS_PER_NODE,
    LIBRARY_GRAPH_RETRY_BACKOFF_SECONDS,
};
pub use library_graph::{
    LibraryGraphCluster, LibraryGraphEdge, LibraryGraphNode, LibraryGraphQuery, LibraryGraphView,
};
pub(crate) use library_graph::{LibraryGraphRebuildSummary, library_graph_facets_for_changes};
pub use neighbors::{
    DISCOVERY_NEIGHBORS_DEFAULT_LIMIT, DiscoveryNeighborsQuery, DiscoveryNeighborsView,
};
pub(crate) use records::{
    pending_context_changes_need_snapshot_reconciliation,
    pending_context_changes_resolved_key_count, snapshot_facet_records,
    title_more_like_this_item_records, title_recommendations_subject,
};

/// How the viewer's library holds one discovery item.
pub(super) struct DiscoveryItemOwnership {
    /// A title in a library the viewer can read that stands for the item.
    pub(super) readable_title_id: Option<String>,
    /// A readable series holds the item as one of its linked movies.
    pub(super) owned_by_series_movie: bool,
}

impl DiscoveryItemOwnership {
    pub(super) fn is_owned(&self) -> bool {
        self.readable_title_id.is_some() || self.owned_by_series_movie
    }
}

impl AppUseCase {
    pub async fn title_more_like_this(
        &self,
        actor: &User,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let requested_limit = limit.clamp(0, 100) as usize;
        if requested_limit == 0 {
            return Ok(Vec::new());
        }
        let source_title = self.get_title(actor, title_id).await?.ok_or_else(|| {
            AppError::not_found_reason(
                crate::error_reasons::catalog::TITLE_NOT_FOUND,
                format!("title {title_id}"),
            )
        })?;
        if let Err(error) = self
            .queue_title_more_like_this_refresh_if_due(
                &source_title,
                crate::catalog_workflow::HydrationSource::Interactive,
            )
            .await
        {
            warn!(
                title_id = %title_id,
                error = %error,
                "failed to refresh title recommendations while loading more-like-this cache"
            );
        }
        let readable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let candidate_limit = requested_limit.saturating_mul(4).clamp(24, 100) as i64;
        let mut items = self
            .services
            .library
            .discovery
            .list_title_more_like_this_items(title_id, candidate_limit)
            .await?;
        for item in &mut items {
            item.resolved_title_id = None;
            item.owned_in_input = false;
        }
        let ownership = self
            .discovery_items_readable_ownership(&items, &readable_library_ids)
            .await?;
        let mut filtered_items = Vec::with_capacity(requested_limit.min(items.len()));
        for (mut item, owned) in items.into_iter().zip(ownership) {
            if owned.is_owned() {
                continue;
            }

            item.resolved = false;
            item.resolved_title_id = None;
            item.owned_in_input = false;
            filtered_items.push(item);
            if filtered_items.len() >= requested_limit {
                break;
            }
        }
        Ok(filtered_items)
    }

    /// Which title in a library the viewer can read stands for each item, if
    /// any, matched on the item's target key the way the library stores
    /// external ids. One entry per item, in order.
    pub(super) async fn discovery_items_readable_ownership(
        &self,
        items: &[DiscoveryItemRecord],
        readable_library_ids: &HashSet<String>,
    ) -> AppResult<Vec<DiscoveryItemOwnership>> {
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let mut item_lookup_indexes = vec![Vec::<usize>::new(); items.len()];
        let mut lookups = Vec::new();
        let mut series_movie_lookups = Vec::new();
        for (item_index, item) in items.iter().enumerate() {
            let Some((source, kind, value)) = discovery_target_key_parts(&item.target_key) else {
                continue;
            };
            let is_movie = kind == "movie";
            let values = discovery_local_external_id_values(&kind, &value);
            for source in discovery_local_external_id_sources(&source, &kind) {
                for external_id in &values {
                    let lookup_index = lookups.len();
                    let lookup = TitleExternalIdLookup {
                        lookup_index,
                        source: source.clone(),
                        external_id: external_id.clone(),
                    };
                    if is_movie {
                        series_movie_lookups.push(lookup.clone());
                    }
                    lookups.push(lookup);
                    item_lookup_indexes[item_index].push(lookup_index);
                }
            }
        }
        let mut matches_by_lookup_index = HashMap::<usize, Vec<Title>>::new();
        for lookup_match in self
            .services
            .catalog
            .titles
            .list_by_external_id_lookups(&lookups)
            .await?
        {
            matches_by_lookup_index
                .entry(lookup_match.lookup_index)
                .or_default()
                .push(lookup_match.title);
        }
        let series_movie_owned_lookup_indexes = self
            .services
            .catalog
            .shows
            .list_series_movie_external_id_lookup_matches(
                &readable_library_id_list,
                &series_movie_lookups,
            )
            .await?
            .into_iter()
            .map(|matched| matched.lookup_index)
            .collect::<HashSet<_>>();
        Ok(item_lookup_indexes
            .into_iter()
            .map(|lookup_indexes| DiscoveryItemOwnership {
                readable_title_id: lookup_indexes.iter().find_map(|lookup_index| {
                    matches_by_lookup_index
                        .get(lookup_index)
                        .and_then(|titles| {
                            titles
                                .iter()
                                .find(|candidate| {
                                    readable_library_ids.contains(candidate.library_id.as_str())
                                })
                                .map(|title| title.id.clone())
                        })
                }),
                owned_by_series_movie: lookup_indexes
                    .iter()
                    .any(|lookup_index| series_movie_owned_lookup_indexes.contains(lookup_index)),
            })
            .collect())
    }

    pub async fn discovery_home(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
    ) -> AppResult<DiscoveryHomeResult> {
        self.discovery_home_projected(
            actor,
            query,
            crate::DiscoveryHomeProjection {
                full_cards: true,
                hero: true,
                browse_labels: true,
                facets: true,
            },
        )
        .await
    }

    pub async fn discovery_home_cards(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
    ) -> AppResult<DiscoveryHomeResult> {
        self.discovery_home_projected(
            actor,
            query,
            crate::DiscoveryHomeProjection {
                full_cards: false,
                hero: true,
                browse_labels: true,
                facets: false,
            },
        )
        .await
    }

    pub async fn discovery_home_filter_options(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
    ) -> AppResult<DiscoveryHomeFilterOptions> {
        let visibility = self.discovery_visibility(actor).await?;
        let readable_library_ids = &visibility.readable_library_ids;
        // The instance-wide switch is a second input to the same per-caller
        // gate, so every read path below serves public rows only when it is
        // off without needing a branch of its own.
        let can_view_personalized =
            !readable_library_ids.is_empty() && self.personalized_discovery_enabled().await?;
        let status = self
            .load_discovery_sync_status_for_visibility(can_view_personalized)
            .await?;
        let mut allowed_media_kinds = visibility
            .allowed_media_kinds
            .iter()
            .map(|media_kind| (*media_kind).to_string())
            .collect::<Vec<_>>();
        allowed_media_kinds.sort();
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let options = self
            .services
            .library
            .discovery
            .list_discovery_home_filter_options(
                query
                    .include_public
                    .then_some(status.state.last_public_feed_generation_id.as_deref())
                    .flatten(),
                (can_view_personalized && query.include_personalized)
                    .then_some(status.state.last_success_generation_id.as_deref())
                    .flatten(),
                &readable_library_id_list,
                &allowed_media_kinds,
                query.include_unresolved,
            )
            .await?;
        Ok(options)
    }

    pub async fn discovery_home_projected(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
        projection: crate::DiscoveryHomeProjection,
    ) -> AppResult<DiscoveryHomeResult> {
        let discovery_home_started_at = Instant::now();
        let visibility = self.discovery_visibility(actor).await?;
        let readable_library_ids = &visibility.readable_library_ids;
        // The instance-wide switch is a second input to the same per-caller
        // gate, so every read path below serves public rows only when it is
        // off without needing a branch of its own.
        let personalized_discovery_enabled = self.personalized_discovery_enabled().await?;
        let can_view_personalized =
            !readable_library_ids.is_empty() && personalized_discovery_enabled;
        let status = self
            .load_discovery_sync_status_for_visibility(can_view_personalized)
            .await?;
        let limit = discovery_section_limit(query.limit_per_section);
        let include_unresolved = query.include_unresolved;
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let mut allowed_media_kinds = visibility
            .allowed_media_kinds
            .iter()
            .map(|media_kind| (*media_kind).to_string())
            .collect::<Vec<_>>();
        allowed_media_kinds.sort();
        let owned_library_id_list = sorted_discovery_library_ids(&visibility.owned_library_ids);
        let owned_visibility = self
            .discovery_home_owned_visibility(&owned_library_id_list, &status.state)
            .await?;
        let candidate_load_started_at = Instant::now();
        let mut candidates_by_id = HashMap::<String, DiscoveryHomeCandidate>::new();
        let mut home_submitted_subjects = Vec::new();
        let mut home_subject_run_id = None;
        let mut public_candidate_count = 0usize;
        let mut personalized_candidate_count = 0usize;
        let mut complete_collection_candidate_count = 0usize;

        let mut public_sections = Vec::<DiscoverySectionResult>::new();
        if query.include_public {
            let public_candidate_limit = public_home_candidate_limit(limit);
            if let Some(public_run_id) = status.state.last_public_feed_generation_id.as_deref() {
                let public_section_items = self
                    .services
                    .library
                    .discovery
                    .list_public_discovery_section_items(
                        public_run_id,
                        &[],
                        &allowed_media_kinds,
                        include_unresolved,
                        &query.filters,
                        public_candidate_limit as i64,
                    )
                    .await?;
                public_candidate_count = public_section_items
                    .iter()
                    .map(|record| record.items.len())
                    .sum();
                let public_section_results = public_section_items
                    .into_iter()
                    .filter_map(|record| {
                        home_section_candidates_record_to_result(record, &mut candidates_by_id)
                    })
                    .collect::<Vec<_>>();
                public_sections = filter_discovery_sections_for_owned_items(
                    public_section_results,
                    &owned_visibility,
                    &visibility,
                    limit,
                );
            }
        }

        // The viewer's own watchlist composes ahead of every library rail and
        // claims its titles first, so a title the viewer already asked for is
        // never offered again as "because you like".
        let watchlist = if query.include_personalized && personalized_discovery_enabled {
            self.discovery_watchlist_composition(
                actor,
                &visibility,
                &owned_library_id_list,
                &status.state,
                can_view_personalized,
                &query.filters,
                include_unresolved,
                limit,
            )
            .await?
        } else {
            DiscoveryWatchlistComposition::default()
        };

        let mut personalized_sections = Vec::new();
        let mut complete_collection = None;
        let mut facets = Vec::new();
        if can_view_personalized
            && query.include_personalized
            && let Some(context_run_id) = status.state.last_success_generation_id.as_deref()
        {
            let personalized_candidates = self
                .services
                .library
                .discovery
                .list_personalized_discovery_home_items(
                    context_run_id,
                    &readable_library_id_list,
                    &allowed_media_kinds,
                    include_unresolved,
                    false,
                    &query.filters,
                    personalized_home_candidate_limit(limit) as i64,
                )
                .await?;
            personalized_candidate_count = personalized_candidates.len();
            let mut personalized_items = home_selection_items_from_candidates(
                personalized_candidates,
                &mut candidates_by_id,
            );
            personalized_items.retain(|item| visibility.allows_item(item));
            let complete_collection_candidates = self
                .services
                .library
                .discovery
                .list_personalized_complete_collection_items(
                    context_run_id,
                    &readable_library_id_list,
                    &allowed_media_kinds,
                    include_unresolved,
                    &query.filters,
                    complete_collection_candidate_limit(limit) as i64,
                )
                .await?;
            complete_collection_candidate_count = complete_collection_candidates.len();
            let mut complete_collection_items = home_selection_items_from_candidates(
                complete_collection_candidates,
                &mut candidates_by_id,
            );
            complete_collection_items.retain(|item| visibility.allows_item(item));
            home_subject_run_id = Some(context_run_id.to_string());
            home_submitted_subjects = self
                .discovery_submitted_subjects_for_items(
                    context_run_id,
                    &readable_library_id_list,
                    personalized_items
                        .iter()
                        .chain(complete_collection_items.iter()),
                )
                .await?;
            resolve_discovery_matched_subjects(&mut personalized_items, &home_submitted_subjects)?;
            let rail_settings = self.discovery_rail_composition_settings().await;
            let library_profile = self
                .discovery_library_affinity_profile(
                    readable_library_ids,
                    Some(context_run_id),
                    rail_settings.genre_rail_corroboration,
                )
                .await?;
            resolve_discovery_matched_subjects(
                &mut complete_collection_items,
                &home_submitted_subjects,
            )?;
            complete_collection =
                complete_collection_section(&complete_collection_items, include_unresolved, limit);
            let visible_items =
                discovery_home_visible_items(&personalized_items, include_unresolved);
            let rail_context = DiscoveryRailContext::new(
                &candidates_by_id,
                rail_settings,
                &library_profile,
                &visible_items,
                limit,
            );
            personalized_sections = personalized_section_results_in_context(
                visible_items,
                &library_profile,
                &rail_context,
                &watchlist.claimed_item_keys,
            );
            if projection.facets {
                facets = self
                    .services
                    .library
                    .discovery
                    .list_personalized_discovery_facets(
                        context_run_id,
                        &readable_library_id_list,
                        &allowed_media_kinds,
                        include_unresolved,
                    )
                    .await?;
            }
        }

        let mut browse_query = self
            .discovery_browse_query_with_owned(
                &visibility,
                &status.state,
                can_view_personalized && query.include_personalized,
                &DiscoveryItemsQuery {
                    include_unresolved,
                    ..DiscoveryItemsQuery::default()
                },
                &crate::ports::DiscoverySectionItemsQuery {
                    filters: query.filters.clone(),
                    ..Default::default()
                },
                &owned_visibility,
            )
            .await?;
        if !query.include_public {
            browse_query.public_run_id = None;
        }
        let browse_labels = if projection.browse_labels {
            self.discovery_pool_browse_labels(&browse_query).await?
        } else {
            Vec::new()
        };

        let public_top_rated_run_id = query
            .include_public
            .then_some(status.state.last_public_feed_generation_id.as_deref())
            .flatten();
        let context_top_rated_run_id = if can_view_personalized && query.include_personalized {
            status.state.last_success_generation_id.as_deref()
        } else {
            None
        };
        let top_rated_candidates = self
            .services
            .library
            .discovery
            .list_discovery_home_top_rated_items(
                public_top_rated_run_id,
                context_top_rated_run_id,
                &readable_library_id_list,
                &allowed_media_kinds,
                &readable_library_id_list,
                &owned_visibility.excluded_discovery_identity_keys(),
                include_unresolved,
                &query.filters,
                top_rated_home_candidate_limit(limit) as i64,
            )
            .await?;
        let top_rated_candidate_count = top_rated_candidates.len();
        let mut top_rated_items =
            home_selection_items_from_candidates(top_rated_candidates, &mut candidates_by_id);
        top_rated_items
            .retain(|item| visibility.allows_item(item) && !owned_visibility.item_is_owned(item));

        debug!(
            operation = "discovery_home",
            stage = "candidate_load",
            candidate_count = candidates_by_id.len(),
            public_candidate_count,
            personalized_candidate_count,
            complete_collection_candidate_count,
            top_rated_candidate_count,
            elapsed_ms = discovery_home_elapsed_ms(candidate_load_started_at),
            "discovery home candidates loaded"
        );

        let section_selection_started_at = Instant::now();

        if let Some(top_rated_section) = top_rated_discovery_home_section_with_candidates(
            &top_rated_items,
            &[],
            &candidates_by_id,
            include_unresolved,
            limit,
        ) {
            if top_rated_section
                .items
                .iter()
                .any(discovery_home_item_is_personalized)
            {
                personalized_sections.push(top_rated_section);
            } else {
                public_sections.push(top_rated_section);
            }
        }

        let hero_item = if projection.hero {
            select_discovery_home_hero_with_candidates(
                &public_sections,
                &personalized_sections,
                &candidates_by_id,
            )
        } else {
            None
        };
        // Added after the hero is chosen: watchlist cards are resolved outside
        // the candidate pool, so they carry no candidate for hero hydration.
        if let Some(watchlist_section) = watchlist.section {
            personalized_sections.insert(0, watchlist_section);
        }
        let mut result = DiscoveryHomeResult {
            status,
            hero_item,
            public_sections,
            personalized_sections,
            complete_collection,
            facets,
            can_view_personalized,
            browse_labels,
        };
        debug!(
            operation = "discovery_home",
            stage = "section_selection",
            public_section_count = result.public_sections.len(),
            personalized_section_count = result.personalized_sections.len(),
            complete_collection = result.complete_collection.is_some(),
            elapsed_ms = discovery_home_elapsed_ms(section_selection_started_at),
            "discovery home sections selected"
        );
        if projection.full_cards {
            self.hydrate_discovery_home_result(&mut result, &candidates_by_id)
                .await?;
        } else if projection.hero {
            self.hydrate_discovery_home_hero(&mut result, &candidates_by_id)
                .await?;
        }
        if let Some(run_id) = home_subject_run_id.as_deref() {
            // Selection only ever draws from the items resolved above; read
            // whatever a composed section carries beyond them.
            let covered = home_submitted_subjects
                .iter()
                .map(|subject| subject.subject_key.clone())
                .collect::<HashSet<_>>();
            let missing = discovery_home_subject_keys(&result)
                .into_iter()
                .filter(|key| !covered.contains(key))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                home_submitted_subjects.extend(
                    self.services
                        .library
                        .discovery
                        .list_discovery_submitted_subjects_for_keys(
                            run_id,
                            &readable_library_id_list,
                            &missing,
                        )
                        .await?,
                );
            }
        }
        resolve_discovery_home_selected_subjects(&mut result, &home_submitted_subjects)?;
        debug!(
            operation = "discovery_home",
            stage = "total",
            elapsed_ms = discovery_home_elapsed_ms(discovery_home_started_at),
            "discovery home loaded"
        );
        Ok(result)
    }

    async fn hydrate_discovery_home_result(
        &self,
        result: &mut DiscoveryHomeResult,
        candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
    ) -> AppResult<()> {
        let hydration_started_at = Instant::now();
        let selected_ids = selected_discovery_home_item_ids(result);
        let mut selected_candidates = selected_ids
            .iter()
            .filter_map(|id| candidates_by_id.get(id).cloned())
            .collect::<Vec<_>>();
        if selected_candidates.is_empty() {
            return Ok(());
        }
        self.services
            .library
            .discovery
            .hydrate_discovery_home_candidates(&mut selected_candidates)
            .await?;
        let hydrated_by_id = selected_candidates
            .into_iter()
            .map(|candidate| candidate.item)
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let subject_resolution_ids = discovery_home_subject_resolution_item_ids(result);
        replace_discovery_home_result_items(result, &hydrated_by_id, &subject_resolution_ids);
        if let Some(hero_item) = &mut result.hero_item {
            replace_discovery_home_item(
                hero_item,
                &hydrated_by_id,
                subject_resolution_ids.contains(&hero_item.id),
            );
        }
        debug!(
            operation = "discovery_home",
            stage = "selected_hydration",
            selected_card_count = hydrated_by_id.len(),
            selected_item_count = selected_ids.len(),
            elapsed_ms = discovery_home_elapsed_ms(hydration_started_at),
            "discovery home selected cards hydrated"
        );
        Ok(())
    }

    async fn hydrate_discovery_home_hero(
        &self,
        result: &mut DiscoveryHomeResult,
        candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
    ) -> AppResult<()> {
        let hydration_started_at = Instant::now();
        let Some(hero_id) = result.hero_item.as_ref().map(|item| item.id.clone()) else {
            return Ok(());
        };
        let Some(mut candidate) = candidates_by_id.get(&hero_id).cloned() else {
            return Err(AppError::not_found_reason(
                crate::error_reasons::discovery::HOME_HERO_NOT_FOUND,
                format!("discovery home hero {hero_id}"),
            ));
        };
        self.services
            .library
            .discovery
            .hydrate_discovery_home_hero(&mut candidate)
            .await?;
        result.hero_item = Some(candidate.item);
        debug!(
            operation = "discovery_home",
            stage = "hero_presentation_hydration",
            selected_item_count = 1,
            selected_title_count = 1,
            elapsed_ms = discovery_home_elapsed_ms(hydration_started_at),
            "discovery home hero presentation hydrated"
        );
        Ok(())
    }

    pub async fn discovery_items(
        &self,
        actor: &User,
        query: DiscoveryItemsQuery,
    ) -> AppResult<DiscoveryItemsResult> {
        if let Some(section_query) = &query.section {
            return self
                .discovery_section_items(actor, &query, section_query)
                .await;
        }
        let visibility = self.discovery_visibility(actor).await?;
        let readable_library_ids = &visibility.readable_library_ids;
        // The instance-wide switch is a second input to the same per-caller
        // gate, so every read path below serves public rows only when it is
        // off without needing a branch of its own.
        let can_view_personalized =
            !readable_library_ids.is_empty() && self.personalized_discovery_enabled().await?;
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let limit = discovery_items_limit(query.limit);
        let offset = query.offset;
        let storage_query = DiscoveryItemsStorageQuery {
            context_run_id: can_view_personalized
                .then(|| state.last_success_generation_id.clone())
                .flatten(),
            public_run_id: (query.include_public || !can_view_personalized)
                .then(|| state.last_public_feed_generation_id.clone())
                .flatten(),
            readable_library_ids: readable_library_id_list,
            allowed_media_kinds: visibility.sorted_allowed_media_kinds(),
            filters: query,
            home_filters: crate::ports::DiscoveryHomeFilters::default(),
            limit,
            offset,
            ..DiscoveryItemsStorageQuery::default()
        };
        let mut page = self
            .services
            .library
            .discovery
            .query_discovery_items(&storage_query)
            .await?;
        if let Some(context_run_id) = state.last_success_generation_id.as_deref() {
            let submitted_subjects = self
                .discovery_submitted_subjects_for_items(
                    context_run_id,
                    &sorted_discovery_library_ids(readable_library_ids),
                    page.items.iter(),
                )
                .await?;
            resolve_discovery_matched_subjects(&mut page.items, &submitted_subjects)?;
        }

        Ok(DiscoveryItemsResult {
            items: page.items,
            total_count: page.total_count,
            can_view_personalized,
            section: None,
            generation_ids: Vec::new(),
        })
    }

    pub async fn discovery_item_detail(
        &self,
        actor: &User,
        query: DiscoveryItemDetailQuery,
    ) -> AppResult<Option<DiscoveryItemRecord>> {
        let target_key = query.target_key.trim();
        if target_key.is_empty() {
            return Ok(None);
        }

        let visibility = self.discovery_visibility(actor).await?;
        let readable_library_ids = &visibility.readable_library_ids;
        // The instance-wide switch is a second input to the same per-caller
        // gate, so every read path below serves public rows only when it is
        // off without needing a branch of its own.
        let can_view_personalized =
            !readable_library_ids.is_empty() && self.personalized_discovery_enabled().await?;
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let context_run_id = can_view_personalized
            .then(|| state.last_success_generation_id.clone())
            .flatten();
        let storage_query = DiscoveryItemsStorageQuery {
            context_run_id: context_run_id.clone(),
            public_run_id: state.last_public_feed_generation_id.clone(),
            readable_library_ids: readable_library_id_list,
            allowed_media_kinds: visibility.sorted_allowed_media_kinds(),
            filters: DiscoveryItemsQuery {
                target_keys: vec![target_key.to_string()],
                include_owned: true,
                include_unresolved: query.include_unresolved,
                include_public: true,
                limit: 1,
                offset: 0,
                ..DiscoveryItemsQuery::default()
            },
            home_filters: crate::ports::DiscoveryHomeFilters::default(),
            limit: 1,
            offset: 0,
            ..DiscoveryItemsStorageQuery::default()
        };
        let mut page = self
            .services
            .library
            .discovery
            .query_discovery_items(&storage_query)
            .await?;
        if page.items.is_empty() {
            // A watchlist card built from the viewer's own list rows has no
            // stored item; it reads back under the rail's own gates.
            return self
                .discovery_watchlist_row_card_detail(
                    actor,
                    &visibility,
                    target_key,
                    query.include_unresolved,
                )
                .await;
        }
        if let Some(context_run_id) = context_run_id.as_deref() {
            let submitted_subjects = self
                .discovery_submitted_subjects_for_items(
                    context_run_id,
                    &sorted_discovery_library_ids(readable_library_ids),
                    page.items.iter(),
                )
                .await?;
            resolve_discovery_matched_subjects(&mut page.items, &submitted_subjects)?;
        }

        Ok(page.items.into_iter().next())
    }

    pub async fn catalog_discovery(
        &self,
        actor: &User,
        query: CatalogDiscoveryQuery,
    ) -> AppResult<CatalogDiscoveryResult> {
        let visibility = self.discovery_visibility(actor).await?;
        if !visibility.allows_facet(&query.facet) {
            return Ok(CatalogDiscoveryResult {
                groups: Vec::new(),
                can_view_personalized: false,
            });
        }

        let requested_library_ids = query
            .library_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let effective_library_ids = visibility
            .owned_library_ids
            .iter()
            .filter(|id| {
                visibility.facets_by_library_id.get(*id) == Some(&query.facet)
                    && (requested_library_ids.is_empty()
                        || requested_library_ids.contains(id.as_str()))
            })
            .cloned()
            .collect::<HashSet<_>>();
        let can_view_personalized =
            !effective_library_ids.is_empty() && self.personalized_discovery_enabled().await?;
        let effective_library_id_list = sorted_discovery_library_ids(&effective_library_ids);
        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let media_kind = discovery_media_kind_for_facet(query.facet.clone());
        let limit = catalog_discovery_group_limit(query.limit_per_group);
        let max_groups = catalog_discovery_max_groups(query.max_groups);
        let candidate_limit = catalog_discovery_candidate_limit(limit, max_groups);
        let owned_library_id_list = sorted_discovery_library_ids(&visibility.owned_library_ids);
        let owned_visibility = self
            .owned_discovery_visibility(&owned_library_id_list, &state)
            .await?;
        let excluded_public_identity_keys = owned_visibility.excluded_discovery_identity_keys();

        let mut public_sections =
            if let Some(public_run_id) = state.last_public_feed_generation_id.as_deref() {
                self.services
                    .library
                    .discovery
                    .list_catalog_public_discovery_sections(
                        public_run_id,
                        &effective_library_id_list,
                        &excluded_public_identity_keys,
                        media_kind,
                        query.include_unresolved,
                        candidate_limit as i64,
                    )
                    .await?
            } else {
                Default::default()
            };
        for section in &mut public_sections {
            section
                .items
                .retain(|item| !owned_visibility.item_is_owned(item));
        }

        let mut personalized_candidates = Vec::new();
        let mut affinity_run_id = None;
        let mut submitted_subjects = Vec::new();
        if can_view_personalized
            && let Some(context_run_id) = state.last_success_generation_id.as_deref()
        {
            let mut candidates = self
                .services
                .library
                .discovery
                .list_catalog_personalized_discovery_items(
                    context_run_id,
                    &effective_library_id_list,
                    media_kind,
                    query.include_unresolved,
                    candidate_limit as i64,
                )
                .await?
                .items;
            candidates.retain(|item| !owned_visibility.item_is_owned(item));
            affinity_run_id = Some(context_run_id);
            submitted_subjects = self
                .discovery_submitted_subjects_for_items(
                    context_run_id,
                    &effective_library_id_list,
                    candidates.iter(),
                )
                .await?;
            resolve_discovery_matched_subjects(&mut candidates, &submitted_subjects)?;
            personalized_candidates = candidates;
        }

        let mut groups = Vec::new();
        let mut emitted_item_keys = HashSet::new();
        if media_kind == "anime" {
            catalog_filter_anime_public_sections(&mut public_sections);
        }
        if let Some(public_top_section) =
            catalog_public_top_section(&mut public_sections, media_kind)
            && let Some(group) = catalog_public_top_group(
                public_top_section,
                media_kind,
                limit,
                &mut emitted_item_keys,
            )
        {
            groups.push(group);
        }

        if media_kind == "anime" {
            for section_id in CATALOG_ANIME_PRIORITY_SECTION_IDS {
                if groups.len() >= max_groups {
                    break;
                }
                if let Some(public_section) =
                    catalog_take_public_section(&mut public_sections, section_id)
                    && let Some(group) =
                        catalog_public_section_group(public_section, limit, &mut emitted_item_keys)
                {
                    groups.push(group);
                }
            }
        }
        let remaining_public_sections = public_sections;

        if !personalized_candidates.is_empty() && groups.len() < max_groups {
            let rail_settings = self.discovery_rail_composition_settings().await;
            let library_profile = self
                .discovery_library_affinity_profile(
                    &effective_library_ids,
                    affinity_run_id,
                    rail_settings.genre_rail_corroboration,
                )
                .await?;
            catalog_personalized_groups(
                &mut groups,
                &personalized_candidates,
                &library_profile,
                rail_settings,
                limit,
                max_groups,
                &mut emitted_item_keys,
            );
        }

        for public_section in remaining_public_sections {
            if groups.len() >= max_groups {
                break;
            }
            if let Some(group) =
                catalog_public_section_group(public_section, limit, &mut emitted_item_keys)
            {
                groups.push(group);
            }
        }

        let mut selected_items = groups
            .iter()
            .flat_map(|group| group.items.iter().cloned())
            .collect::<Vec<_>>();
        self.services
            .library
            .discovery
            .hydrate_catalog_discovery_items(&mut selected_items)
            .await?;
        let mut selected_items = selected_items.into_iter();
        for group in &mut groups {
            for item in &mut group.items {
                if let Some(hydrated) = selected_items.next() {
                    *item = hydrated;
                }
            }
            if matches!(group.surface, CatalogDiscoverySurface::Personalized) {
                resolve_discovery_matched_subjects(&mut group.items, &submitted_subjects)?;
            }
        }
        Ok(CatalogDiscoveryResult {
            groups,
            can_view_personalized,
        })
    }

    /// The libraries of `facet` a personalized discovery read covers: every
    /// library the actor can view, narrowed to `requested_library_ids` when
    /// any non-blank id is given. An id the actor cannot view is ignored.
    async fn discovery_effective_library_ids(
        &self,
        actor: &User,
        facet: &MediaFacet,
        requested_library_ids: &[String],
    ) -> AppResult<HashSet<String>> {
        let readable_library_ids = self
            .authorized_library_ids(actor, Some(facet.clone()), LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requested_library_ids = requested_library_ids
            .iter()
            .map(|library_id| library_id.trim())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();
        if requested_library_ids.is_empty() {
            return Ok(readable_library_ids);
        }
        Ok(readable_library_ids
            .into_iter()
            .filter(|library_id| requested_library_ids.contains(library_id.as_str()))
            .collect())
    }

    async fn discovery_visibility(&self, actor: &User) -> AppResult<DiscoveryVisibility> {
        // One read of the libraries and the actor's grants serves every
        // permission below and the facet map.
        let permissions = self.library_permission_context(actor, None).await?;
        let requestable_library_ids = permissions.library_ids(LibraryPermission::Request);
        let manageable_library_ids = permissions.library_ids(LibraryPermission::ManageTitles);
        let readable_library_ids = permissions.library_ids(LibraryPermission::View);

        let mut facets_by_library_id = permissions
            .libraries
            .into_iter()
            .map(|library| (library.id, library.facet))
            .collect::<HashMap<_, _>>();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            facets_by_library_id
                .entry(scryer_domain::default_library_id_for_facet(&facet))
                .or_insert(facet);
        }

        let mut visibility = DiscoveryVisibility {
            owned_library_ids: readable_library_ids.iter().cloned().collect(),
            requestable_library_ids: requestable_library_ids.iter().cloned().collect(),
            ..Default::default()
        };
        let discoverable_library_ids = requestable_library_ids
            .into_iter()
            .chain(manageable_library_ids)
            .collect::<HashSet<_>>();
        for library_id in &discoverable_library_ids {
            if let Some(facet) = facets_by_library_id.get(library_id) {
                visibility
                    .allowed_media_kinds
                    .insert(discovery_media_kind_for_facet(facet.clone()));
            }
        }
        visibility
            .readable_library_ids
            .extend(readable_library_ids.into_iter().filter(|library_id| {
                facets_by_library_id.get(library_id).is_some_and(|facet| {
                    visibility
                        .allowed_media_kinds
                        .contains(discovery_media_kind_for_facet(facet.clone()))
                })
            }));
        visibility.facets_by_library_id = facets_by_library_id;
        Ok(visibility)
    }

    async fn load_discovery_sync_status_for_visibility(
        &self,
        can_view_personalized: bool,
    ) -> AppResult<DiscoverySyncStatus> {
        let mut state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let mut recent_runs = self
            .services
            .library
            .discovery
            .list_recent_discovery_sync_runs(10)
            .await?;
        let mut pending_context_change_count = self
            .services
            .library
            .discovery
            .count_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?;

        if !can_view_personalized {
            state.last_success_generation_id = None;
            state.last_subject_fingerprint = None;
            state.last_context_snapshot_completed_at = None;
            state.last_incremental_reload_completed_at = None;
            state.dirty_since = None;
            state.dirty_reason_mask = 0;
            state.bootstrap_started_at = None;
            state.bootstrap_quiet_until = None;
            state.next_context_snapshot_eligible_at = None;
            state.next_incremental_reload_eligible_at = None;
            state.backoff_until = None;
            state.inflight_subject_fingerprint = None;
            state.inflight_domain_event_sequence = None;
            recent_runs.retain(|run| run.kind == "public_feed");
            pending_context_change_count = 0;
        }

        Ok(DiscoverySyncStatus {
            state,
            recent_runs,
            pending_context_change_count,
        })
    }

    /// Which of the items in the current generations (`state`'s context and
    /// public runs) the catalog in `readable_library_ids` already owns. It
    /// reads only the catalog titles and series-linked movies those items
    /// can name, so its cost follows the discovery pool, not the catalog;
    /// the decisions it makes for those items are the ones a visibility
    /// built from the whole catalog would make.
    async fn owned_discovery_visibility(
        &self,
        readable_library_ids: &[String],
        state: &crate::ports::DiscoverySyncStateRecord,
    ) -> AppResult<CatalogOwnedVisibility> {
        let run_ids = state
            .last_success_generation_id
            .iter()
            .chain(state.last_public_feed_generation_id.iter())
            .cloned()
            .collect::<Vec<_>>();
        if readable_library_ids.is_empty() || run_ids.is_empty() {
            return Ok(CatalogOwnedVisibility::default());
        }
        let targets = self
            .services
            .library
            .discovery
            .list_discovery_item_targets(&run_ids)
            .await?;
        self.owned_visibility_for_targets(
            readable_library_ids,
            targets.iter().map(|target| {
                (
                    target.target_key.as_str(),
                    target.resolved_title_id.as_deref(),
                )
            }),
        )
        .await
    }

    async fn discovery_home_owned_visibility(
        &self,
        readable_library_ids: &[String],
        state: &crate::ports::DiscoverySyncStateRecord,
    ) -> AppResult<CatalogOwnedVisibility> {
        self.owned_discovery_visibility(readable_library_ids, state)
            .await
    }

    /// Ownership decided for `targets` (target key, resolved title id) only:
    /// [`CatalogOwnedVisibility::item_is_owned`] and
    /// [`CatalogOwnedVisibility::target_key_is_owned`] answer exactly for
    /// those, and the excluded keys and title ids cover every owned one.
    async fn owned_visibility_for_targets<'a>(
        &self,
        readable_library_ids: &[String],
        targets: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    ) -> AppResult<CatalogOwnedVisibility> {
        if readable_library_ids.is_empty() {
            return Ok(CatalogOwnedVisibility::default());
        }
        let probe = catalog_owned_probe(targets);
        if probe.is_empty() {
            return Ok(CatalogOwnedVisibility::default());
        }
        let titles = self
            .services
            .catalog
            .titles
            .list_catalog_owned_title_records_matching(readable_library_ids, &probe)
            .await?;
        let series_movies = match self
            .services
            .catalog
            .shows
            .list_catalog_owned_series_movies_matching(readable_library_ids, &probe)
            .await?
        {
            Some(series_movies) => series_movies,
            None => {
                // A repository without a probe-scoped read: every link of
                // every title in those libraries.
                let title_ids = self
                    .services
                    .catalog
                    .titles
                    .list_catalog_owned_title_records(readable_library_ids)
                    .await?
                    .into_iter()
                    .map(|title| title.id)
                    .collect::<Vec<_>>();
                self.services
                    .catalog
                    .shows
                    .list_series_movie_links_for_titles(&title_ids)
                    .await?
                    .iter()
                    .map(CatalogOwnedSeriesMovieRecord::from)
                    .collect()
            }
        };
        Ok(CatalogOwnedVisibility::from_title_records_and_series_movies(&titles, &series_movies))
    }
}

impl AppUseCase {
    /// The submitted subjects of `run_id` in `library_ids` that any of
    /// `items` matched: the only ones card reasons can name.
    pub(super) async fn discovery_submitted_subjects_for_items<'a>(
        &self,
        run_id: &str,
        library_ids: &[String],
        items: impl IntoIterator<Item = &'a DiscoveryItemRecord>,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        let subject_keys = items
            .into_iter()
            .flat_map(|item| item.matched_subject_keys.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if subject_keys.is_empty() || library_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.services
            .library
            .discovery
            .list_discovery_submitted_subjects_for_keys(run_id, library_ids, &subject_keys)
            .await
    }

    async fn discovery_library_affinity_profile(
        &self,
        allowed_library_ids: &HashSet<String>,
        run_id: Option<&str>,
        genre_rail_corroboration: bool,
    ) -> AppResult<DiscoveryLibraryAffinityProfile> {
        let library_ids = sorted_discovery_library_ids(allowed_library_ids);
        let Some(revision) = self
            .services
            .catalog
            .titles
            .discovery_affinity_revision()
            .await?
        else {
            return self
                .discovery_library_affinity_profile_uncached(
                    &library_ids,
                    run_id,
                    genre_rail_corroboration,
                )
                .await;
        };
        let key = affinity_cache::DiscoveryAffinityCacheKey {
            run_id: run_id.map(str::to_owned),
            library_ids: library_ids.clone(),
            genre_rail_corroboration,
            revision,
        };
        let profile = self
            .runtime
            .catalog
            .discovery_affinity_cache
            .get_or_fill(
                key,
                || {
                    self.discovery_library_affinity_profile_uncached(
                        &library_ids,
                        run_id,
                        genre_rail_corroboration,
                    )
                },
                || self.services.catalog.titles.discovery_affinity_revision(),
            )
            .await?;
        Ok((*profile).clone())
    }

    async fn discovery_library_affinity_profile_uncached(
        &self,
        library_ids: &[String],
        run_id: Option<&str>,
        genre_rail_corroboration: bool,
    ) -> AppResult<DiscoveryLibraryAffinityProfile> {
        let title_ids = match run_id {
            Some(run_id) => self
                .services
                .library
                .discovery
                .list_discovery_submitted_title_ids(run_id, library_ids)
                .await?
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let started = Instant::now();
        let titles = self
            .services
            .catalog
            .titles
            .list_discovery_affinity_titles(library_ids, run_id, &title_ids)
            .await?;
        debug!(
            operation = "discovery_affinity",
            title_count = titles.len(),
            elapsed_ms = discovery_home_elapsed_ms(started),
            "discovery affinity evidence loaded"
        );
        Ok(discovery_library_affinity_profile_from_projection(
            &titles,
            genre_rail_corroboration,
        ))
    }

    /// Reads the four rail-quality rollback levers. They are unseeded, so an
    /// absent row is the default and a fresh install needs no data change.
    pub(crate) async fn discovery_rail_composition_settings(
        &self,
    ) -> DiscoveryRailCompositionSettings {
        let defaults = DiscoveryRailCompositionSettings::default();
        let rail_order = self
            .read_setting_string_value_for_scope(
                SETTINGS_SCOPE_SYSTEM,
                DISCOVERY_RAIL_ORDER_KEY,
                None,
            )
            .await
            .ok()
            .flatten()
            .filter(|value| {
                value
                    .trim()
                    .eq_ignore_ascii_case(DISCOVERY_RAIL_ORDER_LEGACY)
            })
            .map(|_| DiscoveryRailOrder::Legacy)
            .unwrap_or(defaults.rail_order);
        DiscoveryRailCompositionSettings {
            medium_affinity_gate: self
                .read_discovery_rail_switch(
                    DISCOVERY_MEDIUM_AFFINITY_GATE_KEY,
                    defaults.medium_affinity_gate,
                )
                .await,
            genre_rail_corroboration: self
                .read_discovery_rail_switch(
                    DISCOVERY_GENRE_RAIL_CORROBORATION_KEY,
                    defaults.genre_rail_corroboration,
                )
                .await,
            evidence_gate: self
                .read_discovery_rail_switch(
                    DISCOVERY_GENRE_RAIL_CREDIBILITY_GATE_KEY,
                    defaults.evidence_gate,
                )
                .await,
            rail_order,
        }
    }

    async fn read_discovery_rail_switch(&self, key: &str, default: bool) -> bool {
        self.read_setting_bool_value(key, None)
            .await
            .ok()
            .flatten()
            .unwrap_or(default)
    }
}

/// The library profile, computed from owned titles alone so the same numbers
/// drive the medium mix sent to SMG and the gate applied to what comes back.
#[cfg(test)]
pub(super) fn discovery_library_affinity_profile_from_titles(
    titles: &[Title],
    genre_rail_corroboration: bool,
) -> DiscoveryLibraryAffinityProfile {
    let titles = titles
        .iter()
        .map(|title| crate::DiscoveryAffinityTitle {
            id: title.id.clone(),
            facet: title.facet.clone(),
            canonical_tags: title.canonical_tags.clone(),
        })
        .collect::<Vec<_>>();
    discovery_library_affinity_profile_from_projection(&titles, genre_rail_corroboration)
}

fn discovery_library_affinity_profile_from_projection(
    titles: &[crate::DiscoveryAffinityTitle],
    genre_rail_corroboration: bool,
) -> DiscoveryLibraryAffinityProfile {
    let support_floor = discovery_rail_label_support_floor(titles.len());
    DiscoveryLibraryAffinityProfile {
        genre_labels: top_owned_title_labels(
            titles,
            DiscoveryRailLabelFamily::Genre,
            |title| {
                corroborated_canonical_genre_labels(&title.canonical_tags, genre_rail_corroboration)
            },
            DISCOVERY_RAIL_LADDER_LABELS,
            support_floor,
        ),
        // Theme affinity used to be read out of `title.tags`, which is the
        // one place a user's own tag vocabulary lives. Tags are private
        // catalog state and are never sent to SMG, so the theme rails are
        // sourced from canonical theme tags, exactly as the genre rails are
        // sourced from canonical genre tags.
        theme_labels: top_owned_title_labels(
            titles,
            DiscoveryRailLabelFamily::Theme,
            |title| qualified_owned_theme_labels(&title.canonical_tags),
            DISCOVERY_RAIL_LADDER_LABELS,
            support_floor,
        ),
        medium_mix: {
            let mut mix = DiscoveryLibraryMediumMix::default();
            for title in titles {
                mix.add(discovery_medium_from_genres(
                    &title.facet,
                    title
                        .canonical_tags
                        .iter()
                        .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
                        .map(|tag| tag.name.as_str()),
                ));
            }
            mix
        },
        owned_title_count: titles.len(),
    }
}

#[cfg(test)]
mod tests;
