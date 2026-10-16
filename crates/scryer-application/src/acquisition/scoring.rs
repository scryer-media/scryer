//! Search rank: how one set of search results is *ordered*.
//!
//! Deliberately separate from [`crate::canonical_scoring`], which decides what a
//! release is *worth*. The distinction is the whole point of this module.
//!
//! A score is compared across time — a candidate today against a file that
//! landed months ago — so it may only contain properties of the release itself.
//! A rank is compared only within one search, among results that are all
//! available right now, so it may freely use things that are true of the
//! *listing*: how fresh it is, which indexer carried it, how many peers it has.
//!
//! Blending the two is what caused the defect this module exists to prevent. A
//! freshness bonus folded into the score made a same-size re-grab read as an
//! upgrade; a pack penalty folded into the score made the same release worth
//! different amounts depending on what was being searched for. Neither is a
//! property of the file, and neither survives on a media row — so at import,
//! where the bar is re-derived, both silently vanished and the two sides
//! disagreed.
//!
//! Sonarr draws the same line: `DownloadDecisionComparer` chains quality →
//! custom-format score → protocol → episode coverage/number → indexer priority
//! → swarm/age → size as *comparator steps*, while `UpgradableSpecification`
//! compares only quality and custom-format score. Nothing in a rank ever
//! reaches the upgrade decision.
//!
//! Ranks are therefore built per search, keyed by release, and dropped when the
//! search ends. Nothing here is ever persisted.
//!
//! The listing facts user rules read — publish time, votes, password
//! protection, indexer languages and `extra` scalars — are a different matter.
//! They are frozen into a listing snapshot when the release is evaluated for
//! grab, persisted with the submission and copied to the media row, so they
//! survive where the bar is re-derived and count as release properties in the
//! score (see [`crate::canonical_scoring`]). Freshness, indexer priority and
//! peers stay here, unpersisted.

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::IndexerSearchResult;

/// Canonical numeric release revision, shared by search, queue and import.
pub(crate) fn revision_rank(parsed: &crate::ParsedReleaseMetadata) -> i32 {
    parsed.revision
}

/// Shared rank head: eligibility, tier, context-derived revision preference,
/// then score. Missing scopes and Ignore mode have no revision preference.
/// The language steps of [`crate::AudioLanguageRank`] slot in: the preferred rank
/// before the tier or after the revision, optional hits after the revision.
///
/// Split out because two places order search results: the rank comparator used
/// while scoring, and `compare_release_search_results`, which the interactive
/// search's incremental merge re-sorts a partial snapshot with. They must not
/// disagree, and they did: the merge compared allowed → score only, so with the
/// tier out of the score a 720p release scoring +300 listed above a 2160p one
/// scoring +100 (D11). Sharing the key makes that structurally impossible.
///
/// Everything below this — indexer priority, seeders, age, coverage — is a
/// property of the *listing*, available only while the search is running, and
/// stays on [`SearchRank`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RankHead {
    /// Rejected releases sort last, whatever else they have going for them.
    pub blocked: bool,
    /// Position in the profile's quality ordering; lower is better. `usize::MAX`
    /// for a quality the profile does not list.
    pub tier_index: usize,
    /// Negated revision preference, granted only by related-incumbent context.
    pub negated_revision: i32,
    /// The canonical score, negated so that higher wins.
    pub negated_score: i32,
    /// Size fit breaks otherwise equal release preferences; it is not a score.
    pub size_fit_penalty: i32,
    /// Standing against the title's preferred and optional audio languages.
    pub language: crate::AudioLanguageRank,
}

impl RankHead {
    /// Read the head straight off a scored result.
    ///
    /// Everything it needs already travels on the result: the profile decision
    /// carries eligibility, tier, score and the context-derived revision key.
    /// An unscored result sorts as allowed,
    /// untiered and zero rather than being dropped — ordering must never fail a
    /// search.
    pub(crate) fn from_result(result: &IndexerSearchResult) -> Self {
        let decision = result.quality_profile_decision.as_ref();
        Self {
            blocked: decision.is_some_and(|decision| !decision.allowed),
            tier_index: crate::admission::tier_sort_key(
                decision.and_then(|decision| decision.tier_index),
            ),
            negated_revision: decision
                .map_or(0, |decision| decision.revision_preference.saturating_neg()),
            negated_score: decision
                .map_or(0, |decision| decision.preference_score.saturating_neg()),
            size_fit_penalty: decision.map_or(0, |decision| decision.size_fit_penalty),
            language: decision.map_or_else(Default::default, |decision| decision.language),
        }
    }

    pub(crate) fn key(&self) -> RankHeadKey {
        (
            self.blocked,
            self.language.before_tier(),
            self.tier_index,
            self.negated_revision,
            self.language.after_revision(),
            self.language.negated_optional_hits(),
            self.negated_score,
            self.size_fit_penalty,
        )
    }

    /// Order two scored results by release worth alone.
    pub(crate) fn compare(left: &IndexerSearchResult, right: &IndexerSearchResult) -> Ordering {
        Self::from_result(left)
            .key()
            .cmp(&Self::from_result(right).key())
    }
}

/// One release's ordering key within a single search.
///
/// Fields are compared in declaration order and each is already oriented so
/// that "less" means "better", which keeps the comparator itself trivial.
#[derive(Debug, Clone, Default)]
pub(crate) struct SearchRank {
    /// Release worth: allowed, tier, revision, score. Shared with
    /// `compare_release_search_results`.
    pub head: RankHead,
    /// Whether this release uses a non-preferred protocol. Preferred protocol
    /// is compared after release worth and before listing-specific tie-breakers.
    pub non_preferred_protocol: bool,
    /// How many episodes this release covers, oriented by what was asked for:
    /// a single-episode search prefers the single, a season search prefers the
    /// pack. This replaces the old `single_episode_preference_penalty`, which
    /// applied the same preference as a *score* delta.
    pub coverage_distance: usize,
    /// Earliest episode number, so a deterministic order survives ties.
    pub episode_number: u32,
    /// Indexer priority, lower value = higher priority (Sonarr's convention).
    pub indexer_priority: i64,
    /// Torrent seeders, negated so more is better.
    ///
    /// `0` for usenet and for a torrent whose indexer did not report a count:
    /// "no information" is not the same as "no peers", and sorting an unknown
    /// count below a torrent with one seeder would quietly bury every release
    /// from an indexer that omits the field. It ties with them instead and the
    /// next step decides.
    pub negated_seeders: i64,
    /// Usenet age in whole hours. Fresher releases sort first; non-Usenet
    /// releases tie because swarm is their protocol-specific listing signal.
    pub usenet_age_hours: i64,
}

pub(crate) type RankHeadKey = (bool, u16, usize, i32, u16, i32, i32, i32);

type SearchRankKey = (RankHeadKey, bool, usize, u32, i64, i64, i64);

impl SearchRank {
    fn key(&self) -> SearchRankKey {
        (
            self.head.key(),
            self.non_preferred_protocol,
            self.coverage_distance,
            self.episode_number,
            self.indexer_priority,
            self.negated_seeders,
            self.usenet_age_hours,
        )
    }
}

/// Seeders as a rank step: negated so more is better, and `0` when the listing
/// reports none (usenet, or a torrent indexer that omits the field).
///
/// Reads the same `extra` key `candidate_meets_minimum_seeders` does, so the
/// gate and the ordering cannot disagree about how many peers a release has.
pub(crate) fn listing_negated_seeders(result: &IndexerSearchResult) -> i64 {
    crate::acquisition::seed_goals::seeders_from_extra(&result.extra)
        .filter(|seeders| *seeders > 0)
        .map_or(0, |seeders| -seeders)
}

/// Age in whole hours, or `i64::MAX` when the listing gave no publish date —
/// an unknown age sorts last rather than pretending to be brand new.
pub(crate) fn listing_age_hours(published_at: Option<&str>, now: DateTime<Utc>) -> i64 {
    published_at
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|published| {
            now.signed_duration_since(published.with_timezone(&Utc))
                .num_hours()
                .max(0)
        })
        .unwrap_or(i64::MAX)
}

/// Order search results best-first.
///
/// Ranks come from `rank_by_key`, built while the results were being scored;
/// a release with no rank recorded sorts as though it had a default one rather
/// than panicking, because ordering must never fail a search.
pub(crate) fn compare_ranked_results(
    left: &IndexerSearchResult,
    right: &IndexerSearchResult,
    rank_by_key: &HashMap<String, SearchRank>,
    key_of: impl Fn(&IndexerSearchResult) -> String,
) -> Ordering {
    let fallback = SearchRank::default();
    let left_rank = rank_by_key.get(&key_of(left)).unwrap_or(&fallback);
    let right_rank = rank_by_key.get(&key_of(right)).unwrap_or(&fallback);
    left_rank.key().cmp(&right_rank.key())
}

#[cfg(test)]
mod tests {
    use super::{RankHead, SearchRank};

    fn rank() -> SearchRank {
        SearchRank {
            head: RankHead::default(),
            non_preferred_protocol: false,
            coverage_distance: 0,
            episode_number: 0,
            indexer_priority: 0,
            negated_seeders: 0,
            usenet_age_hours: 0,
        }
    }

    #[test]
    fn automatic_rank_uses_sonarr_protocol_and_provider_order() {
        let preferred = SearchRank {
            coverage_distance: 1,
            indexer_priority: 100,
            non_preferred_protocol: false,
            ..rank()
        };
        let non_preferred = SearchRank {
            coverage_distance: 0,
            indexer_priority: 0,
            non_preferred_protocol: true,
            ..rank()
        };
        assert!(preferred.key() < non_preferred.key());

        let better_coverage = SearchRank {
            coverage_distance: 0,
            indexer_priority: 100,
            ..rank()
        };
        let better_indexer = SearchRank {
            coverage_distance: 1,
            indexer_priority: 0,
            ..rank()
        };
        assert!(better_coverage.key() < better_indexer.key());

        let better_episode = SearchRank {
            episode_number: 1,
            indexer_priority: 100,
            ..rank()
        };
        let better_indexer = SearchRank {
            episode_number: 2,
            indexer_priority: 0,
            ..rank()
        };
        assert!(better_episode.key() < better_indexer.key());
    }

    #[test]
    fn automatic_rank_uses_swarm_age_and_size_as_final_tiebreakers() {
        let better_indexer = SearchRank {
            indexer_priority: 0,
            negated_seeders: -100,
            ..rank()
        };
        let better_swarm = SearchRank {
            indexer_priority: 1,
            negated_seeders: -200,
            ..rank()
        };
        assert!(better_indexer.key() < better_swarm.key());

        let better_swarm = SearchRank {
            negated_seeders: -200,
            usenet_age_hours: 100,
            ..rank()
        };
        let fresher = SearchRank {
            negated_seeders: -100,
            usenet_age_hours: 1,
            ..rank()
        };
        assert!(better_swarm.key() < fresher.key());

        let fresher = SearchRank {
            usenet_age_hours: 1,
            ..rank()
        };
        let larger = SearchRank {
            usenet_age_hours: 2,
            ..rank()
        };
        assert!(fresher.key() < larger.key());

        let plausible = rank();
        let mut outlier = rank();
        outlier.head.size_fit_penalty = 100;
        assert!(plausible.key() < outlier.key());
        outlier.head.negated_score = -1;
        assert!(
            outlier.key() < plausible.key(),
            "size cannot outweigh score"
        );
    }

    #[test]
    fn a_lower_quality_is_a_fallback_even_with_a_much_higher_score() {
        let low = RankHead {
            tier_index: 1,
            negated_score: -2000,
            ..Default::default()
        };
        let high = RankHead {
            tier_index: 0,
            negated_score: -400,
            size_fit_penalty: 200,
            ..Default::default()
        };
        assert!(high.key() < low.key());
        let excluded = RankHead {
            blocked: true,
            ..high.clone()
        };
        assert!(
            low.key() < excluded.key(),
            "fallback wins if the higher quality is excluded"
        );
        let proper = RankHead {
            negated_revision: -1,
            ..high.clone()
        };
        assert!(proper.key() < high.key());
    }

    fn language(
        preferred_rank: u16,
        optional_hits: u16,
        language_first: bool,
    ) -> crate::AudioLanguageRank {
        crate::AudioLanguageRank {
            matched: crate::AudioLanguageMatch {
                preferred_rank,
                optional_hits,
            },
            language_first,
        }
    }

    #[test]
    fn language_first_puts_the_preferred_language_above_a_better_tier() {
        let german_1080p = RankHead {
            tier_index: 1,
            language: language(0, 0, true),
            ..Default::default()
        };
        let english_2160p = RankHead {
            tier_index: 0,
            negated_score: -5000,
            language: language(1, 1, true),
            ..Default::default()
        };
        assert!(german_1080p.key() < english_2160p.key());
    }

    #[test]
    fn quality_first_keeps_the_tier_ahead_of_the_preferred_language() {
        let german_1080p = RankHead {
            tier_index: 1,
            language: language(0, 0, false),
            ..Default::default()
        };
        let english_2160p = RankHead {
            tier_index: 0,
            language: language(1, 0, false),
            ..Default::default()
        };
        assert!(english_2160p.key() < german_1080p.key());
        let german_2160p = RankHead {
            tier_index: 0,
            negated_score: 0,
            language: language(0, 0, false),
            ..Default::default()
        };
        let english_2160p_high_score = RankHead {
            negated_score: -9000,
            ..english_2160p
        };
        assert!(german_2160p.key() < english_2160p_high_score.key());
    }

    #[test]
    fn optional_languages_follow_the_revision_and_precede_the_score() {
        let dual = RankHead {
            tier_index: 0,
            language: language(0, 1, true),
            ..Default::default()
        };
        let single_high_score = RankHead {
            tier_index: 0,
            negated_score: -9000,
            language: language(0, 0, true),
            ..Default::default()
        };
        assert!(dual.key() < single_high_score.key());
        let single_proper = RankHead {
            negated_revision: -1,
            ..single_high_score.clone()
        };
        assert!(single_proper.key() < dual.key());
        let better_tier_single = RankHead {
            tier_index: 0,
            ..single_high_score.clone()
        };
        let worse_tier_dual = RankHead {
            tier_index: 1,
            ..dual.clone()
        };
        assert!(better_tier_single.key() < worse_tier_dual.key());
    }

    #[test]
    fn without_preferences_the_language_steps_are_inert() {
        let a = RankHead {
            tier_index: 1,
            negated_score: -100,
            ..Default::default()
        };
        let b = RankHead {
            tier_index: 0,
            negated_score: -50,
            ..Default::default()
        };
        assert!(b.key() < a.key());
        assert_eq!(a.language, crate::AudioLanguageRank::default());
    }
}
