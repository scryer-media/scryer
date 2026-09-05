use super::*;

pub(super) fn discovery_section_limit(limit: usize) -> usize {
    if limit == 0 { 25 } else { limit.clamp(1, 100) }
}

pub(super) fn public_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(1) * 4).clamp(section_limit, 100)
}

pub(super) fn top_rated_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 80).clamp(DISCOVERY_HOME_MIN_CANDIDATES, DISCOVERY_HOME_MAX_CANDIDATES)
}

pub(super) fn personalized_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 40).clamp(DISCOVERY_HOME_MIN_CANDIDATES, DISCOVERY_HOME_MAX_CANDIDATES)
}

pub(super) fn complete_collection_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 8).clamp(
        DISCOVERY_COMPLETE_COLLECTION_MIN_CANDIDATES,
        DISCOVERY_COMPLETE_COLLECTION_MAX_CANDIDATES,
    )
}

pub(super) fn discovery_items_limit(limit: usize) -> usize {
    if limit == 0 { 50 } else { limit.clamp(1, 200) }
}

pub(super) fn sorted_discovery_library_ids(library_ids: &HashSet<String>) -> Vec<String> {
    let mut library_ids = library_ids.iter().cloned().collect::<Vec<_>>();
    library_ids.sort();
    library_ids
}

pub(super) fn section_items_record_to_result(
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

pub(super) fn home_section_candidates_record_to_result(
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

pub(super) fn home_selection_items_from_candidates(
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

pub(super) fn home_candidate_selection_item(
    candidate: &DiscoveryHomeCandidate,
) -> DiscoveryItemRecord {
    let mut item = candidate.item.clone();
    item.matched_subject_keys = candidate.matched_subject_keys.clone();
    item.facet_terms = candidate.affinity_terms.clone();
    item
}

pub(super) fn selected_discovery_home_item_ids(result: &DiscoveryHomeResult) -> BTreeSet<String> {
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

pub(super) fn discovery_home_subject_resolution_item_ids(
    result: &DiscoveryHomeResult,
) -> HashSet<String> {
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

pub(super) fn resolve_discovery_home_selected_subjects(
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

pub(super) fn replace_discovery_home_result_items(
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

pub(super) fn replace_discovery_home_item(
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

pub(super) fn discovery_home_elapsed_ms(started_at: Instant) -> u128 {
    started_at.elapsed().as_millis()
}

pub(super) fn filter_discovery_sections_for_owned_items(
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
pub(super) fn select_discovery_home_hero(
    public_sections: &[DiscoverySectionResult],
    personalized_sections: &[DiscoverySectionResult],
) -> Option<DiscoveryItemRecord> {
    select_discovery_home_hero_with_candidates(
        public_sections,
        personalized_sections,
        &HashMap::new(),
    )
}

pub(super) fn select_discovery_home_hero_with_candidates(
    public_sections: &[DiscoverySectionResult],
    personalized_sections: &[DiscoverySectionResult],
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Option<DiscoveryItemRecord> {
    select_personalized_discovery_home_hero(personalized_sections, candidates_by_id).or_else(|| {
        select_public_discovery_home_hero_with_candidates(public_sections, candidates_by_id)
    })
}

#[cfg(test)]
pub(super) fn top_rated_discovery_home_section(
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

pub(super) fn top_rated_discovery_home_section_with_candidates(
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

pub(super) fn discovery_home_item_is_personalized(item: &DiscoveryItemRecord) -> bool {
    !item
        .source_run_kind
        .trim()
        .eq_ignore_ascii_case("public_feed")
}

pub(super) fn select_personalized_discovery_home_hero(
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
pub(super) fn select_public_discovery_home_hero(
    sections: &[DiscoverySectionResult],
) -> Option<DiscoveryItemRecord> {
    select_public_discovery_home_hero_with_candidates(sections, &HashMap::new())
}

pub(super) fn select_public_discovery_home_hero_with_candidates(
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

pub(super) fn compare_personalized_discovery_home_hero_items(
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

pub(super) fn compare_public_discovery_home_hero_items(
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

pub(super) fn discovery_item_has_hero_backdrop(item: &DiscoveryItemRecord) -> bool {
    item.background_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
}

pub(super) fn discovery_home_item_has_hero_backdrop(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> bool {
    candidates_by_id
        .get(&item.id)
        .map(|candidate| candidate.has_hero_backdrop)
        .unwrap_or_else(|| discovery_item_has_hero_backdrop(item))
}

pub(super) fn compare_discovery_item_rating_desc(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> Ordering {
    discovery_item_comparable_rating(right).total_cmp(&discovery_item_comparable_rating(left))
}

/// Collapse a raw rating-source string to a provider identity so aliases and
/// per-provider sub-metrics ("mal" vs "MyAnimeList.net", RT critic vs audience)
/// cannot inflate the distinct-provider count. Mirrors the web app's
/// `normalizedRatingSource` alias handling in `lib/utils/title-ratings.ts`.
pub(super) fn canonical_rating_source_identity(source: &str) -> String {
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
pub(super) fn discovery_item_distinct_rating_source_count(item: &DiscoveryItemRecord) -> usize {
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
pub(super) fn discovery_item_has_credible_rating_evidence(item: &DiscoveryItemRecord) -> bool {
    let has_rating_signal = discovery_item_comparable_rating(item) > 0.0
        || discovery_item_best_external_rating_score(item).is_some();
    if !has_rating_signal {
        return false;
    }
    discovery_item_distinct_rating_source_count(item) >= DISCOVERY_MIN_CREDIBLE_RATING_SOURCE_COUNT
        || discovery_item_external_rating_vote_count(item) >= DISCOVERY_CREDIBLE_RATING_MIN_VOTES
}

pub(super) fn compare_top_rated_discovery_home_items(
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

pub(super) fn discovery_home_item_has_credible_rating_evidence(
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

pub(super) fn discovery_home_item_best_external_rating_score(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Option<f64> {
    candidates_by_id
        .get(&item.id)
        .and_then(|candidate| candidate.best_external_rating)
        .or_else(|| discovery_item_best_external_rating_score(item))
}

pub(super) fn discovery_home_item_external_rating_vote_count(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> i32 {
    candidates_by_id
        .get(&item.id)
        .map(|candidate| candidate.best_external_rating_votes)
        .unwrap_or_else(|| discovery_item_external_rating_vote_count(item))
}

pub(super) fn discovery_item_best_external_rating_score(item: &DiscoveryItemRecord) -> Option<f64> {
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

pub(super) fn discovery_item_external_rating_vote_count(item: &DiscoveryItemRecord) -> i32 {
    item.external_ratings
        .iter()
        .filter_map(|rating| rating.votes)
        .max()
        .unwrap_or_default()
}

// Missing and non-finite values both collapse to 0.0 so the comparator stays a
// total order (a NaN score must never make sort_by panic or misorder).
pub(super) fn comparable_finite_f64(value: Option<f64>) -> f64 {
    value
        .filter(|candidate| candidate.is_finite())
        .unwrap_or_default()
}

pub(super) fn compare_optional_f64_desc(left: Option<f64>, right: Option<f64>) -> Ordering {
    comparable_finite_f64(right).total_cmp(&comparable_finite_f64(left))
}

pub(super) fn personalized_section_results(
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
        &library_profile.theme_labels,
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

pub(super) fn canonical_affinity_labels_for_profile(
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

pub(super) fn label_affinity_sections(
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
/// unguarded "Because You Like Animation" rail comingles Paperman with Silver Horizon.
/// This is a deliberate two-label special case at the point where items are
/// matched to a label, not a general taxonomy: the animation rail drops anime
/// items, an anime rail keeps only anime items, and every other label is
/// untouched.
///
/// The guard is keyed on the label name and is **kind-agnostic by design**: a
/// theme/tag rail named "Anime" or "Animation" carries exactly the same meaning
/// its genre namesake does, so it has to split the same way.
pub(super) fn affinity_label_keeps_item_across_anime_boundary(
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
pub(super) fn discovery_item_is_anime(item: &DiscoveryItemRecord) -> bool {
    discovery_item_media_kind(item) == Some("anime")
        || discovery_item_canonical_facet_labels(item, "genre")
            .iter()
            .any(|label| normalize_discovery_affinity_key(label) == "anime")
}

/// What an owner's library says they like, expressed only in canonical labels.
///
/// This deliberately carries no user tags. User tags are private catalog
/// vocabulary an operator invented for their own workflow ("keep", "needs
/// review"); feeding them into affinity would send them out with every
/// discovery request and would compose rails out of labels that mean nothing to
/// the metadata gateway. Both rail families are therefore sourced from the
/// canonical tags SMG itself assigned.
#[derive(Clone, Debug, Default)]
pub(super) struct DiscoveryLibraryAffinityProfile {
    pub(super) genre_labels: Vec<String>,
    pub(super) theme_labels: Vec<String>,
}
