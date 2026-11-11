//! One discovery home section paged in full.
//!
//! A section's first page is the rail exactly as the dashboard renders it,
//! and the pages after it continue the same list without repeating a title
//! the dashboard already showed above the rail:
//!
//! - "because you like" and For You rails rebuild the personalized shelf from
//!   the home's own candidate pool, then continue the target rail's eligible
//!   list over a deeper pool. Earlier rails keep the titles they claimed.
//! - "Browse by" sections list every pool title carrying one allowlisted
//!   genre or theme.
//! - Top Rated, Complete the Collection and public sections compose the home,
//!   reproduce the dashboard's reading order and cross-rail dedupe to find
//!   the rail's rendered window and every title shown before it, and continue
//!   with their own full list.
//!
//! Owned titles never change what the home shows or claims; showing them
//! only widens what a section lists after its first window.

use super::*;
use crate::ports::{
    DiscoveryBrowseLabel, DiscoveryBrowseLabelFamily, DiscoverySectionItemsQuery,
    DiscoverySectionSort, DiscoverySyncStateRecord, MediaRequestQuery,
};
use scryer_domain::MediaRequestStatus;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

const DISCOVERY_TOP_RATED_SECTION_ID: &str = "top_rated";
const DISCOVERY_COMPLETE_COLLECTION_SECTION_ID: &str = "complete_the_collection";
const DISCOVERY_LABEL_RAIL_SECTION_PREFIXES: &[&str] =
    &["because_you_like_tag_", "because_you_like_genre_"];
const DISCOVERY_BROWSE_GENRE_SECTION_PREFIX: &str = "browse_genre";
const DISCOVERY_BROWSE_THEME_SECTION_PREFIX: &str = "browse_theme";
const DISCOVERY_BROWSE_GENRE_SECTION_TYPE: &str = "BROWSE_GENRE";
const DISCOVERY_BROWSE_THEME_SECTION_TYPE: &str = "BROWSE_THEME";

// The dashboard's reading order. Every list below is pinned to the web's
// `apps/scryer-web/lib/contracts/discovery-section-order.json`, which the
// dashboard reads its own order from.

/// Personalized shelf section types, in reading order.
pub(super) const DISCOVERY_PERSONALIZED_SHELF_ORDER: &[&str] = &[
    "FROM_YOUR_WATCHLIST",
    "BECAUSE_YOU_LIKE_TAG",
    "BECAUSE_YOU_LIKE_GENRE",
    "TOP_MOVIES_THIS_WEEK",
    "TOP_SERIES_THIS_WEEK",
    "TOP_ANIME_THIS_WEEK",
    "TOP_RATED_FOR_YOU",
    "TOP_RATED",
];
/// Personalized types shown only when no library-based rail is.
pub(super) const DISCOVERY_GENERIC_FOR_YOU_FALLBACK_TYPES: &[&str] = &["FOR_YOU"];
/// Personalized types that say nothing about the library, so they never stand
/// in for the library context the generic fallback waits on.
pub(super) const DISCOVERY_NON_LIBRARY_PERSONALIZED_TYPES: &[&str] = &["FROM_YOUR_WATCHLIST"];
/// Public types pinned between the personalized shelf and the world shelf.
pub(super) const DISCOVERY_PUBLIC_PROMOTION_TYPES: &[&str] =
    &["NEW_ON_STREAMING", "NEW_ON_PHYSICAL"];
/// World shelf slots, in reading order.
pub(super) const DISCOVERY_WORLD_SHELF_ORDER: &[&str] = &[
    "POPULAR_RIGHT_NOW",
    "POPULAR_MOVIES",
    "POPULAR_SERIES",
    "UPCOMING_MOVIES",
    "UPCOMING_SERIES",
    "TOP_RATED",
    "EVERGREEN_POPULAR",
    "ANIME_THIS_WEEK",
    "ANIME_SEASON_STANDOUTS",
    "MOST_ANTICIPATED_ANIME",
    "UPCOMING_NEXT_SEASON",
];
/// Public types that close the page, after every other public rail.
pub(super) const DISCOVERY_CURATED_TYPES: &[&str] = &["CURATED"];
/// Retired types the dashboard never renders.
pub(super) const DISCOVERY_SUPPRESSED_LEGACY_TYPES: &[&str] = &[
    "BECAUSE_YOU_HAVE",
    "MOVIES_FOR_YOU",
    "SERIES_FOR_YOU",
    "ANIME_FOR_YOU",
];
/// Public section types the dashboard folds into another rail (source,
/// target). The folded rail is the target's items followed by the source's.
pub(super) const DISCOVERY_PUBLIC_FOLDED_SECTION_TYPES: &[(&str, &str)] = &[
    ("TRENDING_NOW", "POPULAR_RIGHT_NOW"),
    ("POPULAR_WITH_ANIME_FANS", "ANIME_THIS_WEEK"),
];
/// Public section types the dashboard narrows to anime, and the ones it
/// strips anime from.
pub(super) const DISCOVERY_PUBLIC_ANIME_ONLY_SECTION_TYPES: &[&str] =
    &["ANIME_THIS_WEEK", "POPULAR_WITH_ANIME_FANS"];
pub(super) const DISCOVERY_PUBLIC_ANIME_EXCLUDING_SECTION_TYPES: &[&str] =
    &["TRENDING_NOW", "POPULAR_RIGHT_NOW", "POPULAR_SERIES"];
/// A public rail left with fewer titles than this after cross-rail dedupe is
/// hidden; its titles still count as shown.
pub(super) const DISCOVERY_MIN_PUBLIC_RAIL_ITEMS: usize = 3;

/// Composed section lists kept so later pages only slice and hydrate.
const DISCOVERY_SECTION_CACHE_ENTRIES: usize = 8;
const DISCOVERY_SECTION_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub(super) enum DiscoverySectionTarget {
    Watchlist,
    PersonalizedRail,
    Browse(&'static DiscoveryRailLabel),
    TopRated,
    CompleteCollection,
    Public,
}

pub(super) fn discovery_section_target(section_id: &str) -> DiscoverySectionTarget {
    if section_id == DISCOVERY_WATCHLIST_SECTION_ID {
        return DiscoverySectionTarget::Watchlist;
    }
    if section_id == DISCOVERY_FOR_YOU_SECTION_ID
        || DISCOVERY_LABEL_RAIL_SECTION_PREFIXES
            .iter()
            .any(|prefix| section_id.starts_with(prefix))
    {
        return DiscoverySectionTarget::PersonalizedRail;
    }
    if section_id == DISCOVERY_TOP_RATED_SECTION_ID {
        return DiscoverySectionTarget::TopRated;
    }
    if section_id == DISCOVERY_COMPLETE_COLLECTION_SECTION_ID {
        return DiscoverySectionTarget::CompleteCollection;
    }
    if let Some(entry) = DISCOVERY_RAIL_LABELS
        .iter()
        .find(|entry| discovery_browse_section_id(entry) == section_id)
    {
        return DiscoverySectionTarget::Browse(entry);
    }
    DiscoverySectionTarget::Public
}

pub(super) fn discovery_browse_section_id(entry: &DiscoveryRailLabel) -> String {
    let prefix = match entry.family {
        DiscoveryRailLabelFamily::Genre => DISCOVERY_BROWSE_GENRE_SECTION_PREFIX,
        DiscoveryRailLabelFamily::Theme => DISCOVERY_BROWSE_THEME_SECTION_PREFIX,
    };
    format!("{prefix}_{}", slugify_discovery_section_part(entry.key))
}

fn discovery_rail_label_family_kind(family: DiscoveryRailLabelFamily) -> &'static str {
    match family {
        DiscoveryRailLabelFamily::Genre => "genre",
        DiscoveryRailLabelFamily::Theme => "theme",
    }
}

/// The English noun a browse card falls back to for a client that does not
/// know its key.
fn discovery_browse_title(entry: &DiscoveryRailLabel) -> String {
    entry.browse_noun.to_string()
}

fn discovery_public_section_folds_into(source_type: &str) -> Option<&'static str> {
    DISCOVERY_PUBLIC_FOLDED_SECTION_TYPES
        .iter()
        .find(|(source, _)| source.eq_ignore_ascii_case(source_type))
        .map(|(_, target)| *target)
}

/// Narrows a public section's items the way the dashboard does: the anime
/// rails keep only anime, and the general "right now" rails drop it.
fn discovery_public_section_keeps_item(section_type: &str, item: &DiscoveryItemRecord) -> bool {
    let section_type = section_type.trim().to_ascii_uppercase();
    let is_anime = discovery_item_media_kind(item) == Some("anime");
    if DISCOVERY_PUBLIC_ANIME_ONLY_SECTION_TYPES.contains(&section_type.as_str()) {
        is_anime
    } else if DISCOVERY_PUBLIC_ANIME_EXCLUDING_SECTION_TYPES.contains(&section_type.as_str()) {
        !is_anime
    } else {
        true
    }
}

/// The ids of the public sections a section page for `section_id` reads: the
/// section itself, then every section the dashboard folds into it.
pub(super) fn discovery_public_section_sources(
    section_id: &str,
    sections: &[DiscoverySectionRecord],
) -> Vec<String> {
    let Some(target) = sections
        .iter()
        .find(|section| section.section_id == section_id)
    else {
        return Vec::new();
    };
    let mut ids = vec![target.section_id.clone()];
    ids.extend(
        sections
            .iter()
            .filter(|section| {
                discovery_public_section_folds_into(&section.section_type)
                    .is_some_and(|folded| folded.eq_ignore_ascii_case(&target.section_type))
            })
            .map(|section| section.section_id.clone()),
    );
    ids
}

/// Folds loaded public section records into the one section a page shows,
/// the way the dashboard folds the rails it was sent: first the first
/// `window` items of the target and then of each source, the size each rail
/// arrives cut to, then the rest of the target and of each source. Each
/// record is narrowed by its own type after the cut, and titles are deduped
/// by the dashboard's key.
pub(super) fn fold_discovery_public_section(
    section_id: &str,
    records: Vec<DiscoverySectionResult>,
    window: usize,
) -> Option<DiscoverySectionResult> {
    let mut records = records;
    let target_index = records
        .iter()
        .position(|record| record.section_id == section_id)?;
    let target = records.remove(target_index);
    let ordered = std::iter::once(&target)
        .chain(records.iter())
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for (skip, take) in [(0, window), (window, usize::MAX)] {
        for record in &ordered {
            for item in record.items.iter().skip(skip).take(take) {
                if discovery_public_section_keeps_item(&record.section_type, item)
                    && seen.insert(discovery_presented_item_key(item))
                {
                    items.push(item.clone());
                }
            }
        }
    }
    if items.is_empty() {
        return None;
    }
    Some(DiscoverySectionResult {
        total_count: items.len() as i64,
        items,
        ..target
    })
}

/// Stable re-sort of a section's members. Relevance keeps the section's own
/// order; every other sort falls back to it on ties.
pub(super) fn sort_discovery_section_items(
    items: &mut [DiscoveryItemRecord],
    sort: DiscoverySectionSort,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) {
    match sort {
        DiscoverySectionSort::Relevance => {}
        DiscoverySectionSort::Popularity => {
            items.sort_by(|left, right| compare_optional_f64_desc(left.base_rank, right.base_rank))
        }
        DiscoverySectionSort::Rating => items.sort_by(|left, right| {
            compare_optional_f64_desc(
                discovery_section_item_rating(left, candidates_by_id),
                discovery_section_item_rating(right, candidates_by_id),
            )
        }),
        // Titles without a year sort last: an unknown date is not "oldest".
        DiscoverySectionSort::ReleaseDate => {
            items.sort_by(|left, right| match (left.year, right.year) {
                (Some(left), Some(right)) => right.cmp(&left),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
        }
        DiscoverySectionSort::Title => items.sort_by_cached_key(|item| {
            item.sort_title
                .as_deref()
                .unwrap_or(&item.display_title)
                .to_lowercase()
        }),
    }
}

fn discovery_section_item_rating(
    item: &DiscoveryItemRecord,
    candidates_by_id: &HashMap<String, DiscoveryHomeCandidate>,
) -> Option<f64> {
    discovery_home_item_best_external_rating_score(item, candidates_by_id).or(item.rating)
}

/// The window of a section's members one page shows, with the section's
/// total before paging.
/// The key the dashboard dedupes rails by: media kind and target key.
fn discovery_presented_item_key(item: &DiscoveryItemRecord) -> String {
    format!(
        "{}:{}",
        item.target_kind.trim().to_ascii_uppercase(),
        item.target_key
    )
}

fn discovery_section_type_key(section: &DiscoverySectionResult) -> String {
    section.section_type.trim().to_ascii_uppercase()
}

fn discovery_section_renders(section: &DiscoverySectionResult) -> bool {
    !section.items.is_empty()
        && !DISCOVERY_SUPPRESSED_LEGACY_TYPES
            .contains(&discovery_section_type_key(section).as_str())
}

/// The home's public sections as the dashboard receives them: anime rails
/// narrowed to anime, the general rails stripped of it, and any section left
/// empty dropped.
fn narrow_discovery_public_sections(
    sections: &[DiscoverySectionResult],
) -> Vec<DiscoverySectionResult> {
    sections
        .iter()
        .filter_map(|section| {
            let items = section
                .items
                .iter()
                .filter(|item| discovery_public_section_keeps_item(&section.section_type, item))
                .cloned()
                .collect::<Vec<_>>();
            (!items.is_empty()).then(|| DiscoverySectionResult {
                total_count: items.len() as i64,
                items,
                ..section.clone()
            })
        })
        .collect()
}

/// Folds each source public section into its target when the target is
/// present: the target keeps its place and identity and lists its own items,
/// then the source's. A source without its target stays as it is.
fn merge_discovery_public_sections(
    sections: Vec<DiscoverySectionResult>,
) -> Vec<DiscoverySectionResult> {
    let mut target_index_by_type = HashMap::new();
    for (index, section) in sections.iter().enumerate() {
        target_index_by_type
            .entry(discovery_section_type_key(section))
            .or_insert(index);
    }
    let mut merged_items = HashMap::<usize, Vec<DiscoveryItemRecord>>::new();
    let mut merged_away = HashSet::new();
    for (index, section) in sections.iter().enumerate() {
        let Some(target_index) = discovery_public_section_folds_into(&section.section_type)
            .and_then(|target_type| target_index_by_type.get(target_type))
        else {
            continue;
        };
        merged_away.insert(index);
        merged_items
            .entry(*target_index)
            .or_default()
            .extend(section.items.iter().cloned());
    }
    sections
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !merged_away.contains(index))
        .map(|(index, mut section)| {
            if let Some(extra) = merged_items.remove(&index) {
                let mut seen = HashSet::new();
                section.items = std::mem::take(&mut section.items)
                    .into_iter()
                    .chain(extra)
                    .filter(|item| seen.insert(discovery_presented_item_key(item)))
                    .collect();
                section.total_count = section.items.len() as i64;
            }
            section
        })
        .collect()
}

/// The world-shelf slot a public section takes: a fold source without its
/// target renders where the target would have.
fn discovery_world_shelf_slot(section: &DiscoverySectionResult) -> String {
    discovery_public_section_folds_into(&section.section_type)
        .map(str::to_string)
        .unwrap_or_else(|| discovery_section_type_key(section))
}

/// One rail as the dashboard renders it, with every title shown above it.
#[derive(Debug)]
pub(super) struct DiscoveryRenderedWindow {
    /// The rail with the items the dashboard shows in it.
    pub(super) section: DiscoverySectionResult,
    /// Presented keys of every title the dashboard shows before this rail,
    /// hidden thin rails included.
    pub(super) preceding_keys: HashSet<String>,
}

/// Reproduces the dashboard's reading order and cross-rail dedupe over a
/// composed home and returns the rendered window of `section_id`, or `None`
/// when the dashboard does not render that rail. `hero_key` is the presented
/// key of the home's hero: when a rendered rail shows it, the dashboard puts
/// it in the hero and takes it out of every rail, so it counts as shown
/// before all of them.
pub(super) fn discovery_rendered_window(
    home: &DiscoveryHomeResult,
    section_id: &str,
    hero_key: Option<&str>,
) -> Option<DiscoveryRenderedWindow> {
    let public = merge_discovery_public_sections(
        narrow_discovery_public_sections(&home.public_sections)
            .into_iter()
            .filter(discovery_section_renders)
            .collect(),
    );
    let personalized = home
        .personalized_sections
        .iter()
        .filter(|section| discovery_section_renders(section))
        .collect::<Vec<_>>();
    let complete_collection = home
        .complete_collection
        .as_ref()
        .filter(|section| discovery_section_renders(section));

    let mut shelved = HashSet::new();
    let mut shelf = Vec::new();
    for section_type in DISCOVERY_PERSONALIZED_SHELF_ORDER {
        for (index, section) in personalized.iter().enumerate() {
            if discovery_section_type_key(section) == *section_type {
                shelved.insert(index);
                shelf.push(*section);
            }
        }
    }
    let is_fallback = |section: &DiscoverySectionResult| {
        DISCOVERY_GENERIC_FOR_YOU_FALLBACK_TYPES
            .contains(&discovery_section_type_key(section).as_str())
    };
    let unknown = personalized
        .iter()
        .enumerate()
        .filter(|(index, section)| !shelved.contains(index) && !is_fallback(section))
        .map(|(_, section)| *section)
        .collect::<Vec<_>>();
    let has_library_sections = shelf.iter().any(|section| {
        !DISCOVERY_NON_LIBRARY_PERSONALIZED_TYPES
            .contains(&discovery_section_type_key(section).as_str())
    }) || complete_collection.is_some()
        || !unknown.is_empty();
    let fallback = if has_library_sections {
        Vec::new()
    } else {
        personalized
            .iter()
            .filter(|section| is_fallback(section))
            .copied()
            .collect()
    };

    let indices_where = |matches: &dyn Fn(&DiscoverySectionResult, &str) -> bool,
                         order: &[&str]| {
        order
            .iter()
            .flat_map(|slot| {
                public
                    .iter()
                    .enumerate()
                    .filter(move |(_, section)| matches(section, slot))
                    .map(|(index, _)| index)
            })
            .collect::<Vec<_>>()
    };
    let by_type =
        |section: &DiscoverySectionResult, slot: &str| discovery_section_type_key(section) == slot;
    let by_world_slot =
        |section: &DiscoverySectionResult, slot: &str| discovery_world_shelf_slot(section) == slot;
    let promotion = indices_where(&by_type, DISCOVERY_PUBLIC_PROMOTION_TYPES);
    let world = indices_where(&by_world_slot, DISCOVERY_WORLD_SHELF_ORDER);
    let curated = indices_where(&by_type, DISCOVERY_CURATED_TYPES);
    let placed = promotion
        .iter()
        .chain(&world)
        .chain(&curated)
        .copied()
        .collect::<HashSet<_>>();
    let remaining = (0..public.len()).filter(|index| !placed.contains(index));
    let public_order = promotion
        .iter()
        .chain(&world)
        .copied()
        .chain(remaining)
        .chain(curated.iter().copied());

    // Library rails never hide for being thin; public rails do.
    let ordered = shelf
        .into_iter()
        .chain(complete_collection)
        .chain(unknown)
        .chain(fallback)
        .map(|section| (section, true))
        .chain(public_order.map(|index| (&public[index], false)));

    let mut seen = HashSet::new();
    let mut target = None;
    let mut hero_shown = false;
    for (section, floor_exempt) in ordered {
        let preceding_keys =
            (target.is_none() && section.section_id == section_id).then(|| seen.clone());
        let items = section
            .items
            .iter()
            .filter(|item| seen.insert(discovery_presented_item_key(item)))
            .cloned()
            .collect::<Vec<_>>();
        let renders =
            !items.is_empty() && (floor_exempt || items.len() >= DISCOVERY_MIN_PUBLIC_RAIL_ITEMS);
        if renders && let Some(hero_key) = hero_key {
            hero_shown |= items
                .iter()
                .any(|item| discovery_presented_item_key(item) == hero_key);
        }
        if let Some(preceding_keys) = preceding_keys {
            if !renders {
                return None;
            }
            target = Some((
                DiscoverySectionResult {
                    total_count: items.len() as i64,
                    items,
                    ..section.clone()
                },
                preceding_keys,
            ));
        }
    }
    let (mut section, mut preceding_keys) = target?;
    if hero_shown && let Some(hero_key) = hero_key {
        section
            .items
            .retain(|item| discovery_presented_item_key(item) != hero_key);
        if section.items.is_empty() {
            return None;
        }
        section.total_count = section.items.len() as i64;
        preceding_keys.insert(hero_key.to_string());
    }
    Some(DiscoveryRenderedWindow {
        section,
        preceding_keys,
    })
}

/// The presented key of a composed home's hero, for [`discovery_rendered_window`].
pub(super) fn discovery_home_hero_key(home: &DiscoveryHomeResult) -> Option<String> {
    home.hero_item.as_ref().map(discovery_presented_item_key)
}

/// A section's full list behind its rendered window: the window first, then
/// every title of `full` the dashboard has not already shown above or in it.
/// Without a rendered window the full list stands as it is.
pub(super) fn continue_discovery_rendered_window(
    window: Option<DiscoveryRenderedWindow>,
    full: Option<DiscoverySectionResult>,
) -> Option<DiscoverySectionResult> {
    let Some(DiscoveryRenderedWindow {
        mut section,
        preceding_keys: mut seen,
    }) = window
    else {
        return full;
    };
    seen.extend(section.items.iter().map(discovery_presented_item_key));
    if let Some(full) = full {
        section.items.extend(
            full.items
                .into_iter()
                .filter(|item| seen.insert(discovery_presented_item_key(item))),
        );
    }
    section.total_count = section.items.len() as i64;
    Some(section)
}

/// A personalized rail from a rebuilt home shelf, continued past its window
/// over `listable`: the rail's own eligible titles not claimed by it, by an
/// earlier rail or by the viewer's watchlist (`watchlist_claims`), in rail
/// windows.
pub(super) fn continue_discovery_personalized_rail(
    section_id: &str,
    shelf: Vec<DiscoverySectionResult>,
    listable: Vec<DiscoveryItemRecord>,
    context: &DiscoveryRailContext<'_>,
    watchlist_claims: &HashSet<String>,
) -> Option<DiscoverySectionResult> {
    let position = shelf
        .iter()
        .position(|section| section.section_id == section_id)?;
    let mut claimed = watchlist_claims.clone();
    for section in &shelf[..=position] {
        claimed.extend(
            section
                .items
                .iter()
                .map(|item| discovery_item_identity_key(item).to_string()),
        );
    }
    let mut section = shelf.into_iter().nth(position)?;
    let family = match discovery_section_type_key(&section).as_str() {
        "BECAUSE_YOU_LIKE_TAG" => Some(DiscoveryRailLabelFamily::Theme),
        "BECAUSE_YOU_LIKE_GENRE" => Some(DiscoveryRailLabelFamily::Genre),
        _ => None,
    };
    let members = match family {
        Some(family) => {
            let entry = discovery_rail_label(family, section.label_key.as_deref()?)?;
            listable
                .into_iter()
                .filter(|item| {
                    item.matched_subject_count > 0
                        && discovery_item_matches_affinity_label(
                            item,
                            entry.key,
                            discovery_rail_label_family_kind(entry.family),
                            context.settings.genre_rail_corroboration,
                        )
                })
                .collect()
        }
        None => listable,
    };
    section
        .items
        .extend(compose_discovery_browse_items(members, context, &claimed));
    section.total_count = section.items.len() as i64;
    Some(section)
}

/// Titles with a request still waiting, matched by media kind and external
/// id together, so a request for a movie never hides the series that shares
/// its id number.
#[derive(Debug, Default)]
pub(super) struct DiscoveryRequestedVisibility {
    keys: HashSet<String>,
}

impl DiscoveryRequestedVisibility {
    pub(super) fn from_media_requests(requests: &[scryer_domain::MediaRequest]) -> Self {
        let mut keys = HashSet::new();
        for request in requests {
            let mut kinds = vec![discovery_media_kind_for_facet(request.facet.clone())];
            // An anime title can arrive as a series without a content type.
            if request.facet == MediaFacet::Anime {
                kinds.push("series");
            }
            for external_id in &request.external_ids {
                for source in discovery_requested_source_aliases(&external_id.source) {
                    for value in discovery_requested_value_aliases(&external_id.value) {
                        for kind in &kinds {
                            keys.insert(format!("{kind}:{source}:{value}"));
                        }
                    }
                }
            }
        }
        Self { keys }
    }

    pub(super) fn item_is_requested(&self, item: &DiscoveryItemRecord) -> bool {
        if self.keys.is_empty() {
            return false;
        }
        let Some(kind) = discovery_item_media_kind(item) else {
            return false;
        };
        let parts = item
            .target_key
            .split(':')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.len() < 3 {
            return false;
        }
        let value = parts[2..].join(":");
        discovery_requested_source_aliases(parts[0])
            .iter()
            .any(|source| {
                discovery_requested_value_aliases(&value)
                    .iter()
                    .any(|value| self.keys.contains(&format!("{kind}:{source}:{value}")))
            })
    }
}

fn discovery_requested_source_aliases(source: &str) -> Vec<String> {
    let raw = normalize_catalog_owned_key(source);
    let mut aliases = vec![raw.clone()];
    if let Some(canonical) = normalize_supported_external_id_source(source)
        && canonical != raw
    {
        aliases.push(canonical);
    }
    aliases.retain(|alias| !alias.is_empty());
    aliases
}

fn discovery_requested_value_aliases(value: &str) -> Vec<String> {
    let raw = normalize_catalog_owned_key(value);
    let mut aliases = vec![raw.clone()];
    if let Some(numeric) = parse_positive_external_numeric_id(value).map(|id| id.to_string())
        && numeric != raw
    {
        aliases.push(numeric);
    }
    aliases.retain(|alias| !alias.is_empty());
    aliases
}

/// A section's composed member list with the pool rows it was ranked from.
#[derive(Debug, Default)]
pub(super) struct DiscoverySectionMembers {
    section: Option<DiscoverySectionResult>,
    candidates_by_id: HashMap<String, DiscoveryHomeCandidate>,
    /// The pending requests read while composing, if any were.
    pending_requests: Option<Vec<scryer_domain::MediaRequest>>,
}

/// A composed, filtered and sorted section as the cache holds it: the
/// header, then each member once, in order.
#[derive(Debug)]
pub(super) struct DiscoverySectionPages {
    /// The section's header fields; its item list is empty.
    header: DiscoverySectionResult,
    members: Vec<DiscoverySectionMember>,
}

#[derive(Debug)]
enum DiscoverySectionMember {
    /// A pool row; its `item` is the section's item, subject resolution
    /// included, and the rest is what hydration reads.
    Candidate(Box<DiscoveryHomeCandidate>),
    /// A title the composed home showed with no pool row to hydrate from.
    Item(Box<DiscoveryItemRecord>),
}

impl DiscoverySectionPages {
    fn from_members(members: DiscoverySectionMembers) -> Option<Self> {
        let DiscoverySectionMembers {
            section,
            mut candidates_by_id,
            ..
        } = members;
        let mut header = section?;
        let members = std::mem::take(&mut header.items)
            .into_iter()
            .map(|item| match candidates_by_id.remove(&item.id) {
                Some(mut candidate) => {
                    candidate.item = item;
                    DiscoverySectionMember::Candidate(Box::new(candidate))
                }
                None => DiscoverySectionMember::Item(Box::new(item)),
            })
            .collect::<Vec<_>>();
        header.total_count = members.len() as i64;
        Some(Self { header, members })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DiscoverySectionCacheKey {
    actor_id: String,
    readable_library_ids: Vec<String>,
    allowed_media_kinds: Vec<String>,
    can_view_personalized: bool,
    generation_ids: Vec<String>,
    section_id: String,
    /// Debug form: the filters carry a float, so they have no `Hash`.
    filters: String,
    sort: DiscoverySectionSort,
    include_owned: bool,
    include_unresolved: bool,
    hide_requested: bool,
    rail_limit: usize,
}

/// Composed section member lists per viewer and section state, so paging
/// past the first page only slices and hydrates. Page one always recomposes
/// and replaces its entry; entries expire and the oldest is evicted first.
#[derive(Default)]
pub(crate) struct DiscoverySectionCache {
    entries: Mutex<HashMap<DiscoverySectionCacheKey, (Instant, Arc<DiscoverySectionPages>)>>,
}

fn discovery_section_cache_entry_live(stored_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(stored_at) < DISCOVERY_SECTION_CACHE_TTL
}

impl DiscoverySectionCache {
    /// The cached pages a request at `offset` may reuse: none for the first
    /// page, which always recomposes.
    fn pages_for_offset(
        &self,
        key: &DiscoverySectionCacheKey,
        offset: usize,
        now: Instant,
    ) -> Option<Arc<DiscoverySectionPages>> {
        if offset == 0 {
            return None;
        }
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|_, (stored_at, _)| discovery_section_cache_entry_live(*stored_at, now));
        entries.get(key).map(|(_, pages)| Arc::clone(pages))
    }

    fn insert(
        &self,
        key: DiscoverySectionCacheKey,
        pages: Arc<DiscoverySectionPages>,
        now: Instant,
    ) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|_, (stored_at, _)| discovery_section_cache_entry_live(*stored_at, now));
        if entries.len() >= DISCOVERY_SECTION_CACHE_ENTRIES
            && !entries.contains_key(&key)
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (stored_at, _))| *stored_at)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(key, (now, pages));
    }
}

/// The discovery generations a section page is composed from: the
/// personalized context run when the viewer may see it, then the public feed
/// run. A client that sees them change between pages starts over.
fn discovery_section_generation_ids(
    status: &DiscoverySyncStatus,
    can_view_personalized: bool,
) -> Vec<String> {
    can_view_personalized
        .then_some(status.state.last_success_generation_id.as_deref())
        .flatten()
        .into_iter()
        .chain(status.state.last_public_feed_generation_id.as_deref())
        .map(str::to_string)
        .collect()
}

impl AppUseCase {
    pub(super) async fn discovery_browse_query(
        &self,
        visibility: &DiscoveryVisibility,
        state: &DiscoverySyncStateRecord,
        can_view_personalized: bool,
        query: &DiscoveryItemsQuery,
        section: &DiscoverySectionItemsQuery,
    ) -> AppResult<DiscoveryItemsStorageQuery> {
        let owned = if query.include_owned {
            CatalogOwnedVisibility::default()
        } else {
            self.discovery_home_owned_visibility(
                &sorted_discovery_library_ids(&visibility.readable_library_ids),
                state,
            )
            .await?
        };
        self.discovery_browse_query_with_owned(
            visibility,
            state,
            can_view_personalized,
            query,
            section,
            &owned,
        )
        .await
    }

    pub(super) async fn discovery_browse_query_with_owned(
        &self,
        visibility: &DiscoveryVisibility,
        state: &DiscoverySyncStateRecord,
        can_view_personalized: bool,
        query: &DiscoveryItemsQuery,
        section: &DiscoverySectionItemsQuery,
        owned: &CatalogOwnedVisibility,
    ) -> AppResult<DiscoveryItemsStorageQuery> {
        let requested_keys = if section.hide_requested {
            self.discovery_pending_request_visibility(visibility)
                .await?
                .keys
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        let browse_label = match discovery_section_target(section.section_id.trim()) {
            DiscoverySectionTarget::Browse(entry) => Some((
                discovery_rail_label_family_kind(entry.family).to_string(),
                entry.key.to_string(),
            )),
            _ => None,
        };
        Ok(DiscoveryItemsStorageQuery {
            context_run_id: can_view_personalized
                .then(|| state.last_success_generation_id.clone())
                .flatten(),
            public_run_id: state.last_public_feed_generation_id.clone(),
            readable_library_ids: sorted_discovery_library_ids(&visibility.readable_library_ids),
            allowed_media_kinds: visibility.sorted_allowed_media_kinds(),
            filters: DiscoveryItemsQuery {
                include_owned: query.include_owned,
                include_unresolved: query.include_unresolved,
                ..DiscoveryItemsQuery::default()
            },
            home_filters: section.filters.clone(),
            browse_label,
            sort: Some(section.sort),
            excluded_identity_keys: owned.excluded_discovery_identity_keys(),
            excluded_title_ids: owned.excluded_title_ids(),
            requested_keys,
            limit: discovery_items_limit(query.limit),
            offset: query.offset,
        })
    }

    pub(super) async fn discovery_pool_browse_labels(
        &self,
        query: &DiscoveryItemsStorageQuery,
    ) -> AppResult<Vec<DiscoveryBrowseLabel>> {
        let labels = DISCOVERY_RAIL_LABELS
            .iter()
            .map(|entry| {
                (
                    discovery_rail_label_family_kind(entry.family).to_string(),
                    entry.key.to_string(),
                )
            })
            .collect::<Vec<_>>();
        let counts = self
            .services
            .library
            .discovery
            .count_discovery_browse_labels(query, &labels)
            .await?;
        Ok(DISCOVERY_RAIL_LABELS
            .iter()
            .zip(counts)
            .filter(|(_, count)| *count > 0)
            .map(|(entry, item_count)| DiscoveryBrowseLabel {
                family: match entry.family {
                    DiscoveryRailLabelFamily::Genre => DiscoveryBrowseLabelFamily::Genre,
                    DiscoveryRailLabelFamily::Theme => DiscoveryBrowseLabelFamily::Theme,
                },
                label_key: entry.key.to_string(),
                section_id: discovery_browse_section_id(entry),
                title: discovery_browse_title(entry),
                item_count,
            })
            .collect())
    }

    pub(super) async fn discovery_section_items(
        &self,
        actor: &User,
        query: &DiscoveryItemsQuery,
        section_query: &DiscoverySectionItemsQuery,
    ) -> AppResult<DiscoveryItemsResult> {
        let visibility = self.discovery_visibility(actor).await?;
        let can_view_personalized = !visibility.readable_library_ids.is_empty()
            && self.personalized_discovery_enabled().await?;
        let status = self
            .load_discovery_sync_status_for_visibility(can_view_personalized)
            .await?;
        let generation_ids = discovery_section_generation_ids(&status, can_view_personalized);
        let browse_entry = match discovery_section_target(section_query.section_id.trim()) {
            DiscoverySectionTarget::Browse(entry) => Some(entry),
            _ => None,
        };
        if section_query.section_id.trim() == "browse_all" || browse_entry.is_some() {
            let storage_query = self
                .discovery_browse_query(
                    &visibility,
                    &status.state,
                    can_view_personalized,
                    query,
                    section_query,
                )
                .await?;
            let mut page = self
                .services
                .library
                .discovery
                .query_discovery_items(&storage_query)
                .await?;
            let subjects = if let Some(run_id) = storage_query.context_run_id.as_deref() {
                self.discovery_submitted_subjects_for_items(
                    run_id,
                    &sorted_discovery_library_ids(&visibility.readable_library_ids),
                    page.items.iter(),
                )
                .await?
            } else {
                Vec::new()
            };
            // Reasons shown on cards remain scoped to readable library titles;
            // the presence of a reason never decides browse membership.
            resolve_discovery_matched_subjects(&mut page.items, &subjects)?;
            let section = DiscoverySectionResult {
                section_id: section_query.section_id.clone(),
                section_type: browse_entry
                    .map(|entry| match entry.family {
                        DiscoveryRailLabelFamily::Genre => DISCOVERY_BROWSE_GENRE_SECTION_TYPE,
                        DiscoveryRailLabelFamily::Theme => DISCOVERY_BROWSE_THEME_SECTION_TYPE,
                    })
                    .unwrap_or("BROWSE_ALL")
                    .to_string(),
                title: browse_entry
                    .map(discovery_browse_title)
                    .unwrap_or_else(|| "Discovery".to_string()),
                surface: match (
                    storage_query.context_run_id.is_some(),
                    storage_query.public_run_id.is_some(),
                ) {
                    (true, true) => "mixed",
                    (true, false) => "personalized",
                    (false, _) => "public",
                }
                .to_string(),
                total_count: page.total_count,
                items: page.items.clone(),
                label_key: browse_entry.map(|entry| entry.key.to_string()),
            };
            return Ok(DiscoveryItemsResult {
                items: page.items,
                total_count: page.total_count,
                can_view_personalized,
                section: Some(section),
                generation_ids,
            });
        }
        let cache_key = DiscoverySectionCacheKey {
            actor_id: actor.id.clone(),
            readable_library_ids: sorted_discovery_library_ids(&visibility.readable_library_ids),
            allowed_media_kinds: visibility.sorted_allowed_media_kinds(),
            can_view_personalized,
            generation_ids: generation_ids.clone(),
            section_id: section_query.section_id.trim().to_string(),
            filters: format!("{:?}", section_query.filters),
            sort: section_query.sort,
            include_owned: query.include_owned,
            include_unresolved: query.include_unresolved,
            hide_requested: section_query.hide_requested,
            rail_limit: discovery_section_limit(section_query.rail_limit),
        };
        let cache = &self.runtime.library.discovery_section_cache;
        let cached = cache.pages_for_offset(&cache_key, query.offset, Instant::now());
        let pages = match cached {
            Some(pages) => Some(pages),
            None => {
                let mut members = self
                    .discovery_section_members(
                        actor,
                        &visibility,
                        can_view_personalized,
                        &status,
                        section_query,
                        query.include_owned,
                        query.include_unresolved,
                    )
                    .await?;
                if let Some(section) = members.section.as_mut() {
                    // The watchlist never offers a requested title, so it
                    // needs no second pass.
                    if section_query.hide_requested
                        && !matches!(
                            discovery_section_target(section_query.section_id.trim()),
                            DiscoverySectionTarget::Watchlist
                        )
                    {
                        // The requests the composition already read, else one read.
                        let requested = match members.pending_requests.as_deref() {
                            Some(pending) => {
                                DiscoveryRequestedVisibility::from_media_requests(pending)
                            }
                            None => {
                                self.discovery_pending_request_visibility(&visibility)
                                    .await?
                            }
                        };
                        section
                            .items
                            .retain(|item| !requested.item_is_requested(item));
                    }
                    sort_discovery_section_items(
                        &mut section.items,
                        section_query.sort,
                        &members.candidates_by_id,
                    );
                }
                let pages = DiscoverySectionPages::from_members(members).map(Arc::new);
                if let Some(pages) = &pages {
                    cache.insert(cache_key, Arc::clone(pages), Instant::now());
                }
                pages
            }
        };

        let Some(pages) = pages else {
            return Ok(DiscoveryItemsResult {
                items: Vec::new(),
                total_count: 0,
                can_view_personalized,
                section: None,
                generation_ids,
            });
        };
        let page = self
            .discovery_section_page(&pages, query.offset, discovery_items_limit(query.limit))
            .await?;

        Ok(DiscoveryItemsResult {
            items: page.items.clone(),
            total_count: page.total_count,
            can_view_personalized,
            section: Some(page),
            generation_ids,
        })
    }

    /// The home as the section's dashboard composed it: same filters, same
    /// rail size, owned titles excluded.
    /// It reuses the viewer's visibility and sync status this request already
    /// read; the status is read again only when the personalized switch read
    /// here gives a different personalized gate. The pending requests the
    /// home's watchlist read come back with it, since they depend only on the
    /// viewer's visibility.
    async fn discovery_section_home(
        &self,
        actor: &User,
        visibility: &DiscoveryVisibility,
        can_view_personalized: bool,
        status: &DiscoverySyncStatus,
        section_query: &DiscoverySectionItemsQuery,
        include_unresolved: bool,
    ) -> AppResult<(
        DiscoveryHomeResult,
        Option<CatalogOwnedVisibility>,
        Option<Vec<scryer_domain::MediaRequest>>,
    )> {
        let started_at = std::time::Instant::now();
        let personalized_discovery_enabled = self.personalized_discovery_enabled().await?;
        let home_can_view_personalized =
            !visibility.readable_library_ids.is_empty() && personalized_discovery_enabled;
        let reuses_status = home_can_view_personalized == can_view_personalized;
        let home_status = if reuses_status {
            status.clone()
        } else {
            self.load_discovery_sync_status_for_visibility(home_can_view_personalized)
                .await?
        };
        let (home, owned_visibility, pending_requests) = self
            .discovery_home_projected_for(
                actor,
                DiscoveryHomeQuery {
                    include_public: true,
                    include_personalized: true,
                    include_unresolved,
                    limit_per_section: discovery_section_limit(section_query.rail_limit),
                    filters: section_query.filters.clone(),
                },
                // The hero is selected so the section leaves out the title the
                // home's hero shows, as the home's rails do.
                crate::DiscoveryHomeProjection {
                    hero: true,
                    ..crate::DiscoveryHomeProjection::default()
                },
                visibility,
                personalized_discovery_enabled,
                home_status,
                started_at,
            )
            .await?;
        // The home's owned visibility answers for the section only when it was
        // built from the section's own sync state.
        Ok((
            home,
            reuses_status.then_some(owned_visibility),
            pending_requests,
        ))
    }

    /// The owned visibility the section's home computed, or the same one read
    /// for the section's sync state.
    async fn discovery_section_owned_visibility(
        &self,
        visibility: &DiscoveryVisibility,
        status: &DiscoverySyncStatus,
        home_owned_visibility: Option<CatalogOwnedVisibility>,
    ) -> AppResult<CatalogOwnedVisibility> {
        match home_owned_visibility {
            Some(owned_visibility) => Ok(owned_visibility),
            None => {
                self.discovery_home_owned_visibility(
                    &sorted_discovery_library_ids(&visibility.owned_library_ids),
                    &status.state,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn discovery_section_members(
        &self,
        actor: &User,
        visibility: &DiscoveryVisibility,
        can_view_personalized: bool,
        status: &DiscoverySyncStatus,
        section_query: &DiscoverySectionItemsQuery,
        include_owned: bool,
        include_unresolved: bool,
    ) -> AppResult<DiscoverySectionMembers> {
        let section_id = section_query.section_id.trim();
        let rail_limit = discovery_section_limit(section_query.rail_limit);
        let filters = &section_query.filters;
        let readable_library_id_list =
            sorted_discovery_library_ids(&visibility.readable_library_ids);
        let allowed_media_kinds = visibility.sorted_allowed_media_kinds();
        let context_run_id = can_view_personalized
            .then_some(status.state.last_success_generation_id.as_deref())
            .flatten();
        let public_run_id = status.state.last_public_feed_generation_id.as_deref();
        let mut candidates_by_id = HashMap::<String, DiscoveryHomeCandidate>::new();
        let mut pending_requests = None;

        let section = match discovery_section_target(section_id) {
            DiscoverySectionTarget::Watchlist => {
                // Composed as the home composes it, every member listed; its
                // cards are whole already, so nothing is hydrated per page.
                self.discovery_section_watchlist(
                    actor,
                    visibility,
                    can_view_personalized,
                    status,
                    filters,
                    include_unresolved,
                    usize::MAX,
                )
                .await?
                .section
            }
            DiscoverySectionTarget::Browse(_) => {
                unreachable!("browse sections are paged directly by discovery_section_items")
            }
            DiscoverySectionTarget::PersonalizedRail => {
                let Some(context_run_id) = context_run_id else {
                    return Ok(DiscoverySectionMembers::default());
                };
                let pool = self
                    .services
                    .library
                    .discovery
                    .list_personalized_discovery_home_items(
                        context_run_id,
                        &readable_library_id_list,
                        &allowed_media_kinds,
                        include_unresolved,
                        include_owned,
                        filters,
                        DISCOVERY_PERSONALIZED_SECTION_MAX_CANDIDATES as i64,
                    )
                    .await?;
                let mut items = home_selection_items_from_candidates(pool, &mut candidates_by_id);
                // The home reads the first rows of the same ordered pool with
                // owned titles left out; those rows alone set the evidence
                // median and the rails' claims.
                let home_candidate_limit = personalized_home_candidate_limit(rail_limit);
                let home_item_ids = items
                    .iter()
                    .filter(|item| !item.owned_in_input)
                    .take(home_candidate_limit)
                    .map(|item| item.id.clone())
                    .collect::<HashSet<_>>();
                items.retain(|item| visibility.allows_item(item));
                let submitted_subjects = self
                    .discovery_submitted_subjects_for_items(
                        context_run_id,
                        &sorted_discovery_library_ids(&visibility.readable_library_ids),
                        items.iter(),
                    )
                    .await?;
                resolve_discovery_matched_subjects(&mut items, &submitted_subjects)?;
                let rail_settings = self.discovery_rail_composition_settings().await;
                let library_profile = self
                    .discovery_library_affinity_profile(
                        &visibility.readable_library_ids,
                        Some(context_run_id),
                        rail_settings.genre_rail_corroboration,
                    )
                    .await?;
                let home_items = items
                    .iter()
                    .filter(|item| home_item_ids.contains(&item.id))
                    .cloned()
                    .collect::<Vec<_>>();
                let home_visible_items =
                    discovery_home_visible_items(&home_items, include_unresolved);
                // Showing owned titles widens what may be listed, nothing
                // else: the resolved-only rule still applies.
                let listable_items = items
                    .into_iter()
                    .filter(|item| {
                        (include_owned || !item.owned_in_input)
                            && (include_unresolved || item.resolved)
                    })
                    .collect::<Vec<_>>();
                let context = DiscoveryRailContext::new(
                    &candidates_by_id,
                    rail_settings,
                    &library_profile,
                    &home_visible_items,
                    rail_limit,
                );
                // The home's own watchlist claims, so page one is the
                // home rail and later pages never offer a title the
                // watchlist holds.
                let watchlist = self
                    .discovery_section_watchlist(
                        actor,
                        visibility,
                        can_view_personalized,
                        status,
                        filters,
                        include_unresolved,
                        rail_limit,
                    )
                    .await?;
                pending_requests = watchlist.pending_requests;
                let watchlist_claims = watchlist.claimed_item_keys;
                let shelf = personalized_section_results_in_context(
                    home_visible_items,
                    &library_profile,
                    &context,
                    &watchlist_claims,
                );
                continue_discovery_personalized_rail(
                    section_id,
                    shelf,
                    listable_items,
                    &context,
                    &watchlist_claims,
                )
            }
            DiscoverySectionTarget::TopRated => {
                let (home, home_owned_visibility, home_pending_requests) = self
                    .discovery_section_home(
                        actor,
                        visibility,
                        can_view_personalized,
                        status,
                        section_query,
                        include_unresolved,
                    )
                    .await?;
                pending_requests = home_pending_requests;
                let hero_key = discovery_home_hero_key(&home);
                let window = discovery_rendered_window(&home, section_id, hero_key.as_deref());
                let owned_visibility = self
                    .discovery_section_owned_visibility(visibility, status, home_owned_visibility)
                    .await?;
                let excluded_identity_keys = if include_owned {
                    Vec::new()
                } else {
                    owned_visibility.excluded_discovery_identity_keys()
                };
                let candidates = self
                    .services
                    .library
                    .discovery
                    .list_discovery_home_top_rated_items(
                        public_run_id,
                        context_run_id,
                        &readable_library_id_list,
                        &allowed_media_kinds,
                        &readable_library_id_list,
                        &excluded_identity_keys,
                        include_unresolved,
                        filters,
                        top_rated_home_candidate_limit(rail_limit) as i64,
                    )
                    .await?;
                let mut items =
                    home_selection_items_from_candidates(candidates, &mut candidates_by_id);
                items.retain(|item| {
                    visibility.allows_item(item)
                        && (include_owned || !owned_visibility.item_is_owned(item))
                });
                let items = ranked_top_rated_discovery_items(
                    items.iter(),
                    &candidates_by_id,
                    include_unresolved,
                    include_owned,
                );
                let full = (!items.is_empty()).then(|| DiscoverySectionResult {
                    section_id: DISCOVERY_TOP_RATED_SECTION_ID.to_string(),
                    section_type: "TOP_RATED".to_string(),
                    title: "Top Rated".to_string(),
                    surface: "mixed".to_string(),
                    total_count: items.len() as i64,
                    items,
                    label_key: None,
                });
                continue_discovery_rendered_window(window, full)
            }
            DiscoverySectionTarget::CompleteCollection => {
                let Some(context_run_id) = context_run_id else {
                    return Ok(DiscoverySectionMembers::default());
                };
                let (home, _, home_pending_requests) = self
                    .discovery_section_home(
                        actor,
                        visibility,
                        can_view_personalized,
                        status,
                        section_query,
                        include_unresolved,
                    )
                    .await?;
                pending_requests = home_pending_requests;
                let hero_key = discovery_home_hero_key(&home);
                let window = discovery_rendered_window(&home, section_id, hero_key.as_deref());
                let candidates = self
                    .services
                    .library
                    .discovery
                    .list_personalized_complete_collection_items(
                        context_run_id,
                        &readable_library_id_list,
                        &allowed_media_kinds,
                        include_unresolved,
                        filters,
                        complete_collection_candidate_limit(rail_limit) as i64,
                    )
                    .await?;
                let mut items =
                    home_selection_items_from_candidates(candidates, &mut candidates_by_id);
                items.retain(|item| visibility.allows_item(item));
                let submitted_subjects = self
                    .discovery_submitted_subjects_for_items(
                        context_run_id,
                        &sorted_discovery_library_ids(&visibility.readable_library_ids),
                        items.iter(),
                    )
                    .await?;
                resolve_discovery_matched_subjects(&mut items, &submitted_subjects)?;
                continue_discovery_rendered_window(
                    window,
                    complete_collection_section(&items, include_unresolved, usize::MAX),
                )
            }
            DiscoverySectionTarget::Public => {
                let Some(public_run_id) = public_run_id else {
                    return Ok(DiscoverySectionMembers::default());
                };
                let sections = self
                    .services
                    .library
                    .discovery
                    .list_discovery_sections(public_run_id, Some("public"))
                    .await?;
                let source_ids = discovery_public_section_sources(section_id, &sections);
                if source_ids.is_empty() {
                    return Ok(DiscoverySectionMembers::default());
                }
                let (home, home_owned_visibility, home_pending_requests) = self
                    .discovery_section_home(
                        actor,
                        visibility,
                        can_view_personalized,
                        status,
                        section_query,
                        include_unresolved,
                    )
                    .await?;
                pending_requests = home_pending_requests;
                let hero_key = discovery_home_hero_key(&home);
                let window = discovery_rendered_window(&home, section_id, hero_key.as_deref());
                let records = self
                    .services
                    .library
                    .discovery
                    .list_public_discovery_section_items_in(
                        public_run_id,
                        &sections,
                        &source_ids,
                        &allowed_media_kinds,
                        include_unresolved,
                        filters,
                        DISCOVERY_PUBLIC_SECTION_MAX_ITEMS as i64,
                    )
                    .await?;
                let owned_visibility = if include_owned {
                    CatalogOwnedVisibility::default()
                } else {
                    self.discovery_section_owned_visibility(
                        visibility,
                        status,
                        home_owned_visibility,
                    )
                    .await?
                };
                let records = records
                    .into_iter()
                    .filter_map(|record| {
                        home_section_candidates_record_to_result(record, &mut candidates_by_id)
                    })
                    .map(|mut record| {
                        record.items.retain(|item| {
                            visibility.allows_item(item)
                                && (include_owned || !owned_visibility.item_is_owned(item))
                        });
                        record
                    })
                    .collect::<Vec<_>>();
                continue_discovery_rendered_window(
                    window,
                    fold_discovery_public_section(section_id, records, rail_limit),
                )
            }
        };

        Ok(DiscoverySectionMembers {
            section,
            candidates_by_id,
            pending_requests,
        })
    }

    /// The viewer's watchlist under the home's gates: the personalized
    /// switch, the viewer's visibility and owned titles, the filter bar and
    /// `include_unresolved`.
    #[allow(clippy::too_many_arguments)]
    async fn discovery_section_watchlist(
        &self,
        actor: &User,
        visibility: &DiscoveryVisibility,
        can_view_personalized: bool,
        status: &DiscoverySyncStatus,
        filters: &crate::ports::DiscoveryHomeFilters,
        include_unresolved: bool,
        limit: usize,
    ) -> AppResult<DiscoveryWatchlistComposition> {
        if !self.personalized_discovery_enabled().await? {
            return Ok(DiscoveryWatchlistComposition::default());
        }
        let owned_library_ids = &visibility.owned_library_ids;
        self.discovery_watchlist_composition(
            actor,
            visibility,
            &sorted_discovery_library_ids(owned_library_ids),
            &status.state,
            can_view_personalized,
            filters,
            include_unresolved,
            limit,
        )
        .await
    }

    /// Pending requests in every library the viewer can request into or
    /// read. The toggle exists to avoid duplicate requests, so it counts
    /// every requester's pending requests, not only the viewer's.
    async fn discovery_pending_request_visibility(
        &self,
        visibility: &DiscoveryVisibility,
    ) -> AppResult<DiscoveryRequestedVisibility> {
        Ok(DiscoveryRequestedVisibility::from_media_requests(
            &self.discovery_pending_requests(visibility).await?,
        ))
    }

    /// Every pending request in a library the viewer can request into or
    /// read, whoever asked.
    pub(super) async fn discovery_pending_requests(
        &self,
        visibility: &DiscoveryVisibility,
    ) -> AppResult<Vec<scryer_domain::MediaRequest>> {
        let mut library_ids = visibility
            .requestable_library_ids
            .iter()
            .chain(&visibility.owned_library_ids)
            .cloned()
            .collect::<Vec<_>>();
        library_ids.sort();
        library_ids.dedup();
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.services
            .catalog
            .media_requests
            .list(MediaRequestQuery {
                library_ids: Some(library_ids),
                status: Some(MediaRequestStatus::Pending),
                ..MediaRequestQuery::default()
            })
            .await
    }

    /// One page of a composed section, its pool rows hydrated for display.
    async fn discovery_section_page(
        &self,
        pages: &DiscoverySectionPages,
        offset: usize,
        limit: usize,
    ) -> AppResult<DiscoverySectionResult> {
        let mut items = Vec::new();
        let mut candidates = Vec::new();
        for member in pages.members.iter().skip(offset).take(limit) {
            match member {
                DiscoverySectionMember::Candidate(candidate) => {
                    items.push(candidate.item.clone());
                    candidates.push(candidate.as_ref().clone());
                }
                DiscoverySectionMember::Item(item) => items.push(item.as_ref().clone()),
            }
        }
        let header = &pages.header;
        let mut page = DiscoverySectionResult {
            section_id: header.section_id.clone(),
            section_type: header.section_type.clone(),
            title: header.title.clone(),
            surface: header.surface.clone(),
            total_count: header.total_count,
            items,
            label_key: header.label_key.clone(),
        };
        if candidates.is_empty() {
            return Ok(page);
        }
        self.services
            .library
            .discovery
            .hydrate_discovery_home_candidates(&mut candidates)
            .await?;
        let hydrated_by_id = candidates
            .into_iter()
            .map(|candidate| (candidate.item.id.clone(), candidate.item))
            .collect::<HashMap<_, _>>();
        // Subject resolution already ran on the pool these items came from;
        // the hydrated rows do not carry it.
        for item in page.items.iter_mut() {
            replace_discovery_home_item(item, &hydrated_by_id, false);
        }
        Ok(page)
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn key(actor_id: &str) -> DiscoverySectionCacheKey {
        DiscoverySectionCacheKey {
            actor_id: actor_id.to_string(),
            readable_library_ids: vec!["library-a".to_string()],
            allowed_media_kinds: vec!["movie".to_string()],
            can_view_personalized: true,
            generation_ids: vec!["context-1".to_string(), "public-1".to_string()],
            section_id: "for_you".to_string(),
            filters: String::new(),
            sort: DiscoverySectionSort::default(),
            include_owned: false,
            include_unresolved: false,
            hide_requested: false,
            rail_limit: 25,
        }
    }

    fn pages(title: &str) -> Arc<DiscoverySectionPages> {
        Arc::new(DiscoverySectionPages {
            header: DiscoverySectionResult {
                section_id: "for_you".to_string(),
                section_type: "FOR_YOU".to_string(),
                title: title.to_string(),
                surface: "personalized".to_string(),
                total_count: 0,
                items: Vec::new(),
                label_key: None,
            },
            members: Vec::new(),
        })
    }

    #[test]
    fn later_pages_reuse_the_composed_section_and_page_one_replaces_it() {
        let cache = DiscoverySectionCache::default();
        let start = Instant::now();
        let first = pages("Synthetic First");
        cache.insert(key("viewer-a"), Arc::clone(&first), start);

        let second_page = cache
            .pages_for_offset(&key("viewer-a"), 40, start)
            .expect("a later page is served from the cache");
        assert!(Arc::ptr_eq(&second_page, &first));

        // Page one never reads the cache; its recomposed section replaces
        // the entry.
        assert!(cache.pages_for_offset(&key("viewer-a"), 0, start).is_none());
        let recomposed = pages("Synthetic Recomposed");
        cache.insert(key("viewer-a"), Arc::clone(&recomposed), start);
        let after = cache
            .pages_for_offset(&key("viewer-a"), 40, start)
            .expect("the replaced entry is served");
        assert!(Arc::ptr_eq(&after, &recomposed));

        assert!(
            cache
                .pages_for_offset(&key("viewer-b"), 40, start)
                .is_none()
        );

        let expired_at = start + DISCOVERY_SECTION_CACHE_TTL;
        assert!(
            cache
                .pages_for_offset(&key("viewer-a"), 40, expired_at)
                .is_none()
        );
        let entries = cache.entries.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(entries.is_empty(), "a read drops expired entries");
    }

    #[test]
    fn cached_pages_hold_each_member_once() {
        let mut item = crate::discovery::tests::test_discovery_item("item-1", "movie", None);
        item.matched_subject_count = 3;
        let loose = crate::discovery::tests::test_discovery_item("item-2", "movie", None);
        let mut candidate_item = item.clone();
        candidate_item.matched_subject_count = 0;
        let candidate = DiscoveryHomeCandidate {
            item: candidate_item,
            discovery_title_id: "title-1".to_string(),
            matched_subject_keys: Vec::new(),
            affinity_terms: Vec::new(),
            has_hero_backdrop: false,
            rating_source_count: 0,
            best_external_rating: None,
            best_external_rating_votes: 0,
        };
        let pages = DiscoverySectionPages::from_members(DiscoverySectionMembers {
            section: Some(DiscoverySectionResult {
                section_id: "for_you".to_string(),
                section_type: "FOR_YOU".to_string(),
                title: "Synthetic".to_string(),
                surface: "personalized".to_string(),
                total_count: 9,
                items: vec![item, loose],
                label_key: None,
            }),
            candidates_by_id: HashMap::from([("item-1".to_string(), candidate)]),
            pending_requests: None,
        })
        .expect("a composed section caches");
        assert!(pages.header.items.is_empty());
        assert_eq!(pages.header.total_count, 2);
        match &pages.members[..] {
            [
                DiscoverySectionMember::Candidate(candidate),
                DiscoverySectionMember::Item(loose),
            ] => {
                assert_eq!(candidate.discovery_title_id, "title-1");
                // The section's item, subject resolution included, replaces
                // the pool row's.
                assert_eq!(candidate.item.matched_subject_count, 3);
                assert_eq!(loose.id, "item-2");
            }
            other => panic!("unexpected members: {other:?}"),
        }
    }
}
