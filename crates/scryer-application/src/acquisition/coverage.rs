use super::*;
use crate::quality_profile::CoverageSizeBasis;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseCoverage {
    SingleEpisode(String),
    EpisodeSet(Vec<String>),
    Collection(String),
    Title,
    Unknown,
}

impl crate::AcquisitionScopeState {
    /// The scope this ledger row stands for.
    ///
    /// Gates resolve incumbents by scope, so a row has to be able to say what it
    /// covers without the caller re-deriving it from three optional columns.
    pub(crate) fn submission_scope(&self) -> SubmissionScope {
        if let Some(episode_id) = self
            .episode_id
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            return SubmissionScope::Episode {
                episode_id: episode_id.clone(),
            };
        }
        if let Some(series_movie_link_id) = self
            .series_movie_link_id
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            return SubmissionScope::SeriesMovie {
                series_movie_link_id: series_movie_link_id.clone(),
            };
        }
        if let Some(collection_id) = self
            .collection_id
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            return SubmissionScope::Collection {
                collection_id: collection_id.clone(),
            };
        }
        SubmissionScope::Title
    }
}

impl ReleaseCoverage {
    pub(crate) fn submission_scope(&self) -> SubmissionScope {
        match self {
            Self::SingleEpisode(episode_id) => SubmissionScope::Episode {
                episode_id: episode_id.clone(),
            },
            Self::EpisodeSet(episode_ids) => SubmissionScope::EpisodeSet {
                episode_ids: episode_ids.clone(),
            },
            Self::Collection(collection_id) => SubmissionScope::Collection {
                collection_id: collection_id.clone(),
            },
            Self::Title => SubmissionScope::Title,
            Self::Unknown => SubmissionScope::Title,
        }
    }

    pub(crate) fn submission_scope_or(&self, fallback: &SubmissionScope) -> SubmissionScope {
        match self {
            Self::Title | Self::Unknown => fallback.clone(),
            _ => self.submission_scope(),
        }
    }

    pub(crate) fn covers_episode(&self, episode: &Episode) -> bool {
        match self {
            Self::SingleEpisode(episode_id) => episode_id == &episode.id,
            Self::EpisodeSet(episode_ids) => episode_ids.iter().any(|id| id == &episode.id),
            Self::Collection(collection_id) => {
                episode.collection_id.as_deref() == Some(collection_id)
            }
            Self::Title => false,
            Self::Unknown => false,
        }
    }

    /// How far this release's span is from what the search asked for. Lower is
    /// a closer match, and it is a *sort* key only.
    ///
    /// This used to be `single_episode_preference_penalty`, subtracted from the
    /// score. As a score term it made the same release worth different amounts
    /// depending on the search that found it, and none of it survived to import
    /// — so grab and import valued a season pack differently. Pack preference is
    /// a property of the search, so it belongs in the ordering.
    pub(crate) fn coverage_distance(&self, requested_episode: Option<&Episode>) -> usize {
        let Some(episode) = requested_episode else {
            return 0;
        };
        match self {
            Self::SingleEpisode(episode_id) if episode_id == &episode.id => 0,
            Self::EpisodeSet(episode_ids) if episode_ids.iter().any(|id| id == &episode.id) => 1,
            Self::Collection(collection_id)
                if episode.collection_id.as_deref() == Some(collection_id.as_str()) =>
            {
                2
            }
            _ => 0,
        }
    }
}

pub(crate) fn resolve_release_coverage(
    parsed: &ParsedReleaseMetadata,
    episodes: &[Episode],
    collections: &[Collection],
    requested_episode: Option<&Episode>,
) -> ReleaseCoverage {
    let Some(episode) = parsed.episode.as_ref() else {
        return ReleaseCoverage::Title;
    };

    if episode.is_series_pack {
        let covered = eligible_series_pack_episode_ids(episodes, &episode.season_numbers);
        return if covered.is_empty() {
            ReleaseCoverage::Unknown
        } else {
            ReleaseCoverage::EpisodeSet(covered)
        };
    }

    if episode.release_type == ParsedEpisodeReleaseType::SeasonPack {
        if let Some(season) = episode.season {
            if let Some(collection_id) = collection_id_for_season(collections, season) {
                return ReleaseCoverage::Collection(collection_id);
            }
            if let Some(requested) = requested_episode
                && requested
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    == Some(season)
                && let Some(collection_id) = requested.collection_id.clone()
            {
                return ReleaseCoverage::Collection(collection_id);
            }
        }
        return requested_episode
            .and_then(|episode| episode.collection_id.clone())
            .map(ReleaseCoverage::Collection)
            .unwrap_or(ReleaseCoverage::Unknown);
    }

    let mut covered = Vec::new();
    if let Some(season) = episode.season
        && !episode.episode_numbers.is_empty()
    {
        let wanted = episode
            .episode_numbers
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for catalog_episode in episodes {
            let catalog_season = catalog_episode
                .season_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            let catalog_number = catalog_episode
                .episode_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            if catalog_season == Some(season)
                && catalog_number.is_some_and(|number| wanted.contains(&number))
            {
                covered.push(catalog_episode.id.clone());
            }
        }
    }

    if covered.is_empty() && !episode.absolute_episode_numbers.is_empty() {
        let wanted = episode
            .absolute_episode_numbers
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for catalog_episode in episodes {
            let absolute = catalog_episode
                .absolute_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            if absolute.is_some_and(|number| wanted.contains(&number)) {
                covered.push(catalog_episode.id.clone());
            }
        }
    }

    if covered.is_empty()
        && let Some(absolute_episode) = episode.absolute_episode
    {
        for catalog_episode in episodes {
            let absolute = catalog_episode
                .absolute_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok());
            if absolute == Some(absolute_episode) {
                covered.push(catalog_episode.id.clone());
            }
        }
    }

    coverage_from_episode_ids(covered).unwrap_or(ReleaseCoverage::Unknown)
}

/// Whether a release's parsed numbering actively contradicts a wanted
/// episode's numbering, absolute numbering included.
///
/// A veto fires only on a positive contradiction: the release asserts a
/// numbering scheme the subject also carries, and the assertion cannot cover
/// the subject. A release that asserts nothing comparable is left alone — the
/// coverage resolver and the admission ladder own the benefit-of-the-doubt
/// cases. An explicit episode-number agreement wins outright, so a stray
/// absolute parse can never veto a release that already names the wanted
/// episode.
///
/// The absolute arm is what keeps an absolute-numbered release for a *different*
/// episode from being adopted by an episode-scoped search: without it, an anime
/// release named only by absolute number sails past the season/episode checks
/// and the Unknown-coverage fallback stamps it as covering the wanted episode.
pub(crate) fn parsed_numbering_contradicts_episode(
    expected_season: Option<u32>,
    expected_episode: Option<u32>,
    expected_absolute: Option<u32>,
    episode: &ParsedEpisodeMetadata,
) -> bool {
    if expected_season.is_none() && expected_episode.is_none() && expected_absolute.is_none() {
        return false;
    }
    if let (Some(expected), Some(found)) = (expected_season, episode.season)
        && expected != found
    {
        return true;
    }
    if let Some(expected) = expected_episode
        && !episode.episode_numbers.is_empty()
        && !episode.episode_numbers.contains(&expected)
    {
        return true;
    }
    if expected_episode.is_some_and(|expected| episode.episode_numbers.contains(&expected)) {
        return false;
    }
    if let Some(expected) = expected_absolute
        && (episode.absolute_episode.is_some() || !episode.absolute_episode_numbers.is_empty())
        && episode.absolute_episode != Some(expected)
        && !episode.absolute_episode_numbers.contains(&expected)
    {
        return true;
    }
    false
}

/// [`parsed_numbering_contradicts_episode`] against a catalog episode record.
pub(crate) fn parsed_release_contradicts_requested_episode(
    parsed: &ParsedReleaseMetadata,
    requested: &Episode,
) -> bool {
    let Some(episode) = parsed.episode.as_ref() else {
        return false;
    };
    parsed_numbering_contradicts_episode(
        requested
            .season_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        requested
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        requested
            .absolute_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok()),
        episode,
    )
}

/// A series pack is worth considering only when it can fill more than 30% of
/// the regular, already-aired episodes the operator monitors. This deliberately
/// counts episodes not already owned by a primary file or an in-flight
/// submission, not quality upgrades: the guard is about avoiding a huge
/// download for a small hole in an otherwise complete series.
pub(crate) fn series_pack_missing_ratio_qualifies(
    parsed: &ParsedReleaseMetadata,
    episodes: &[Episode],
    owned_episode_ids: &std::collections::HashSet<String>,
) -> bool {
    let Some(series_pack) = parsed
        .episode
        .as_ref()
        .filter(|episode| episode.is_series_pack)
    else {
        return true;
    };

    series_pack_missing_ratio_qualifies_for_seasons(
        episodes,
        owned_episode_ids,
        &series_pack.season_numbers,
    )
}

/// Whether a title should issue its one series-pack lookup. A later
/// result-specific check applies the threshold to the exact seasons the pack
/// covers. Searching when any season clears the threshold cannot miss a
/// qualifying multi-season pack: a weighted average above 30% has at least
/// one component season above 30%.
pub(crate) fn title_series_pack_missing_ratio_qualifies(
    episodes: &[Episode],
    owned_episode_ids: &std::collections::HashSet<String>,
) -> bool {
    series_pack_missing_ratio_qualifies_for_seasons(episodes, owned_episode_ids, &[])
        || eligible_series_pack_season_numbers(episodes)
            .into_iter()
            .any(|season| {
                series_pack_missing_ratio_qualifies_for_seasons(
                    episodes,
                    owned_episode_ids,
                    &[season],
                )
            })
}

/// Collection ids represented by the title's monitored, aired, standard
/// episodes. These are the members of the title-level series-pack search set.
pub(crate) fn eligible_series_pack_collection_ids(episodes: &[Episode]) -> Vec<String> {
    eligible_series_pack_collection_ids_for_seasons(episodes, &[])
}

/// Collection ids exactly covered by a parsed series pack.
pub(crate) fn series_pack_collection_ids(
    parsed: &ParsedReleaseMetadata,
    episodes: &[Episode],
) -> Vec<String> {
    let Some(series_pack) = parsed
        .episode
        .as_ref()
        .filter(|episode| episode.is_series_pack)
    else {
        return Vec::new();
    };
    eligible_series_pack_collection_ids_for_seasons(episodes, &series_pack.season_numbers)
}

/// Eligible missing episodes across a title. This is intentionally distinct
/// from the ratio gate: a single small hole should not initiate a pack lookup.
pub(crate) fn eligible_missing_series_pack_episode_count(
    episodes: &[Episode],
    owned_episode_ids: &std::collections::HashSet<String>,
) -> usize {
    eligible_series_pack_episode_ids(episodes, &[])
        .into_iter()
        .filter(|episode_id| !owned_episode_ids.contains(episode_id))
        .count()
}

/// Eligible episodes occupied by a live submission. The queued predicate is
/// the same one admission uses, so failed submissions immediately become
/// missing again while downloads that have completed into import work remain
/// owned until their media files land.
pub(crate) fn in_flight_series_pack_episode_ids(
    episodes: &[Episode],
    submissions: &[crate::DownloadSubmission],
    tracked_states: &std::collections::HashMap<
        crate::contracts::ClientJobLocator,
        scryer_domain::TrackedDownloadState,
    >,
    dl_snapshot: &crate::acquisition_workflow::DownloadClientSnapshot,
) -> std::collections::HashSet<String> {
    let now = chrono::Utc::now();
    episodes
        .iter()
        .filter(|episode| is_eligible_series_pack_episode(episode, &now))
        .filter(|episode| {
            let episode_ids = std::slice::from_ref(&episode.id);
            let collection_ids = episode.collection_id.as_slice();
            let membership = crate::acquisition_workflow::ScopeMembership {
                episode_ids,
                collection_ids,
                series_movie_link_id: None,
            };
            submissions.iter().any(|submission| {
                let identity = crate::contracts::ClientJobLocator::from_submission(submission);
                crate::acquisition_workflow::submission_is_live_claim(
                    submission,
                    tracked_states.get(&identity).copied(),
                    dl_snapshot,
                ) && crate::acquisition_workflow::submission_scope_intersects(
                    &submission.scope,
                    &membership,
                )
            })
        })
        .map(|episode| episode.id.clone())
        .collect()
}

fn series_pack_missing_ratio_qualifies_for_seasons(
    episodes: &[Episode],
    owned_episode_ids: &std::collections::HashSet<String>,
    season_numbers: &[u32],
) -> bool {
    let episode_ids = eligible_series_pack_episode_ids(episodes, season_numbers);
    if episode_ids.is_empty() {
        return false;
    }
    let missing = episode_ids
        .iter()
        .filter(|episode_id| !owned_episode_ids.contains(*episode_id))
        .count();

    missing.saturating_mul(100) > episode_ids.len().saturating_mul(30)
}

fn eligible_series_pack_season_numbers(episodes: &[Episode]) -> std::collections::HashSet<u32> {
    let now = chrono::Utc::now();
    episodes
        .iter()
        .filter(|episode| is_eligible_series_pack_episode(episode, &now))
        .filter_map(|episode| episode.season_number.as_deref())
        .filter_map(|season| season.parse::<u32>().ok())
        .collect()
}

fn eligible_series_pack_episode_ids(episodes: &[Episode], season_numbers: &[u32]) -> Vec<String> {
    let now = chrono::Utc::now();
    episodes
        .iter()
        .filter(|episode| is_eligible_series_pack_episode(episode, &now))
        .filter(|episode| {
            season_numbers.is_empty()
                || episode
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|season| season_numbers.contains(&season))
        })
        .map(|episode| episode.id.clone())
        .collect()
}

fn eligible_series_pack_collection_ids_for_seasons(
    episodes: &[Episode],
    season_numbers: &[u32],
) -> Vec<String> {
    let now = chrono::Utc::now();
    let mut collection_ids = episodes
        .iter()
        .filter(|episode| is_eligible_series_pack_episode(episode, &now))
        .filter(|episode| {
            season_numbers.is_empty()
                || episode
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|season| season_numbers.contains(&season))
        })
        .filter_map(|episode| episode.collection_id.clone())
        .collect::<Vec<_>>();
    collection_ids.sort_unstable();
    collection_ids.dedup();
    collection_ids
}

fn is_eligible_series_pack_episode(episode: &Episode, now: &chrono::DateTime<chrono::Utc>) -> bool {
    episode.episode_type == scryer_domain::EpisodeType::Standard
        && episode.monitored
        && !crate::acquisition_policy::episode_is_unaired(episode.air_date.as_deref(), now)
}

pub(crate) fn coverage_runtime_minutes(
    coverage: &ReleaseCoverage,
    parsed: &ParsedReleaseMetadata,
    episodes: &[Episode],
    default_runtime_minutes: Option<i32>,
) -> Option<i32> {
    coverage_size_basis(coverage, parsed, episodes, default_runtime_minutes).total_runtime_minutes
}

/// The runtime basis size scoring reads a release against: the whole coverage's
/// runtime, one member's, and how many members there are.
///
/// The total is exactly what [`coverage_runtime_minutes`] has always returned —
/// that function now reads it off this one, so the two cannot drift. What is new
/// is the other two fields, which let the scorer notice that an aggregate
/// release's reported bytes describe one member instead of the payload
/// (`quality_profile::CoverageSizeBasis`).
///
/// A member runtime is the coverage's own average rather than any single
/// episode's: uneven durations inside a pack are exactly the case where picking
/// one member would be arbitrary.
pub(crate) fn coverage_size_basis(
    coverage: &ReleaseCoverage,
    parsed: &ParsedReleaseMetadata,
    episodes: &[Episode],
    default_runtime_minutes: Option<i32>,
) -> CoverageSizeBasis {
    match coverage {
        ReleaseCoverage::SingleEpisode(episode_id) => {
            CoverageSizeBasis::single(episode_span_runtime_minutes(
                episodes,
                std::slice::from_ref(episode_id),
                default_runtime_minutes,
            ))
        }
        ReleaseCoverage::EpisodeSet(episode_ids) => {
            episode_span_size_basis(episodes, episode_ids, default_runtime_minutes)
        }
        ReleaseCoverage::Collection(collection_id) => {
            let season_episodes = episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
                .collect::<Vec<_>>();
            if season_episodes.is_empty() {
                return CoverageSizeBasis::single(default_runtime_minutes);
            }
            let count = i32::try_from(season_episodes.len()).unwrap_or(0);
            if parsed
                .episode
                .as_ref()
                .is_some_and(|episode| episode.is_partial_season)
            {
                // Half a season, as the estimate has always read it. The member
                // runtime is the title default because a partial season names no
                // episodes to average over.
                let member_runtime =
                    default_runtime_minutes.unwrap_or(UNKNOWN_EPISODE_RUNTIME_MINUTES);
                let effective_count = (count.max(2) / 2).max(1);
                return CoverageSizeBasis::aggregate(
                    Some(member_runtime * effective_count),
                    Some(member_runtime),
                    effective_count,
                );
            }
            let total = season_episodes
                .iter()
                .map(|episode| {
                    episode
                        .duration_seconds
                        .map(|seconds| (seconds / 60) as i32)
                })
                .map(|runtime| {
                    runtime.unwrap_or(
                        default_runtime_minutes.unwrap_or(UNKNOWN_EPISODE_RUNTIME_MINUTES),
                    )
                })
                .sum::<i32>();
            CoverageSizeBasis::aggregate(
                (total > 0).then_some(total),
                mean_member_runtime_minutes(total, count, default_runtime_minutes),
                count,
            )
        }
        ReleaseCoverage::Title | ReleaseCoverage::Unknown => {
            CoverageSizeBasis::single(default_runtime_minutes)
        }
    }
}

/// [`episode_span_runtime_minutes`] as a size basis: the same total, plus the
/// member count and average the scorer needs to recognise a pack-shaped size.
///
/// The import lane and a re-derived incumbent bar go through this, and the grab
/// lane through [`coverage_size_basis`], so the same file gets the same basis
/// wherever it is judged.
pub(crate) fn episode_span_size_basis(
    episodes: &[Episode],
    episode_ids: &[String],
    default_runtime_minutes: Option<i32>,
) -> CoverageSizeBasis {
    let count = i32::try_from(episode_ids.len()).unwrap_or(i32::MAX);
    if count == 0 {
        // No span to speak of, exactly as `episode_span_runtime_minutes` says.
        // The caller decides what to fall back to.
        return CoverageSizeBasis::single(None);
    }
    let total = episode_span_runtime_minutes(episodes, episode_ids, default_runtime_minutes);
    CoverageSizeBasis::aggregate(
        total,
        mean_member_runtime_minutes(total.unwrap_or(0), count, default_runtime_minutes),
        count,
    )
}

/// The average member runtime of a span, falling back to the caller's default
/// when the span has no runtime to divide.
fn mean_member_runtime_minutes(
    total_runtime_minutes: i32,
    member_count: i32,
    default_runtime_minutes: Option<i32>,
) -> Option<i32> {
    if member_count > 1 && total_runtime_minutes > 0 {
        return Some((total_runtime_minutes / member_count).max(1));
    }
    (total_runtime_minutes > 0)
        .then_some(total_runtime_minutes)
        .or(default_runtime_minutes)
}

fn coverage_from_episode_ids(mut episode_ids: Vec<String>) -> Option<ReleaseCoverage> {
    episode_ids.retain(|episode_id| !episode_id.trim().is_empty());
    episode_ids.sort();
    episode_ids.dedup();
    match episode_ids.len() {
        0 => None,
        1 => episode_ids
            .into_iter()
            .next()
            .map(ReleaseCoverage::SingleEpisode),
        _ => Some(ReleaseCoverage::EpisodeSet(episode_ids)),
    }
}

fn collection_id_for_season(collections: &[Collection], season: u32) -> Option<String> {
    collections
        .iter()
        .find(|collection| collection.collection_index.trim().parse::<u32>().ok() == Some(season))
        .map(|collection| collection.id.clone())
}

/// Assumed length of an episode whose runtime nobody recorded, when at least
/// one sibling in the same span *does* have one. Sonarr's own fallback.
const UNKNOWN_EPISODE_RUNTIME_MINUTES: i32 = 45;

/// Minutes of content a set of episodes represents — **the one runtime basis**
/// (D4).
///
/// Size scoring is runtime-derived, so the expected size of a release depends
/// entirely on this number, and the three places that need it must agree or the
/// same file scores differently at grab, at import, and when its bar is
/// re-derived. A double-length premiere, a 7-minute special and an anime OVA all
/// move the size bucket several steps away from the series average.
///
/// A single episode with no recorded runtime falls back to the caller's default
/// (the title's average, or nothing); in a multi-episode span the missing ones
/// are assumed to be [`UNKNOWN_EPISODE_RUNTIME_MINUTES`] rather than dropped,
/// because dropping them would make a twelve-episode pack look twelve times too
/// large.
pub(crate) fn episode_span_runtime_minutes(
    episodes: &[Episode],
    episode_ids: &[String],
    default_runtime_minutes: Option<i32>,
) -> Option<i32> {
    match episode_ids {
        [] => None,
        [only] => episode_runtime_minutes(episodes, only).or(default_runtime_minutes),
        many => {
            let mut total = 0i32;
            let mut missing = 0i32;
            for episode_id in many {
                match episode_runtime_minutes(episodes, episode_id) {
                    Some(runtime) => total = total.saturating_add(runtime),
                    None => missing += 1,
                }
            }
            if missing > 0 {
                total = total.saturating_add(
                    default_runtime_minutes
                        .unwrap_or(UNKNOWN_EPISODE_RUNTIME_MINUTES)
                        .saturating_mul(missing),
                );
            }
            (total > 0).then_some(total)
        }
    }
}

fn episode_runtime_minutes(episodes: &[Episode], episode_id: &str) -> Option<i32> {
    episodes
        .iter()
        .find(|episode| episode.id == episode_id)
        .and_then(|episode| episode.duration_seconds)
        .map(|seconds| (seconds / 60) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{CollectionType, EpisodeType};

    fn episode(id: &str, season: &str, number: &str, absolute: Option<&str>) -> Episode {
        Episode {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_id: Some(format!("season-{season}")),
            episode_type: EpisodeType::Standard,
            episode_number: Some(number.to_string()),
            season_number: Some(season.to_string()),
            episode_label: None,
            title: None,
            air_date: None,
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: absolute.map(str::to_string),
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn collection(id: &str, index: &str) -> Collection {
        Collection {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_type: CollectionType::Season,
            collection_index: index.to_string(),
            label: None,
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn parsed_with_episode(episode: ParsedEpisodeMetadata) -> ParsedReleaseMetadata {
        let mut parsed = ParsedReleaseMetadata::empty("release", "test");
        parsed.episode = Some(episode);
        parsed
    }

    #[test]
    fn absolute_only_release_for_a_different_episode_contradicts_the_wanted_episode() {
        let mismatched = ParsedEpisodeMetadata {
            absolute_episode: Some(18),
            absolute_episode_numbers: vec![18],
            ..Default::default()
        };
        assert!(parsed_numbering_contradicts_episode(
            Some(20),
            Some(122),
            Some(1344),
            &mismatched
        ));

        let matching = ParsedEpisodeMetadata {
            absolute_episode: Some(1344),
            absolute_episode_numbers: vec![1344],
            ..Default::default()
        };
        assert!(!parsed_numbering_contradicts_episode(
            Some(20),
            Some(122),
            Some(1344),
            &matching
        ));
    }

    #[test]
    fn explicit_episode_agreement_overrides_a_stray_absolute_parse() {
        let episode = ParsedEpisodeMetadata {
            season: Some(20),
            episode_numbers: vec![122],
            absolute_episode: Some(3),
            ..Default::default()
        };
        assert!(!parsed_numbering_contradicts_episode(
            Some(20),
            Some(122),
            Some(1344),
            &episode
        ));
    }

    #[test]
    fn release_without_numbering_assertions_is_not_a_contradiction() {
        let episode = ParsedEpisodeMetadata::default();
        assert!(!parsed_numbering_contradicts_episode(
            Some(20),
            Some(122),
            Some(1344),
            &episode
        ));
        assert!(!parsed_numbering_contradicts_episode(
            None, None, None, &episode
        ));
    }

    #[test]
    fn season_and_episode_number_contradictions_still_veto() {
        let wrong_season = ParsedEpisodeMetadata {
            season: Some(1),
            episode_numbers: vec![122],
            ..Default::default()
        };
        assert!(parsed_numbering_contradicts_episode(
            Some(20),
            Some(122),
            None,
            &wrong_season
        ));

        let wrong_episode = ParsedEpisodeMetadata {
            season: Some(20),
            episode_numbers: vec![121],
            ..Default::default()
        };
        assert!(parsed_numbering_contradicts_episode(
            Some(20),
            Some(122),
            Some(1344),
            &wrong_episode
        ));
    }

    #[test]
    fn contradiction_against_catalog_episode_reads_absolute_number() {
        let requested = episode("ep-122", "20", "122", Some("1344"));
        let mismatched = parsed_with_episode(ParsedEpisodeMetadata {
            absolute_episode: Some(18),
            absolute_episode_numbers: vec![18],
            ..Default::default()
        });
        assert!(parsed_release_contradicts_requested_episode(
            &mismatched,
            &requested
        ));

        let unnumbered = ParsedReleaseMetadata::empty("release", "test");
        assert!(!parsed_release_contradicts_requested_episode(
            &unnumbered,
            &requested
        ));
    }

    #[test]
    fn absolute_range_resolves_to_episode_set_scope() {
        let episodes = vec![
            episode("ep-14", "1", "14", Some("14")),
            episode("ep-15", "1", "15", Some("15")),
            episode("ep-16", "1", "16", Some("16")),
        ];
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            absolute_episode: Some(14),
            absolute_episode_numbers: vec![14, 15, 16],
            release_type: ParsedEpisodeReleaseType::RangePack,
            ..Default::default()
        });

        let coverage = resolve_release_coverage(&parsed, &episodes, &[], None);

        assert_eq!(
            coverage,
            ReleaseCoverage::EpisodeSet(vec![
                "ep-14".to_string(),
                "ep-15".to_string(),
                "ep-16".to_string()
            ])
        );
        assert_eq!(
            coverage.submission_scope(),
            SubmissionScope::EpisodeSet {
                episode_ids: vec![
                    "ep-14".to_string(),
                    "ep-15".to_string(),
                    "ep-16".to_string()
                ]
            }
        );
    }

    #[test]
    fn season_pack_resolves_to_collection_scope() {
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            season: Some(1),
            full_season: true,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });

        let coverage = resolve_release_coverage(&parsed, &[], &[collection("season-1", "1")], None);

        assert_eq!(
            coverage,
            ReleaseCoverage::Collection("season-1".to_string())
        );
        assert_eq!(
            coverage.submission_scope(),
            SubmissionScope::Collection {
                collection_id: "season-1".to_string()
            }
        );
    }

    #[test]
    fn explicit_range_runtime_uses_covered_episode_total() {
        let episodes = vec![
            episode("ep-14", "1", "14", Some("14")),
            episode("ep-15", "1", "15", Some("15")),
        ];
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            absolute_episode: Some(14),
            absolute_episode_numbers: vec![14, 15],
            release_type: ParsedEpisodeReleaseType::RangePack,
            ..Default::default()
        });
        let coverage = resolve_release_coverage(&parsed, &episodes, &[], None);

        assert_eq!(
            coverage_runtime_minutes(&coverage, &parsed, &episodes, Some(45)),
            Some(50)
        );
    }

    #[test]
    fn season_seventeen_part_two_size_runtime_uses_partial_season_span() {
        let episodes = (1..=26)
            .map(|number| {
                let number = number.to_string();
                episode(&format!("ep-{number}"), "17", &number, Some(&number))
            })
            .collect::<Vec<_>>();
        let mut parsed = parsed_with_episode(ParsedEpisodeMetadata {
            season: Some(17),
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            full_season: true,
            is_partial_season: true,
            season_part: Some(2),
            ..Default::default()
        });
        parsed.raw_title = "Fixture.Anime.Continuation.S17.Part.2.1080p.WEB-DL-GRP".to_string();
        let episode = parsed.episode.as_ref().expect("season-pack metadata");
        assert_eq!(episode.season, Some(17));
        assert_eq!(episode.season_part, Some(2));
        assert!(episode.is_partial_season);

        let coverage = resolve_release_coverage(&parsed, &episodes, &[], episodes.first());
        assert_eq!(
            coverage,
            ReleaseCoverage::Collection("season-17".to_string())
        );
        assert_eq!(
            coverage_runtime_minutes(&coverage, &parsed, &episodes, Some(24)),
            Some(13 * 24)
        );
    }

    // ── size bases ────────────────────────────────────────────────────────

    fn episode_of(id: &str, season: &str, number: &str, minutes: i64) -> Episode {
        let mut episode = episode(id, season, number, Some(number));
        episode.duration_seconds = Some(minutes * 60);
        episode
    }

    /// A range pack's basis carries what a member is, not only what the span
    /// sums to: the member runtime is what a size reported per episode is read
    /// against.
    #[test]
    fn an_episode_range_basis_carries_the_member_runtime() {
        let episodes = vec![
            episode("ep-14", "1", "14", Some("14")),
            episode("ep-15", "1", "15", Some("15")),
        ];
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            absolute_episode: Some(14),
            absolute_episode_numbers: vec![14, 15],
            release_type: ParsedEpisodeReleaseType::RangePack,
            ..Default::default()
        });
        let coverage = resolve_release_coverage(&parsed, &episodes, &[], None);

        let basis = coverage_size_basis(&coverage, &parsed, &episodes, Some(45));
        assert_eq!(basis.total_runtime_minutes, Some(50));
        assert_eq!(basis.member_runtime_minutes, Some(25));
        assert_eq!(basis.member_count, 2);
        assert!(basis.covers_multiple_members());
    }

    /// Uneven durations average. Picking any one episode of a pack would be
    /// arbitrary, and a season with a double-length finale is the ordinary case,
    /// not the exception.
    #[test]
    fn a_full_season_basis_averages_uneven_episode_durations() {
        let episodes = vec![
            episode_of("ep-1", "1", "1", 20),
            episode_of("ep-2", "1", "2", 25),
            episode_of("ep-3", "1", "3", 45),
        ];
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            season: Some(1),
            full_season: true,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });

        let basis = coverage_size_basis(
            &ReleaseCoverage::Collection("season-1".to_string()),
            &parsed,
            &episodes,
            Some(25),
        );
        assert_eq!(basis.total_runtime_minutes, Some(90));
        assert_eq!(basis.member_runtime_minutes, Some(30));
        assert_eq!(basis.member_count, 3);
    }

    /// The partial-season estimate is unchanged — half the season, rounded the
    /// way it always was — and the member count is the estimate's own, so the
    /// two halves of the basis describe the same release.
    #[test]
    fn a_partial_season_basis_keeps_the_half_season_estimate() {
        let episodes = (1..=26)
            .map(|number| {
                let number = number.to_string();
                episode(&format!("ep-{number}"), "17", &number, Some(&number))
            })
            .collect::<Vec<_>>();
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            season: Some(17),
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            full_season: true,
            is_partial_season: true,
            season_part: Some(2),
            ..Default::default()
        });

        let basis = coverage_size_basis(
            &ReleaseCoverage::Collection("season-17".to_string()),
            &parsed,
            &episodes,
            Some(24),
        );
        assert_eq!(basis.total_runtime_minutes, Some(13 * 24));
        assert_eq!(basis.member_runtime_minutes, Some(24));
        assert_eq!(basis.member_count, 13);
        // …and the total is still exactly what the runtime accessor reports.
        assert_eq!(
            coverage_runtime_minutes(
                &ReleaseCoverage::Collection("season-17".to_string()),
                &parsed,
                &episodes,
                Some(24)
            ),
            basis.total_runtime_minutes
        );
    }

    /// A multi-season or complete-series pack arrives as an `EpisodeSet` of
    /// every eligible episode; the basis spans all of them and still knows what
    /// one of them is.
    #[test]
    fn a_multi_season_pack_basis_spans_every_member() {
        let episodes = (1..=12)
            .map(|number| {
                let number = number.to_string();
                episode_of(&format!("s1-{number}"), "1", &number, 25)
            })
            .chain((1..=12).map(|number| {
                let number = number.to_string();
                episode_of(&format!("s2-{number}"), "2", &number, 25)
            }))
            .collect::<Vec<_>>();
        let episode_ids = episodes
            .iter()
            .map(|episode| episode.id.clone())
            .collect::<Vec<_>>();

        let basis = episode_span_size_basis(&episodes, &episode_ids, Some(25));
        assert_eq!(basis.total_runtime_minutes, Some(24 * 25));
        assert_eq!(basis.member_runtime_minutes, Some(25));
        assert_eq!(basis.member_count, 24);
    }

    /// Episodes the catalog has no duration for fall back to the title's own
    /// runtime, exactly as the total always did — the member runtime cannot
    /// invent evidence the total does not have.
    #[test]
    fn missing_episode_runtimes_fall_back_to_the_title_default() {
        let episodes = (1..=6)
            .map(|number| {
                let number = number.to_string();
                let mut episode = episode(&format!("ep-{number}"), "1", &number, Some(&number));
                episode.duration_seconds = None;
                episode
            })
            .collect::<Vec<_>>();
        let episode_ids = episodes
            .iter()
            .map(|episode| episode.id.clone())
            .collect::<Vec<_>>();

        let basis = episode_span_size_basis(&episodes, &episode_ids, Some(50));
        assert_eq!(basis.total_runtime_minutes, Some(300));
        assert_eq!(basis.member_runtime_minutes, Some(50));
        assert_eq!(basis.member_count, 6);

        // With no title runtime either, the assumed episode length stands in.
        let unknown = episode_span_size_basis(&episodes, &episode_ids, None);
        assert_eq!(
            unknown.total_runtime_minutes,
            Some(6 * UNKNOWN_EPISODE_RUNTIME_MINUTES)
        );
        assert_eq!(
            unknown.member_runtime_minutes,
            Some(UNKNOWN_EPISODE_RUNTIME_MINUTES)
        );
    }

    /// A scope with no episodes has no basis of its own; the caller's fallback
    /// decides, which is how a title or link scope keeps the movie's runtime.
    #[test]
    fn an_empty_span_defers_to_the_callers_runtime() {
        let basis = episode_span_size_basis(&[], &[], Some(45));
        assert_eq!(basis, CoverageSizeBasis::single(None));
        assert_eq!(
            basis.or_runtime(Some(118)),
            CoverageSizeBasis::single(Some(118))
        );
    }

    /// Title and movie coverage is one member, so nothing about it is ever
    /// reinterpreted.
    #[test]
    fn a_title_scope_is_a_single_member_basis() {
        let parsed = parsed_with_episode(ParsedEpisodeMetadata::default());
        for coverage in [ReleaseCoverage::Title, ReleaseCoverage::Unknown] {
            let basis = coverage_size_basis(&coverage, &parsed, &[], Some(118));
            assert_eq!(basis, CoverageSizeBasis::single(Some(118)));
            assert!(!basis.covers_multiple_members());
        }
    }

    #[test]
    fn title_only_coverage_does_not_cover_requested_episode() {
        let episode = episode("ep-1", "1", "1", Some("1"));

        assert!(!ReleaseCoverage::Title.covers_episode(&episode));
        assert!(!ReleaseCoverage::Unknown.covers_episode(&episode));
    }

    #[test]
    fn unresolved_coverage_uses_requested_scope_instead_of_widening_to_title() {
        let fallback = SubmissionScope::Episode {
            episode_id: "ep-1".to_string(),
        };

        assert_eq!(
            ReleaseCoverage::Unknown.submission_scope_or(&fallback),
            fallback
        );
    }

    #[test]
    fn multi_season_series_pack_uses_only_the_pack_coverage_for_qualification() {
        let episodes = (1..=10)
            .flat_map(|season| {
                (1..=4).map(move |number| {
                    episode(
                        &format!("s{season}-e{number}"),
                        &season.to_string(),
                        &number.to_string(),
                        None,
                    )
                })
            })
            .collect::<Vec<_>>();
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            season_numbers: vec![1, 2, 3, 4],
            full_season: true,
            is_series_pack: true,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });
        let primary_episode_ids = episodes
            .iter()
            .filter(|episode| {
                !matches!(
                    episode.id.as_str(),
                    "s1-e1" | "s1-e2" | "s1-e3" | "s1-e4" | "s2-e1"
                )
            })
            .map(|episode| episode.id.clone())
            .collect();

        assert!(series_pack_missing_ratio_qualifies(
            &parsed,
            &episodes,
            &primary_episode_ids
        ));
        assert!(title_series_pack_missing_ratio_qualifies(
            &episodes,
            &primary_episode_ids
        ));
    }

    #[test]
    fn series_pack_title_membership_and_missing_guard_use_eligible_episodes() {
        let episodes = vec![
            episode("s1-e1", "1", "1", None),
            episode("s1-e2", "1", "2", None),
            episode("s2-e1", "2", "1", None),
        ];
        let primary_episode_ids = ["s1-e2", "s2-e1"].into_iter().map(str::to_string).collect();
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            season_numbers: vec![1],
            full_season: true,
            is_series_pack: true,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });

        assert_eq!(
            eligible_series_pack_collection_ids(&episodes),
            vec!["season-1".to_string(), "season-2".to_string()]
        );
        assert_eq!(
            series_pack_collection_ids(&parsed, &episodes),
            vec!["season-1".to_string()]
        );
        assert_eq!(
            eligible_missing_series_pack_episode_count(&episodes, &primary_episode_ids),
            1
        );
    }

    #[test]
    fn series_pack_threshold_rejects_thirty_percent_and_accepts_thirty_one_percent() {
        let episodes = (1..=100)
            .map(|number| episode(&format!("ep-{number}"), "1", &number.to_string(), None))
            .collect::<Vec<_>>();
        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            full_season: true,
            is_series_pack: true,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });
        let seventy_owned = (1..=70)
            .map(|number| format!("ep-{number}"))
            .collect::<std::collections::HashSet<_>>();
        assert!(!series_pack_missing_ratio_qualifies(
            &parsed,
            &episodes,
            &seventy_owned
        ));

        let sixty_nine_owned = (1..=69)
            .map(|number| format!("ep-{number}"))
            .collect::<std::collections::HashSet<_>>();
        assert!(series_pack_missing_ratio_qualifies(
            &parsed,
            &episodes,
            &sixty_nine_owned
        ));
    }

    #[test]
    fn series_pack_threshold_excludes_special_unmonitored_and_unaired_episodes() {
        let mut episodes = (1..=10)
            .map(|number| episode(&format!("ep-{number}"), "1", &number.to_string(), None))
            .collect::<Vec<_>>();
        let mut special = episode("special-1", "0", "1", None);
        special.episode_type = EpisodeType::Special;
        episodes.push(special);

        let mut unmonitored = episode("unmonitored-1", "1", "11", None);
        unmonitored.monitored = false;
        episodes.push(unmonitored);

        let mut unaired = episode("unaired-1", "1", "12", None);
        unaired.air_date = Some("2999-01-01".to_string());
        episodes.push(unaired);

        let parsed = parsed_with_episode(ParsedEpisodeMetadata {
            full_season: true,
            is_series_pack: true,
            release_type: ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });
        let primary_episode_ids = (1..=7)
            .map(|number| format!("ep-{number}"))
            .collect::<std::collections::HashSet<_>>();

        assert!(!series_pack_missing_ratio_qualifies(
            &parsed,
            &episodes,
            &primary_episode_ids
        ));
        assert!(!title_series_pack_missing_ratio_qualifies(
            &episodes,
            &primary_episode_ids
        ));

        let primary_episode_ids = (1..=6)
            .map(|number| format!("ep-{number}"))
            .collect::<std::collections::HashSet<_>>();
        assert!(series_pack_missing_ratio_qualifies(
            &parsed,
            &episodes,
            &primary_episode_ids
        ));
        assert!(title_series_pack_missing_ratio_qualifies(
            &episodes,
            &primary_episode_ids
        ));
    }
}
