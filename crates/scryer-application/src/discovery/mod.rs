use crate::library_scan::{
    DiscoveryContextChangeType, DiscoveryContextChangedSubjectInput, DiscoveryContextChangesInput,
    DiscoveryContextSnapshotPageResult, DiscoveryContextSnapshotSubmitInput,
    DiscoveryDashboardResult, DiscoveryDashboardSection, DiscoveryExternalIdInput,
    DiscoveryPublicFeedInput, DiscoverySubjectInput, DiscoveryTitle,
};
use crate::ports::{
    CatalogDiscoveryGroup, CatalogDiscoveryGroupKind, CatalogDiscoveryQuery,
    CatalogDiscoveryResult, CatalogDiscoverySectionCandidatesRecord, CatalogDiscoverySurface,
    CatalogOwnedTitleRecord, DISCOVERY_DEFAULT_SCOPE_KEY, DiscoveryExternalIdRecord,
    DiscoveryFacetRecord, DiscoveryHomeCandidate, DiscoveryHomeFilterOptions, DiscoveryHomeQuery,
    DiscoveryHomeResult, DiscoveryHomeSectionCandidatesRecord, DiscoveryItemDetailQuery,
    DiscoveryItemLibraryProvenanceRecord, DiscoveryItemRecord, DiscoveryItemsQuery,
    DiscoveryItemsResult, DiscoveryItemsStorageQuery, DiscoveryPendingContextChangeRecord,
    DiscoveryRankComponentRecord, DiscoverySectionItemsRecord, DiscoverySectionRecord,
    DiscoverySectionResult, DiscoverySourceTagRecord, DiscoverySubmittedSubjectRecord,
    DiscoverySyncStatus, TitleExternalIdLookup,
};
use crate::{AppError, AppResult, AppUseCase};
use chrono::{DateTime, Utc};
use scryer_domain::{
    CanonicalMediaTag, DomainEvent, DomainEventPayload, DomainExternalIds, ExternalId,
    LibraryPermission, MediaFacet, SeriesMovieLink, Title, TitleContextSnapshot, User,
    title_catalog_sort_input,
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
const DISCOVERY_COMPLETE_COLLECTION_MIN_CANDIDATES: usize = 100;
const DISCOVERY_COMPLETE_COLLECTION_MAX_CANDIDATES: usize = 500;
const CATALOG_ANIME_WEEKLY_SECTION_ID: &str = "anime_this_week";
const CATALOG_ANIME_SUPPRESSED_PUBLIC_SECTION_IDS: &[&str] = &["trending_now", "popular_series"];
const CATALOG_ANIME_PRIORITY_SECTION_IDS: &[&str] = &["new_on_streaming", "most_anticipated_anime"];
/// Minimum number of distinct rating providers required before an item's rating is
/// treated as corroborated. A lone source (for example a single trakt vote)
/// can spike to 10.0 without corroboration, so such fossils must lose to
/// corroborated evidence whenever ratings decide ordering.
const DISCOVERY_MIN_CREDIBLE_RATING_SOURCE_COUNT: usize = 2;
/// A single-provider rating still counts as credible when it is backed by at
/// least this many votes: an aggregator score built from thousands of votes
/// (a MAL-only anime, for instance) is stronger evidence than two thinly
/// sourced provider entries, and must not be demoted below them. Kept in
/// lockstep with SMG's display-evidence bar (CanonicalRatingDisplayMinVotes)
/// so a rating SMG displays is never ranked non-credible here for votes alone.
const DISCOVERY_CREDIBLE_RATING_MIN_VOTES: i32 = 25;

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
    subjects: &'a [CanonicalSubject],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoverySubjectParts {
    facet: String,
    subject_key: String,
    subject: DiscoverySubjectInput,
    canonical: CanonicalSubject,
}

mod catalog;
mod context;
mod home;
mod records;

use catalog::*;
use context::*;
use home::*;
use records::*;

pub(crate) use context::{
    build_discovery_library_context, coalesce_pending_context_change, incremental_item_records,
    pending_context_change_from_domain_event, public_feed_item_records,
    public_feed_section_records, snapshot_item_records,
};
pub(crate) use records::{
    pending_context_changes_need_snapshot_reconciliation,
    pending_context_changes_resolved_key_count, snapshot_facet_records,
    title_more_like_this_item_records, title_recommendations_subject,
};

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
        let source_title = self
            .get_title(actor, title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
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
        let readable_library_id_list = sorted_discovery_library_ids(&readable_library_ids);
        let candidate_limit = requested_limit.saturating_mul(4).clamp(24, 100) as i64;
        let mut items = self
            .services
            .library
            .discovery
            .list_title_more_like_this_items(title_id, candidate_limit)
            .await?;
        let mut item_lookup_indexes = vec![Vec::<usize>::new(); items.len()];
        let mut lookups = Vec::new();
        let mut series_movie_lookups = Vec::new();
        for item in &mut items {
            item.resolved_title_id = None;
            item.owned_in_input = false;
        }
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
        let mut filtered_items = Vec::with_capacity(requested_limit.min(items.len()));
        for (mut item, lookup_indexes) in items.into_iter().zip(item_lookup_indexes) {
            let readable_local_title = lookup_indexes.iter().find_map(|lookup_index| {
                matches_by_lookup_index
                    .get(lookup_index)
                    .and_then(|titles| {
                        titles.iter().find(|candidate| {
                            readable_library_ids.contains(candidate.library_id.as_str())
                        })
                    })
            });
            if readable_local_title.is_some()
                || lookup_indexes
                    .iter()
                    .any(|lookup_index| series_movie_owned_lookup_indexes.contains(lookup_index))
            {
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

    pub async fn discovery_home(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
    ) -> AppResult<DiscoveryHomeResult> {
        self.discovery_home_with_selected_card_hydration(actor, query, true)
            .await
    }

    pub async fn discovery_home_cards(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
    ) -> AppResult<DiscoveryHomeResult> {
        self.discovery_home_with_selected_card_hydration(actor, query, false)
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

    async fn discovery_home_with_selected_card_hydration(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
        hydrate_selected_cards: bool,
    ) -> AppResult<DiscoveryHomeResult> {
        let discovery_home_started_at = Instant::now();
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
        let limit = discovery_section_limit(query.limit_per_section);
        let include_unresolved = query.include_unresolved;
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let mut allowed_media_kinds = visibility
            .allowed_media_kinds
            .iter()
            .map(|media_kind| (*media_kind).to_string())
            .collect::<Vec<_>>();
        allowed_media_kinds.sort();
        let owned_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let owned_library_id_list = sorted_discovery_library_ids(&owned_library_ids);
        let owned_visibility = self
            .discovery_home_owned_visibility(&owned_library_id_list)
            .await?;
        let candidate_load_started_at = Instant::now();
        let mut candidates_by_id = HashMap::<String, DiscoveryHomeCandidate>::new();
        let mut home_submitted_subjects = Vec::new();
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
            let submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            let submitted_subjects =
                filter_submitted_subjects_for_libraries(&submitted_subjects, readable_library_ids);
            home_submitted_subjects = submitted_subjects.clone();
            resolve_discovery_matched_subjects(&mut personalized_items, &submitted_subjects)?;
            let library_profile = self
                .discovery_library_affinity_profile(readable_library_ids, &submitted_subjects)
                .await?;
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
            resolve_discovery_matched_subjects(
                &mut complete_collection_items,
                &submitted_subjects,
            )?;
            complete_collection =
                complete_collection_section(&complete_collection_items, include_unresolved, limit);
            personalized_sections = personalized_section_results(
                &personalized_items,
                &library_profile,
                include_unresolved,
                limit,
            );
            if hydrate_selected_cards {
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

        let hero_item = select_discovery_home_hero_with_candidates(
            &public_sections,
            &personalized_sections,
            &candidates_by_id,
        );
        let mut result = DiscoveryHomeResult {
            status,
            hero_item,
            public_sections,
            personalized_sections,
            complete_collection,
            facets,
            can_view_personalized,
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
        if hydrate_selected_cards {
            self.hydrate_discovery_home_result(&mut result, &candidates_by_id)
                .await?;
        } else {
            self.hydrate_discovery_home_hero(&mut result, &candidates_by_id)
                .await?;
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
            return Err(AppError::NotFound(format!("discovery home hero {hero_id}")));
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
            limit,
            offset,
        };
        let mut page = self
            .services
            .library
            .discovery
            .query_discovery_items(&storage_query)
            .await?;
        if let Some(context_run_id) = state.last_success_generation_id.as_deref() {
            let submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            let submitted_subjects =
                filter_submitted_subjects_for_libraries(&submitted_subjects, readable_library_ids);
            resolve_discovery_matched_subjects(&mut page.items, &submitted_subjects)?;
        }

        Ok(DiscoveryItemsResult {
            items: page.items,
            total_count: page.total_count,
            can_view_personalized,
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
            limit: 1,
            offset: 0,
        };
        let mut page = self
            .services
            .library
            .discovery
            .query_discovery_items(&storage_query)
            .await?;
        if page.items.is_empty() {
            return Ok(None);
        }
        if let Some(context_run_id) = context_run_id.as_deref() {
            let submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            let submitted_subjects =
                filter_submitted_subjects_for_libraries(&submitted_subjects, readable_library_ids);
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

        let readable_library_ids = self
            .authorized_library_ids(actor, Some(query.facet.clone()), LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requested_library_ids = query
            .library_ids
            .iter()
            .map(|library_id| library_id.trim())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();
        let effective_library_ids = if requested_library_ids.is_empty() {
            readable_library_ids.clone()
        } else {
            readable_library_ids
                .iter()
                .filter(|library_id| requested_library_ids.contains(library_id.as_str()))
                .cloned()
                .collect()
        };
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
        let owned_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let owned_library_id_list = sorted_discovery_library_ids(&owned_library_ids);
        let owned_visibility = self
            .owned_discovery_visibility(&owned_library_id_list)
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
            submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            submitted_subjects = filter_submitted_subjects_for_libraries(
                &submitted_subjects,
                &effective_library_ids,
            );
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
            let library_profile = self
                .discovery_library_affinity_profile(&effective_library_ids, &submitted_subjects)
                .await?;
            catalog_personalized_groups(
                &mut groups,
                &personalized_candidates,
                &library_profile,
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

        Ok(CatalogDiscoveryResult {
            groups,
            can_view_personalized,
        })
    }

    async fn discovery_visibility(&self, actor: &User) -> AppResult<DiscoveryVisibility> {
        let requestable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::Request)
            .await?;
        let manageable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::ManageTitles)
            .await?;
        let readable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?;

        let mut facets_by_library_id = self
            .services
            .catalog
            .libraries
            .list(None)
            .await?
            .into_iter()
            .map(|library| (library.id, library.facet))
            .collect::<HashMap<_, _>>();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            facets_by_library_id
                .entry(scryer_domain::default_library_id_for_facet(&facet))
                .or_insert(facet);
        }

        let discoverable_library_ids = requestable_library_ids
            .into_iter()
            .chain(manageable_library_ids)
            .collect::<HashSet<_>>();
        let mut visibility = DiscoveryVisibility::default();
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

    async fn owned_discovery_visibility(
        &self,
        readable_library_ids: &[String],
    ) -> AppResult<CatalogOwnedVisibility> {
        if readable_library_ids.is_empty() {
            return Ok(CatalogOwnedVisibility::default());
        }
        let titles = self
            .services
            .catalog
            .titles
            .list_catalog_owned_title_records(readable_library_ids)
            .await?;
        let title_ids = titles
            .iter()
            .map(|title| title.id.clone())
            .collect::<Vec<_>>();
        let series_movies = self
            .services
            .catalog
            .shows
            .list_series_movie_links_for_titles(&title_ids)
            .await?;
        Ok(CatalogOwnedVisibility::from_title_records_and_series_movies(&titles, &series_movies))
    }

    async fn discovery_home_owned_visibility(
        &self,
        readable_library_ids: &[String],
    ) -> AppResult<CatalogOwnedVisibility> {
        self.owned_discovery_visibility(readable_library_ids).await
    }
}

impl AppUseCase {
    async fn discovery_library_affinity_profile(
        &self,
        allowed_library_ids: &HashSet<String>,
        submitted_subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<DiscoveryLibraryAffinityProfile> {
        let mut library_ids = allowed_library_ids.iter().cloned().collect::<Vec<_>>();
        library_ids.sort();
        let mut titles = Vec::new();
        let mut seen_title_ids = HashSet::new();
        for title_id in submitted_subjects
            .iter()
            .filter_map(|subject| subject.title_id.as_deref())
            .filter(|title_id| seen_title_ids.insert((*title_id).to_string()))
        {
            if let Some(title) = self.services.catalog.titles.get_by_id(title_id).await?
                && allowed_library_ids.contains(&title.library_id)
            {
                titles.push(title);
            }
        }
        if titles.is_empty() {
            titles = self
                .services
                .catalog
                .titles
                .list_for_libraries(None, &library_ids, None)
                .await?;
        }
        Ok(DiscoveryLibraryAffinityProfile {
            genre_labels: top_owned_title_labels(
                &titles,
                |title| canonical_tag_labels(&title.canonical_tags, "genre"),
                2,
            ),
            // Theme affinity used to be read out of `title.tags`, which is the
            // one place a user's own tag vocabulary lives. Tags are private
            // catalog state and are never sent to SMG, so the theme rails are
            // sourced from canonical theme tags, exactly as the genre rails are
            // sourced from canonical genre tags.
            theme_labels: top_owned_title_labels(
                &titles,
                |title| canonical_tag_labels(&title.canonical_tags, "theme"),
                2,
            ),
        })
    }
}

#[cfg(test)]
mod tests;
