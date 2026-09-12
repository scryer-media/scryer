use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct CatalogOwnedVisibility {
    title_ids: HashSet<String>,
    keys: HashSet<String>,
    identity_keys: HashSet<String>,
}

impl CatalogOwnedVisibility {
    #[cfg(test)]
    pub(super) fn from_titles(titles: &[Title]) -> Self {
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

    pub(super) fn from_title_records(titles: &[CatalogOwnedTitleRecord]) -> Self {
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

    pub(super) fn from_title_records_and_series_movies(
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

    pub(super) fn excluded_discovery_identity_keys(&self) -> Vec<String> {
        let mut keys = self.identity_keys.iter().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    pub(super) fn item_is_owned(&self, item: &DiscoveryItemRecord) -> bool {
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

pub(super) fn add_catalog_owned_external_keys(
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

pub(super) fn insert_catalog_owned_key(
    keys: &mut HashSet<String>,
    identity_keys: &mut HashSet<String>,
    source: &str,
    value: &str,
) {
    let key = format!("{source}:{value}");
    keys.insert(key.clone());
    identity_keys.insert(key);
}

pub(super) fn discovery_item_ownership_keys(item: &DiscoveryItemRecord) -> HashSet<String> {
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

pub(super) fn normalize_catalog_owned_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn discovery_media_kind_for_facet(facet: MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Series => "series",
        MediaFacet::Anime => "anime",
    }
}

pub(super) fn catalog_discovery_group_limit(limit: usize) -> usize {
    if limit == 0 { 12 } else { limit.clamp(1, 12) }
}

pub(super) fn catalog_discovery_max_groups(max_groups: usize) -> usize {
    if max_groups == 0 {
        6
    } else {
        max_groups.clamp(1, 10)
    }
}

pub(super) fn catalog_discovery_candidate_limit(limit: usize, max_groups: usize) -> usize {
    (limit.max(6) * max_groups.max(4) * 8).clamp(48, 400)
}

pub(super) fn catalog_filter_anime_public_sections(
    public_sections: &mut Vec<CatalogDiscoverySectionCandidatesRecord>,
) {
    public_sections.retain(|section| {
        !CATALOG_ANIME_SUPPRESSED_PUBLIC_SECTION_IDS.contains(&section.section_id.as_str())
    });
}

pub(super) fn catalog_take_public_section(
    public_sections: &mut Vec<CatalogDiscoverySectionCandidatesRecord>,
    section_id: &str,
) -> Option<CatalogDiscoverySectionCandidatesRecord> {
    public_sections
        .iter()
        .position(|section| section.section_id == section_id)
        .map(|index| public_sections.remove(index))
}

pub(super) fn catalog_public_top_section(
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

pub(super) fn catalog_public_top_group(
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

pub(super) fn catalog_public_section_group(
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

pub(super) fn catalog_public_section_label(
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

pub(super) fn normalized_catalog_group_id(value: &str) -> String {
    let normalized = normalize_discovery_affinity_key(value).replace(' ', "_");
    if normalized.is_empty() {
        "section".to_string()
    } else {
        normalized
    }
}

pub(super) fn catalog_personalized_groups(
    groups: &mut Vec<CatalogDiscoveryGroup>,
    items: &[DiscoveryItemRecord],
    library_profile: &DiscoveryLibraryAffinityProfile,
    settings: DiscoveryRailCompositionSettings,
    limit: usize,
    max_groups: usize,
    emitted_item_keys: &mut HashSet<String>,
) {
    let personalized_group_start = groups.len();

    // The zero rule is a law about personalized surfaces, not about the home
    // page: a library that owns no anime must not be shown anime here either.
    // Applied once, up front, so every group below - including the collection
    // and fallback groups - inherits it.
    let gated_items = items
        .iter()
        .filter(|item| {
            !settings.medium_affinity_gate
                || library_profile
                    .medium_mix
                    .admits(discovery_item_medium(item))
        })
        .cloned()
        .collect::<Vec<_>>();
    let items = gated_items.as_slice();

    // Same specificity ladder and same anime/animation boundary as the home
    // composer (`personalized_section_results`): theme/tag groups claim titles
    // before the broader genre groups, and neither may comingle Western animation
    // with anime. The two surfaces must not disagree about what a rail means.
    for label in
        canonical_affinity_labels_for_profile(items, &library_profile.theme_labels, "theme")
    {
        if groups.len() >= max_groups {
            return;
        }
        let mut section_items = items
            .iter()
            .filter(|item| {
                item.matched_subject_count > 0
                    && discovery_item_matches_affinity_label(
                        item,
                        &label,
                        "theme",
                        settings.genre_rail_corroboration,
                    )
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
                    && discovery_item_matches_affinity_label(
                        item,
                        &label,
                        "genre",
                        settings.genre_rail_corroboration,
                    )
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

pub(super) struct CatalogDiscoveryGroupDraft {
    id: String,
    kind: CatalogDiscoveryGroupKind,
    surface: CatalogDiscoverySurface,
    label_value: Option<String>,
    total_count: Option<i64>,
}

pub(super) fn catalog_group_excluding_emitted(
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

pub(super) fn discovery_item_canonical_facet_labels(
    item: &DiscoveryItemRecord,
    kind: &str,
) -> Vec<String> {
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

pub(super) fn discovery_item_matches_affinity_label(
    item: &DiscoveryItemRecord,
    label: &str,
    canonical_kind: &str,
    require_corroboration: bool,
) -> bool {
    let label_key = normalize_discovery_affinity_key(label);
    if label_key.is_empty() {
        return false;
    }
    let matches_facet = discovery_item_canonical_facet_labels(item, canonical_kind)
        .into_iter()
        .any(|candidate| affinity_value_matches_label(&candidate, &label_key));
    if !matches_facet {
        return false;
    }
    if !require_corroboration || canonical_kind != "genre" {
        return true;
    }
    // The facet term carries no provenance, so corroboration is read off the
    // canonical tag the pool loader hydrates alongside it. An item whose genre
    // tags were not hydrated cannot prove corroboration and does not get the
    // benefit of the doubt: that is the whole point of the gate.
    item.canonical_tags
        .iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
        .filter(|tag| normalize_discovery_affinity_key(&tag.name) == label_key)
        .any(canonical_genre_tag_is_corroborated)
}

/// A canonical genre is load-bearing only when the metadata gateway is actually
/// confident about it and something other than a single free-text tag says so.
///
/// SMG assigns 0.95 to a provider's own genre list, caps name-alias matches at
/// 0.9 and phrase-sequence guesses at 0.85, so
/// [`DISCOVERY_CORROBORATED_GENRE_MIN_CONFIDENCE`] draws the line just under the
/// alias cap and excludes the guesses. Above that line a genre still needs
/// corroboration: either two independent sources agreed, or one provider listed
/// it as a genre rather than as a community tag. A lone `anilist:tag:<hash>` is
/// how "Crime" ends up on a high-school romance, and it is exactly what this
/// rejects.
pub(super) fn canonical_genre_tag_is_corroborated(tag: &CanonicalMediaTag) -> bool {
    if tag
        .confidence
        .filter(|confidence| confidence.is_finite())
        .is_none_or(|confidence| confidence < DISCOVERY_CORROBORATED_GENRE_MIN_CONFIDENCE)
    {
        return false;
    }
    distinct_canonical_tag_source_count(tag) >= DISCOVERY_CORROBORATED_GENRE_MIN_SOURCES
        || tag
            .source_tag_keys
            .iter()
            .any(|key| canonical_tag_source_key_is_provider_genre(key))
}

fn distinct_canonical_tag_source_count(tag: &CanonicalMediaTag) -> usize {
    tag.sources
        .iter()
        .map(|source| source.trim().to_ascii_lowercase())
        .filter(|source| !source.is_empty())
        .collect::<HashSet<_>>()
        .len()
}

/// Provider genre keys come in two shapes: a raw provider genre list flattens to
/// `genre:<slug>`, and a provider's own genre-typed source tag to
/// `<provider>:genre:<slug>`. Community tag keys (`anilist:tag:<hash>`,
/// `mal:tag:<slug>`) match neither, which is the distinction the gate needs.
fn canonical_tag_source_key_is_provider_genre(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    let mut segments = key.split(':').filter(|segment| !segment.is_empty());
    matches!(
        (segments.next(), segments.next()),
        (Some("genre"), Some(_)) | (Some(_), Some("genre"))
    )
}

pub(super) fn affinity_value_matches_label(value: &str, label_key: &str) -> bool {
    let value_key = normalize_discovery_affinity_key(value);
    value_key == label_key
}

#[cfg(test)]
pub(super) fn discovery_item_matches_canonical_facet_filters(
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

pub(super) fn push_unique_discovery_label(
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

pub(super) fn canonical_tag_labels(tags: &[CanonicalMediaTag], category: &str) -> Vec<String> {
    tags.iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case(category))
        .map(|tag| tag.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The owned-title half of the corroboration gate. The same rule has to run
/// here and in [`discovery_item_matches_affinity_label`]: a library profile
/// built from uncorroborated genres would name a rail the pool matcher then
/// refuses to fill.
pub(super) fn corroborated_canonical_genre_labels(
    tags: &[CanonicalMediaTag],
    require_corroboration: bool,
) -> Vec<String> {
    tags.iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
        .filter(|tag| !require_corroboration || canonical_genre_tag_is_corroborated(tag))
        .map(|tag| tag.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Ranked labels the library actually supports.
///
/// `min_carriers` is the evidence floor: a label carried by fewer owned titles
/// than that is skipped and the next-ranked label considered, rather than
/// promoted into a rail the library never earned. Three crime titles out of ten
/// is not a crime library.
pub(super) fn top_owned_title_labels<'a, F, I>(
    titles: &'a [Title],
    labels_for_title: F,
    limit: usize,
    min_carriers: usize,
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
        .filter(|(_, carriers)| *carriers >= min_carriers.max(1))
        .take(limit)
        .map(|(label, _)| label)
        .collect()
}

/// How many owned titles must carry a label before it may name a rail:
/// [`DISCOVERY_RAIL_LABEL_SUPPORT_MIN_CARRIERS`] titles, or
/// [`DISCOVERY_RAIL_LABEL_SUPPORT_SHARE`] of the library, whichever is larger.
/// The share term is what stops a large library's long tail from earning rails.
pub(super) fn discovery_rail_label_support_floor(owned_title_count: usize) -> usize {
    let share_floor = (owned_title_count as f64 * DISCOVERY_RAIL_LABEL_SUPPORT_SHARE).ceil();
    let share_floor = if share_floor.is_finite() && share_floor > 0.0 {
        share_floor as usize
    } else {
        0
    };
    share_floor.max(DISCOVERY_RAIL_LABEL_SUPPORT_MIN_CARRIERS)
}

/// Whether the library has grown enough since its last snapshot run that the
/// rails built from that run are describing a library the user no longer has.
///
/// A fresh install rebuilds its library over hours, not days: the first
/// snapshot lands on a handful of titles and the normal cadence would then
/// leave the user staring at rails built for that handful. Growth past the
/// label-support floor means a label can now exist (or lose) its evidence, and
/// growth of [`DISCOVERY_LIBRARY_GROWTH_RESNAPSHOT_SHARE`] means the mix,
/// the floors and the proportional caps have all moved under the rails.
pub(crate) fn discovery_library_growth_warrants_snapshot(
    previous_subject_count: usize,
    current_subject_count: usize,
) -> bool {
    let Some(growth) = current_subject_count
        .checked_sub(previous_subject_count)
        .filter(|growth| *growth > 0)
    else {
        return false;
    };
    if growth >= discovery_rail_label_support_floor(current_subject_count) {
        return true;
    }
    previous_subject_count > 0
        && growth as f64
            >= previous_subject_count as f64 * DISCOVERY_LIBRARY_GROWTH_RESNAPSHOT_SHARE
}

pub(super) fn display_discovery_affinity_label(value: &str) -> String {
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

pub(super) fn discovery_item_comparable_rating(item: &DiscoveryItemRecord) -> f64 {
    item.rating
        .filter(|rating| rating.is_finite())
        .map(|rating| if rating <= 1.0 { rating * 10.0 } else { rating })
        .unwrap_or_default()
}

pub(super) fn discovery_affinity_label_is_generic(normalized: &str) -> bool {
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

pub(super) fn normalize_discovery_affinity_key(value: &str) -> String {
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

pub(super) fn slugify_discovery_section_part(value: &str) -> String {
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

pub(super) fn complete_collection_section(
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

pub(super) fn discovery_item_has_collection_signal(item: &DiscoveryItemRecord) -> bool {
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

pub(super) fn section_result(
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

pub(super) fn home_item_visible(item: &DiscoveryItemRecord, include_unresolved: bool) -> bool {
    !item.owned_in_input && (include_unresolved || item.resolved)
}

pub(super) fn discovery_item_media_kind(item: &DiscoveryItemRecord) -> Option<&'static str> {
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

pub(super) fn normalized_discovery_media_kind(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "anime" => Some("anime"),
        "movie" => Some("movie"),
        "series" => Some("series"),
        _ => None,
    }
}

pub(super) fn resolve_discovery_matched_subjects(
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

pub(super) fn filter_submitted_subjects_for_libraries(
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
pub(super) fn item_matches_discovery_items_query(
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
pub(super) fn matches_optional_text_query(item: &DiscoveryItemRecord, query: Option<&str>) -> bool {
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

pub(super) fn dedupe_and_sort_discovery_items(items: &mut Vec<DiscoveryItemRecord>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(discovery_item_identity_key(item).to_string()));
    items.sort_by(compare_discovery_items);
}

pub(super) fn discovery_item_identity_key(item: &DiscoveryItemRecord) -> &str {
    if item.target_key.trim().is_empty() {
        item.id.as_str()
    } else {
        item.target_key.as_str()
    }
}

pub(super) fn compare_discovery_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> Ordering {
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
pub(super) fn text_values_contain_any(values: &[String], filters: &[String]) -> bool {
    filters.iter().any(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
    })
}

#[cfg(test)]
pub(super) fn text_values_or_optional_contains_any(
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

pub(super) fn normalize_discovery_filter_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
pub(super) fn contains_case_insensitive(values: &[String], candidate: &str) -> bool {
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(candidate))
}

pub(super) fn collect_json_text_values(value: &JsonValue, values: &mut Vec<String>) {
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
