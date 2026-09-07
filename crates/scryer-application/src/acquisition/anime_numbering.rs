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
    if input.bridge.is_empty() || !parse_is_translatable(input.parsed) {
        return NumberingResolution::Unchanged;
    }

    let official = official_candidate(input);
    let mut candidates = Vec::new();
    candidates.extend(official.clone());
    candidates.extend(community_candidates(input));
    candidates.extend(absolute_candidate(input));
    candidates.extend(title_anchored_candidates(input));

    select(candidates, official.as_ref(), input)
}

/// Only plain episode releases are translated. Season packs and series packs
/// resolve by collection rather than by episode number, and translating their
/// season token would move a whole pack onto the wrong season on the strength
/// of a single number — a much worse failure than the one this fixes.
fn parse_is_translatable(parsed: &ParsedEpisodeMetadata) -> bool {
    if parsed.is_series_pack
        || parsed.full_season
        || parsed.is_multi_season
        || matches!(parsed.release_type, ParsedEpisodeReleaseType::SeasonPack)
    {
        return false;
    }
    !parsed.episode_numbers.is_empty()
        || parsed.absolute_episode.is_some()
        || !parsed.absolute_episode_numbers.is_empty()
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

    absolute_start_candidates(input, NumberingCandidateKind::Community)
}

/// An absolute-only release on a catalog with no absolute numbers of its own:
/// the community seasons carry `absolute_start`, so the absolute number picks
/// the season and the offset inside it.
///
/// Every season that could hold the number is offered. Overlapping seasons
/// normally agree — `absolute_start` plus an offset is the same TVDB episode
/// whichever season you count from — and collapse into one answer; where they
/// genuinely disagree the caller sees the disagreement.
fn absolute_start_candidates(
    input: &NumberingInput<'_>,
    kind: NumberingCandidateKind,
) -> Vec<NumberingCandidate> {
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
                kind,
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

    if !input.parsed.episode_numbers.is_empty() {
        return map_community_episodes(
            input,
            community_season,
            &input.parsed.episode_numbers,
            NumberingCandidateKind::TitleAnchored,
            &format!(
                "release names community season {} (\"{}\")",
                community_season.index,
                community_season.titles.first().map_or("", String::as_str)
            ),
        )
        .into_iter()
        .collect();
    }

    absolute_start_candidates(input, NumberingCandidateKind::TitleAnchored)
}

/// The single community season whose own title the release names, when the
/// release does *not* equally name the series itself.
///
/// Both halves matter. A release titled with the plain series name carries no
/// season evidence, and a name that matches two community seasons pins nothing.
fn anchored_community_season<'a>(input: &NumberingInput<'a>) -> Option<&'a AnimeCommunitySeason> {
    let parsed_titles = normalized_parsed_titles(input.parsed_title_variants);
    if parsed_titles.is_empty() {
        return None;
    }

    let series_titles = normalized_series_titles(input.title);
    if parsed_titles
        .iter()
        .any(|parsed| series_titles.iter().any(|series| series == parsed))
    {
        return None;
    }

    let mut matched: Option<&AnimeCommunitySeason> = None;
    for season in &input.bridge.seasons {
        let hit = season.titles.iter().any(|season_title| {
            let normalized = crate::app_usecase_rss::normalize_for_matching(season_title);
            !normalized.is_empty() && parsed_titles.iter().any(|parsed| parsed == &normalized)
        });
        if !hit {
            continue;
        }
        if matched.is_some() {
            // Two community seasons answer to the same name; that pins nothing.
            return None;
        }
        matched = Some(season);
    }
    matched
}

fn absolute_candidate(input: &NumberingInput<'_>) -> Option<NumberingCandidate> {
    let absolutes = parsed_absolute_numbers(input.parsed);
    if absolutes.is_empty() {
        return None;
    }
    let mut matches = Vec::new();
    for catalog_episode in input.episodes {
        let absolute = parse_u32(catalog_episode.absolute_number.as_deref());
        if absolute.is_some_and(|number| absolutes.contains(&number)) {
            matches.push(catalog_episode);
        }
    }
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

fn normalized_series_titles(title: &Title) -> Vec<String> {
    let mut titles = Vec::new();
    for name in std::iter::once(title.name.as_str())
        .chain(title.aliases.iter().map(String::as_str))
        .chain(title.tagged_aliases.iter().map(|alias| alias.name.as_str()))
    {
        let normalized = crate::app_usecase_rss::normalize_for_matching(name);
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
    if parsed.episode.is_none() {
        return NumberingResolution::Unchanged;
    }

    let variants = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };
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

    if let NumberingResolution::Resolved(candidate) = &resolution {
        parsed.season = Some(candidate.season);
        parsed.episode_numbers = candidate.episode_numbers.clone();
    }
    resolution
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
