//! Translating anime release numbering into the catalog's TVDB official order.
//!
//! Scryer's catalog follows TVDB's official order. For a large share of anime
//! that order carries one long season where the community (AniDB/AniList/MAL —
//! and therefore the release groups) carries one season per cour. A group that
//! releases `S04E20` means community season 4 episode 20, which TVDB records as
//! `S01E56`. Today that parse positively contradicts the wanted episode, so the
//! release is vetoed and nothing is ever grabbed.
//!
//! SMG hands Scryer an [`AnimeNumberingBridge`] describing the community layout.
//! This module is the pure translator over it: given a bridge, the title's
//! catalog episodes and a parsed release, it produces every numbering
//! interpretation that lands on real catalog episodes and picks between them.
//!
//! Everything here is a pure function of its inputs. The lanes that use it
//! (grab, import, library scan, search-query building) own the gating: this
//! module is only ever entered for an Anime-facet title that has a stored
//! bridge, so every other title keeps exactly today's behaviour.

use chrono::NaiveDate;
use scryer_domain::{AnimeCommunitySeason, AnimeNumberingBridge, Episode, Title};

use crate::ParsedEpisodeMetadata;
use crate::release_parser::ParsedEpisodeReleaseType;

/// How a numbering interpretation was arrived at. The order is the precedence
/// order: a higher-ranked interpretation wins a disagreement outright.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NumberingCandidateKind {
    /// The catalog's own numbering, read literally off the parse.
    Absolute,
    /// Parsed (season, episode) taken literally against the catalog.
    Official,
    /// The parsed season read as a community season index.
    Community,
    /// The release names one community season's own title, which pins the
    /// season regardless of what season token the release carries.
    TitleAnchored,
}

impl NumberingCandidateKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::Official => "official",
            Self::Community => "community",
            Self::TitleAnchored => "title_anchored",
        }
    }
}

/// One numbering interpretation of a release, resolved onto catalog episodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NumberingCandidate {
    pub(crate) kind: NumberingCandidateKind,
    /// The TVDB season the release lands in. Every candidate resolves inside a
    /// single season; an interpretation that would straddle two is discarded
    /// rather than guessed at.
    pub(crate) season: u32,
    /// TVDB episode numbers, ascending.
    pub(crate) episode_numbers: Vec<u32>,
    /// Catalog episode ids, in the same order.
    pub(crate) episode_ids: Vec<String>,
    /// Why this interpretation exists, in words an operator can read off a
    /// decision record.
    pub(crate) explanation: String,
}

impl NumberingCandidate {
    fn key(&self) -> Vec<String> {
        let mut ids = self.episode_ids.clone();
        ids.sort();
        ids
    }

    /// Whether this reading lands on a given catalog episode. Only the tests
    /// ask; the lanes read the rewritten parse instead.
    #[cfg(test)]
    pub(crate) fn covers_episode_id(&self, episode_id: &str) -> bool {
        self.episode_ids.iter().any(|id| id == episode_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NumberingResolution {
    /// No interpretation other than the catalog's own reading applies. Callers
    /// keep today's behaviour untouched.
    Unchanged,
    /// One interpretation survived, and it is not the literal one.
    Resolved(NumberingCandidate),
    /// Several equally-ranked interpretations land on different episodes. A
    /// release nobody can place must not be grabbed or imported silently.
    Ambiguous(Vec<NumberingCandidate>),
    /// A whole cour-shaped pack supplied insufficient or conflicting evidence
    /// for a bounded projection. This is distinct from an episode-numbering
    /// ambiguity: callers must retain the raw evidence but deny broad pack
    /// scope rather than treating it as a normal title-level release.
    UnresolvedPack,
}

impl NumberingResolution {
    pub(crate) fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous(_))
    }

    /// The winning non-literal interpretation. Lanes read the rewritten parse
    /// rather than the candidate, so this is only how the tests inspect it.
    #[cfg(test)]
    pub(crate) fn resolved(&self) -> Option<&NumberingCandidate> {
        match self {
            Self::Resolved(candidate) => Some(candidate),
            _ => None,
        }
    }

    /// A one-line, operator-readable account of an ambiguous result.
    pub(crate) fn ambiguity_summary(&self) -> Option<String> {
        match self {
            Self::Ambiguous(candidates) => Some(
                candidates
                    .iter()
                    .map(|candidate| candidate.explanation.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            _ => None,
        }
    }
}

/// Everything the translator reads. Borrowed rather than owned so a lane can
/// build one per release without cloning the catalog.
pub(crate) struct NumberingInput<'a> {
    pub(crate) bridge: &'a AnimeNumberingBridge,
    pub(crate) title: &'a Title,
    pub(crate) episodes: &'a [Episode],
    pub(crate) parsed: &'a ParsedEpisodeMetadata,
    /// The parsed release's normalized title variants (falling back to its
    /// single normalized title). Used only for the title-anchored rule.
    pub(crate) parsed_title_variants: &'a [String],
    /// Release posted date or file mtime, when the lane has one. Breaks an
    /// otherwise equal-ranked tie toward the interpretation that aired near it.
    pub(crate) reference_date: Option<NaiveDate>,
}

/// How near a reference date an episode has to have aired for that date to
/// settle a tie. Two weeks covers a late posting and a slow index without
/// reaching the neighbouring cour.
const REFERENCE_DATE_WINDOW_DAYS: i64 = 14;

pub(crate) fn resolve_numbering(input: &NumberingInput<'_>) -> NumberingResolution {
    if input.bridge.is_empty() {
        return NumberingResolution::Unchanged;
    }
    if !parse_is_translatable(input.parsed) {
        return if is_whole_pack(input.parsed) {
            resolve_whole_pack_numbering(input)
        } else {
            NumberingResolution::Unchanged
        };
    }

    // A single season-relative bound cannot describe several selected cours.
    // Keep that mixed coverage unresolved instead of silently dropping a list.
    if input.parsed.season_numbers.len() > 1
        || (!input.parsed.season_numbers.is_empty()
            && input
                .parsed
                .season
                .is_some_and(|season| !input.parsed.season_numbers.contains(&season)))
    {
        return NumberingResolution::UnresolvedPack;
    }
    if (is_whole_pack(input.parsed)
        || input.parsed.episode_numbers.len() > 1
        || input.parsed.absolute_episode_numbers.len() > 1)
        && let ExactCourTitleMatch::Unique(cour) =
            exact_cour_title_match(&input.title.name, input.bridge, input.parsed_title_variants)
        && input
            .parsed
            .season
            .into_iter()
            .chain(input.parsed.season_numbers.iter().copied())
            .any(|season| season != 1 && i32::try_from(season).ok() != Some(cour.index))
    {
        return NumberingResolution::UnresolvedPack;
    }

    let official = official_candidate(input);
    let mut candidates = Vec::new();
    candidates.extend(official.clone());
    candidates.extend(community_candidates(input));
    candidates.extend(absolute_candidate(input));
    candidates.extend(title_anchored_candidates(input));

    select(candidates, official.as_ref(), input)
}

fn is_whole_pack(parsed: &ParsedEpisodeMetadata) -> bool {
    parsed.is_series_pack
        || parsed.full_season
        || parsed.is_multi_season
        || matches!(parsed.release_type, ParsedEpisodeReleaseType::SeasonPack)
}

/// Explicit episode coordinates always outrank completion markers. A pack that
/// says `Complete E03-E04` still names only those two episodes.
fn parse_is_translatable(parsed: &ParsedEpisodeMetadata) -> bool {
    !parsed.episode_numbers.is_empty()
        || parsed.absolute_episode.is_some()
        || !parsed.absolute_episode_numbers.is_empty()
}

/// Resolve a whole cour pack only when its community coordinates form a
/// closed, catalog-backed set. A raw `S01 Complete` is deliberately not
/// enough evidence to widen a community cour into an official season.
fn resolve_whole_pack_numbering(input: &NumberingInput<'_>) -> NumberingResolution {
    if input.parsed.is_season_extra
        || input.parsed.special_kind.is_some()
        || !input.parsed.special_absolute_episode_numbers.is_empty()
    {
        return NumberingResolution::UnresolvedPack;
    }
    let anchor =
        exact_cour_title_match(&input.title.name, input.bridge, input.parsed_title_variants);
    if matches!(anchor, ExactCourTitleMatch::Ambiguous) {
        return NumberingResolution::UnresolvedPack;
    }
    let explicit_seasons = sorted(&input.parsed.season_numbers);
    let parsed_season = input.parsed.season;

    if explicit_seasons.len() != input.parsed.season_numbers.len() {
        return NumberingResolution::UnresolvedPack;
    }

    if matches!(anchor, ExactCourTitleMatch::None)
        && explicit_seasons.is_empty()
        && parsed_season.is_none()
    {
        return if input.parsed.is_series_pack {
            NumberingResolution::Unchanged
        } else {
            NumberingResolution::UnresolvedPack
        };
    }

    let selected = if let ExactCourTitleMatch::Unique(anchor) = anchor {
        let Some(anchor_u32) = u32::try_from(anchor.index).ok().filter(|index| *index > 0) else {
            return NumberingResolution::UnresolvedPack;
        };
        let allowed = explicit_seasons.is_empty()
            || explicit_seasons == [anchor_u32]
            || explicit_seasons == [1];
        let season_allowed = parsed_season.is_none()
            || parsed_season == Some(1)
            || parsed_season == Some(anchor_u32);
        if !allowed || !season_allowed {
            return NumberingResolution::UnresolvedPack;
        }
        vec![anchor]
    } else if !explicit_seasons.is_empty() {
        let Some(indices) = explicit_seasons
            .iter()
            .map(|season| i32::try_from(*season).ok())
            .collect::<Option<Vec<_>>>()
        else {
            return NumberingResolution::UnresolvedPack;
        };
        if indices.iter().any(|season| *season <= 0)
            || parsed_season.is_some_and(|season| !explicit_seasons.contains(&season))
        {
            return NumberingResolution::UnresolvedPack;
        }
        match uniquely_indexed_community_seasons(input.bridge, &indices) {
            Ok(Some(selected)) => selected,
            Ok(None) => Vec::new(),
            Err(()) => return NumberingResolution::UnresolvedPack,
        }
    } else if let Some(season) = parsed_season {
        let Ok(season) = i32::try_from(season) else {
            return NumberingResolution::UnresolvedPack;
        };
        match uniquely_indexed_community_seasons(input.bridge, &[season]) {
            Ok(Some(selected)) => selected,
            Ok(None) => Vec::new(),
            Err(()) => return NumberingResolution::UnresolvedPack,
        }
    } else {
        return NumberingResolution::UnresolvedPack;
    };

    if selected.is_empty() {
        return if official_pack_key(input).is_some() {
            NumberingResolution::Unchanged
        } else {
            NumberingResolution::UnresolvedPack
        };
    }
    // Once a release claims a known cour, an incomplete or conflicting bridge
    // must not fall back to broad official collection coverage.
    let Some(projection) = community_pack_projection(input, &selected) else {
        return NumberingResolution::UnresolvedPack;
    };
    let title_anchored = matches!(anchor, ExactCourTitleMatch::Unique(_));
    if !title_anchored && let Some(official) = official_pack_key(input) {
        let mut community_ids = projection
            .iter()
            .map(|(_, _, id)| id.clone())
            .collect::<Vec<_>>();
        community_ids.sort();
        // Identical official coverage needs no translated representation, even
        // when that official pack spans several seasons.
        return if official == community_ids {
            NumberingResolution::Unchanged
        } else {
            NumberingResolution::UnresolvedPack
        };
    }
    community_pack_candidate(projection, title_anchored)
        .map(NumberingResolution::Resolved)
        .unwrap_or(NumberingResolution::UnresolvedPack)
}

fn official_pack_key(input: &NumberingInput<'_>) -> Option<Vec<String>> {
    let seasons = if input.parsed.season_numbers.is_empty() {
        vec![input.parsed.season?]
    } else {
        let seasons = sorted(&input.parsed.season_numbers);
        input
            .parsed
            .season
            .is_none_or(|season| seasons.contains(&season))
            .then_some(seasons)?
    };
    let mut ids = Vec::new();
    let mut seen_coordinates = std::collections::HashSet::new();
    let mut seen_ids = std::collections::HashSet::new();
    for season in seasons {
        let rows = input
            .episodes
            .iter()
            .filter(|episode| episode.episode_type == scryer_domain::EpisodeType::Standard)
            .filter(|episode| parse_u32(episode.season_number.as_deref()) == Some(season))
            .map(|episode| Some((parse_u32(episode.episode_number.as_deref())?, &episode.id)))
            .collect::<Option<Vec<_>>>()?;
        if rows.is_empty() {
            return None;
        }
        for (number, id) in rows {
            if number == 0
                || !seen_coordinates.insert((season, number))
                || !seen_ids.insert(id.clone())
            {
                return None;
            }
            ids.push(id.clone());
        }
    }
    ids.sort();
    Some(ids)
}

fn community_pack_projection(
    input: &NumberingInput<'_>,
    selected: &[&AnimeCommunitySeason],
) -> Option<Vec<(u32, u32, String)>> {
    if selected.is_empty()
        || selected
            .windows(2)
            .any(|pair| std::ptr::eq(pair[0], pair[1]))
    {
        return None;
    }
    if selected.len() > 1 && !input.parsed.episode_numbers.is_empty() {
        // One range cannot safely mean "that range in every listed cour";
        // accept either one bounded cour or a list of complete cours.
        return None;
    }
    let mut destinations = Vec::<(u32, u32, String)>::new();
    let mut seen_destinations = std::collections::HashSet::new();
    let mut seen_episode_ids = std::collections::HashSet::new();
    for community_season in selected {
        let numbers = complete_community_pack_numbers(input, community_season)?;
        let full_projection = complete_community_projection(community_season)?;
        // A supported cour is closed end to end, not merely at the explicit
        // bounds named by this release. Missing catalog row 10 must invalidate
        // an `E01-E02` pack just as it invalidates a complete pack.
        let catalog_ids = exact_standard_catalog_ids(input.episodes, &full_projection)?;
        for community_number in numbers {
            let target_index = usize::try_from(community_number.checked_sub(1)?).ok()?;
            let (season, episode_number) = *full_projection.get(target_index)?;
            if !seen_destinations.insert((season, episode_number)) {
                return None;
            }
            let episode_id = catalog_ids.get(target_index)?.clone();
            if !seen_episode_ids.insert(episode_id.clone()) {
                return None;
            }
            destinations.push((season, episode_number, episode_id));
        }
    }
    Some(destinations)
}

fn community_pack_candidate(
    mut destinations: Vec<(u32, u32, String)>,
    title_anchored: bool,
) -> Option<NumberingCandidate> {
    let season = destinations.first()?.0;
    if destinations
        .iter()
        .any(|(destination_season, _, _)| *destination_season != season)
    {
        return None;
    }
    destinations.sort_by_key(|(_, episode_number, _)| *episode_number);
    Some(NumberingCandidate {
        kind: if title_anchored {
            NumberingCandidateKind::TitleAnchored
        } else {
            NumberingCandidateKind::Community
        },
        season,
        episode_numbers: destinations
            .iter()
            .map(|(_, episode_number, _)| *episode_number)
            .collect(),
        episode_ids: destinations.into_iter().map(|(_, _, id)| id).collect(),
        explanation: "complete community cour pack".to_string(),
    })
}

fn complete_community_pack_numbers(
    input: &NumberingInput<'_>,
    community_season: &AnimeCommunitySeason,
) -> Option<Vec<u32>> {
    let count = community_season.episode_count?;
    if count <= 0 {
        return None;
    }
    let count = u32::try_from(count).ok()?;
    let count_usize = usize::try_from(count).ok()?;
    let available = input
        .episodes
        .iter()
        .filter(|episode| episode.episode_type == scryer_domain::EpisodeType::Standard)
        .count();
    if count_usize > available {
        return None;
    }
    let explicit = sorted(&input.parsed.episode_numbers);
    if !explicit.is_empty() {
        (explicit.len() == input.parsed.episode_numbers.len()
            && explicit.iter().all(|number| *number > 0))
        .then_some(explicit)
    } else {
        Some((1..=count).collect())
    }
}

/// Validate the full closed mapping before returning even a bounded subset.
/// This keeps a range such as `55..56` from silently becoming a one-episode
/// projection when the catalog or bridge omitted its endpoint.
fn complete_community_projection(
    community_season: &AnimeCommunitySeason,
) -> Option<Vec<(u32, u32)>> {
    let count = community_season.episode_count?;
    if count <= 0 {
        return None;
    }
    let count_usize = usize::try_from(count).ok()?;
    let mut result = vec![None; count_usize];
    let mut destinations = std::collections::HashSet::new();

    for range in &community_season.ranges {
        let source_end = range.community_episode_end?;
        let destination_end = range.tvdb_episode_end?;
        if range.community_episode_start <= 0
            || source_end < range.community_episode_start
            || source_end > count
            || range.tvdb_season <= 0
            || range.tvdb_episode_start <= 0
            || destination_end < range.tvdb_episode_start
        {
            return None;
        }
        let source_length = source_end.checked_sub(range.community_episode_start)?;
        let destination_length = destination_end.checked_sub(range.tvdb_episode_start)?;
        if source_length != destination_length {
            return None;
        }
        for offset in 0..=source_length {
            let source = range.community_episode_start.checked_add(offset)?;
            let destination_episode = range.tvdb_episode_start.checked_add(offset)?;
            let slot = usize::try_from(source.checked_sub(1)?).ok()?;
            let destination = (
                u32::try_from(range.tvdb_season).ok()?,
                u32::try_from(destination_episode).ok()?,
            );
            if result.get(slot)?.is_some() || !destinations.insert(destination) {
                return None;
            }
            result[slot] = Some(destination);
        }
    }
    result.into_iter().collect()
}

fn exact_standard_catalog_ids(
    episodes: &[Episode],
    projection: &[(u32, u32)],
) -> Option<Vec<String>> {
    let mut ids = Vec::with_capacity(projection.len());
    let mut seen_coordinates = std::collections::HashSet::new();
    let mut seen_ids = std::collections::HashSet::new();
    for (season, number) in projection {
        if !seen_coordinates.insert((*season, *number)) {
            return None;
        }
        let matches = episodes
            .iter()
            .filter(|episode| episode.episode_type == scryer_domain::EpisodeType::Standard)
            .filter(|episode| parse_u32(episode.season_number.as_deref()) == Some(*season))
            .filter(|episode| parse_u32(episode.episode_number.as_deref()) == Some(*number))
            .collect::<Vec<_>>();
        let [episode] = matches.as_slice() else {
            return None;
        };
        if !seen_ids.insert(episode.id.clone()) {
            return None;
        }
        ids.push(episode.id.clone());
    }
    Some(ids)
}

fn uniquely_indexed_community_seasons<'a>(
    bridge: &'a AnimeNumberingBridge,
    indexes: &[i32],
) -> Result<Option<Vec<&'a AnimeCommunitySeason>>, ()> {
    let mut selected = Vec::with_capacity(indexes.len());
    let mut found_missing = false;
    for index in indexes {
        let mut matched = bridge
            .seasons
            .iter()
            .filter(|season| season.index == *index);
        match (matched.next(), matched.next()) {
            (Some(season), None) => selected.push(season),
            (None, None) => found_missing = true,
            (_, Some(_)) => return Err(()),
        }
    }
    if found_missing {
        return selected.is_empty().then_some(None).ok_or(());
    }
    Ok(Some(selected))
}

fn official_candidate(input: &NumberingInput<'_>) -> Option<NumberingCandidate> {
    let season = input.parsed.season?;
    if input.parsed.episode_numbers.is_empty() {
        return None;
    }
    let episode_ids = catalog_episode_ids(input.episodes, season, &input.parsed.episode_numbers)?;
    Some(NumberingCandidate {
        kind: NumberingCandidateKind::Official,
        season,
        episode_numbers: sorted(&input.parsed.episode_numbers),
        episode_ids,
        explanation: format!(
            "official order: S{season:02}{} as released",
            format_episode_list(&input.parsed.episode_numbers)
        ),
    })
}

/// Every community reading of the parse.
///
/// Normally there is at most one: the parsed season token is a community season
/// index, and the contract makes that index unique. A bridge that repeats an
/// index yields several, and they are all returned rather than silently
/// resolved to whichever came first — an unplaceable release must be reported,
/// not guessed at.
fn community_candidates(input: &NumberingInput<'_>) -> Vec<NumberingCandidate> {
    // A season token is a community season index; without one there is nothing
    // to reinterpret except an absolute number, which the absolute-start arm
    // below handles.
    if let Some(season) = input.parsed.season.filter(|season| *season >= 1)
        && !input.parsed.episode_numbers.is_empty()
        && let Ok(index) = i32::try_from(season)
    {
        return input
            .bridge
            .seasons
            .iter()
            .filter(|community_season| community_season.index == index)
            .filter_map(|community_season| {
                map_community_episodes(
                    input,
                    community_season,
                    &input.parsed.episode_numbers,
                    NumberingCandidateKind::Community,
                    &format!("community season {season}"),
                )
            })
            .collect();
    }

    absolute_start_candidates(input)
}

/// An absolute-only release on a catalog with no absolute numbers of its own:
/// the community seasons carry `absolute_start`, so the absolute number picks
/// the season and the offset inside it.
///
/// Every season that could hold the number is offered. Overlapping seasons
/// normally agree — `absolute_start` plus an offset is the same TVDB episode
/// whichever season you count from — and collapse into one answer; where they
/// genuinely disagree the caller sees the disagreement.
fn absolute_start_candidates(input: &NumberingInput<'_>) -> Vec<NumberingCandidate> {
    if !input.parsed.episode_numbers.is_empty() || catalog_has_absolute_numbers(input.episodes) {
        return Vec::new();
    }
    let absolutes = parsed_absolute_numbers(input.parsed);
    let Some(first) = absolutes
        .first()
        .and_then(|first| i32::try_from(*first).ok())
    else {
        return Vec::new();
    };
    input
        .bridge
        .seasons
        .iter()
        .filter_map(|community_season| {
            let absolute_start = community_season
                .absolute_start
                .filter(|start| *start <= first)?;
            let community_numbers = absolutes
                .iter()
                .map(|absolute| {
                    i32::try_from(*absolute)
                        .ok()
                        .and_then(|absolute| u32::try_from(absolute - absolute_start + 1).ok())
                })
                .collect::<Option<Vec<_>>>()?;
            map_community_episodes(
                input,
                community_season,
                &community_numbers,
                NumberingCandidateKind::Community,
                &format!(
                    "community season {} by absolute start {absolute_start}",
                    community_season.index
                ),
            )
        })
        .collect()
}

fn title_anchored_candidates(input: &NumberingInput<'_>) -> Vec<NumberingCandidate> {
    let Some(community_season) = anchored_community_season(input) else {
        return Vec::new();
    };
    let reason = format!(
        "release names community season {} (\"{}\")",
        community_season.index,
        community_season.titles.first().map_or("", String::as_str)
    );

    if !input.parsed.episode_numbers.is_empty() {
        return map_community_episodes(
            input,
            community_season,
            &input.parsed.episode_numbers,
            NumberingCandidateKind::TitleAnchored,
            &reason,
        )
        .into_iter()
        .collect();
    }

    // A bare number behind a cour's own name counts within that cour. Groups
    // that title their releases per cour number them per cour too, so reading
    // the number as a series-wide absolute would place it by the one piece of
    // evidence the release did not give. Out-of-range numbers map to nothing
    // and fall through to the absolute reading, which is the right answer for
    // a group that names the cour but numbers absolutely.
    map_community_episodes(
        input,
        community_season,
        &parsed_absolute_numbers(input.parsed),
        NumberingCandidateKind::TitleAnchored,
        &reason,
    )
    .into_iter()
    .collect()
}

/// The single community season whose own title the release names, when that
/// name says more than the series' own does.
///
/// The parser projects every release onto the title it matched, so the first
/// variant is always the series' canonical name. Those variants are skipped
/// rather than treated as a veto: vetoing on them would disqualify every
/// release the parser ever produces, which left this whole rule unreachable.
///
/// Aliases are *not* a veto either. A metadata provider's alias list for a
/// long-running anime is the union of every cour's names, so a cour's own
/// title is normally a series alias too — reading that as "this is just the
/// series" would discard exactly the evidence worth having.
///
/// What is left is guarded twice: a name shared by two community seasons pins
/// nothing, and a cour that answers to the series' own canonical name pins
/// nothing either. That second guard is the one that matters in practice —
/// the first cour of a series is usually catalogued under the bare franchise
/// name, so without it any release naming the franchise would anchor there.
fn anchored_community_season<'a>(input: &NumberingInput<'a>) -> Option<&'a AnimeCommunitySeason> {
    match exact_cour_title_match(&input.title.name, input.bridge, input.parsed_title_variants) {
        ExactCourTitleMatch::Unique(season) => return Some(season),
        ExactCourTitleMatch::Ambiguous => return None,
        ExactCourTitleMatch::None => {}
    }

    let parsed_titles = normalized_parsed_titles(input.parsed_title_variants);
    if parsed_titles.is_empty() {
        return None;
    }
    let canonical = crate::app_usecase_rss::normalize_for_matching(&input.title.name);
    let distinguishing = parsed_titles
        .iter()
        .filter(|parsed| **parsed != canonical)
        .collect::<Vec<_>>();
    if distinguishing.is_empty() {
        return None;
    }

    // A cour catalogued under the series' own name is the franchise, not a
    // season within it, and it is dropped before the fuzzy rule sees it.
    let cours = input
        .bridge
        .seasons
        .iter()
        .map(|season| {
            let season_titles = season
                .titles
                .iter()
                .map(|season_title| crate::app_usecase_rss::normalize_for_matching(season_title))
                .filter(|season_title| !season_title.is_empty())
                .collect::<Vec<_>>();
            (season, season_titles)
        })
        .filter(|(_, season_titles)| !season_titles.contains(&canonical))
        .collect::<Vec<_>>();

    fuzzy_anchored_community_season(input, &distinguishing, &canonical, &cours)
}

/// Return the only community cour whose own title the parsed release names
/// exactly. Parser-projected canonical titles are ignored, but a different
/// cour title remains competing evidence: shared aliases never pin a cour.
///
/// This deliberately has no fuzzy fallback. Search admission can only make a
/// preliminary exception for the exact rule; canonical translation continues
/// to own its more conservative fuzzy interpretation.
pub enum ExactCourTitleMatch<'a> {
    None,
    Unique(&'a AnimeCommunitySeason),
    Ambiguous,
}

/// Distinguish an absent exact cour title from one that names multiple bridge
/// entries. The latter is blocking evidence: it must not be weakened into a
/// fuzzy match or a whole-series fallback.
pub fn exact_cour_title_match<'a>(
    canonical_title: &str,
    bridge: &'a AnimeNumberingBridge,
    parsed_title_variants: &[String],
) -> ExactCourTitleMatch<'a> {
    let parsed_titles = normalized_parsed_titles(parsed_title_variants);
    if parsed_titles.is_empty() {
        return ExactCourTitleMatch::None;
    }
    let canonical = crate::app_usecase_rss::normalize_for_matching(canonical_title);
    let distinguishing = parsed_titles
        .iter()
        .filter(|parsed| **parsed != canonical)
        .collect::<Vec<_>>();
    if distinguishing.is_empty() {
        return ExactCourTitleMatch::None;
    }

    let mut matched = None;
    for season in &bridge.seasons {
        let season_titles = season
            .titles
            .iter()
            .map(|title| crate::app_usecase_rss::normalize_for_matching(title))
            .filter(|title| !title.is_empty())
            .collect::<Vec<_>>();
        if season_titles.contains(&canonical)
            || !season_titles
                .iter()
                .any(|season_title| distinguishing.contains(&season_title))
        {
            continue;
        }
        if matched.is_some() {
            return ExactCourTitleMatch::Ambiguous;
        }
        matched = Some(season);
    }
    matched.map_or(ExactCourTitleMatch::None, ExactCourTitleMatch::Unique)
}

/// A cour title has to be long enough that a few edits cannot carry it to a
/// different cour. The romanised long forms this rule exists for run past 80
/// characters; the short `… s4` forms that must never fuzz are 20-33.
const FUZZY_ANCHOR_MIN_LENGTH: usize = 40;
/// Roughly one edit of tolerance per this many characters. A group's
/// romanisation of a long title differs from the metadata provider's by a
/// handful of letters, not by a proportion of the title.
const FUZZY_ANCHOR_LENGTH_PER_EDIT: usize = 20;
/// Hard ceiling on that tolerance, whatever the length.
const FUZZY_ANCHOR_MAX_DISTANCE: usize = 4;
/// Every other cour has to be at least this much further away than the winner.
/// Sibling cours of one series share their whole stem and differ only in the
/// subtitle, so a near-tie means the release named neither of them.
const FUZZY_ANCHOR_RUNNER_UP_MARGIN: usize = 8;

/// The community season whose title the release *nearly* names, when no title
/// matches exactly.
///
/// Groups romanise a long Japanese title their own way, so the name on the
/// release and the name in the bridge can sit a few letters apart while
/// denoting the same cour. `TitleAnchored` is the highest-precedence reading,
/// though, so a wrong guess here overrides every other one — which is why this
/// is fenced in rather than being a plain distance threshold:
///
/// - it runs only when exact anchoring found nothing;
/// - only for a release that carries no season and no episode number of its
///   own, since anything numbered needs no anchoring;
/// - only between titles carrying the same numbers, however written — this is
///   the rule the safety rests on, because sibling cours are routinely one
///   edit apart (`… s3` vs `… s4`) and a bare series alias sits closer to a
///   numbered cour than to the cour that actually aired;
/// - only above a length floor, with a tolerance proportional to the length of
///   the two names being compared;
/// - only when every other cour is a clear margin further away;
/// - and only when the series' own name is not that close as well.
fn fuzzy_anchored_community_season<'a>(
    input: &NumberingInput<'a>,
    distinguishing: &[&String],
    canonical: &str,
    cours: &[(&'a AnimeCommunitySeason, Vec<String>)],
) -> Option<&'a AnimeCommunitySeason> {
    if input.parsed.season.is_some() || !input.parsed.episode_numbers.is_empty() {
        return None;
    }

    // Every name's numbers are read against every cour, twice over. Settle
    // them once instead of re-splitting the same strings inside the sweep.
    let parsed_forms = numbered_forms(distinguishing.iter().map(|parsed| parsed.as_str()));
    let series_forms = numbered_forms(std::iter::once(canonical));
    let cour_forms = cours
        .iter()
        .map(|(season, season_titles)| {
            (
                *season,
                numbered_forms(season_titles.iter().map(String::as_str)),
            )
        })
        .collect::<Vec<_>>();

    // The distance to the closest comparable name, where comparable means the
    // two carry the same numbers and the longer of them clears the floor.
    //
    // `bound` overrides the tolerance a pair earns for its own length: the
    // sweeps for a rival cour and for the series measure against the winner's
    // distance plus the margin, not against a tolerance of their own.
    let nearest = |forms: &[(&str, Vec<u32>)], bound: Option<usize>| -> Option<usize> {
        let mut nearest: Option<usize> = None;
        for (candidate, candidate_numbers) in forms {
            for (parsed, parsed_numbers) in &parsed_forms {
                if candidate_numbers != parsed_numbers {
                    continue;
                }
                let length = parsed.chars().count().max(candidate.chars().count());
                if length < FUZZY_ANCHOR_MIN_LENGTH {
                    continue;
                }
                let tolerance = bound.unwrap_or_else(|| {
                    (length / FUZZY_ANCHOR_LENGTH_PER_EDIT).min(FUZZY_ANCHOR_MAX_DISTANCE)
                });
                if let Some(distance) = crate::library::title_matching::bounded_levenshtein_distance(
                    parsed, candidate, tolerance,
                ) {
                    nearest = Some(nearest.map_or(distance, |best| best.min(distance)));
                }
            }
        }
        nearest
    };

    let mut winner: Option<(&AnimeCommunitySeason, usize)> = None;
    for (season, forms) in &cour_forms {
        let Some(distance) = nearest(forms, None) else {
            continue;
        };
        if winner.is_none_or(|(_, best)| distance < best) {
            winner = Some((*season, distance));
        }
    }

    let (season, distance) = winner?;
    let margin = distance.saturating_add(FUZZY_ANCHOR_RUNNER_UP_MARGIN);
    for (other, forms) in &cour_forms {
        if std::ptr::eq(*other, season) {
            continue;
        }
        if nearest(forms, Some(margin)).is_some() {
            return None;
        }
    }
    // The series' own name competes here too. Dropping a cour catalogued under
    // exactly the series name happens before either rule runs, but that test is
    // exact: one character of romanisation drift — a macron written out as the
    // vowel it stands for — walks a franchise-named cour straight past it, and
    // then every release of the show anchors there. A release whose name is as
    // close to the bare franchise as it is to this cour names neither.
    if nearest(&series_forms, Some(margin)).is_some() {
        return None;
    }
    Some(season)
}

/// Each name paired with the numbers it carries, ready to compare.
fn numbered_forms<'t>(titles: impl Iterator<Item = &'t str>) -> Vec<(&'t str, Vec<u32>)> {
    titles
        .map(|title| (title, season_number_tokens(title)))
        .collect()
}

/// The numbers a title carries, however they are written: digit runs anywhere
/// in a token (`s4`, `4th`, `season 2`), ordinal words, and roman numerals.
///
/// Two cour titles that disagree here name different cours no matter how few
/// characters separate them, which is exactly the case fuzzy matching would
/// otherwise get wrong.
fn season_number_tokens(title: &str) -> Vec<u32> {
    let mut numbers = Vec::new();
    for token in title.split_whitespace() {
        let mut digits = String::new();
        let mut saw_digit = false;
        for character in token.chars() {
            if character.is_ascii_digit() {
                digits.push(character);
                saw_digit = true;
            } else if !digits.is_empty() {
                if let Ok(number) = digits.parse::<u32>() {
                    numbers.push(number);
                }
                digits.clear();
            }
        }
        if !digits.is_empty()
            && let Ok(number) = digits.parse::<u32>()
        {
            numbers.push(number);
        }
        if !saw_digit && let Some(number) = written_number(token) {
            numbers.push(number);
        }
    }
    numbers.sort_unstable();
    numbers
}

fn written_number(token: &str) -> Option<u32> {
    match token {
        "first" | "i" => Some(1),
        "second" | "ii" => Some(2),
        "third" | "iii" => Some(3),
        "fourth" | "iv" => Some(4),
        "fifth" | "v" => Some(5),
        "sixth" | "vi" => Some(6),
        "seventh" | "vii" => Some(7),
        "eighth" | "viii" => Some(8),
        "ninth" | "ix" => Some(9),
        "tenth" | "x" => Some(10),
        _ => None,
    }
}

fn absolute_candidate(input: &NumberingInput<'_>) -> Option<NumberingCandidate> {
    let absolutes = parsed_absolute_numbers(input.parsed);
    if absolutes.is_empty() {
        return None;
    }
    if !input.parsed.absolute_episode_numbers.is_empty()
        && absolutes.len() != input.parsed.absolute_episode_numbers.len()
    {
        return None;
    }
    // A range is an all-or-nothing claim. Retaining only the endpoints found
    // in the catalog used to turn `55..56` into a valid one-episode mapping
    // when 56 was absent, which then made a partial cour look complete.
    let matches = absolutes
        .iter()
        .map(|absolute| {
            let matches = input
                .episodes
                .iter()
                .filter(|episode| parse_u32(episode.absolute_number.as_deref()) == Some(*absolute))
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [episode] => Some(*episode),
                _ => None,
            }
        })
        .collect::<Option<Vec<_>>>()?;
    let (season, episode_numbers, episode_ids) = single_season_projection(&matches)?;
    Some(NumberingCandidate {
        kind: NumberingCandidateKind::Absolute,
        season,
        episode_numbers,
        episode_ids,
        explanation: format!(
            "absolute numbering{}",
            format_episode_list(&absolutes).replace('E', " ")
        ),
    })
}

fn map_community_episodes(
    input: &NumberingInput<'_>,
    community_season: &AnimeCommunitySeason,
    community_numbers: &[u32],
    kind: NumberingCandidateKind,
    reason: &str,
) -> Option<NumberingCandidate> {
    let mut mapped = Vec::new();
    for number in community_numbers {
        let community_episode = i32::try_from(*number).ok()?;
        let (tvdb_season, tvdb_episode) =
            community_season.tvdb_for_community_episode(community_episode)?;
        mapped.push((
            u32::try_from(tvdb_season).ok()?,
            u32::try_from(tvdb_episode).ok()?,
        ));
    }
    let season = mapped.first()?.0;
    if mapped
        .iter()
        .any(|(mapped_season, _)| *mapped_season != season)
    {
        // A release that would straddle two TVDB seasons is not translated.
        return None;
    }
    let episode_numbers = mapped
        .iter()
        .map(|(_, episode)| *episode)
        .collect::<Vec<_>>();
    let episode_ids = catalog_episode_ids(input.episodes, season, &episode_numbers)?;
    Some(NumberingCandidate {
        kind,
        season,
        episode_numbers: sorted(&episode_numbers),
        episode_ids,
        explanation: format!(
            "{reason}{} maps to S{season:02}{}",
            format_episode_list(community_numbers),
            format_episode_list(&episode_numbers)
        ),
    })
}

/// Pick between the interpretations that survived.
///
/// Rank decides first, so a title-anchored reading beats a community one, which
/// beats the literal one, which beats a bare absolute. Interpretations that
/// land on exactly the same catalog episodes are the same answer arrived at
/// twice and collapse into the strongest of them. What is left at the top rank,
/// if it is more than one distinct set of episodes, is a genuine ambiguity —
/// unless a reference date settles it.
fn select(
    mut candidates: Vec<NumberingCandidate>,
    official: Option<&NumberingCandidate>,
    input: &NumberingInput<'_>,
) -> NumberingResolution {
    candidates.retain(|candidate| !candidate.episode_ids.is_empty());
    if candidates.is_empty() {
        return NumberingResolution::Unchanged;
    }

    // Collapse duplicates: keep the strongest reading of each episode set.
    candidates.sort_by(|left, right| {
        right
            .kind
            .cmp(&left.kind)
            .then_with(|| left.key().cmp(&right.key()))
    });
    let mut seen_keys: Vec<Vec<String>> = Vec::new();
    candidates.retain(|candidate| {
        let key = candidate.key();
        if seen_keys.contains(&key) {
            return false;
        }
        seen_keys.push(key);
        true
    });

    let best_kind = candidates
        .iter()
        .map(|candidate| candidate.kind)
        .max()
        .expect("non-empty candidates");
    let mut top = candidates
        .into_iter()
        .filter(|candidate| candidate.kind == best_kind)
        .collect::<Vec<_>>();

    if top.len() > 1
        && let Some(reference_date) = input.reference_date
    {
        let near = top
            .iter()
            .filter(|candidate| candidate_airs_near(candidate, input.episodes, reference_date))
            .cloned()
            .collect::<Vec<_>>();
        if near.len() == 1 {
            top = near;
        }
    }

    match top.len() {
        0 => NumberingResolution::Unchanged,
        1 => {
            let candidate = top.into_iter().next().expect("one candidate");
            // Either the literal reading won outright, or a community reading
            // agrees with it episode for episode. Both mean nothing downstream
            // needs to change, which is what keeps a bridge whose seasons
            // coincide with TVDB's completely inert.
            if candidate.kind == NumberingCandidateKind::Official
                || official.is_some_and(|official| official.key() == candidate.key())
            {
                NumberingResolution::Unchanged
            } else {
                NumberingResolution::Resolved(candidate)
            }
        }
        _ => NumberingResolution::Ambiguous(top),
    }
}

fn candidate_airs_near(
    candidate: &NumberingCandidate,
    episodes: &[Episode],
    reference_date: NaiveDate,
) -> bool {
    candidate.episode_ids.iter().any(|episode_id| {
        episodes
            .iter()
            .find(|episode| &episode.id == episode_id)
            .and_then(|episode| episode.air_date.as_deref())
            .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
            .is_some_and(|aired| {
                (aired - reference_date).num_days().abs() <= REFERENCE_DATE_WINDOW_DAYS
            })
    })
}

// ── catalog helpers ───────────────────────────────────────────────────────

/// Every requested episode number resolved inside one season, or `None` when
/// any of them is missing. A partial hit is not an interpretation: it would
/// silently drop half a multi-episode release.
fn catalog_episode_ids(
    episodes: &[Episode],
    season: u32,
    episode_numbers: &[u32],
) -> Option<Vec<String>> {
    if episode_numbers.is_empty() {
        return None;
    }
    let mut ids = Vec::with_capacity(episode_numbers.len());
    for number in sorted(episode_numbers) {
        let found = episodes.iter().find(|episode| {
            parse_u32(episode.season_number.as_deref()) == Some(season)
                && parse_u32(episode.episode_number.as_deref()) == Some(number)
        })?;
        ids.push(found.id.clone());
    }
    Some(ids)
}

fn single_season_projection(matches: &[&Episode]) -> Option<(u32, Vec<u32>, Vec<String>)> {
    if matches.is_empty() {
        return None;
    }
    let season = parse_u32(matches.first()?.season_number.as_deref())?;
    let mut numbered = Vec::with_capacity(matches.len());
    for episode in matches {
        if parse_u32(episode.season_number.as_deref()) != Some(season) {
            return None;
        }
        numbered.push((
            parse_u32(episode.episode_number.as_deref())?,
            episode.id.clone(),
        ));
    }
    numbered.sort_by_key(|(number, _)| *number);
    Some((
        season,
        numbered.iter().map(|(number, _)| *number).collect(),
        numbered.into_iter().map(|(_, id)| id).collect(),
    ))
}

fn catalog_has_absolute_numbers(episodes: &[Episode]) -> bool {
    episodes
        .iter()
        .any(|episode| parse_u32(episode.absolute_number.as_deref()).is_some_and(|value| value > 0))
}

fn parsed_absolute_numbers(parsed: &ParsedEpisodeMetadata) -> Vec<u32> {
    if !parsed.absolute_episode_numbers.is_empty() {
        return sorted(&parsed.absolute_episode_numbers);
    }
    parsed.absolute_episode.into_iter().collect()
}

fn normalized_parsed_titles(variants: &[String]) -> Vec<String> {
    let mut titles = Vec::new();
    for variant in variants {
        let normalized = crate::app_usecase_rss::normalize_for_matching(variant);
        if !normalized.is_empty() && !titles.contains(&normalized) {
            titles.push(normalized);
        }
    }
    titles
}

fn parse_u32(value: Option<&str>) -> Option<u32> {
    value.and_then(|value| value.trim().parse::<u32>().ok())
}

fn sorted(values: &[u32]) -> Vec<u32> {
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    values
}

fn format_episode_list(numbers: &[u32]) -> String {
    sorted(numbers)
        .iter()
        .map(|number| format!("E{number:02}"))
        .collect::<String>()
}

// ── lane entry point ──────────────────────────────────────────────────────

/// Rewrite a parsed release into the catalog's own numbering, in place.
///
/// This is the single door every lane goes through. It answers `Unchanged` for
/// a non-anime title, a title with no bridge, a release the bridge has nothing
/// to say about, and a release whose literal numbering was right all along — in
/// each of those cases `parsed` comes back untouched and the caller behaves
/// exactly as it does today.
///
/// When a community reading wins, the parse is rewritten to the TVDB season and
/// episode numbers it resolves to, so coverage resolution, the numbering veto
/// and episode routing all read the translated numbering without any of them
/// needing to know the bridge exists.
pub(crate) fn translate_release_numbering(
    bridge: Option<&AnimeNumberingBridge>,
    title: &Title,
    episodes: &[Episode],
    parsed: &mut crate::ParsedReleaseMetadata,
    reference_date: Option<NaiveDate>,
) -> NumberingResolution {
    let Some(bridge) = bridge else {
        return NumberingResolution::Unchanged;
    };
    if title.facet != scryer_domain::MediaFacet::Anime {
        return NumberingResolution::Unchanged;
    }
    if parsed
        .parse_hints
        .iter()
        .any(|hint| hint == "identity:unresolved_pack_scope")
    {
        return NumberingResolution::UnresolvedPack;
    }
    if parsed.episode.is_none() {
        return NumberingResolution::Unchanged;
    }

    let variants = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };
    let resolution = {
        let Some(parsed_episode) = parsed.episode.as_mut() else {
            return NumberingResolution::Unchanged;
        };
        translate_parsed_episode_numbering(
            bridge,
            title,
            episodes,
            parsed_episode,
            &variants,
            reference_date,
        )
    };
    if matches!(&resolution, NumberingResolution::UnresolvedPack)
        && !parsed
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    {
        parsed
            .parse_hints
            .push("identity:unresolved_pack_scope".to_string());
    }
    resolution
}

/// The same translation against a bare episode parse, for the import lane —
/// which resolves identity from an episode metadata block (a file stem's or a
/// release name's) rather than from a whole parsed release.
///
/// The facet gate lives in the caller here: import resolves the bridge from
/// the title first and only reaches this function for an anime title that has
/// one.
pub(crate) fn translate_parsed_episode_numbering(
    bridge: &AnimeNumberingBridge,
    title: &Title,
    episodes: &[Episode],
    parsed: &mut ParsedEpisodeMetadata,
    parsed_title_variants: &[String],
    reference_date: Option<NaiveDate>,
) -> NumberingResolution {
    let resolution = resolve_numbering(&NumberingInput {
        bridge,
        title,
        episodes,
        parsed,
        parsed_title_variants,
        reference_date,
    });

    match &resolution {
        NumberingResolution::Resolved(candidate) => {
            apply_resolved_catalog_coordinates(parsed, candidate, episodes);
        }
        NumberingResolution::Unchanged
        | NumberingResolution::Ambiguous(_)
        | NumberingResolution::UnresolvedPack => {}
    }
    resolution
}

/// Project a resolved catalog interpretation onto the parser fields every
/// downstream admission check reads. Absolute coordinates are trustworthy only
/// when every resolved catalog episode supplies one positive, distinct value;
/// the parser's raw evidence remains untouched.
fn apply_resolved_catalog_coordinates(
    parsed: &mut ParsedEpisodeMetadata,
    candidate: &NumberingCandidate,
    episodes: &[Episode],
) {
    let converted_whole_pack = is_whole_pack(parsed) && !parse_is_translatable(parsed);
    parsed.season = Some(candidate.season);
    parsed.season_numbers = vec![candidate.season];
    parsed.episode_numbers = sorted(&candidate.episode_numbers);
    // Resolved identities vouch only for these exact catalog coordinates.
    parsed.full_season = false;
    parsed.is_partial_season = false;
    parsed.is_multi_season = false;
    parsed.is_series_pack = false;
    parsed.season_part = None;
    if converted_whole_pack {
        // Explicit episode bounds keep their parser release-type conventions.
        parsed.release_type = if parsed.episode_numbers.len() > 1 {
            ParsedEpisodeReleaseType::RangePack
        } else {
            ParsedEpisodeReleaseType::SingleEpisode
        };
    }

    let absolute_numbers = candidate
        .episode_ids
        .iter()
        .zip(&parsed.episode_numbers)
        .map(|(episode_id, expected_number)| {
            let episode = episodes.iter().find(|episode| episode.id == *episode_id)?;
            (parse_u32(episode.season_number.as_deref()) == Some(candidate.season)
                && parse_u32(episode.episode_number.as_deref()) == Some(*expected_number))
            .then(|| parse_u32(episode.absolute_number.as_deref()))?
            .filter(|absolute| *absolute > 0)
        })
        .collect::<Option<Vec<_>>>()
        .filter(|numbers| {
            candidate.episode_ids.len() == parsed.episode_numbers.len()
                && sorted(numbers).len() == numbers.len()
        })
        .map(|numbers| sorted(&numbers));

    match absolute_numbers {
        Some(numbers) => {
            parsed.absolute_episode = numbers.first().copied();
            parsed.absolute_episode_numbers = numbers;
        }
        None => {
            parsed.absolute_episode = None;
            parsed.absolute_episode_numbers.clear();
        }
    }
    parsed.special_absolute_episode_numbers.clear();
}

// ── search-side translation (phase 3) ─────────────────────────────────────

/// The community (season, episode) a wanted TVDB episode is known as, plus the
/// community season's own preferred title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommunityCoordinates {
    pub(crate) season: i32,
    pub(crate) episode: i32,
    pub(crate) season_title: Option<String>,
}

/// Translate a wanted TVDB episode into the community numbering release groups
/// use. `None` when no community season covers it.
pub(crate) fn community_coordinates_for_tvdb_episode(
    bridge: &AnimeNumberingBridge,
    tvdb_season: i32,
    tvdb_episode: i32,
) -> Option<CommunityCoordinates> {
    for season in &bridge.seasons {
        if let Some(community_episode) =
            season.community_for_tvdb_episode(tvdb_season, tvdb_episode)
        {
            return Some(CommunityCoordinates {
                season: season.index,
                episode: community_episode,
                season_title: season
                    .titles
                    .first()
                    .map(|title| title.trim().to_string())
                    .filter(|title| !title.is_empty()),
            });
        }
    }
    None
}

#[cfg(test)]
#[path = "anime_numbering_tests.rs"]
mod anime_numbering_tests;
