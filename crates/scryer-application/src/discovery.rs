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
        let can_view_personalized = !readable_library_ids.is_empty();
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
        let can_view_personalized = !readable_library_ids.is_empty();
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
        let can_view_personalized = !readable_library_ids.is_empty();
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
        let can_view_personalized = !readable_library_ids.is_empty();
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
        let can_view_personalized = !effective_library_ids.is_empty();
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
            tag_labels: top_owned_title_labels(&titles, |title| title.tags.iter(), 2),
        })
    }
}

fn discovery_section_limit(limit: usize) -> usize {
    if limit == 0 { 25 } else { limit.clamp(1, 100) }
}

fn public_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(1) * 4).clamp(section_limit, 100)
}

fn top_rated_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 80).clamp(DISCOVERY_HOME_MIN_CANDIDATES, DISCOVERY_HOME_MAX_CANDIDATES)
}

fn personalized_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 40).clamp(DISCOVERY_HOME_MIN_CANDIDATES, DISCOVERY_HOME_MAX_CANDIDATES)
}

fn complete_collection_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 8).clamp(
        DISCOVERY_COMPLETE_COLLECTION_MIN_CANDIDATES,
        DISCOVERY_COMPLETE_COLLECTION_MAX_CANDIDATES,
    )
}

fn discovery_items_limit(limit: usize) -> usize {
    if limit == 0 { 50 } else { limit.clamp(1, 200) }
}

fn sorted_discovery_library_ids(library_ids: &HashSet<String>) -> Vec<String> {
    let mut library_ids = library_ids.iter().cloned().collect::<Vec<_>>();
    library_ids.sort();
    library_ids
}

fn section_items_record_to_result(
    record: DiscoverySectionItemsRecord,
) -> Option<DiscoverySectionResult> {
    if record.items.is_empty() {
        return None;
    }
    Some(DiscoverySectionResult {
        section_id: record.section.section_id,
        section_type: record.section.section_type,
        title: record.section.title,
        surface: record.section.surface,
        total_count: record.total_count,
        items: record.items,
    })
}

fn home_section_candidates_record_to_result(
    record: DiscoveryHomeSectionCandidatesRecord,
    candidates_by_id: &mut HashMap<String, DiscoveryHomeCandidate>,
) -> Option<DiscoverySectionResult> {
    let items = home_selection_items_from_candidates(record.items, candidates_by_id);
    section_items_record_to_result(DiscoverySectionItemsRecord {
        section: record.section,
        total_count: record.total_count,
        items,
    })
}

fn home_selection_items_from_candidates(
    candidates: Vec<DiscoveryHomeCandidate>,
    candidates_by_id: &mut HashMap<String, DiscoveryHomeCandidate>,
) -> Vec<DiscoveryItemRecord> {
    candidates
        .into_iter()
        .map(|candidate| {
            let selection_item = home_candidate_selection_item(&candidate);
            candidates_by_id
                .entry(selection_item.id.clone())
                .or_insert(candidate);
            selection_item
        })
        .collect()
}

fn home_candidate_selection_item(candidate: &DiscoveryHomeCandidate) -> DiscoveryItemRecord {
    let mut item = candidate.item.clone();
    item.matched_subject_keys = candidate.matched_subject_keys.clone();
    item.facet_terms = candidate.affinity_terms.clone();
    item
}

fn selected_discovery_home_item_ids(result: &DiscoveryHomeResult) -> BTreeSet<String> {
    let mut item_ids = BTreeSet::new();
    for item in result
        .public_sections
        .iter()
        .chain(result.personalized_sections.iter())
        .flat_map(|section| section.items.iter())
        .chain(
            result
                .complete_collection
                .iter()
                .flat_map(|section| section.items.iter()),
        )
        .chain(result.hero_item.iter())
    {
        item_ids.insert(item.id.clone());
    }
    item_ids
}

fn discovery_home_subject_resolution_item_ids(result: &DiscoveryHomeResult) -> HashSet<String> {
    result
        .personalized_sections
        .iter()
        .flat_map(|section| section.items.iter())
        .chain(
            result
                .complete_collection
                .iter()
                .flat_map(|section| section.items.iter()),
        )
        .map(|item| item.id.clone())
        .collect()
}

fn resolve_discovery_home_selected_subjects(
    result: &mut DiscoveryHomeResult,
    submitted_subjects: &[DiscoverySubmittedSubjectRecord],
) -> AppResult<()> {
    let subject_resolution_ids = discovery_home_subject_resolution_item_ids(result);
    if subject_resolution_ids.is_empty() {
        return Ok(());
    }
    let mut items_by_id = HashMap::<String, DiscoveryItemRecord>::new();
    for item in result
        .personalized_sections
        .iter()
        .flat_map(|section| section.items.iter())
        .chain(
            result
                .complete_collection
                .iter()
                .flat_map(|section| section.items.iter()),
        )
    {
        items_by_id
            .entry(item.id.clone())
            .or_insert_with(|| item.clone());
    }
    let mut resolved_items = items_by_id.into_values().collect::<Vec<_>>();
    resolve_discovery_matched_subjects(&mut resolved_items, submitted_subjects)?;
    let resolved_by_id = resolved_items
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<HashMap<_, _>>();
    replace_discovery_home_result_items(result, &resolved_by_id, &subject_resolution_ids);
    // The resolved copies originate from section items, which on the card-only
    // path lack presentation fields; merge only what subject resolution
    // produced so the hero keeps its dedicated hydration.
    if let Some(hero_item) = &mut result.hero_item
        && let Some(resolved) = resolved_by_id.get(&hero_item.id)
    {
        hero_item.matched_subject_titles = resolved.matched_subject_titles.clone();
        hero_item.matched_subject_count = resolved.matched_subject_count;
    }
    Ok(())
}

fn replace_discovery_home_result_items(
    result: &mut DiscoveryHomeResult,
    hydrated_by_id: &HashMap<String, DiscoveryItemRecord>,
    subject_resolution_ids: &HashSet<String>,
) {
    for section in &mut result.public_sections {
        for item in &mut section.items {
            replace_discovery_home_item(item, hydrated_by_id, false);
        }
    }
    for section in &mut result.personalized_sections {
        for item in &mut section.items {
            replace_discovery_home_item(
                item,
                hydrated_by_id,
                subject_resolution_ids.contains(&item.id),
            );
        }
    }
    if let Some(section) = &mut result.complete_collection {
        for item in &mut section.items {
            replace_discovery_home_item(
                item,
                hydrated_by_id,
                subject_resolution_ids.contains(&item.id),
            );
        }
    }
    // The hero is deliberately NOT replaced here: on the card-only path the
    // section copies carry the lean candidate projection (NULL background_url/
    // overview), and a wholesale replace would discard the hero's dedicated
    // presentation hydration. Callers that want the hero swapped do it
    // explicitly (hydrate_discovery_home_result); subject resolution merges
    // only its own outputs into the hero (resolve_discovery_home_selected_subjects).
}

fn replace_discovery_home_item(
    item: &mut DiscoveryItemRecord,
    hydrated_by_id: &HashMap<String, DiscoveryItemRecord>,
    use_hydrated_subject_resolution: bool,
) {
    let Some(hydrated) = hydrated_by_id.get(&item.id) else {
        return;
    };
    let mut replacement = hydrated.clone();
    if !use_hydrated_subject_resolution {
        replacement.matched_subject_titles = item.matched_subject_titles.clone();
        replacement.matched_subject_count = item.matched_subject_count;
    }
    *item = replacement;
}

fn discovery_home_elapsed_ms(started_at: Instant) -> u128 {
    started_at.elapsed().as_millis()
}

fn filter_discovery_sections_for_owned_items(
    sections: Vec<DiscoverySectionResult>,
    owned_visibility: &CatalogOwnedVisibility,
    visibility: &DiscoveryVisibility,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    sections
        .into_iter()
        .filter_map(|mut section| {
            let original_len = section.items.len();
            section.items.retain(|item| {
                visibility.allows_item(item) && !owned_visibility.item_is_owned(item)
            });
            if section.items.len() > limit {
                section.items.truncate(limit);
            }
            if section.items.is_empty() {
                return None;
            }
            let removed_count = original_len.saturating_sub(section.items.len()) as i64;
            section.total_count = section
                .total_count
                .saturating_sub(removed_count)
                .max(section.items.len() as i64);
            Some(section)
        })
        .collect()
}

#[cfg(test)]
fn select_discovery_home_hero(
    public_sections: &[DiscoverySectionResult],
    personalized_sections: &[DiscoverySectionResult],
) -> Option<DiscoveryItemRecord> {
    select_discovery_home_hero_with_candidates(
        public_sections,
        personalized_sections,
        &HashMap::new(),
    )
}

fn select_discovery_home_hero_with_candidates(
    public_sections: &[DiscoverySectionResult],
    personalized_sections: &[DiscoverySectionResult],
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Option<DiscoveryItemRecord> {
    select_personalized_discovery_home_hero(personalized_sections, candidates_by_id).or_else(|| {
        select_public_discovery_home_hero_with_candidates(public_sections, candidates_by_id)
    })
}

#[cfg(test)]
fn top_rated_discovery_home_section(
    top_rated_items: &[DiscoveryItemRecord],
    live_public_sections: &[DiscoverySectionResult],
    include_unresolved: bool,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    top_rated_discovery_home_section_with_candidates(
        top_rated_items,
        live_public_sections,
        &HashMap::new(),
        include_unresolved,
        limit,
    )
}

fn top_rated_discovery_home_section_with_candidates(
    top_rated_items: &[DiscoveryItemRecord],
    live_public_sections: &[DiscoverySectionResult],
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
    include_unresolved: bool,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    let mut candidates = top_rated_items
        .iter()
        .chain(
            live_public_sections
                .iter()
                .flat_map(|section| section.items.iter()),
        )
        .filter(|item| !item.owned_in_input && home_item_visible(item, include_unresolved))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        compare_top_rated_discovery_home_items(left, right, candidates_by_id)
    });
    let mut seen = HashSet::new();
    candidates.retain(|item| seen.insert(discovery_item_identity_key(item).to_string()));
    section_result(
        "top_rated".to_string(),
        "TOP_RATED".to_string(),
        "Top Rated".to_string(),
        "mixed".to_string(),
        candidates,
        limit,
    )
}

fn discovery_home_item_is_personalized(item: &DiscoveryItemRecord) -> bool {
    !item
        .source_run_kind
        .trim()
        .eq_ignore_ascii_case("public_feed")
}

fn select_personalized_discovery_home_hero(
    sections: &[DiscoverySectionResult],
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Option<DiscoveryItemRecord> {
    let mut candidates = sections
        .iter()
        .flat_map(|section| section.items.iter())
        .filter(|item| discovery_home_item_is_personalized(item))
        .filter(|item| !item.owned_in_input)
        .filter(|item| discovery_home_item_has_hero_backdrop(item, candidates_by_id))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        compare_personalized_discovery_home_hero_items(left, right, candidates_by_id)
    });
    candidates.into_iter().next()
}

#[cfg(test)]
fn select_public_discovery_home_hero(
    sections: &[DiscoverySectionResult],
) -> Option<DiscoveryItemRecord> {
    select_public_discovery_home_hero_with_candidates(sections, &HashMap::new())
}

fn select_public_discovery_home_hero_with_candidates(
    sections: &[DiscoverySectionResult],
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Option<DiscoveryItemRecord> {
    let mut candidates = sections
        .iter()
        .flat_map(|section| section.items.iter())
        .filter(|item| discovery_home_item_has_hero_backdrop(item, candidates_by_id))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        compare_public_discovery_home_hero_items(left, right, candidates_by_id)
    });
    candidates.into_iter().next()
}

fn compare_personalized_discovery_home_hero_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Ordering {
    discovery_home_item_has_hero_backdrop(right, candidates_by_id)
        .cmp(&discovery_home_item_has_hero_backdrop(
            left,
            candidates_by_id,
        ))
        .then_with(|| right.matched_subject_count.cmp(&left.matched_subject_count))
        .then_with(|| compare_optional_f64_desc(left.rank_score, right.rank_score))
        .then_with(|| compare_discovery_item_rating_desc(left, right))
        .then_with(|| {
            right
                .source_count
                .unwrap_or_default()
                .cmp(&left.source_count.unwrap_or_default())
        })
        .then_with(|| left.target_key.cmp(&right.target_key))
}

fn compare_public_discovery_home_hero_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Ordering {
    discovery_home_item_has_hero_backdrop(right, candidates_by_id)
        .cmp(&discovery_home_item_has_hero_backdrop(
            left,
            candidates_by_id,
        ))
        .then_with(|| compare_optional_f64_desc(left.rank_score, right.rank_score))
        .then_with(|| {
            discovery_home_item_has_credible_rating_evidence(right, candidates_by_id).cmp(
                &discovery_home_item_has_credible_rating_evidence(left, candidates_by_id),
            )
        })
        .then_with(|| compare_discovery_item_rating_desc(left, right))
        .then_with(|| {
            right
                .source_count
                .unwrap_or_default()
                .cmp(&left.source_count.unwrap_or_default())
        })
        .then_with(|| left.target_key.cmp(&right.target_key))
}

fn discovery_item_has_hero_backdrop(item: &DiscoveryItemRecord) -> bool {
    item.background_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
}

fn discovery_home_item_has_hero_backdrop(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> bool {
    candidates_by_id
        .get(&item.id)
        .map(|candidate| candidate.has_hero_backdrop)
        .unwrap_or_else(|| discovery_item_has_hero_backdrop(item))
}

fn compare_discovery_item_rating_desc(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> Ordering {
    discovery_item_comparable_rating(right).total_cmp(&discovery_item_comparable_rating(left))
}

/// Collapse a raw rating-source string to a provider identity so aliases and
/// per-provider sub-metrics ("mal" vs "MyAnimeList.net", RT critic vs audience)
/// cannot inflate the distinct-provider count. Mirrors the web app's
/// `normalizedRatingSource` alias handling in `lib/utils/title-ratings.ts`.
fn canonical_rating_source_identity(source: &str) -> String {
    let normalized: String = source
        .trim()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "rottentomatoes" | "audience" | "popcorn" | "popcornmeter" => "tomatoes".to_string(),
        "mcuser" | "metacriticuser" => "metacritic".to_string(),
        "themoviedb" => "tmdb".to_string(),
        "thetvdb" => "tvdb".to_string(),
        "myanimelist" | "myanimelistnet" => "mal".to_string(),
        _ => normalized,
    }
}

/// Number of distinct rating providers backing an item's rating, drawn from the
/// persisted rating-source list and the per-source external ratings. Both are
/// hydrated onto the item from `discovery_title_metadata_rating_summaries` /
/// `_rating_sources` / `_external_ratings`, so no schema change is needed to
/// reach this signal.
fn discovery_item_distinct_rating_source_count(item: &DiscoveryItemRecord) -> usize {
    let mut seen = HashSet::new();
    for source in item
        .rating_sources
        .iter()
        .chain(item.external_ratings.iter().map(|rating| &rating.source))
    {
        let identity = canonical_rating_source_identity(source);
        if !identity.is_empty() {
            seen.insert(identity);
        }
    }
    seen.len()
}

/// Credible rating evidence means the item actually carries a rating signal AND
/// that signal is corroborated: either multiple distinct providers agree it is
/// rated, or a single provider's score is backed by a meaningful vote count.
/// A bare list of source names with no score, or a lone low-vote score, is not
/// credible.
fn discovery_item_has_credible_rating_evidence(item: &DiscoveryItemRecord) -> bool {
    let has_rating_signal = discovery_item_comparable_rating(item) > 0.0
        || discovery_item_best_external_rating_score(item).is_some();
    if !has_rating_signal {
        return false;
    }
    discovery_item_distinct_rating_source_count(item) >= DISCOVERY_MIN_CREDIBLE_RATING_SOURCE_COUNT
        || discovery_item_external_rating_vote_count(item) >= DISCOVERY_CREDIBLE_RATING_MIN_VOTES
}

fn compare_top_rated_discovery_home_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Ordering {
    let left_external_rating =
        discovery_home_item_best_external_rating_score(left, candidates_by_id);
    let right_external_rating =
        discovery_home_item_best_external_rating_score(right, candidates_by_id);
    discovery_home_item_has_credible_rating_evidence(right, candidates_by_id)
        .cmp(&discovery_home_item_has_credible_rating_evidence(
            left,
            candidates_by_id,
        ))
        .then_with(|| {
            right_external_rating
                .is_some()
                .cmp(&left_external_rating.is_some())
        })
        .then_with(|| compare_optional_f64_desc(left_external_rating, right_external_rating))
        .then_with(|| {
            discovery_home_item_external_rating_vote_count(right, candidates_by_id).cmp(
                &discovery_home_item_external_rating_vote_count(left, candidates_by_id),
            )
        })
        .then_with(|| compare_discovery_item_rating_desc(left, right))
        .then_with(|| compare_optional_f64_desc(left.rank_score, right.rank_score))
        .then_with(|| {
            right
                .source_count
                .unwrap_or_default()
                .cmp(&left.source_count.unwrap_or_default())
        })
        .then_with(|| discovery_item_identity_key(left).cmp(discovery_item_identity_key(right)))
}

fn discovery_home_item_has_credible_rating_evidence(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> bool {
    let Some(candidate) = candidates_by_id.get(&item.id) else {
        return discovery_item_has_credible_rating_evidence(item);
    };
    let has_rating_signal =
        discovery_item_comparable_rating(item) > 0.0 || candidate.best_external_rating.is_some();
    has_rating_signal
        && (candidate.rating_source_count as usize >= DISCOVERY_MIN_CREDIBLE_RATING_SOURCE_COUNT
            || candidate.best_external_rating_votes >= DISCOVERY_CREDIBLE_RATING_MIN_VOTES)
}

fn discovery_home_item_best_external_rating_score(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Option<f64> {
    candidates_by_id
        .get(&item.id)
        .and_then(|candidate| candidate.best_external_rating)
        .or_else(|| discovery_item_best_external_rating_score(item))
}

fn discovery_home_item_external_rating_vote_count(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> i32 {
    candidates_by_id
        .get(&item.id)
        .map(|candidate| candidate.best_external_rating_votes)
        .unwrap_or_else(|| discovery_item_external_rating_vote_count(item))
}

fn discovery_item_best_external_rating_score(item: &DiscoveryItemRecord) -> Option<f64> {
    item.external_ratings
        .iter()
        .filter_map(|rating| {
            let normalized = rating.normalized;
            normalized.is_finite().then_some(if normalized <= 1.0 {
                normalized * 10.0
            } else {
                normalized
            })
        })
        .filter(|rating| *rating > 0.0)
        .max_by(f64::total_cmp)
}

fn discovery_item_external_rating_vote_count(item: &DiscoveryItemRecord) -> i32 {
    item.external_ratings
        .iter()
        .filter_map(|rating| rating.votes)
        .max()
        .unwrap_or_default()
}

// Missing and non-finite values both collapse to 0.0 so the comparator stays a
// total order (a NaN score must never make sort_by panic or misorder).
fn comparable_finite_f64(value: Option<f64>) -> f64 {
    value
        .filter(|candidate| candidate.is_finite())
        .unwrap_or_default()
}

fn compare_optional_f64_desc(left: Option<f64>, right: Option<f64>) -> Ordering {
    comparable_finite_f64(right).total_cmp(&comparable_finite_f64(left))
}

fn personalized_section_results(
    items: &[DiscoveryItemRecord],
    library_profile: &DiscoveryLibraryAffinityProfile,
    include_unresolved: bool,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    let visible_items = items
        .iter()
        .filter(|item| home_item_visible(item, include_unresolved))
        .cloned()
        .collect::<Vec<_>>();
    let mut sections = Vec::new();
    let mut emitted_item_keys = HashSet::new();

    // Composition order is dedupe priority: `emitted_item_keys` lets an earlier
    // section claim a title outright, so the most specific reason must compose
    // first. Tag/theme rails lead the ladder - a title that earns "Because You
    // Like Isekai" must not be eaten first by the far broader "Because You Like
    // Animation" genre rail.
    sections.extend(label_affinity_sections(
        &visible_items,
        &library_profile.tag_labels,
        "theme",
        "BECAUSE_YOU_LIKE_TAG",
        "because_you_like_tag",
        limit,
        &mut emitted_item_keys,
    ));
    sections.extend(label_affinity_sections(
        &visible_items,
        &library_profile.genre_labels,
        "genre",
        "BECAUSE_YOU_LIKE_GENRE",
        "because_you_like_genre",
        limit,
        &mut emitted_item_keys,
    ));

    // FOR_YOU is the single generic personalized rail and composes last so it
    // only sweeps up whatever no reason-based rail claimed. Medium is owned by
    // the dashboard's facet chips, so it must never become a rail identity: the
    // MOVIES_FOR_YOU / SERIES_FOR_YOU / ANIME_FOR_YOU variants are retired. So is
    // BECAUSE_YOU_HAVE, which duplicated the title page's More Like This shelf.
    let mut for_you_items = visible_items;
    dedupe_and_sort_discovery_items(&mut for_you_items);
    sections.extend(section_result_excluding_emitted(
        "for_you".to_string(),
        "FOR_YOU".to_string(),
        "For You".to_string(),
        "personalized".to_string(),
        for_you_items,
        limit,
        &mut emitted_item_keys,
    ));

    sections
}

fn canonical_affinity_labels_for_profile(
    items: &[DiscoveryItemRecord],
    profile_labels: &[String],
    canonical_kind: &str,
) -> Vec<String> {
    let mut canonical_labels_by_key = HashMap::new();
    for item in items {
        for label in discovery_item_canonical_facet_labels(item, canonical_kind) {
            let key = normalize_discovery_affinity_key(&label);
            if !key.is_empty() {
                canonical_labels_by_key.entry(key).or_insert(label);
            }
        }
    }

    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    for profile_label in profile_labels {
        let key = normalize_discovery_affinity_key(profile_label);
        if let Some(label) = canonical_labels_by_key.get(&key) {
            push_unique_discovery_label(&mut labels, &mut seen, label.clone());
        }
    }
    labels
}

fn label_affinity_sections(
    items: &[DiscoveryItemRecord],
    labels: &[String],
    canonical_kind: &str,
    section_type: &str,
    section_id_prefix: &str,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Vec<DiscoverySectionResult> {
    let mut sections = Vec::new();
    for label in canonical_affinity_labels_for_profile(items, labels, canonical_kind) {
        let mut section_items = items
            .iter()
            .filter(|item| {
                item.matched_subject_count > 0
                    && discovery_item_matches_affinity_label(item, &label, canonical_kind)
                    && affinity_label_keeps_item_across_anime_boundary(item, &label)
            })
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(section) = section_result_excluding_emitted(
            format!(
                "{}_{}",
                section_id_prefix,
                slugify_discovery_section_part(&label)
            ),
            section_type.to_string(),
            format!("Because You Like {}", label),
            "personalized".to_string(),
            section_items,
            limit,
            emitted_item_keys,
        ) {
            sections.push(section);
        }
    }
    sections
}

/// Animation is a medium; anime is a tradition. Metadata sources tag anime with
/// the same canonical `animation` genre facet as Western animation, so an
/// unguarded "Because You Like Animation" rail comingles Zootopia with Frieren.
/// This is a deliberate two-label special case at the point where items are
/// matched to a label, not a general taxonomy: the animation rail drops anime
/// items, an anime rail keeps only anime items, and every other label is
/// untouched.
///
/// The guard is keyed on the label name and is **kind-agnostic by design**: a
/// theme/tag rail named "Anime" or "Animation" carries exactly the same meaning
/// its genre namesake does, so it has to split the same way.
fn affinity_label_keeps_item_across_anime_boundary(
    item: &DiscoveryItemRecord,
    label: &str,
) -> bool {
    match normalize_discovery_affinity_key(label).as_str() {
        "animation" => !discovery_item_is_anime(item),
        "anime" => discovery_item_is_anime(item),
        _ => true,
    }
}

/// Media kind is the primary anime signal, but `discovery_item_media_kind` falls
/// back to `target_kind` when `content_type` is missing, which reports an anime
/// as a plain `series`. The canonical `anime` genre facet is the second witness
/// that stops such an item leaking into the animation rail, mirroring how the
/// metadata gateway classifies an anime-tagged title.
fn discovery_item_is_anime(item: &DiscoveryItemRecord) -> bool {
    discovery_item_media_kind(item) == Some("anime")
        || discovery_item_canonical_facet_labels(item, "genre")
            .iter()
            .any(|label| normalize_discovery_affinity_key(label) == "anime")
}

#[derive(Clone, Debug, Default)]
struct DiscoveryLibraryAffinityProfile {
    genre_labels: Vec<String>,
    tag_labels: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct CatalogOwnedVisibility {
    title_ids: HashSet<String>,
    keys: HashSet<String>,
    identity_keys: HashSet<String>,
}

impl CatalogOwnedVisibility {
    #[cfg(test)]
    fn from_titles(titles: &[Title]) -> Self {
        let mut visibility = Self::default();
        for title in titles {
            visibility.title_ids.insert(title.id.clone());
            add_catalog_owned_external_keys(
                &mut visibility.keys,
                &mut visibility.identity_keys,
                "imdb",
                title.imdb_id.as_deref(),
                title.facet.clone(),
            );
            for external_id in &title.external_ids {
                add_catalog_owned_external_keys(
                    &mut visibility.keys,
                    &mut visibility.identity_keys,
                    &external_id.source,
                    Some(external_id.value.as_str()),
                    title.facet.clone(),
                );
            }
        }
        visibility
    }

    fn from_title_records(titles: &[CatalogOwnedTitleRecord]) -> Self {
        let mut visibility = Self::default();
        for title in titles {
            visibility.title_ids.insert(title.id.clone());
            add_catalog_owned_external_keys(
                &mut visibility.keys,
                &mut visibility.identity_keys,
                "imdb",
                title.imdb_id.as_deref(),
                title.facet.clone(),
            );
            for external_id in &title.external_ids {
                add_catalog_owned_external_keys(
                    &mut visibility.keys,
                    &mut visibility.identity_keys,
                    &external_id.source,
                    Some(external_id.value.as_str()),
                    title.facet.clone(),
                );
            }
        }
        visibility
    }

    fn from_title_records_and_series_movies(
        titles: &[CatalogOwnedTitleRecord],
        series_movies: &[SeriesMovieLink],
    ) -> Self {
        let mut visibility = Self::from_title_records(titles);
        for series_movie in series_movies {
            for (source, value) in [
                ("imdb", series_movie.movie.imdb_id.as_deref()),
                ("tvdb", series_movie.movie.tvdb_id.as_deref()),
                ("tmdb", series_movie.movie.tmdb_id.as_deref()),
                ("mal", series_movie.movie.mal_id.as_deref()),
                ("anidb", series_movie.movie.anidb_id.as_deref()),
            ] {
                add_catalog_owned_external_keys(
                    &mut visibility.keys,
                    &mut visibility.identity_keys,
                    source,
                    value,
                    MediaFacet::Movie,
                );
            }
        }
        visibility
    }

    fn excluded_discovery_identity_keys(&self) -> Vec<String> {
        let mut keys = self.identity_keys.iter().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    fn item_is_owned(&self, item: &DiscoveryItemRecord) -> bool {
        if item.owned_in_input {
            return true;
        }
        if item
            .resolved_title_id
            .as_deref()
            .is_some_and(|title_id| self.title_ids.contains(title_id))
        {
            return true;
        }
        discovery_item_ownership_keys(item)
            .into_iter()
            .any(|key| self.keys.contains(&key))
    }
}

fn add_catalog_owned_external_keys(
    keys: &mut HashSet<String>,
    identity_keys: &mut HashSet<String>,
    source: &str,
    value: Option<&str>,
    facet: MediaFacet,
) {
    let raw_source = normalize_catalog_owned_key(source);
    let raw_value = normalize_catalog_owned_key(value.unwrap_or_default());
    if raw_source.is_empty() || raw_value.is_empty() {
        return;
    }

    let mut source_aliases = HashSet::from([raw_source]);
    if let Some(canonical_source) = normalize_supported_external_id_source(source) {
        source_aliases.insert(canonical_source);
    }
    let mut value_aliases = HashSet::from([raw_value]);
    if let Some(canonical_value) = parse_positive_external_numeric_id(value.unwrap_or_default()) {
        value_aliases.insert(canonical_value.to_string());
    }

    for source in source_aliases {
        for value in &value_aliases {
            insert_catalog_owned_key(keys, identity_keys, &source, value);
            insert_catalog_owned_key(
                keys,
                identity_keys,
                &source,
                &format!("{}:{value}", facet.as_str()),
            );
            if facet == MediaFacet::Anime {
                insert_catalog_owned_key(keys, identity_keys, &source, &format!("series:{value}"));
                insert_catalog_owned_key(keys, identity_keys, &source, &format!("anime:{value}"));
            }
        }
    }
}

fn insert_catalog_owned_key(
    keys: &mut HashSet<String>,
    identity_keys: &mut HashSet<String>,
    source: &str,
    value: &str,
) {
    let key = format!("{source}:{value}");
    keys.insert(key.clone());
    identity_keys.insert(key);
}

fn discovery_item_ownership_keys(item: &DiscoveryItemRecord) -> HashSet<String> {
    let mut keys = HashSet::new();
    let target_key = normalize_catalog_owned_key(&item.target_key);
    if !target_key.is_empty() {
        keys.insert(target_key);
    }
    let target_parts = item
        .target_key
        .split(':')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if target_parts.len() >= 3 {
        let source = normalize_catalog_owned_key(target_parts[0]);
        let value = normalize_catalog_owned_key(&target_parts[2..].join(":"));
        if !source.is_empty() && !value.is_empty() {
            keys.insert(format!("{source}:{value}"));
        }
    }
    keys
}

fn normalize_catalog_owned_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn discovery_media_kind_for_facet(facet: MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Series => "series",
        MediaFacet::Anime => "anime",
    }
}

fn catalog_discovery_group_limit(limit: usize) -> usize {
    if limit == 0 { 12 } else { limit.clamp(1, 12) }
}

fn catalog_discovery_max_groups(max_groups: usize) -> usize {
    if max_groups == 0 {
        6
    } else {
        max_groups.clamp(1, 10)
    }
}

fn catalog_discovery_candidate_limit(limit: usize, max_groups: usize) -> usize {
    (limit.max(6) * max_groups.max(4) * 8).clamp(48, 400)
}

fn catalog_filter_anime_public_sections(
    public_sections: &mut Vec<CatalogDiscoverySectionCandidatesRecord>,
) {
    public_sections.retain(|section| {
        !CATALOG_ANIME_SUPPRESSED_PUBLIC_SECTION_IDS.contains(&section.section_id.as_str())
    });
}

fn catalog_take_public_section(
    public_sections: &mut Vec<CatalogDiscoverySectionCandidatesRecord>,
    section_id: &str,
) -> Option<CatalogDiscoverySectionCandidatesRecord> {
    public_sections
        .iter()
        .position(|section| section.section_id == section_id)
        .map(|index| public_sections.remove(index))
}

fn catalog_public_top_section(
    public_sections: &mut Vec<CatalogDiscoverySectionCandidatesRecord>,
    media_kind: &str,
) -> Option<CatalogDiscoverySectionCandidatesRecord> {
    if media_kind == "anime"
        && let Some(index) = public_sections
            .iter()
            .position(|section| section.section_id == CATALOG_ANIME_WEEKLY_SECTION_ID)
    {
        return Some(public_sections.remove(index));
    }

    if public_sections.is_empty() {
        None
    } else {
        Some(public_sections.remove(0))
    }
}

fn catalog_public_top_group(
    section: CatalogDiscoverySectionCandidatesRecord,
    media_kind: &str,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<CatalogDiscoveryGroup> {
    let label_value =
        if media_kind == "anime" && section.section_id == CATALOG_ANIME_WEEKLY_SECTION_ID {
            Some("Trending Now".to_string())
        } else {
            catalog_public_section_label(&section)
        };
    catalog_group_excluding_emitted(
        CatalogDiscoveryGroupDraft {
            id: format!("public_top_{media_kind}"),
            kind: CatalogDiscoveryGroupKind::PublicTop,
            surface: CatalogDiscoverySurface::Public,
            label_value,
            total_count: Some(section.total_count),
        },
        section.items,
        limit,
        emitted_item_keys,
    )
}

fn catalog_public_section_group(
    section: CatalogDiscoverySectionCandidatesRecord,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<CatalogDiscoveryGroup> {
    let id = format!(
        "public_section_{}",
        normalized_catalog_group_id(&section.section_id)
    );
    let label_value = catalog_public_section_label(&section);
    catalog_group_excluding_emitted(
        CatalogDiscoveryGroupDraft {
            id,
            kind: CatalogDiscoveryGroupKind::PublicSection,
            surface: CatalogDiscoverySurface::Public,
            label_value,
            total_count: Some(section.total_count),
        },
        section.items,
        limit,
        emitted_item_keys,
    )
}

fn catalog_public_section_label(
    section: &CatalogDiscoverySectionCandidatesRecord,
) -> Option<String> {
    if section.section_id == "evergreen_popular" {
        // Scryer is an unbiased entry point, so no rail may pitch a provider.
        // The gateway's own title for this section is being renamed in parallel;
        // this override exists so the provider name cannot resurrect here.
        Some("All-Time Favorites".to_string())
    } else {
        section
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| {
                (!section.section_type.trim().is_empty()).then(|| section.section_type.clone())
            })
    }
}

fn normalized_catalog_group_id(value: &str) -> String {
    let normalized = normalize_discovery_affinity_key(value).replace(' ', "_");
    if normalized.is_empty() {
        "section".to_string()
    } else {
        normalized
    }
}

fn catalog_personalized_groups(
    groups: &mut Vec<CatalogDiscoveryGroup>,
    items: &[DiscoveryItemRecord],
    library_profile: &DiscoveryLibraryAffinityProfile,
    limit: usize,
    max_groups: usize,
    emitted_item_keys: &mut HashSet<String>,
) {
    let personalized_group_start = groups.len();

    // Same specificity ladder and same anime/animation boundary as the home
    // composer (`personalized_section_results`): theme/tag groups claim titles
    // before the broader genre groups, and neither may comingle Western animation
    // with anime. The two surfaces must not disagree about what a rail means.
    for label in canonical_affinity_labels_for_profile(items, &library_profile.tag_labels, "theme")
    {
        if groups.len() >= max_groups {
            return;
        }
        let mut section_items = items
            .iter()
            .filter(|item| {
                item.matched_subject_count > 0
                    && discovery_item_matches_affinity_label(item, &label, "theme")
                    && affinity_label_keeps_item_across_anime_boundary(item, &label)
            })
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: format!("theme_{}", slugify_discovery_section_part(&label)),
                kind: CatalogDiscoveryGroupKind::ThemeAffinity,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: Some(label),
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }

    for label in
        canonical_affinity_labels_for_profile(items, &library_profile.genre_labels, "genre")
    {
        if groups.len() >= max_groups {
            return;
        }
        let mut section_items = items
            .iter()
            .filter(|item| {
                item.matched_subject_count > 0
                    && discovery_item_matches_affinity_label(item, &label, "genre")
                    && affinity_label_keeps_item_across_anime_boundary(item, &label)
            })
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: format!("genre_{}", slugify_discovery_section_part(&label)),
                kind: CatalogDiscoveryGroupKind::GenreAffinity,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: Some(label),
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }

    if groups.len() < max_groups {
        let mut section_items = items
            .iter()
            .filter(|item| discovery_item_has_collection_signal(item))
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: "complete_the_collection".to_string(),
                kind: CatalogDiscoveryGroupKind::CompleteCollection,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: None,
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }

    if groups.len() == personalized_group_start && groups.len() < max_groups {
        let mut section_items = items.to_vec();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: "fallback".to_string(),
                kind: CatalogDiscoveryGroupKind::Fallback,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: None,
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }
}

struct CatalogDiscoveryGroupDraft {
    id: String,
    kind: CatalogDiscoveryGroupKind,
    surface: CatalogDiscoverySurface,
    label_value: Option<String>,
    total_count: Option<i64>,
}

fn catalog_group_excluding_emitted(
    draft: CatalogDiscoveryGroupDraft,
    items: Vec<DiscoveryItemRecord>,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<CatalogDiscoveryGroup> {
    let mut available = Vec::new();
    for item in items {
        let key = discovery_item_identity_key(&item).to_string();
        if emitted_item_keys.contains(&key) {
            continue;
        }
        available.push((key, item));
    }
    if available.is_empty() {
        return None;
    }

    let available_count = available.len() as i64;
    let mut items = Vec::new();
    for (key, item) in available.into_iter().take(limit) {
        emitted_item_keys.insert(key);
        items.push(item);
    }
    Some(CatalogDiscoveryGroup {
        id: draft.id,
        kind: draft.kind,
        surface: draft.surface,
        label_value: draft.label_value,
        total_count: draft.total_count.unwrap_or(available_count),
        items,
    })
}

fn discovery_item_canonical_facet_labels(item: &DiscoveryItemRecord, kind: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    for label in item
        .facet_terms
        .iter()
        .filter_map(|term| canonical_discovery_facet_label(term, kind))
    {
        push_unique_discovery_label(&mut labels, &mut seen, label);
    }
    labels
}

fn discovery_item_matches_affinity_label(
    item: &DiscoveryItemRecord,
    label: &str,
    canonical_kind: &str,
) -> bool {
    let label_key = normalize_discovery_affinity_key(label);
    if label_key.is_empty() {
        return false;
    }
    discovery_item_canonical_facet_labels(item, canonical_kind)
        .into_iter()
        .any(|candidate| affinity_value_matches_label(&candidate, &label_key))
}

fn affinity_value_matches_label(value: &str, label_key: &str) -> bool {
    let value_key = normalize_discovery_affinity_key(value);
    value_key == label_key
}

#[cfg(test)]
fn discovery_item_matches_canonical_facet_filters(
    item: &DiscoveryItemRecord,
    kind: &str,
    filters: &[String],
) -> bool {
    let mut filter_keys = filters
        .iter()
        .map(|filter| normalize_discovery_filter_value(filter))
        .filter(|filter| !filter.is_empty())
        .collect::<HashSet<_>>();
    if filter_keys.is_empty() {
        return true;
    }
    item.facet_terms.iter().any(|term| {
        let Some(label) = canonical_discovery_facet_label(term, kind) else {
            return false;
        };
        filter_keys.contains(&normalize_discovery_filter_value(term))
            || filter_keys.remove(&normalize_discovery_filter_value(&label))
    })
}

fn push_unique_discovery_label(
    labels: &mut Vec<String>,
    seen: &mut HashSet<String>,
    label: String,
) {
    let label = label.trim();
    if label.is_empty() {
        return;
    }
    let key = normalize_discovery_filter_value(label);
    if seen.insert(key) {
        labels.push(label.to_string());
    }
}

fn canonical_tag_labels(tags: &[CanonicalMediaTag], category: &str) -> Vec<String> {
    tags.iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case(category))
        .map(|tag| tag.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

fn top_owned_title_labels<'a, F, I>(
    titles: &'a [Title],
    labels_for_title: F,
    limit: usize,
) -> Vec<String>
where
    F: Fn(&'a Title) -> I,
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut counts = HashMap::<String, (String, usize)>::new();
    for title in titles {
        let mut seen_for_title = HashSet::new();
        for raw_label in labels_for_title(title) {
            let raw_label = raw_label.as_ref();
            let label_key = normalize_discovery_affinity_key(raw_label);
            if label_key.is_empty()
                || discovery_affinity_label_is_generic(&label_key)
                || raw_label.trim_start().starts_with("scryer:")
                || !seen_for_title.insert(label_key.clone())
            {
                continue;
            }
            let label = display_discovery_affinity_label(raw_label);
            let entry = counts.entry(label_key).or_insert((label, 0));
            entry.1 += 1;
        }
    }

    let mut counts = counts.into_values().collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    counts
        .into_iter()
        .take(limit)
        .map(|(label, _)| label)
        .collect()
}

fn display_discovery_affinity_label(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed
        .chars()
        .any(|character| character.is_ascii_uppercase())
    {
        return trimmed.to_string();
    }

    trimmed
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn discovery_item_comparable_rating(item: &DiscoveryItemRecord) -> f64 {
    item.rating
        .filter(|rating| rating.is_finite())
        .map(|rating| if rating <= 1.0 { rating * 10.0 } else { rating })
        .unwrap_or_default()
}

fn discovery_affinity_label_is_generic(normalized: &str) -> bool {
    matches!(
        normalized,
        "movie"
            | "movies"
            | "series"
            | "show"
            | "shows"
            // "anime" is deliberately absent: it names a tradition, not a medium
            // noun, so "Because You Like Anime" is a legitimate rail and the
            // anime/animation boundary guard depends on the label being reachable.
            | "recommendation"
            | "recommendations"
            | "similar"
            | "relation"
            | "list"
            | "community"
            | "tmdb"
            | "tvdb"
            | "mal"
            | "anilist"
            | "myanimelist"
    )
}

fn normalize_discovery_affinity_key(value: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_separator = false;
    for character in value.trim().chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !normalized.is_empty() {
            normalized.push(' ');
            last_was_separator = true;
        }
    }
    while normalized.ends_with(' ') {
        normalized.pop();
    }
    normalized
}

fn slugify_discovery_section_part(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('_');
            last_was_separator = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

fn complete_collection_section(
    items: &[DiscoveryItemRecord],
    include_unresolved: bool,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    let mut items = items
        .iter()
        .filter(|item| {
            discovery_item_media_kind(item) == Some("movie")
                && !item.owned_in_input
                && (include_unresolved || item.resolved)
                && discovery_item_has_collection_signal(item)
        })
        .cloned()
        .collect::<Vec<_>>();
    dedupe_and_sort_discovery_items(&mut items);
    section_result(
        "complete_the_collection".to_string(),
        "COMPLETE_THE_COLLECTION".to_string(),
        "Complete the Collection".to_string(),
        "personalized".to_string(),
        items,
        limit,
    )
}

fn discovery_item_has_collection_signal(item: &DiscoveryItemRecord) -> bool {
    if item.tmdb_collection_id.is_some()
        || item
            .tmdb_collection_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
    {
        return true;
    }

    item.relation_types
        .iter()
        .chain(item.relation_subtypes.iter())
        .any(|value| {
            let value = value.trim().to_ascii_lowercase();
            value == "tmdb.collection"
                || value.contains("collection")
                || value.contains("franchise")
        })
}

fn section_result(
    section_id: String,
    section_type: String,
    title: String,
    surface: String,
    items: Vec<DiscoveryItemRecord>,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    if items.is_empty() {
        return None;
    }
    let total_count = items.len() as i64;
    let items = items.into_iter().take(limit).collect();
    Some(DiscoverySectionResult {
        section_id,
        section_type,
        title,
        surface,
        total_count,
        items,
    })
}

fn section_result_excluding_emitted(
    section_id: String,
    section_type: String,
    title: String,
    surface: String,
    items: Vec<DiscoveryItemRecord>,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<DiscoverySectionResult> {
    let mut available = Vec::new();
    for item in items {
        let key = discovery_item_identity_key(&item).to_string();
        if emitted_item_keys.contains(&key) {
            continue;
        }
        available.push((key, item));
    }
    if available.is_empty() {
        return None;
    }

    let total_count = available.len() as i64;
    let mut items = Vec::new();
    for (key, item) in available.into_iter().take(limit) {
        emitted_item_keys.insert(key);
        items.push(item);
    }

    Some(DiscoverySectionResult {
        section_id,
        section_type,
        title,
        surface,
        total_count,
        items,
    })
}

fn home_item_visible(item: &DiscoveryItemRecord, include_unresolved: bool) -> bool {
    !item.owned_in_input && (include_unresolved || item.resolved)
}

fn discovery_item_media_kind(item: &DiscoveryItemRecord) -> Option<&'static str> {
    if let Some(content_type) = item
        .content_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return normalized_discovery_media_kind(content_type);
    }

    normalized_discovery_media_kind(&item.target_kind)
}

fn normalized_discovery_media_kind(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "anime" => Some("anime"),
        "movie" => Some("movie"),
        "series" => Some("series"),
        _ => None,
    }
}

fn resolve_discovery_matched_subjects(
    items: &mut [DiscoveryItemRecord],
    submitted_subjects: &[DiscoverySubmittedSubjectRecord],
) -> AppResult<()> {
    let mut titles_by_subject_key = HashMap::<&str, Vec<String>>::new();
    for subject in submitted_subjects {
        let Some(title) = subject.display_title.as_deref().map(str::trim) else {
            continue;
        };
        if !title.is_empty() {
            titles_by_subject_key
                .entry(subject.subject_key.as_str())
                .or_default()
                .push(title.to_string());
        }
    }

    for item in items {
        let titles = item
            .matched_subject_keys
            .iter()
            .flat_map(|key| {
                titles_by_subject_key
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        item.matched_subject_titles = titles;
        item.matched_subject_count = item.matched_subject_titles.len() as i32;
    }

    Ok(())
}

fn filter_submitted_subjects_for_libraries(
    submitted_subjects: &[DiscoverySubmittedSubjectRecord],
    readable_library_ids: &HashSet<String>,
) -> Vec<DiscoverySubmittedSubjectRecord> {
    submitted_subjects
        .iter()
        .filter(|subject| {
            subject
                .library_id
                .as_deref()
                .is_some_and(|library_id| readable_library_ids.contains(library_id))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
fn item_matches_discovery_items_query(
    item: &DiscoveryItemRecord,
    query: &DiscoveryItemsQuery,
) -> bool {
    if !query.include_owned && item.owned_in_input {
        return false;
    }
    if !query.include_unresolved && !item.resolved {
        return false;
    }
    if !matches_optional_text_query(item, query.query.as_deref()) {
        return false;
    }
    if !query.target_keys.is_empty()
        && !contains_case_insensitive(&query.target_keys, item.target_key.as_str())
    {
        return false;
    }
    if !query.target_kinds.is_empty()
        && !discovery_item_media_kind(item)
            .is_some_and(|kind| contains_case_insensitive(&query.target_kinds, kind))
    {
        return false;
    }
    if !query.sources.is_empty()
        && !text_values_or_optional_contains_any(
            &item.sources,
            item.best_source.as_deref(),
            &query.sources,
        )
    {
        return false;
    }
    if !query.relation_types.is_empty()
        && !text_values_contain_any(&item.relation_types, &query.relation_types)
    {
        return false;
    }
    if !query.relation_subtypes.is_empty()
        && !text_values_contain_any(&item.relation_subtypes, &query.relation_subtypes)
    {
        return false;
    }
    if !query.genres.is_empty()
        && !discovery_item_matches_canonical_facet_filters(item, "genre", &query.genres)
    {
        return false;
    }
    if !query.status_tags.is_empty()
        && !text_values_contain_any(&item.status_tags, &query.status_tags)
    {
        return false;
    }
    if !query.facet_terms.is_empty()
        && !text_values_contain_any(&item.facet_terms, &query.facet_terms)
    {
        return false;
    }
    true
}

#[cfg(test)]
fn matches_optional_text_query(item: &DiscoveryItemRecord, query: Option<&str>) -> bool {
    let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
        return true;
    };
    let query = query.to_ascii_lowercase();
    [
        Some(item.display_title.as_str()),
        item.original_title.as_deref(),
        item.sort_title.as_deref(),
        item.overview.as_deref(),
        item.tmdb_collection_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains(&query))
}

fn dedupe_and_sort_discovery_items(items: &mut Vec<DiscoveryItemRecord>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(discovery_item_identity_key(item).to_string()));
    items.sort_by(compare_discovery_items);
}

fn discovery_item_identity_key(item: &DiscoveryItemRecord) -> &str {
    if item.target_key.trim().is_empty() {
        item.id.as_str()
    } else {
        item.target_key.as_str()
    }
}

fn compare_discovery_items(left: &DiscoveryItemRecord, right: &DiscoveryItemRecord) -> Ordering {
    right
        .rank_score
        .partial_cmp(&left.rank_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            left.sort_title
                .as_deref()
                .unwrap_or(&left.display_title)
                .cmp(right.sort_title.as_deref().unwrap_or(&right.display_title))
        })
        .then_with(|| left.target_key.cmp(&right.target_key))
}

#[cfg(test)]
fn text_values_contain_any(values: &[String], filters: &[String]) -> bool {
    filters.iter().any(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
    })
}

#[cfg(test)]
fn text_values_or_optional_contains_any(
    values: &[String],
    text: Option<&str>,
    filters: &[String],
) -> bool {
    text.is_some_and(|text| {
        filters
            .iter()
            .any(|filter| text.eq_ignore_ascii_case(filter))
    }) || text_values_contain_any(values, filters)
}

fn normalize_discovery_filter_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
fn contains_case_insensitive(values: &[String], candidate: &str) -> bool {
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(candidate))
}

fn collect_json_text_values(value: &JsonValue, values: &mut Vec<String>) {
    match value {
        JsonValue::String(value) => values.push(value.clone()),
        JsonValue::Number(value) => values.push(value.to_string()),
        JsonValue::Bool(value) => values.push(value.to_string()),
        JsonValue::Array(items) => {
            for item in items {
                collect_json_text_values(item, values);
            }
        }
        JsonValue::Object(object) => {
            for value in object.values() {
                collect_json_text_values(value, values);
            }
        }
        JsonValue::Null => {}
    }
}

pub(crate) fn build_discovery_library_context(
    titles: &[Title],
    defaults: DiscoveryContextDefaults,
) -> DiscoveryLibraryContext {
    let mut subject_provenance = titles
        .iter()
        .filter_map(build_discovery_library_subject)
        .collect::<Vec<_>>();

    subject_provenance.sort_by(|left, right| {
        left.subject_key
            .cmp(&right.subject_key)
            .then_with(|| left.canonical.cmp(&right.canonical))
            .then_with(|| left.library_id.cmp(&right.library_id))
            .then_with(|| left.title_id.cmp(&right.title_id))
    });
    subject_provenance.dedup_by(|left, right| {
        left.subject_key == right.subject_key
            && left.title_id == right.title_id
            && left.library_id == right.library_id
    });

    let mut subjects = subject_provenance.clone();
    subjects.dedup_by(|left, right| left.subject_key == right.subject_key);

    let canonical_subjects = subjects
        .iter()
        .map(|subject| subject.canonical.clone())
        .collect::<Vec<_>>();

    DiscoveryLibraryContext {
        subjects,
        subject_provenance,
        fingerprint: discovery_context_fingerprint(&defaults, &canonical_subjects),
    }
}

impl DiscoveryLibraryContext {
    pub(crate) fn snapshot_submit_input(
        &self,
        defaults: &DiscoveryContextDefaults,
    ) -> DiscoveryContextSnapshotSubmitInput {
        DiscoveryContextSnapshotSubmitInput {
            subjects: self
                .subjects
                .iter()
                .map(|subject| subject.subject.clone())
                .collect(),
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            max_items: defaults.max_items as i32,
            include_owned: defaults.include_owned,
            include_unresolved: defaults.include_unresolved,
            context_fingerprint: Some(self.fingerprint.clone()),
        }
    }

    pub(crate) fn incremental_changes_input(
        &self,
        defaults: &DiscoveryContextDefaults,
        pending_changes: &[DiscoveryPendingContextChangeRecord],
        previous_context_fingerprint: &str,
    ) -> AppResult<DiscoveryContextChangesInput> {
        let resolved_key_count = pending_context_changes_resolved_key_count(pending_changes)?;
        if resolved_key_count > DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS {
            return Err(AppError::Validation(format!(
                "discovery incremental reload resolves to {resolved_key_count} changed subjects, above SMG limit {}",
                DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS
            )));
        }

        let context_subject_keys = self
            .subjects
            .iter()
            .map(|subject| subject.subject_key.clone())
            .collect();
        let changed_subjects = pending_changes
            .iter()
            .map(changed_subject_from_pending)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(DiscoveryContextChangesInput {
            context_subject_keys,
            changed_subjects,
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            max_items: defaults.max_items as i32,
            include_owned: defaults.include_owned,
            include_unresolved: defaults.include_unresolved,
            context_fingerprint: Some(self.fingerprint.clone()),
            previous_context_fingerprint: Some(previous_context_fingerprint.to_string()),
        })
    }

    pub(crate) fn submitted_subject_records(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        self.subject_provenance
            .iter()
            .map(|subject| {
                let external_ids_json = serde_json::to_string(&subject.subject.external_ids)
                    .map_err(discovery_json_error)?;
                let raw_subject_json =
                    serde_json::to_string(&subject.subject).map_err(discovery_json_error)?;
                Ok(DiscoverySubmittedSubjectRecord {
                    run_id: run_id.to_string(),
                    subject_key: subject.subject_key.clone(),
                    title_id: Some(subject.title_id.clone()),
                    library_id: Some(subject.library_id.clone()),
                    library_facet: Some(subject.facet.clone()),
                    title_kind: subject.subject.kind.clone(),
                    display_title: Some(subject.title_name.clone()),
                    external_ids_json,
                    raw_subject_json,
                })
            })
            .collect()
    }

    pub(crate) fn subject_provenance_by_key(
        &self,
    ) -> HashMap<String, Vec<DiscoveryLibraryProvenance>> {
        let mut provenance_by_key = HashMap::<String, Vec<DiscoveryLibraryProvenance>>::new();
        for subject in &self.subject_provenance {
            provenance_by_key
                .entry(subject.subject_key.clone())
                .or_default()
                .push(DiscoveryLibraryProvenance {
                    subject_key: subject.subject_key.clone(),
                    title_id: Some(subject.title_id.clone()),
                    library_id: subject.library_id.clone(),
                });
        }
        provenance_by_key
    }
}

pub(crate) fn pending_context_change_from_domain_event(
    scope_key: &str,
    event: &DomainEvent,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    match &event.payload {
        DomainEventPayload::TitleAdded(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "added", None)
        }
        DomainEventPayload::TitleUpdated(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "updated", None)
        }
        DomainEventPayload::TitleDeleted(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "removed", None)
        }
        DomainEventPayload::TitleRematched(data) => {
            let mut current_ids = data.title.external_ids.clone();
            current_ids.tvdb_id = Some(data.new_tvdb_id.clone());
            let previous_ids = data.old_tvdb_id.as_ref().map(|old_tvdb_id| {
                let mut external_ids = data.title.external_ids.clone();
                external_ids.tvdb_id = Some(old_tvdb_id.clone());
                external_ids
            });
            title_context_change_record(
                scope_key,
                event,
                &data.title,
                previous_ids.as_ref(),
                "rematched",
                Some(&current_ids),
            )
        }
        _ => Ok(None),
    }
}

fn build_discovery_library_subject(title: &Title) -> Option<DiscoveryLibrarySubject> {
    let parts =
        build_discovery_subject_parts(&title.facet, normalized_supported_external_ids(title))?;
    Some(DiscoveryLibrarySubject {
        title_id: title.id.clone(),
        library_id: title.library_id.clone(),
        title_name: title.name.clone(),
        facet: parts.facet,
        subject_key: parts.subject_key,
        subject: parts.subject,
        canonical: parts.canonical,
    })
}

fn title_context_change_record(
    scope_key: &str,
    event: &DomainEvent,
    title: &TitleContextSnapshot,
    previous_external_ids: Option<&DomainExternalIds>,
    change_type: &str,
    current_external_ids: Option<&DomainExternalIds>,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    let current = match build_discovery_title_context_subject(
        title,
        current_external_ids.unwrap_or(&title.external_ids),
    ) {
        Some(subject) => subject,
        None => return Ok(None),
    };
    let previous = previous_external_ids
        .and_then(|external_ids| build_discovery_title_context_subject(title, external_ids));
    let title_id = event.title_id.clone();
    let identity = title_id.as_deref().unwrap_or(current.subject_key.as_str());
    let raw_subject_json = serde_json::to_string(&current.subject).map_err(discovery_json_error)?;
    let raw_previous_subject_json = previous
        .as_ref()
        .map(|subject| serde_json::to_string(&subject.subject).map_err(discovery_json_error))
        .transpose()?;

    Ok(Some(DiscoveryPendingContextChangeRecord {
        id: format!("{scope_key}:title:{identity}"),
        scope_key: scope_key.to_string(),
        subject_key: Some(current.subject_key),
        previous_subject_key: previous.map(|subject| subject.subject_key),
        change_type: change_type.to_string(),
        title_id,
        previous_title_id: None,
        library_facet: Some(current.facet),
        raw_subject_json: Some(raw_subject_json),
        raw_previous_subject_json,
        first_seen_sequence: Some(event.sequence),
        last_seen_sequence: Some(event.sequence),
        first_seen_at: event.occurred_at,
        last_seen_at: event.occurred_at,
    }))
}

pub(crate) fn coalesce_pending_context_change(
    existing: Option<&DiscoveryPendingContextChangeRecord>,
    incoming: DiscoveryPendingContextChangeRecord,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    let Some(existing) = existing else {
        return Ok(Some(incoming));
    };

    let existing_type = discovery_change_type_from_str(&existing.change_type)?;
    let incoming_type = discovery_change_type_from_str(&incoming.change_type)?;

    if matches!(existing_type, DiscoveryContextChangeType::Added)
        && matches!(incoming_type, DiscoveryContextChangeType::Removed)
    {
        return Ok(None);
    }

    let mut merged = incoming;
    merged.id = existing.id.clone();
    merged.scope_key = existing.scope_key.clone();
    merged.first_seen_sequence = existing.first_seen_sequence.or(merged.first_seen_sequence);
    merged.first_seen_at = existing.first_seen_at;

    match (existing_type, incoming_type) {
        (DiscoveryContextChangeType::Added, _) => {
            merged.change_type = "added".to_string();
            merged.previous_subject_key = None;
            merged.previous_title_id = None;
            merged.raw_previous_subject_json = None;
        }
        (_, DiscoveryContextChangeType::Removed) => {
            merged.change_type = "removed".to_string();
            if merged.previous_subject_key.is_none() {
                merged.previous_subject_key = existing
                    .previous_subject_key
                    .clone()
                    .or_else(|| existing.subject_key.clone());
            }
            if merged.raw_previous_subject_json.is_none() {
                merged.raw_previous_subject_json = existing
                    .raw_previous_subject_json
                    .clone()
                    .or_else(|| existing.raw_subject_json.clone());
            }
            if merged.previous_title_id.is_none() {
                merged.previous_title_id = existing
                    .previous_title_id
                    .clone()
                    .or_else(|| existing.title_id.clone());
            }
        }
        (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Added)
        | (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Updated) => {
            merged.change_type = "rematched".to_string();
            merged.previous_subject_key = existing
                .previous_subject_key
                .clone()
                .or_else(|| existing.subject_key.clone());
            merged.raw_previous_subject_json = existing
                .raw_previous_subject_json
                .clone()
                .or_else(|| existing.raw_subject_json.clone());
            merged.previous_title_id = existing
                .previous_title_id
                .clone()
                .or_else(|| existing.title_id.clone());
        }
        (DiscoveryContextChangeType::Updated, DiscoveryContextChangeType::Updated) => {
            merged.change_type = "updated".to_string();
            merged.previous_subject_key = existing.previous_subject_key.clone();
            merged.raw_previous_subject_json = existing.raw_previous_subject_json.clone();
            merged.previous_title_id = existing.previous_title_id.clone();
        }
        (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Rematched)
        | (DiscoveryContextChangeType::Updated, DiscoveryContextChangeType::Rematched)
        | (DiscoveryContextChangeType::Rematched, DiscoveryContextChangeType::Updated)
        | (DiscoveryContextChangeType::Rematched, DiscoveryContextChangeType::Rematched) => {
            merged.change_type = "rematched".to_string();
            if merged.previous_subject_key.is_none() {
                merged.previous_subject_key = existing
                    .previous_subject_key
                    .clone()
                    .or_else(|| existing.subject_key.clone());
            }
            if merged.raw_previous_subject_json.is_none() {
                merged.raw_previous_subject_json = existing
                    .raw_previous_subject_json
                    .clone()
                    .or_else(|| existing.raw_subject_json.clone());
            }
            if merged.previous_title_id.is_none() {
                merged.previous_title_id = existing
                    .previous_title_id
                    .clone()
                    .or_else(|| existing.title_id.clone());
            }
        }
        (_, DiscoveryContextChangeType::Added) => {
            merged.change_type = "added".to_string();
            merged.previous_subject_key = None;
            merged.previous_title_id = None;
            merged.raw_previous_subject_json = None;
        }
    }

    Ok(Some(merged))
}

fn build_discovery_title_context_subject(
    title: &TitleContextSnapshot,
    external_ids: &DomainExternalIds,
) -> Option<DiscoverySubjectParts> {
    build_discovery_subject_parts(
        &title.facet,
        normalized_supported_domain_external_ids(external_ids),
    )
}

fn build_discovery_subject_parts(
    facet: &MediaFacet,
    external_ids: Vec<CanonicalExternalId>,
) -> Option<DiscoverySubjectParts> {
    if external_ids.is_empty() {
        return None;
    }

    let facet_name = facet.as_str().to_string();
    let kind = discovery_resolver_kind_from_facet(facet);
    let tvdb_id = unique_i32_external_id(&external_ids, "tvdb");
    let tmdb_id = unique_i32_external_id(&external_ids, "tmdb");
    let mal_id = unique_i32_external_id(&external_ids, "mal");
    let anidb_id = unique_i32_external_id(&external_ids, "anidb");
    let subject_key =
        fallback_discovery_subject_key(&kind, &external_ids, tvdb_id, tmdb_id, mal_id, anidb_id);
    let canonical = CanonicalSubject {
        subject_key: subject_key.clone(),
        key: None,
        kind,
        facet: facet_name.clone(),
        external_ids,
    };

    let subject = DiscoverySubjectInput {
        key: canonical.key.clone(),
        tvdb_id,
        tmdb_id,
        mal_id,
        anidb_id,
        kind: Some(canonical.kind.clone()),
        facet: Some(canonical.facet.clone()),
        external_ids: canonical
            .external_ids
            .iter()
            .map(|external_id| DiscoveryExternalIdInput {
                source: external_id.source.clone(),
                value: external_id.value.clone(),
            })
            .collect(),
    };

    Some(DiscoverySubjectParts {
        facet: facet_name,
        subject_key,
        subject,
        canonical,
    })
}

fn normalized_supported_external_ids(title: &Title) -> Vec<CanonicalExternalId> {
    title
        .external_ids
        .iter()
        .filter_map(|external_id| {
            normalize_supported_external_id(&external_id.source, &external_id.value)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalized_supported_domain_external_ids(
    external_ids: &DomainExternalIds,
) -> Vec<CanonicalExternalId> {
    [
        ("tvdb", external_ids.tvdb_id.as_deref()),
        ("tmdb", external_ids.tmdb_id.as_deref()),
        ("anidb", external_ids.anidb_id.as_deref()),
    ]
    .into_iter()
    .filter_map(|(source, value)| normalize_supported_external_id(source, value?))
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect()
}

fn discovery_resolver_kind_from_facet(facet: &MediaFacet) -> String {
    match facet {
        MediaFacet::Anime => "series".to_string(),
        _ => facet.as_str().to_string(),
    }
}

fn normalize_supported_external_id(source: &str, value: &str) -> Option<CanonicalExternalId> {
    let source = normalize_supported_external_id_source(source)?;
    let value = parse_positive_external_numeric_id(value)?.to_string();
    Some(CanonicalExternalId { source, value })
}

fn normalize_supported_external_id_source(source: &str) -> Option<String> {
    match source.trim().to_ascii_lowercase().as_str() {
        "tvdb" | "thetvdb" | "tvdb_show" | "tvdb_series" | "tvdb_movie" => Some("tvdb".to_string()),
        "tmdb" | "themoviedb" | "tmdb_tv" | "tmdb_show" | "tmdb_series" | "tmdb_movie" => {
            Some("tmdb".to_string())
        }
        "anidb" => Some("anidb".to_string()),
        "mal" | "myanimelist" => Some("mal".to_string()),
        "anilist" | "anilist_anime" | "anilist:anime" => Some("anilist".to_string()),
        _ => None,
    }
}

fn parse_positive_external_numeric_id(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value = value.rsplit(':').next().unwrap_or(value).trim();
    value.parse::<i64>().ok().filter(|id| *id > 0)
}

fn unique_i32_external_id(external_ids: &[CanonicalExternalId], source: &str) -> Option<i32> {
    let values = external_ids
        .iter()
        .filter(|external_id| external_id.source == source)
        .filter_map(|external_id| external_id.value.parse::<i32>().ok())
        .filter(|id| *id > 0)
        .collect::<BTreeSet<_>>();

    if values.len() == 1 {
        values.into_iter().next()
    } else {
        None
    }
}

fn fallback_discovery_subject_key(
    kind: &str,
    external_ids: &[CanonicalExternalId],
    tvdb_id: Option<i32>,
    tmdb_id: Option<i32>,
    mal_id: Option<i32>,
    anidb_id: Option<i32>,
) -> String {
    if let Some(tvdb_id) = tvdb_id {
        return format!("tvdb:{kind}:{tvdb_id}");
    }
    if let Some(tmdb_id) = tmdb_id {
        return format!("tmdb:{kind}:{tmdb_id}");
    }
    if let Some(mal_id) = mal_id {
        return format!("mal:anime:{mal_id}");
    }
    if let Some(anidb_id) = anidb_id {
        return format!("anidb:anime:{anidb_id}");
    }

    for source in ["tvdb", "tmdb", "anidb", "mal", "anilist"] {
        for external_id in external_ids
            .iter()
            .filter(|external_id| external_id.source == source)
        {
            match source {
                "tvdb" => return format!("tvdb:{kind}:{}", external_id.value),
                "tmdb" => return format!("tmdb:{kind}:{}", external_id.value),
                "mal" => return format!("mal:anime:{}", external_id.value),
                "anidb" => return format!("anidb:anime:{}", external_id.value),
                "anilist" => return format!("anilist:anime:{}", external_id.value),
                _ => {}
            }
        }
    }

    let bytes = serde_json::to_vec(external_ids)
        .expect("discovery subject key fallback input should always serialize");
    format!("local:{}", blake3::hash(&bytes).to_hex())
}

fn discovery_context_fingerprint(
    defaults: &DiscoveryContextDefaults,
    subjects: &[CanonicalSubject],
) -> String {
    let context = CanonicalContext {
        schema_version: 1,
        defaults,
        subjects,
    };
    let bytes = serde_json::to_vec(&context)
        .expect("discovery context fingerprint input should always serialize");
    format!("blake3:{}", blake3::hash(&bytes).to_hex())
}

pub(crate) fn snapshot_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            discovery_item_record(
                run_id,
                base_generation_id,
                "context_snapshot",
                None,
                index,
                item,
                provenance_by_subject_key,
                now,
            )
        })
        .collect()
}

pub(crate) fn incremental_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            discovery_item_record(
                run_id,
                base_generation_id,
                "context_incremental",
                None,
                index,
                item,
                provenance_by_subject_key,
                now,
            )
        })
        .collect()
}

pub(crate) fn public_feed_section_records(
    run_id: &str,
    result: &DiscoveryDashboardResult,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoverySectionRecord>> {
    public_feed_sections(result)
        .enumerate()
        .map(|(index, section)| public_feed_section_record(run_id, index, section, now))
        .collect()
}

pub(crate) fn public_feed_item_records(
    run_id: &str,
    result: &DiscoveryDashboardResult,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let mut records = Vec::new();
    let empty_provenance = HashMap::<String, Vec<DiscoveryLibraryProvenance>>::new();
    for (section_index, section) in public_feed_sections(result).enumerate() {
        for (item_index, item) in section.items.iter().enumerate() {
            let mut record = discovery_item_record(
                run_id,
                run_id,
                "public_feed",
                Some(section.section_id.clone()),
                section_index * 10_000 + item_index,
                item,
                &empty_provenance,
                now,
            )?;
            record.matched_subject_keys.clear();
            record.matched_subject_titles.clear();
            record.matched_subject_count = 0;
            record.library_provenance.clear();
            records.push(record);
        }
    }
    Ok(records)
}

fn public_feed_sections(
    result: &DiscoveryDashboardResult,
) -> impl Iterator<Item = &DiscoveryDashboardSection> {
    result
        .sections
        .iter()
        .filter(|section| !discovery_section_is_complete_the_collection(&section.section_type))
}

fn discovery_section_is_complete_the_collection(section_type: &str) -> bool {
    section_type
        .trim()
        .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
}

fn public_feed_section_record(
    run_id: &str,
    index: usize,
    section: &DiscoveryDashboardSection,
    now: DateTime<Utc>,
) -> AppResult<DiscoverySectionRecord> {
    Ok(DiscoverySectionRecord {
        id: format!("{run_id}:section:{index}"),
        run_id: run_id.to_string(),
        section_id: section.section_id.clone(),
        section_type: section.section_type.clone(),
        surface: "public".to_string(),
        title: section.title.clone(),
        sort_index: index as i32,
        created_at: now,
        updated_at: now,
    })
}

pub(crate) fn snapshot_facet_records(
    run_id: &str,
    pages: &[DiscoveryContextSnapshotPageResult],
) -> AppResult<Vec<DiscoveryFacetRecord>> {
    let mut facets = Vec::new();
    for page in pages {
        for group in &page.facets {
            for value in &group.values {
                facets.push(DiscoveryFacetRecord {
                    run_id: run_id.to_string(),
                    facet_name: group.name.clone(),
                    facet_value: value.value.clone(),
                    smg_count: Some(i64::from(value.count)),
                    local_count: None,
                });
            }
        }
    }
    Ok(facets)
}

#[expect(
    clippy::too_many_arguments,
    reason = "discovery item persistence maps explicit run and projection fields"
)]
fn discovery_item_record(
    run_id: &str,
    base_generation_id: &str,
    source_run_kind: &str,
    section_id: Option<String>,
    index: usize,
    item: &DiscoveryTitle,
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryItemRecord> {
    let library_provenance =
        discovery_item_library_provenance_records(item, provenance_by_subject_key);
    Ok(DiscoveryItemRecord {
        id: format!("{run_id}:item:{index}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(base_generation_id.to_string()),
        source_run_kind: source_run_kind.to_string(),
        section_id,
        sort_index: index as i32,
        target_key: item.target_key.clone(),
        target_kind: item.target_kind.clone(),
        resolved: item.resolved,
        resolved_title_id: None,
        display_title: discovery_display_title(item).unwrap_or_default(),
        original_title: non_identifier_discovery_title(&item.original_title).map(str::to_string),
        sort_title: discovery_sort_title(item),
        year: item.year,
        poster_path: non_empty_string(&item.poster_path),
        poster_url: non_empty_string(&item.poster_url),
        background_url: non_empty_string(&item.background_url),
        overview: non_empty_string(&item.overview),
        content_type: non_empty_string(&item.content_type),
        canonical_tags: discovery_canonical_tags(item),
        rating: item.rating,
        rating_sources: item.rating_sources.clone(),
        external_ratings: item.external_ratings.clone(),
        external_ids: discovery_external_id_records(item),
        status_tags: item.status_tags.clone(),
        source_tags: discovery_source_tag_records(&item.source_tags),
        sources: item.sources.clone(),
        best_source: non_empty_string(&item.best_source),
        relation_types: item.relation_types.clone(),
        relation_subtypes: item.relation_subtypes.clone(),
        chart_signals: discovery_json_signal_values(&item.chart_signals),
        provider_signals: discovery_json_signal_values(&item.provider_signals),
        rank_components: discovery_rank_component_records(&item.rank_components),
        source_count: Some(item.source_count),
        edge_count: Some(item.edge_count),
        relation_count: Some(item.relation_count),
        source_subject_count: Some(item.source_subject_count),
        rank_score: Some(item.rank_score),
        matched_subject_keys: item.matched_subject_keys.clone(),
        matched_subject_titles: item.matched_subject_titles.clone(),
        matched_subject_count: item.matched_subject_count,
        library_provenance,
        tmdb_collection_id: item.tmdb_collection_id.map(|id| id.to_string()),
        tmdb_collection_name: non_empty_string(&item.tmdb_collection_name),
        owned_in_input: item.owned_in_input,
        studio_slug: item.studio_slug.clone(),
        person_ids: item.person_ids.clone(),
        facet_terms: discovery_canonical_facet_terms(item),
        context_terms: item.context_terms.clone(),
        change_subject_keys: item.change_subject_keys.clone(),
        removed_subject_keys: item.removed_subject_keys.clone(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        created_at: now,
        updated_at: now,
    })
}

pub(crate) fn title_more_like_this_item_records(
    title_id: &str,
    source_target_keys: &[String],
    items: &[DiscoveryTitle],
    limit: usize,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let run_id = format!("title:{title_id}:more_like_this");
    let provenance = HashMap::new();
    let source_keys = source_target_keys
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    for item in items {
        let target_key = item.target_key.trim();
        if target_key.is_empty()
            || discovery_target_key_parts(target_key).is_none()
            || discovery_display_title(item).is_none()
            || source_keys.contains(target_key)
            || !seen.insert(target_key.to_string())
        {
            continue;
        }
        let mut record = discovery_item_record(
            &run_id,
            &run_id,
            "title_more_like_this",
            None,
            records.len(),
            item,
            &provenance,
            now,
        )?;
        record.base_generation_id = None;
        record.matched_subject_keys.clear();
        record.matched_subject_titles.clear();
        record.matched_subject_count = 0;
        record.library_provenance.clear();
        record.owned_in_input = false;
        record.resolved_title_id = None;
        records.push(record);
        if records.len() >= limit {
            break;
        }
    }
    Ok(records)
}

pub(crate) fn title_recommendations_subject(
    title: &Title,
    external_ids: &[ExternalId],
) -> Option<(DiscoverySubjectInput, Vec<String>)> {
    let mut ids = BTreeSet::<(String, String)>::new();
    for external_id in title.external_ids.iter().chain(external_ids.iter()) {
        let raw_source = external_id.source.trim().to_ascii_lowercase();
        let source = if matches!(raw_source.as_str(), "imdb" | "imdb_id") {
            "imdb".to_string()
        } else {
            let Some(source) = normalize_supported_external_id_source(&external_id.source) else {
                continue;
            };
            source
        };
        let value = external_id.value.trim();
        if value.is_empty() {
            continue;
        }
        let value = if source == "imdb" {
            let Some(normalized) = crate::normalize::normalize_imdb_id(value) else {
                continue;
            };
            normalized
        } else {
            let Some(id) = parse_positive_external_numeric_id(value) else {
                continue;
            };
            id.to_string()
        };
        ids.insert((source, value));
    }
    if let Some(imdb_id) = title
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
    {
        ids.insert(("imdb".to_string(), imdb_id));
    }

    if ids.is_empty() {
        return None;
    }

    let mut subject = DiscoverySubjectInput {
        kind: Some(title.facet.as_str().to_string()),
        facet: Some(title.facet.as_str().to_string()),
        ..Default::default()
    };
    let mut target_keys = Vec::new();
    for (source, value) in &ids {
        subject.external_ids.push(DiscoveryExternalIdInput {
            source: source.clone(),
            value: value.clone(),
        });
        match source.as_str() {
            "tvdb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.tvdb_id = subject.tvdb_id.or(Some(id));
                    let key = keyed_discovery_target_key("tvdb", &title.facet, value);
                    target_keys.push(key);
                }
            }
            "tmdb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.tmdb_id = subject.tmdb_id.or(Some(id));
                    let key = keyed_discovery_target_key("tmdb", &title.facet, value);
                    target_keys.push(key);
                }
            }
            "mal" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.mal_id = subject.mal_id.or(Some(id));
                    let key = format!("mal:anime:{value}");
                    target_keys.push(key);
                }
            }
            "anidb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.anidb_id = subject.anidb_id.or(Some(id));
                    let key = format!("anidb:anime:{value}");
                    target_keys.push(key);
                }
            }
            "anilist" => {
                let key = format!("anilist:anime:{value}");
                target_keys.push(key);
            }
            "imdb" => {
                let key = format!("imdb:title:{value}");
                target_keys.push(key);
            }
            _ => {}
        }
    }
    subject.key = title_recommendations_preferred_subject_key(&ids, &title.facet);
    Some((subject, unique_discovery_text_terms(target_keys)))
}

fn title_recommendations_preferred_subject_key(
    ids: &BTreeSet<(String, String)>,
    facet: &MediaFacet,
) -> Option<String> {
    for source in ["tvdb", "tmdb", "mal", "anidb", "anilist", "imdb"] {
        let Some((_, value)) = ids.iter().find(|(candidate, _)| candidate == source) else {
            continue;
        };
        return Some(match source {
            "tvdb" | "tmdb" => keyed_discovery_target_key(source, facet, value),
            "mal" | "anidb" | "anilist" => format!("{source}:anime:{value}"),
            "imdb" => format!("imdb:title:{value}"),
            _ => unreachable!("source priority list contains only known sources"),
        });
    }
    None
}

fn keyed_discovery_target_key(source: &str, facet: &MediaFacet, value: &str) -> String {
    let kind = match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Series | MediaFacet::Anime => "series",
    };
    format!("{source}:{kind}:{value}")
}

fn parse_positive_i32(value: &str) -> Option<i32> {
    value.trim().parse::<i32>().ok().filter(|value| *value > 0)
}

fn discovery_target_key_parts(target_key: &str) -> Option<(String, String, String)> {
    let mut parts = target_key.split(':');
    let source = parts.next()?.trim();
    let kind = parts.next()?.trim();
    let value = parts.next()?.trim();
    if source.is_empty() || kind.is_empty() || value.is_empty() {
        return None;
    }
    Some((
        source.to_ascii_lowercase(),
        kind.to_ascii_lowercase(),
        value.to_string(),
    ))
}

fn discovery_local_external_id_sources(source: &str, kind: &str) -> Vec<String> {
    match source {
        "tvdb" => match kind {
            "movie" => vec!["tvdb".to_string(), "tvdb_movie".to_string()],
            _ => vec![
                "tvdb".to_string(),
                "tvdb_series".to_string(),
                "tvdb_show".to_string(),
            ],
        },
        "tmdb" => match kind {
            "movie" => vec!["tmdb".to_string(), "tmdb_movie".to_string()],
            _ => vec![
                "tmdb".to_string(),
                "tmdb_series".to_string(),
                "tmdb_tv".to_string(),
                "tmdb_show".to_string(),
            ],
        },
        "mal" => vec!["mal".to_string(), "myanimelist".to_string()],
        "anilist" => vec![
            "anilist".to_string(),
            "anilist_anime".to_string(),
            "anilist:anime".to_string(),
        ],
        "anidb" => vec!["anidb".to_string()],
        "imdb" => vec!["imdb".to_string()],
        _ => vec![source.to_string()],
    }
}

fn discovery_local_external_id_values(kind: &str, value: &str) -> Vec<String> {
    unique_discovery_text_terms(vec![value.to_string(), format!("{kind}:{value}")])
}

fn discovery_item_library_provenance_records(
    item: &DiscoveryTitle,
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
) -> Vec<DiscoveryItemLibraryProvenanceRecord> {
    let mut provenance = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push_subject_key = |subject_key: &str| {
        let subject_key = subject_key.trim();
        if subject_key.is_empty() {
            return;
        }
        let Some(entries) = provenance_by_subject_key.get(subject_key) else {
            return;
        };
        for entry in entries {
            if seen.insert((
                entry.subject_key.clone(),
                entry.title_id.clone(),
                entry.library_id.clone(),
            )) {
                provenance.push(DiscoveryItemLibraryProvenanceRecord {
                    subject_key: entry.subject_key.clone(),
                    title_id: entry.title_id.clone(),
                    library_id: Some(entry.library_id.clone()),
                });
            }
        }
    };

    for subject_key in &item.matched_subject_keys {
        push_subject_key(subject_key);
    }
    for subject_key in &item.change_subject_keys {
        push_subject_key(subject_key);
    }
    for subject_key in &item.removed_subject_keys {
        push_subject_key(subject_key);
    }
    if item.owned_in_input {
        push_subject_key(&item.target_key);
    }

    provenance
}

fn discovery_source_tag_records(values: &[JsonValue]) -> Vec<DiscoverySourceTagRecord> {
    values
        .iter()
        .map(|value| {
            let category = json_object_string(value, &["category", "type"]);
            let name = json_object_string(value, &["name", "label", "value"]);
            DiscoverySourceTagRecord {
                category,
                name,
                values: unique_json_text_values(value),
            }
        })
        .collect()
}

fn discovery_external_id_records(item: &DiscoveryTitle) -> Vec<DiscoveryExternalIdRecord> {
    item.external_ids
        .iter()
        .filter_map(|external_id| {
            let source = external_id.source.trim();
            let id = external_id.id.trim();
            let key = external_id.key.trim();
            if source.is_empty() || (id.is_empty() && key.is_empty()) {
                return None;
            }
            Some(DiscoveryExternalIdRecord {
                source: source.to_ascii_lowercase(),
                kind: external_id.kind.trim().to_ascii_lowercase(),
                id: id.to_string(),
                key: key.to_string(),
            })
        })
        .collect()
}

fn discovery_canonical_facet_terms(item: &DiscoveryTitle) -> Vec<String> {
    let mut values = item.facet_terms.clone();
    for canonical_tag in &item.canonical_tags {
        values.extend(canonical_discovery_terms_from_canonical_tag(canonical_tag));
    }
    unique_discovery_text_terms(values)
}

fn discovery_canonical_tags(item: &DiscoveryTitle) -> Vec<scryer_domain::CanonicalMediaTag> {
    item.canonical_tags
        .iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

fn canonical_discovery_terms_from_canonical_tag(value: &JsonValue) -> Vec<String> {
    let mut terms = unique_json_text_values(value)
        .into_iter()
        .filter_map(|value| canonical_discovery_term(&value).map(str::to_string))
        .collect::<Vec<_>>();
    if !terms.is_empty() {
        return unique_discovery_text_terms(terms);
    }

    if !value.is_object() {
        return Vec::new();
    }
    let Some(category) = json_object_string(value, &["category", "type"]) else {
        return Vec::new();
    };
    let category = category.trim().to_ascii_lowercase();
    if category != "genre" && category != "theme" {
        return Vec::new();
    }
    let Some(label) = json_object_string(value, &["key", "name", "label", "value"]) else {
        return Vec::new();
    };
    let label = label.trim();
    if label.is_empty() {
        return Vec::new();
    }
    let tail = label
        .rsplit(':')
        .next()
        .unwrap_or(label)
        .trim()
        .to_ascii_lowercase()
        .replace(|character: char| !character.is_ascii_alphanumeric(), "-")
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if tail.is_empty() {
        return Vec::new();
    }
    terms.push(format!("canonical:{category}:{tail}"));
    unique_discovery_text_terms(terms)
}

fn unique_discovery_text_terms(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                return None;
            }
            seen.insert(normalize_discovery_filter_value(&value))
                .then_some(value)
        })
        .collect()
}

fn canonical_discovery_term(value: &str) -> Option<&str> {
    let value = value.trim();
    if canonical_discovery_term_tail(value, "genre").is_some()
        || canonical_discovery_term_tail(value, "theme").is_some()
    {
        Some(value)
    } else {
        None
    }
}

fn canonical_discovery_term_tail<'a>(value: &'a str, kind: &str) -> Option<&'a str> {
    let value = value.trim();
    let mut parts = value.splitn(3, ':');
    if !parts.next()?.eq_ignore_ascii_case("canonical") {
        return None;
    }
    if !parts.next()?.eq_ignore_ascii_case(kind) {
        return None;
    }
    let tail = parts.next()?.trim();
    if tail.is_empty() {
        return None;
    }
    Some(tail)
}

fn canonical_discovery_facet_label(value: &str, kind: &str) -> Option<String> {
    canonical_discovery_term_tail(value, kind).map(format_canonical_discovery_label)
}

fn format_canonical_discovery_label(value: &str) -> String {
    value
        .split(|character: char| {
            character == '-' || character == '_' || character == ':' || character.is_whitespace()
        })
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().collect::<String>();
                    word.extend(characters.flat_map(char::to_lowercase));
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn discovery_json_signal_values(values: &[JsonValue]) -> Vec<String> {
    let mut signals = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        for signal in unique_json_text_values(value) {
            let key = normalize_discovery_filter_value(&signal);
            if !key.is_empty() && seen.insert(key) {
                signals.push(signal);
            }
        }
    }
    signals
}

fn discovery_rank_component_records(values: &[JsonValue]) -> Vec<DiscoveryRankComponentRecord> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| DiscoveryRankComponentRecord {
            component_index: index as i32,
            component_name: json_object_string(value, &["name", "key", "component", "type"]),
            component_value: json_object_string(
                value,
                &["value", "score", "weight", "contribution"],
            )
            .or_else(|| unique_json_text_values(value).first().cloned()),
        })
        .collect()
}

fn json_object_string(value: &JsonValue, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(json_scalar_string)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn json_scalar_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => Some(value.clone()),
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn unique_json_text_values(value: &JsonValue) -> Vec<String> {
    let mut values = Vec::new();
    collect_json_text_values(value, &mut values);
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                return None;
            }
            let key = normalize_discovery_filter_value(&value);
            seen.insert(key).then_some(value)
        })
        .collect()
}

pub(crate) fn pending_context_changes_need_snapshot_reconciliation(
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> bool {
    match pending_context_changes_resolved_key_count(pending_changes) {
        Ok(count) => count > DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS,
        Err(_) => true,
    }
}

pub(crate) fn pending_context_changes_resolved_key_count(
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> AppResult<usize> {
    let mut keys = BTreeSet::new();
    for change in pending_changes {
        let change_type = discovery_change_type_from_str(&change.change_type)?;
        match change_type {
            DiscoveryContextChangeType::Added | DiscoveryContextChangeType::Updated => {
                keys.insert(required_pending_context_subject_key(change)?);
            }
            DiscoveryContextChangeType::Removed => {
                keys.insert(
                    change
                        .previous_subject_key
                        .clone()
                        .or_else(|| change.subject_key.clone())
                        .ok_or_else(|| {
                            AppError::Validation(format!(
                                "pending discovery removal {} is missing subject key",
                                change.id
                            ))
                        })?,
                );
            }
            DiscoveryContextChangeType::Rematched => {
                keys.insert(required_pending_context_subject_key(change)?);
                keys.insert(change.previous_subject_key.clone().ok_or_else(|| {
                    AppError::Validation(format!(
                        "pending discovery rematch {} is missing previous subject key",
                        change.id
                    ))
                })?);
            }
        }
    }
    Ok(keys.len())
}

fn required_pending_context_subject_key(
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<String> {
    change.subject_key.clone().ok_or_else(|| {
        AppError::Validation(format!(
            "pending discovery change {} is missing subject key",
            change.id
        ))
    })
}

fn changed_subject_from_pending(
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<DiscoveryContextChangedSubjectInput> {
    let raw_subject = change.raw_subject_json.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "pending discovery change {} is missing raw subject JSON",
            change.id
        ))
    })?;
    let subject = serde_json::from_str::<DiscoverySubjectInput>(raw_subject).map_err(|error| {
        AppError::Validation(format!(
            "pending discovery change {} has invalid raw subject JSON: {error}",
            change.id
        ))
    })?;
    let previous_subject = change
        .raw_previous_subject_json
        .as_deref()
        .map(|raw| {
            serde_json::from_str::<DiscoverySubjectInput>(raw).map_err(|error| {
                AppError::Validation(format!(
                    "pending discovery change {} has invalid previous subject JSON: {error}",
                    change.id
                ))
            })
        })
        .transpose()?;
    Ok(DiscoveryContextChangedSubjectInput {
        subject,
        change_type: discovery_change_type_from_str(&change.change_type)?,
        previous_subject,
    })
}

fn discovery_change_type_from_str(value: &str) -> AppResult<DiscoveryContextChangeType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "added" | "add" => Ok(DiscoveryContextChangeType::Added),
        "updated" | "update" => Ok(DiscoveryContextChangeType::Updated),
        "removed" | "delete" | "deleted" => Ok(DiscoveryContextChangeType::Removed),
        "rematched" | "rematch" => Ok(DiscoveryContextChangeType::Rematched),
        other => Err(AppError::Validation(format!(
            "unsupported discovery context change type {other}"
        ))),
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn non_identifier_discovery_title(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() || value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let mut parts = value.splitn(3, ':');
    let Some(provider) = parts.next() else {
        return Some(value);
    };
    let Some(kind) = parts.next() else {
        return Some(value);
    };
    let Some(_) = parts.next() else {
        return Some(value);
    };
    let source_like = !provider.is_empty()
        && provider
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'))
        && provider
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        && !kind.is_empty()
        && kind
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'));
    (!source_like).then_some(value)
}

fn discovery_display_title(item: &DiscoveryTitle) -> Option<String> {
    non_identifier_discovery_title(&item.display_title)
        .or_else(|| non_identifier_discovery_title(&item.original_title))
        .map(str::to_string)
}

fn discovery_sort_title(item: &DiscoveryTitle) -> Option<String> {
    let title = discovery_display_title(item)?;
    let sort_title = title_catalog_sort_input(&title);
    non_identifier_discovery_title(&sort_title)
        .or_else(|| non_identifier_discovery_title(&title))
        .map(str::to_string)
}

fn discovery_json_error(error: serde_json::Error) -> AppError {
    AppError::Repository(format!("failed to encode discovery payload JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TitleExternalRating;
    use chrono::{TimeZone, Utc};
    use scryer_domain::{ExternalId, MediaFacet};

    #[test]
    fn catalog_public_top_section_prefers_anime_this_week() {
        let mut sections = vec![
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "trending_now".to_string(),
                ..Default::default()
            },
            CatalogDiscoverySectionCandidatesRecord {
                section_id: CATALOG_ANIME_WEEKLY_SECTION_ID.to_string(),
                ..Default::default()
            },
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "popular_right_now".to_string(),
                ..Default::default()
            },
        ];

        let top = catalog_public_top_section(&mut sections, "anime")
            .expect("anime weekly section should be selected");

        assert_eq!(top.section_id, CATALOG_ANIME_WEEKLY_SECTION_ID);
        assert_eq!(
            sections
                .iter()
                .map(|section| section.section_id.as_str())
                .collect::<Vec<_>>(),
            vec!["trending_now", "popular_right_now"]
        );
    }

    #[test]
    fn catalog_anime_public_policy_never_falls_back_to_generic_trending() {
        let mut sections = vec![
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "trending_now".to_string(),
                ..Default::default()
            },
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "popular_series".to_string(),
                ..Default::default()
            },
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "popular_right_now".to_string(),
                ..Default::default()
            },
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "new_on_streaming".to_string(),
                ..Default::default()
            },
        ];

        catalog_filter_anime_public_sections(&mut sections);
        let top = catalog_public_top_section(&mut sections, "anime")
            .expect("a remaining Anime public section should become the lead");

        assert_eq!(top.section_id, "popular_right_now");
        assert_eq!(
            sections
                .iter()
                .map(|section| section.section_id.as_str())
                .collect::<Vec<_>>(),
            vec!["new_on_streaming"]
        );
    }

    #[test]
    fn catalog_public_top_section_keeps_first_section_for_non_anime() {
        let mut sections = vec![
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "trending_now".to_string(),
                ..Default::default()
            },
            CatalogDiscoverySectionCandidatesRecord {
                section_id: CATALOG_ANIME_WEEKLY_SECTION_ID.to_string(),
                ..Default::default()
            },
        ];

        let top = catalog_public_top_section(&mut sections, "movie")
            .expect("first public section should be selected");

        assert_eq!(top.section_id, "trending_now");
    }

    #[test]
    fn catalog_public_top_group_keeps_source_label_and_refills_after_deduplication() {
        let duplicate = test_discovery_item("already-shown", "movie", Some("movie"));
        let first_unique = test_discovery_item("first-unique", "movie", Some("movie"));
        let second_unique = test_discovery_item("second-unique", "movie", Some("movie"));
        let mut emitted_item_keys =
            HashSet::from([discovery_item_identity_key(&duplicate).to_string()]);

        let group = catalog_public_top_group(
            CatalogDiscoverySectionCandidatesRecord {
                section_id: "trending_now".to_string(),
                section_type: "TRENDING_NOW".to_string(),
                title: Some("Trending Now".to_string()),
                total_count: 3,
                items: vec![duplicate, first_unique, second_unique],
            },
            "movie",
            2,
            &mut emitted_item_keys,
        )
        .expect("remaining candidates should produce a group");

        assert_eq!(group.label_value.as_deref(), Some("Trending Now"));
        assert_eq!(group.total_count, 3);
        assert_eq!(group.items.len(), 2);
        assert_eq!(
            group
                .items
                .iter()
                .map(|item| item.target_key.as_str())
                .collect::<Vec<_>>(),
            vec!["movie:first-unique", "movie:second-unique"]
        );
    }

    #[test]
    fn pending_context_change_coalescing_drops_add_then_delete() {
        let existing = test_pending_change("change-1", "added", 1, 10);
        let incoming = test_pending_change("change-1", "removed", 2, 10);

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed");

        assert!(merged.is_none());
    }

    #[test]
    fn pending_context_change_coalescing_preserves_added_and_first_seen() {
        let existing = test_pending_change("change-1", "added", 1, 10);
        let incoming = test_pending_change("change-1", "updated", 4, 11);

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed")
            .expect("change should remain pending");

        assert_eq!(merged.change_type, "added");
        assert_eq!(merged.first_seen_sequence, Some(1));
        assert_eq!(merged.last_seen_sequence, Some(4));
        assert_eq!(merged.previous_subject_key, None);
    }

    #[test]
    fn pending_context_change_coalescing_update_then_delete_becomes_removed() {
        let existing = test_pending_change("change-1", "updated", 1, 10);
        let incoming = test_pending_change("change-1", "removed", 4, 10);

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed")
            .expect("change should remain pending");

        assert_eq!(merged.change_type, "removed");
        assert_eq!(
            merged.previous_subject_key.as_deref(),
            Some("tmdb:movie:10")
        );
        assert_eq!(merged.first_seen_sequence, Some(1));
        assert_eq!(merged.last_seen_sequence, Some(4));
    }

    #[test]
    fn pending_context_change_coalescing_rematch_preserves_previous_subject() {
        let existing = test_pending_change("change-1", "updated", 1, 10);
        let mut incoming = test_pending_change("change-1", "rematched", 4, 11);
        incoming.previous_subject_key = Some("tmdb:movie:10".to_string());
        incoming.raw_previous_subject_json = existing.raw_subject_json.clone();

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed")
            .expect("change should remain pending");

        assert_eq!(merged.change_type, "rematched");
        assert_eq!(merged.subject_key.as_deref(), Some("tmdb:movie:11"));
        assert_eq!(
            merged.previous_subject_key.as_deref(),
            Some("tmdb:movie:10")
        );
        assert_eq!(merged.first_seen_sequence, Some(1));
        assert_eq!(merged.last_seen_sequence, Some(4));
    }

    #[test]
    fn discovery_context_fingerprint_is_stable_across_title_and_external_id_order() {
        let left = build_discovery_library_context(
            &[
                test_title(
                    "series",
                    "The Example Show",
                    MediaFacet::Series,
                    vec![("tmdb_tv", "456"), ("thetvdb", "tvdb:123")],
                ),
                test_title(
                    "anime",
                    "Example Anime",
                    MediaFacet::Anime,
                    vec![("myanimelist", "7"), ("anilist:anime", "9")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );
        let right = build_discovery_library_context(
            &[
                test_title(
                    "anime",
                    "Example Anime",
                    MediaFacet::Anime,
                    vec![("anilist_anime", "9"), ("mal", "7")],
                ),
                test_title(
                    "series",
                    "The Example Show",
                    MediaFacet::Series,
                    vec![("tvdb_series", "123"), ("themoviedb", "456")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );

        assert_eq!(left.fingerprint, right.fingerprint);
        assert_eq!(left.subjects, right.subjects);
    }

    #[test]
    fn discovery_context_only_builds_subjects_with_smg_supported_ids() {
        let mut imdb_only = test_title(
            "imdb-only",
            "IMDb Only",
            MediaFacet::Movie,
            vec![("imdb", "tt0133093")],
        );
        imdb_only.imdb_id = Some("tt0133093".to_string());

        let context = build_discovery_library_context(
            &[
                imdb_only,
                test_title(
                    "unsupported",
                    "Unsupported",
                    MediaFacet::Movie,
                    vec![("otherdb", "100")],
                ),
                test_title(
                    "movie",
                    "The Example Movie",
                    MediaFacet::Movie,
                    vec![("tmdb_movie", "movie:603")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );

        assert_eq!(context.subjects.len(), 1);
        assert_eq!(context.subjects[0].title_id, "movie");
        assert_eq!(context.subjects[0].subject_key, "tmdb:movie:603");
        assert_eq!(context.subjects[0].subject.tmdb_id, Some(603));
        assert_eq!(
            context.subjects[0].subject.external_ids,
            vec![DiscoveryExternalIdInput {
                source: "tmdb".to_string(),
                value: "603".to_string(),
            }]
        );
    }

    #[test]
    fn discovery_context_uses_unique_typed_ids_and_keeps_external_ids() {
        let context = build_discovery_library_context(
            &[test_title(
                "series",
                "Series",
                MediaFacet::Series,
                vec![("tvdb", "10"), ("thetvdb", "11"), ("tmdb", "20")],
            )],
            DiscoveryContextDefaults::default(),
        );

        let subject = &context.subjects[0].subject;
        assert_eq!(context.subjects[0].subject_key, "tmdb:series:20");
        assert_eq!(subject.tvdb_id, None);
        assert_eq!(subject.tmdb_id, Some(20));
        assert_eq!(subject.kind.as_deref(), Some("series"));
        assert_eq!(subject.facet.as_deref(), Some("series"));
        assert_eq!(
            subject.external_ids,
            vec![
                DiscoveryExternalIdInput {
                    source: "tmdb".to_string(),
                    value: "20".to_string(),
                },
                DiscoveryExternalIdInput {
                    source: "tvdb".to_string(),
                    value: "10".to_string(),
                },
                DiscoveryExternalIdInput {
                    source: "tvdb".to_string(),
                    value: "11".to_string(),
                },
            ]
        );
    }

    #[test]
    fn discovery_context_uses_series_resolver_kind_for_anime_subjects() {
        let context = build_discovery_library_context(
            &[test_title(
                "anime",
                "Anime",
                MediaFacet::Anime,
                vec![("tvdb", "100"), ("mal", "200")],
            )],
            DiscoveryContextDefaults::default(),
        );

        let subject = &context.subjects[0].subject;
        assert_eq!(context.subjects[0].subject_key, "tvdb:series:100");
        assert_eq!(subject.kind.as_deref(), Some("series"));
        assert_eq!(subject.facet.as_deref(), Some("anime"));
        assert_eq!(subject.tvdb_id, Some(100));
        assert_eq!(subject.mal_id, Some(200));
    }

    #[test]
    fn discovery_home_public_sections_filter_owned_catalog_titles() {
        let owned_visibility = CatalogOwnedVisibility::from_titles(&[test_title(
            "house-of-the-dragon",
            "House of the Dragon",
            MediaFacet::Series,
            vec![("tvdb", "371572")],
        )]);
        let mut owned_item = test_discovery_item("owned", "series", Some("series"));
        owned_item.target_key = "tvdb:series:371572".to_string();
        owned_item.display_title = "House of the Dragon".to_string();
        let mut visible_item = test_discovery_item("visible", "series", Some("series"));
        visible_item.target_key = "tmdb:series:100".to_string();
        visible_item.display_title = "Visible".to_string();
        let mut refill_item = test_discovery_item("refill", "series", Some("series"));
        refill_item.target_key = "tmdb:series:101".to_string();
        refill_item.display_title = "Refill".to_string();
        let visibility = DiscoveryVisibility {
            allowed_media_kinds: HashSet::from(["series"]),
            ..DiscoveryVisibility::default()
        };

        let sections = filter_discovery_sections_for_owned_items(
            vec![DiscoverySectionResult {
                section_id: "trending_now".to_string(),
                section_type: "TRENDING_NOW".to_string(),
                title: "Top Series This Week".to_string(),
                surface: "public".to_string(),
                total_count: 3,
                items: vec![owned_item, visible_item, refill_item],
            }],
            &owned_visibility,
            &visibility,
            2,
        );

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].total_count, 2);
        assert_eq!(
            sections[0]
                .items
                .iter()
                .map(|item| item.display_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Visible", "Refill"]
        );
    }

    #[test]
    fn discovery_home_top_rated_prefers_external_rating_provenance_and_dedupes() {
        let mut scalar_only = test_discovery_item("scalar", "movie", Some("movie"));
        scalar_only.source_run_kind = "public_feed".to_string();
        scalar_only.target_key = "tmdb:movie:scalar".to_string();
        scalar_only.rating = Some(10.0);
        scalar_only.rank_score = Some(100.0);

        let mut weaker_public_duplicate =
            test_discovery_item("shared-public", "movie", Some("movie"));
        weaker_public_duplicate.source_run_kind = "public_feed".to_string();
        weaker_public_duplicate.target_key = "tmdb:movie:shared".to_string();
        weaker_public_duplicate.rating = Some(6.0);
        weaker_public_duplicate.rank_score = Some(1.0);

        let mut external_rated = test_discovery_item("shared-context", "movie", Some("movie"));
        external_rated.target_key = "tmdb:movie:shared".to_string();
        external_rated.rating = Some(5.0);
        external_rated.external_ratings = vec![TitleExternalRating {
            source: "imdb".to_string(),
            value: Some(8.8),
            score: Some(8.8),
            normalized: 0.88,
            votes: Some(100_000),
            url: "https://imdb.com/title/tt0000001".to_string(),
        }];

        let section = top_rated_discovery_home_section(
            &[scalar_only, weaker_public_duplicate, external_rated],
            &[],
            true,
            10,
        )
        .expect("top rated section");

        assert_eq!(section.section_type, "TOP_RATED");
        assert_eq!(section.total_count, 2);
        assert_eq!(
            section
                .items
                .iter()
                .map(|item| item.target_key.as_str())
                .collect::<Vec<_>>(),
            vec!["tmdb:movie:shared", "tmdb:movie:scalar"]
        );
    }

    #[test]
    fn discovery_home_top_rated_keeps_short_sections() {
        let mut only_item = test_discovery_item("only", "series", Some("series"));
        only_item.source_run_kind = "public_feed".to_string();
        only_item.target_key = "tmdb:series:only".to_string();
        only_item.rating = Some(7.0);

        let section = top_rated_discovery_home_section(&[only_item], &[], true, 6)
            .expect("top rated section");

        assert_eq!(section.total_count, 1);
        assert_eq!(section.items.len(), 1);
        assert_eq!(section.items[0].target_key, "tmdb:series:only");
    }

    #[test]
    fn discovery_home_hero_prefers_visible_personalized_item() {
        let mut public_item = test_discovery_item("public", "movie", Some("movie"));
        public_item.target_key = "tmdb:movie:public".to_string();
        public_item.rating = Some(10.0);
        public_item.rank_score = Some(99.0);
        public_item.source_count = Some(9);
        public_item.background_url = Some("https://images.example/public.jpg".to_string());

        let mut personalized_item = test_discovery_item("personalized", "movie", Some("movie"));
        personalized_item.target_key = "tmdb:movie:personalized".to_string();
        personalized_item.rating = Some(1.0);
        personalized_item.rank_score = Some(1.0);
        personalized_item.matched_subject_count = 1;
        personalized_item.background_url =
            Some("https://images.example/personalized.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[test_discovery_section("public", vec![public_item])],
            &[test_discovery_section(
                "personalized",
                vec![personalized_item],
            )],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:personalized");
    }

    #[test]
    fn discovery_home_hero_ignores_public_items_inside_mixed_personalized_sections() {
        let mut public_item = test_discovery_item("public", "movie", Some("movie"));
        public_item.source_run_kind = "public_feed".to_string();
        public_item.target_key = "tmdb:movie:public".to_string();
        public_item.rating = Some(10.0);
        public_item.rank_score = Some(100.0);
        public_item.background_url = Some("https://images.example/public.jpg".to_string());

        let mut personalized_item = test_discovery_item("personalized", "movie", Some("movie"));
        personalized_item.source_run_kind = "context_snapshot".to_string();
        personalized_item.target_key = "tmdb:movie:personalized".to_string();
        personalized_item.matched_subject_count = 1;
        personalized_item.rating = Some(1.0);
        personalized_item.rank_score = Some(1.0);
        personalized_item.background_url =
            Some("https://images.example/personalized.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[],
            &[test_discovery_section(
                "top_rated",
                vec![public_item, personalized_item],
            )],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:personalized");
    }

    #[test]
    fn discovery_home_hero_skips_owned_personalized_items() {
        let mut owned_item = test_discovery_item("owned", "series", Some("series"));
        owned_item.target_key = "tmdb:series:owned".to_string();
        owned_item.owned_in_input = true;
        owned_item.matched_subject_count = 100;
        owned_item.background_url = Some("https://images.example/owned.jpg".to_string());

        let mut visible_item = test_discovery_item("visible", "series", Some("series"));
        visible_item.target_key = "tmdb:series:visible".to_string();
        visible_item.matched_subject_count = 1;
        visible_item.background_url = Some("https://images.example/visible.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[],
            &[test_discovery_section(
                "personalized",
                vec![owned_item, visible_item],
            )],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:series:visible");
    }

    #[test]
    fn discovery_home_hero_falls_back_to_highest_ranked_public_item() {
        // The public hero now mirrors the personalized philosophy: rank_score
        // leads and a bare rating only breaks ties. A strongly ranked item wins
        // even against a rival carrying a higher but un-corroborated rating, so a
        // lone inflated rating can no longer commandeer the hero slot.
        let mut higher_ranked = test_discovery_item("higher-rank", "movie", Some("movie"));
        higher_ranked.target_key = "tmdb:movie:higher-rank".to_string();
        higher_ranked.rating = Some(6.0);
        higher_ranked.rank_score = Some(100.0);
        higher_ranked.background_url = Some("https://images.example/higher-rank.jpg".to_string());

        let mut higher_rating = test_discovery_item("higher-rating", "movie", Some("movie"));
        higher_rating.target_key = "tmdb:movie:higher-rating".to_string();
        higher_rating.rating = Some(8.5);
        higher_rating.rank_score = Some(1.0);
        higher_rating.background_url = Some("https://images.example/higher-rating.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[test_discovery_section(
                "public",
                vec![higher_rating, higher_ranked],
            )],
            &[],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:higher-rank");
    }

    #[test]
    fn discovery_home_hero_breaks_public_rank_ties_by_rating() {
        // With equal rank_score the credible-rating tiebreak decides, so a
        // healthy rating still wins when the ranking signal is level.
        let mut lower_rated = test_discovery_item("lower", "movie", Some("movie"));
        lower_rated.source_run_kind = "public_feed".to_string();
        lower_rated.target_key = "tmdb:movie:lower".to_string();
        lower_rated.rating = Some(6.0);
        lower_rated.rank_score = Some(10.0);
        lower_rated.background_url = Some("https://images.example/lower.jpg".to_string());

        let mut higher_rated = test_discovery_item("higher", "movie", Some("movie"));
        higher_rated.source_run_kind = "public_feed".to_string();
        higher_rated.target_key = "tmdb:movie:higher".to_string();
        higher_rated.rating = Some(8.5);
        higher_rated.rank_score = Some(10.0);
        higher_rated.background_url = Some("https://images.example/higher.jpg".to_string());

        let hero = select_public_discovery_home_hero(&[test_discovery_section(
            "public",
            vec![lower_rated, higher_rated],
        )])
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:higher");
    }

    #[test]
    fn discovery_home_hero_treats_blank_backdrop_as_missing() {
        let mut blank_backdrop = test_discovery_item("blank", "movie", Some("movie"));
        blank_backdrop.target_key = "tmdb:movie:blank".to_string();
        blank_backdrop.rating = Some(10.0);
        blank_backdrop.rank_score = Some(100.0);
        blank_backdrop.background_url = Some("   ".to_string());

        let mut real_backdrop = test_discovery_item("real", "movie", Some("movie"));
        real_backdrop.target_key = "tmdb:movie:real".to_string();
        real_backdrop.rating = Some(1.0);
        real_backdrop.rank_score = Some(1.0);
        real_backdrop.background_url = Some("https://images.example/real.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[test_discovery_section(
                "public",
                vec![blank_backdrop, real_backdrop],
            )],
            &[],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:real");
    }

    #[test]
    fn discovery_home_hero_requires_a_backdrop() {
        let item = test_discovery_item("poster-only", "movie", Some("movie"));

        let hero = select_discovery_home_hero(&[test_discovery_section("public", vec![item])], &[]);

        assert!(hero.is_none());
    }

    #[test]
    fn discovery_home_hero_tie_breaks_by_target_key_without_raw_labels() {
        let mut later_key = test_discovery_item("later", "anime", Some("anime"));
        later_key.target_key = "tmdb:anime:z".to_string();
        later_key.background_url = Some("https://images.example/z.jpg".to_string());
        later_key.source_tags = vec![DiscoverySourceTagRecord {
            category: Some("theme".to_string()),
            name: Some("Isekai".to_string()),
            values: vec!["Isekai".to_string()],
        }];

        let mut earlier_key = test_discovery_item("earlier", "anime", Some("anime"));
        earlier_key.target_key = "tmdb:anime:a".to_string();
        earlier_key.background_url = Some("https://images.example/a.jpg".to_string());
        earlier_key.facet_terms = vec!["canonical:theme:isekai".to_string()];

        let hero = select_discovery_home_hero(
            &[test_discovery_section(
                "public",
                vec![later_key, earlier_key],
            )],
            &[],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:anime:a");
    }

    #[test]
    fn discovery_context_deduplicates_identical_subjects() {
        let context = build_discovery_library_context(
            &[
                test_title(
                    "library-a",
                    "Movie A",
                    MediaFacet::Movie,
                    vec![("tmdb", "603")],
                ),
                test_title(
                    "library-b",
                    "Movie B",
                    MediaFacet::Movie,
                    vec![("tmdb_movie", "603")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );

        assert_eq!(context.subjects.len(), 1);
        assert_eq!(context.subjects[0].title_id, "library-a");
    }

    #[test]
    fn discovery_context_fallback_key_uses_external_id_priority_after_ambiguous_typed_ids() {
        let context = build_discovery_library_context(
            &[test_title(
                "anime",
                "Anime",
                MediaFacet::Anime,
                vec![
                    ("mal", "200"),
                    ("myanimelist", "201"),
                    ("anidb", "10"),
                    ("anidb", "11"),
                ],
            )],
            DiscoveryContextDefaults::default(),
        );

        let subject = &context.subjects[0].subject;
        assert_eq!(subject.mal_id, None);
        assert_eq!(subject.anidb_id, None);
        assert_eq!(context.subjects[0].subject_key, "anidb:anime:10");
    }

    #[test]
    fn title_recommendations_subject_prefers_tvdb_then_tmdb_then_imdb() {
        let title = test_title(
            "movie",
            "Movie",
            MediaFacet::Movie,
            vec![("imdb", "tt0133093"), ("tmdb", "603"), ("tvdb", "78874")],
        );

        let (subject, source_target_keys) =
            title_recommendations_subject(&title, &[]).expect("subject should build");

        assert_eq!(subject.key.as_deref(), Some("tvdb:movie:78874"));
        assert_eq!(subject.tvdb_id, Some(78874));
        assert_eq!(subject.tmdb_id, Some(603));
        assert!(
            source_target_keys
                .iter()
                .any(|key| key == "imdb:title:tt0133093")
        );
        assert!(
            subject
                .external_ids
                .iter()
                .any(|external_id| external_id.source == "imdb")
        );
    }

    #[test]
    fn title_recommendations_subject_uses_anime_ids_after_tvdb_tmdb() {
        let tvdb_title = test_title(
            "anime-tvdb",
            "Anime",
            MediaFacet::Anime,
            vec![("mal", "200"), ("anidb", "10"), ("tvdb", "100")],
        );
        let (subject, _) =
            title_recommendations_subject(&tvdb_title, &[]).expect("subject should build");
        assert_eq!(subject.key.as_deref(), Some("tvdb:series:100"));

        let anime_id_title = test_title(
            "anime-mal",
            "Anime",
            MediaFacet::Anime,
            vec![("anidb", "10"), ("mal", "200"), ("anilist", "300")],
        );
        let (subject, _) =
            title_recommendations_subject(&anime_id_title, &[]).expect("subject should build");
        assert_eq!(subject.key.as_deref(), Some("mal:anime:200"));
    }

    #[test]
    fn discovery_item_records_do_not_persist_smg_resolved_title_id_as_local_fk() {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        let item = DiscoveryTitle {
            target_key: "tmdb:movie:603".to_string(),
            target_kind: "movie".to_string(),
            resolved: true,
            resolved_title_id: "smg-title-603".to_string(),
            display_title: "The Example".to_string(),
            ..DiscoveryTitle::default()
        };

        let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
            .expect("discovery item records should build");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resolved_title_id, None);
    }

    #[test]
    fn discovery_item_records_derive_local_sort_title_from_human_title() {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        let item = DiscoveryTitle {
            target_key: "tvdb:movie:603".to_string(),
            target_kind: "movie".to_string(),
            resolved: true,
            display_title: "tvdb:movie:603".to_string(),
            original_title: "\u{ff34}\u{ff48}\u{ff45} Matrix".to_string(),
            ..DiscoveryTitle::default()
        };

        let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
            .expect("discovery item records should build");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].display_title, "\u{ff34}\u{ff48}\u{ff45} Matrix");
        assert_eq!(records[0].sort_title.as_deref(), Some("Matrix"));
    }

    #[test]
    fn discovery_item_records_wire_canonical_genre_and_theme_terms() {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        let item = DiscoveryTitle {
            target_key: "tmdb:movie:603".to_string(),
            target_kind: "movie".to_string(),
            resolved: true,
            display_title: "The Example".to_string(),
            source_tags: vec![
                serde_json::json!({
                    "source": "mal",
                    "category": "theme",
                    "name": "mal:theme:psychological",
                    "canonical": "canonical:theme:psychological"
                }),
                serde_json::json!("canonical:theme:survival"),
            ],
            canonical_tags: vec![
                serde_json::json!({
                    "key": "canonical:genre:action",
                    "category": "genre",
                    "name": "action",
                    "confidence": 1.0,
                }),
                serde_json::json!({
                    "key": "canonical:genre:drama",
                    "category": "genre",
                    "name": "Drama",
                    "confidence": 1.0,
                }),
                serde_json::json!({
                    "key": "canonical:theme:isekai",
                    "category": "theme",
                    "name": "Isekai",
                    "confidence": 1.0,
                }),
                serde_json::json!({
                    "key": "adult-cast",
                    "category": "theme",
                    "name": "Adult Cast",
                    "confidence": 1.0,
                }),
            ],
            facet_terms: vec![
                "raw:compat".to_string(),
                "canonical:genre:drama".to_string(),
            ],
            ..DiscoveryTitle::default()
        };

        let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
            .expect("discovery item records should build");

        assert_eq!(records.len(), 1);
        assert!(records[0].facet_terms.contains(&"raw:compat".to_string()));
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:genre:action".to_string())
        );
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:genre:drama".to_string())
        );
        assert_eq!(
            records[0]
                .facet_terms
                .iter()
                .filter(|term| term.as_str() == "canonical:genre:action")
                .count(),
            1
        );
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:theme:isekai".to_string())
        );
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:theme:adult-cast".to_string())
        );
        assert!(
            !records[0]
                .facet_terms
                .contains(&"canonical:theme:psychological".to_string())
        );
    }

    #[test]
    fn discovery_item_genre_query_uses_canonical_facet_terms() {
        fn matches_genre(item: &DiscoveryItemRecord, genre: &str) -> bool {
            item_matches_discovery_items_query(
                item,
                &DiscoveryItemsQuery {
                    genres: vec![genre.to_string()],
                    include_unresolved: false,
                    ..DiscoveryItemsQuery::default()
                },
            )
        }

        let mut item = test_discovery_item("canonical", "movie", Some("movie"));
        item.facet_terms = vec!["canonical:genre:action".to_string()];

        assert!(matches_genre(&item, "Action"));
        assert!(matches_genre(&item, "canonical:genre:action"));
        assert!(!matches_genre(&item, "Drama"));
    }

    #[test]
    fn discovery_item_media_kind_uses_v1_content_type_contract() {
        fn matches_target_kind(item: &DiscoveryItemRecord, target_kind: &str) -> bool {
            item_matches_discovery_items_query(
                item,
                &DiscoveryItemsQuery {
                    target_kinds: vec![target_kind.to_string()],
                    include_unresolved: false,
                    ..DiscoveryItemsQuery::default()
                },
            )
        }

        let anime = test_discovery_item("anime", "series", Some("anime"));
        assert!(matches_target_kind(&anime, "anime"));
        assert!(!matches_target_kind(&anime, "series"));

        let series = test_discovery_item("series", "series", Some("series"));
        assert!(matches_target_kind(&series, "series"));
        assert!(!matches_target_kind(&series, "anime"));

        let movie = test_discovery_item("movie", "movie", Some("movie"));
        assert!(matches_target_kind(&movie, "movie"));
        assert!(!matches_target_kind(&movie, "series"));

        let fallback = test_discovery_item("fallback", "anime", Some(""));
        assert!(matches_target_kind(&fallback, "anime"));
        assert!(!matches_target_kind(&fallback, "series"));

        let unknown = test_discovery_item("unknown", "series", Some("tv"));
        assert!(!matches_target_kind(&unknown, "series"));
        assert!(!matches_target_kind(&unknown, "anime"));
    }

    #[test]
    fn personalized_sections_dedupe_derived_items_and_require_subject_match() {
        fn discovery_item(
            id: &str,
            title: &str,
            genre_labels: &[&str],
            rank_score: f64,
            matched_subject_count: i32,
        ) -> DiscoveryItemRecord {
            let mut item = test_discovery_item(id, "movie", Some("movie"));
            item.target_key = format!("tmdb:movie:{id}");
            item.display_title = title.to_string();
            item.sort_title = Some(title.to_string());
            item.facet_terms = genre_labels
                .iter()
                .map(|genre| format!("canonical:genre:{}", genre.to_ascii_lowercase()))
                .collect();
            item.rank_score = Some(rank_score);
            item.matched_subject_count = matched_subject_count;
            item
        }

        let profile = DiscoveryLibraryAffinityProfile {
            genre_labels: vec!["Adventure".to_string(), "Animation".to_string()],
            tag_labels: Vec::new(),
        };
        let items = vec![
            discovery_item("1", "Shared Match", &["Adventure", "Animation"], 100.0, 1),
            discovery_item("2", "Unlinked Animation", &["Animation"], 95.0, 0),
            discovery_item("3", "Adventure Match", &["Adventure"], 90.0, 1),
            discovery_item("4", "Animation Match", &["Animation"], 80.0, 1),
        ];

        let sections = personalized_section_results(&items, &profile, true, 10);
        let adventure = sections
            .iter()
            .find(|section| section.title == "Because You Like Adventure")
            .expect("adventure section");
        let animation = sections
            .iter()
            .find(|section| section.title == "Because You Like Animation")
            .expect("animation section");

        assert_eq!(
            adventure
                .items
                .iter()
                .map(|item| item.display_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Shared Match", "Adventure Match"]
        );
        assert_eq!(
            animation
                .items
                .iter()
                .map(|item| item.display_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Animation Match"]
        );

        let mut seen = HashSet::new();
        for item in sections.iter().flat_map(|section| section.items.iter()) {
            assert!(
                seen.insert(discovery_item_identity_key(item).to_string()),
                "duplicate discovery item {} in derived sections",
                item.display_title
            );
        }
    }

    fn affinity_test_item(
        id: &str,
        content_type: &str,
        facet_terms: &[&str],
    ) -> DiscoveryItemRecord {
        let target_kind = if content_type == "movie" {
            "movie"
        } else {
            "series"
        };
        let mut item = test_discovery_item(id, target_kind, Some(content_type));
        item.target_key = format!("tmdb:{content_type}:{id}");
        item.display_title = format!("Title {id}");
        item.sort_title = Some(format!("Title {id}"));
        item.facet_terms = facet_terms.iter().map(|term| (*term).to_string()).collect();
        item.matched_subject_count = 1;
        item
    }

    fn affinity_section_titles(section: &DiscoverySectionResult) -> Vec<&str> {
        section
            .items
            .iter()
            .map(|item| item.display_title.as_str())
            .collect()
    }

    #[test]
    fn personalized_sections_retire_medium_and_library_rails() {
        // Medium is owned by the dashboard's facet chips, so the per-medium
        // "For You" rails and BECAUSE_YOU_HAVE are gone. Eight items per medium
        // clears the retired medium rails' old six-item floor, every item has a
        // matched subject so BECAUSE_YOU_HAVE would have qualified too, and the
        // small limit leaves plenty of unclaimed items for them: this fixture
        // emitted all four retired sections before the change.
        let profile = DiscoveryLibraryAffinityProfile::default();
        let mut items = Vec::new();
        for index in 0..8 {
            items.push(affinity_test_item(&format!("m{index}"), "movie", &[]));
            items.push(affinity_test_item(&format!("s{index}"), "series", &[]));
            items.push(affinity_test_item(&format!("a{index}"), "anime", &[]));
        }

        let sections = personalized_section_results(&items, &profile, true, 5);
        let section_types = sections
            .iter()
            .map(|section| section.section_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(section_types, vec!["FOR_YOU"]);
    }

    #[test]
    fn personalized_sections_prefer_specific_tag_rail_over_broad_genre_rail() {
        // Composition order is dedupe priority. A title that earns the narrow
        // "Because You Like Isekai" theme rail must not be eaten first by the
        // far broader "Because You Like Animation" genre rail.
        let profile = DiscoveryLibraryAffinityProfile {
            genre_labels: vec!["Animation".to_string()],
            tag_labels: vec!["Isekai".to_string()],
        };
        let items = vec![
            affinity_test_item(
                "1",
                "movie",
                &["canonical:genre:animation", "canonical:theme:isekai"],
            ),
            affinity_test_item("2", "movie", &["canonical:genre:animation"]),
        ];

        let sections = personalized_section_results(&items, &profile, true, 10);
        let isekai = sections
            .iter()
            .find(|section| section.title == "Because You Like Isekai")
            .expect("isekai theme section");
        assert_eq!(isekai.section_type, "BECAUSE_YOU_LIKE_TAG");
        assert_eq!(affinity_section_titles(isekai), vec!["Title 1"]);

        let animation = sections
            .iter()
            .find(|section| section.title == "Because You Like Animation")
            .expect("animation genre section");
        assert_eq!(animation.section_type, "BECAUSE_YOU_LIKE_GENRE");
        assert_eq!(affinity_section_titles(animation), vec!["Title 2"]);

        let tag_index = sections
            .iter()
            .position(|section| section.section_type == "BECAUSE_YOU_LIKE_TAG")
            .expect("tag section index");
        let genre_index = sections
            .iter()
            .position(|section| section.section_type == "BECAUSE_YOU_LIKE_GENRE")
            .expect("genre section index");
        assert!(tag_index < genre_index);
    }

    #[test]
    fn affinity_label_rails_keep_animation_and_anime_apart() {
        // Animation is a medium, anime is a tradition. Both titles carry the
        // canonical `animation` genre facet, so only the media-kind guard keeps
        // Western animation and anime out of each other's rails.
        let profile = DiscoveryLibraryAffinityProfile {
            genre_labels: vec!["Animation".to_string(), "Anime".to_string()],
            tag_labels: Vec::new(),
        };
        let items = vec![
            affinity_test_item("western", "movie", &["canonical:genre:animation"]),
            affinity_test_item(
                "shonen",
                "anime",
                &["canonical:genre:animation", "canonical:genre:anime"],
            ),
        ];

        let sections = personalized_section_results(&items, &profile, true, 10);
        let animation = sections
            .iter()
            .find(|section| section.title == "Because You Like Animation")
            .expect("animation genre section");
        assert_eq!(affinity_section_titles(animation), vec!["Title western"]);

        let anime = sections
            .iter()
            .find(|section| section.title == "Because You Like Anime")
            .expect("anime genre section");
        assert_eq!(affinity_section_titles(anime), vec!["Title shonen"]);
    }

    #[test]
    fn anime_affinity_label_survives_the_generic_label_filter() {
        // Reachability guard: the boundary's "anime" arm is dead code unless a
        // real profile can actually carry the label. The profile is built by
        // `top_owned_title_labels` over owned titles, which drops labels that
        // `discovery_affinity_label_is_generic` rejects - "anime" must not be
        // among them, or "Because You Like Anime" can never exist.
        fn anime_title(id: &str) -> Title {
            let mut title = test_title(id, id, MediaFacet::Series, Vec::new());
            title.canonical_tags = ["Anime", "Animation"]
                .into_iter()
                .map(|name| CanonicalMediaTag {
                    key: format!("canonical:genre:{}", name.to_ascii_lowercase()),
                    category: "genre".to_string(),
                    name: name.to_string(),
                    confidence: None,
                    sources: Vec::new(),
                    source_tag_keys: Vec::new(),
                    is_adult: false,
                    is_spoiler: false,
                })
                .collect();
            title
        }

        let titles = vec![anime_title("a1"), anime_title("a2")];
        let genre_labels = top_owned_title_labels(
            &titles,
            |title| canonical_tag_labels(&title.canonical_tags, "genre"),
            2,
        );
        assert!(
            genre_labels.iter().any(|label| label == "Anime"),
            "anime-heavy library produced no Anime label: {genre_labels:?}"
        );

        // ...and the label, once reachable, splits the two traditions apart.
        let profile = DiscoveryLibraryAffinityProfile {
            genre_labels,
            tag_labels: Vec::new(),
        };
        let items = vec![
            affinity_test_item("western", "movie", &["canonical:genre:animation"]),
            affinity_test_item(
                "shonen",
                "anime",
                &["canonical:genre:animation", "canonical:genre:anime"],
            ),
        ];
        let sections = personalized_section_results(&items, &profile, true, 10);
        let anime = sections
            .iter()
            .find(|section| section.title == "Because You Like Anime")
            .expect("anime genre section should be reachable from a real profile");
        assert_eq!(affinity_section_titles(anime), vec!["Title shonen"]);
        let animation = sections
            .iter()
            .find(|section| section.title == "Because You Like Animation")
            .expect("animation genre section");
        assert_eq!(affinity_section_titles(animation), vec!["Title western"]);
    }

    #[test]
    fn anime_without_content_type_does_not_leak_into_the_animation_rail() {
        // `discovery_item_media_kind` falls back to target_kind, so a
        // content_type-less anime reports as a plain "series". Only the canonical
        // anime genre facet keeps it out of the Western-animation rail.
        let mut untyped_anime = test_discovery_item("untyped", "series", None);
        untyped_anime.target_key = "mal:anime:untyped".to_string();
        untyped_anime.display_title = "Title untyped".to_string();
        untyped_anime.sort_title = Some("Title untyped".to_string());
        untyped_anime.facet_terms = vec![
            "canonical:genre:animation".to_string(),
            "canonical:genre:anime".to_string(),
        ];
        untyped_anime.matched_subject_count = 1;
        assert_eq!(discovery_item_media_kind(&untyped_anime), Some("series"));

        let profile = DiscoveryLibraryAffinityProfile {
            genre_labels: vec!["Animation".to_string(), "Anime".to_string()],
            tag_labels: Vec::new(),
        };
        let items = vec![
            affinity_test_item("western", "movie", &["canonical:genre:animation"]),
            untyped_anime,
        ];

        let sections = personalized_section_results(&items, &profile, true, 10);
        let animation = sections
            .iter()
            .find(|section| section.title == "Because You Like Animation")
            .expect("animation genre section");
        assert_eq!(affinity_section_titles(animation), vec!["Title western"]);
        let anime = sections
            .iter()
            .find(|section| section.title == "Because You Like Anime")
            .expect("anime genre section");
        assert_eq!(affinity_section_titles(anime), vec!["Title untyped"]);
    }

    fn test_pending_change(
        id: &str,
        change_type: &str,
        sequence: i64,
        tmdb_id: i64,
    ) -> DiscoveryPendingContextChangeRecord {
        let observed_at = Utc.timestamp_opt(sequence, 0).unwrap();
        DiscoveryPendingContextChangeRecord {
            id: id.to_string(),
            scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some(format!("tmdb:movie:{tmdb_id}")),
            previous_subject_key: None,
            change_type: change_type.to_string(),
            title_id: Some(id.to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": tmdb_id,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": tmdb_id.to_string()}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(sequence),
            last_seen_sequence: Some(sequence),
            first_seen_at: observed_at,
            last_seen_at: observed_at,
        }
    }

    fn test_title(
        id: &str,
        name: &str,
        facet: MediaFacet,
        external_ids: Vec<(&str, &str)>,
    ) -> Title {
        Title {
            id: id.to_string(),
            library_id: "library".to_string(),
            name: name.to_string(),
            facet,
            monitored: true,
            tags: Vec::new(),
            canonical_tags: vec![],
            external_ids: external_ids
                .into_iter()
                .map(|(source, value)| ExternalId {
                    source: source.to_string(),
                    value: value.to_string(),
                })
                .collect(),
            root_folder_id: "root".to_string(),
            created_by: None,
            created_at: Utc.timestamp_opt(0, 0).unwrap(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[test]
    fn discovery_home_public_hero_prefers_multi_source_rating_over_single_source_fossil() {
        // Both carry a hero backdrop and an equal, healthy rank_score, so ordering
        // falls through to the credible-rating tiebreak. The single-source 10.0
        // must lose to the multi-source 9.4.
        let mut fossil = test_discovery_item("fossil", "movie", Some("movie"));
        fossil.source_run_kind = "public_feed".to_string();
        fossil.target_key = "tmdb:movie:fossil".to_string();
        fossil.rating = Some(10.0);
        fossil.rating_sources = vec!["trakt".to_string()];
        fossil.rank_score = Some(50.0);
        fossil.background_url = Some("https://images.example/fossil.jpg".to_string());

        let mut credible = test_discovery_item("credible", "movie", Some("movie"));
        credible.source_run_kind = "public_feed".to_string();
        credible.target_key = "tmdb:movie:credible".to_string();
        credible.rating = Some(9.4);
        credible.rating_sources = vec!["imdb".to_string(), "tmdb".to_string(), "trakt".to_string()];
        credible.rank_score = Some(50.0);
        credible.background_url = Some("https://images.example/credible.jpg".to_string());

        let hero = select_public_discovery_home_hero(&[test_discovery_section(
            "public",
            vec![fossil, credible],
        )])
        .expect("public hero item");

        assert_eq!(hero.target_key, "tmdb:movie:credible");
    }

    #[test]
    fn discovery_home_top_rated_demotes_single_source_fossil_below_multi_source() {
        let mut fossil = test_discovery_item("fossil", "movie", Some("movie"));
        fossil.source_run_kind = "public_feed".to_string();
        fossil.target_key = "tmdb:movie:fossil".to_string();
        fossil.rating = Some(10.0);
        fossil.rating_sources = vec!["trakt".to_string()];
        fossil.external_ratings = vec![TitleExternalRating {
            source: "trakt".to_string(),
            value: Some(10.0),
            score: Some(10.0),
            normalized: 1.0,
            votes: Some(1),
            url: String::new(),
        }];

        let mut credible = test_discovery_item("credible", "movie", Some("movie"));
        credible.source_run_kind = "public_feed".to_string();
        credible.target_key = "tmdb:movie:credible".to_string();
        credible.rating = Some(9.4);
        credible.rating_sources = vec!["imdb".to_string(), "tmdb".to_string()];
        credible.external_ratings = vec![
            TitleExternalRating {
                source: "imdb".to_string(),
                value: Some(9.4),
                score: Some(9.4),
                normalized: 0.94,
                votes: Some(500_000),
                url: String::new(),
            },
            TitleExternalRating {
                source: "tmdb".to_string(),
                value: Some(9.2),
                score: Some(9.2),
                normalized: 0.92,
                votes: Some(120_000),
                url: String::new(),
            },
        ];

        let section = top_rated_discovery_home_section(&[fossil, credible], &[], true, 10)
            .expect("top rated section");

        assert_eq!(
            section
                .items
                .iter()
                .map(|item| item.target_key.as_str())
                .collect::<Vec<_>>(),
            vec!["tmdb:movie:credible", "tmdb:movie:fossil"]
        );
    }

    #[test]
    fn discovery_home_top_rated_keeps_vote_backed_single_source_above_multi_source() {
        // A MAL-only anime score built from hundreds of thousands of votes is
        // credible evidence: it must not be demoted below a lower multi-source
        // score just because only one provider carries it.
        let mut mal_only = test_discovery_item("mal-only", "series", Some("anime"));
        mal_only.source_run_kind = "public_feed".to_string();
        mal_only.target_key = "tvdb:series:mal-only".to_string();
        mal_only.rating = Some(9.3);
        mal_only.rating_sources = vec!["mal".to_string()];
        mal_only.external_ratings = vec![TitleExternalRating {
            source: "mal".to_string(),
            value: Some(9.3),
            score: Some(9.3),
            normalized: 0.93,
            votes: Some(500_000),
            url: String::new(),
        }];

        let mut multi_source = test_discovery_item("multi", "movie", Some("movie"));
        multi_source.source_run_kind = "public_feed".to_string();
        multi_source.target_key = "tmdb:movie:multi".to_string();
        multi_source.rating = Some(7.8);
        multi_source.rating_sources = vec!["imdb".to_string(), "tmdb".to_string()];
        multi_source.external_ratings = vec![
            TitleExternalRating {
                source: "imdb".to_string(),
                value: Some(7.8),
                score: Some(7.8),
                normalized: 0.78,
                votes: Some(90_000),
                url: String::new(),
            },
            TitleExternalRating {
                source: "tmdb".to_string(),
                value: Some(7.6),
                score: Some(7.6),
                normalized: 0.76,
                votes: Some(4_000),
                url: String::new(),
            },
        ];

        let section = top_rated_discovery_home_section(&[multi_source, mal_only], &[], true, 10)
            .expect("top rated section");

        assert_eq!(
            section
                .items
                .iter()
                .map(|item| item.target_key.as_str())
                .collect::<Vec<_>>(),
            vec!["tvdb:series:mal-only", "tmdb:movie:multi"]
        );
    }

    #[test]
    fn discovery_home_top_rated_prefers_vote_backed_score_over_scoreless_source_names() {
        // During the per-source rating rollout an item can carry source names and
        // a summary rating but no external rating rows. Its bare source-name
        // count must not hoist it above a vote-backed real external score.
        let mut names_only = test_discovery_item("names-only", "movie", Some("movie"));
        names_only.source_run_kind = "public_feed".to_string();
        names_only.target_key = "tmdb:movie:names-only".to_string();
        names_only.rating = Some(7.0);
        names_only.rating_sources = vec!["imdb".to_string(), "tmdb".to_string()];

        let mut vote_backed = test_discovery_item("vote-backed", "movie", Some("movie"));
        vote_backed.source_run_kind = "public_feed".to_string();
        vote_backed.target_key = "tmdb:movie:vote-backed".to_string();
        vote_backed.rating = Some(9.2);
        vote_backed.rating_sources = vec!["imdb".to_string()];
        vote_backed.external_ratings = vec![TitleExternalRating {
            source: "imdb".to_string(),
            value: Some(9.2),
            score: Some(9.2),
            normalized: 0.92,
            votes: Some(80_000),
            url: String::new(),
        }];

        let section = top_rated_discovery_home_section(&[names_only, vote_backed], &[], true, 10)
            .expect("top rated section");

        assert_eq!(
            section
                .items
                .iter()
                .map(|item| item.target_key.as_str())
                .collect::<Vec<_>>(),
            vec!["tmdb:movie:vote-backed", "tmdb:movie:names-only"]
        );
    }

    #[test]
    fn discovery_rating_source_aliases_collapse_to_one_provider() {
        // Alias spellings of a single provider must not fake corroboration.
        let mut aliased = test_discovery_item("aliased", "series", Some("anime"));
        aliased.rating = Some(10.0);
        aliased.rating_sources = vec![
            "mal".to_string(),
            "MyAnimeList".to_string(),
            "MyAnimeList.net".to_string(),
        ];

        assert_eq!(discovery_item_distinct_rating_source_count(&aliased), 1);
        assert!(!discovery_item_has_credible_rating_evidence(&aliased));
    }

    #[test]
    fn discovery_comparators_tolerate_nan_scores() {
        // A NaN score must collapse to the missing-value ordering instead of
        // producing a non-total comparator (which can panic sort_by).
        assert_eq!(
            compare_optional_f64_desc(Some(f64::NAN), Some(5.0)),
            compare_optional_f64_desc(Some(0.0), Some(5.0))
        );
        assert_eq!(
            compare_optional_f64_desc(Some(f64::NAN), Some(f64::NAN)),
            Ordering::Equal
        );

        let mut nan_rated = test_discovery_item("nan-rated", "movie", Some("movie"));
        nan_rated.rating = Some(f64::NAN);
        assert_eq!(discovery_item_comparable_rating(&nan_rated), 0.0);
    }

    fn test_discovery_item(
        id: &str,
        target_kind: &str,
        content_type: Option<&str>,
    ) -> DiscoveryItemRecord {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        DiscoveryItemRecord {
            id: id.to_string(),
            run_id: "run-1".to_string(),
            base_generation_id: Some("run-1".to_string()),
            source_run_kind: "context_snapshot".to_string(),
            section_id: None,
            sort_index: 0,
            target_key: format!("{target_kind}:{id}"),
            target_kind: target_kind.to_string(),
            resolved: true,
            resolved_title_id: None,
            display_title: "Example".to_string(),
            original_title: None,
            sort_title: Some("Example".to_string()),
            year: None,
            poster_path: None,
            poster_url: None,
            background_url: None,
            overview: None,
            content_type: content_type.map(str::to_string),
            canonical_tags: vec![],
            rating: None,
            rating_sources: Vec::new(),
            external_ratings: Vec::new(),
            external_ids: Vec::new(),
            status_tags: Vec::new(),
            source_tags: Vec::new(),
            sources: Vec::new(),
            best_source: None,
            relation_types: Vec::new(),
            relation_subtypes: Vec::new(),
            chart_signals: Vec::new(),
            provider_signals: Vec::new(),
            rank_components: Vec::new(),
            source_count: None,
            edge_count: None,
            relation_count: None,
            source_subject_count: None,
            rank_score: None,
            matched_subject_keys: Vec::new(),
            matched_subject_titles: Vec::new(),
            matched_subject_count: 0,
            library_provenance: Vec::new(),
            tmdb_collection_id: None,
            tmdb_collection_name: None,
            owned_in_input: false,
            studio_slug: None,
            person_ids: Vec::new(),
            facet_terms: Vec::new(),
            context_terms: Vec::new(),
            change_subject_keys: Vec::new(),
            removed_subject_keys: Vec::new(),
            tombstoned_by_run_id: None,
            tombstoned_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_discovery_section(
        surface: &str,
        items: Vec<DiscoveryItemRecord>,
    ) -> DiscoverySectionResult {
        DiscoverySectionResult {
            section_id: format!("{surface}_section"),
            section_type: "TEST".to_string(),
            title: "Test".to_string(),
            surface: surface.to_string(),
            total_count: items.len() as i64,
            items,
        }
    }
}
