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
//! compares only quality and custom-format score. Nothing about the listing
//! ever reaches the upgrade decision.
//!
//! Ranks are therefore built per search, keyed by release, and dropped when the
//! search ends. Nothing here is ever persisted.

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::IndexerSearchResult;

/// PROPER/REPACK rank — Sonarr's `QualityModel.Revision`, coarsened.
///
/// Sonarr counts real versions (`v2`, `v3`, …) and treats PROPER and REPACK as
/// separate increments; Scryer's parser reports two booleans and no version
/// counter, so a release is `0`, `1` (a PROPER *or* a REPACK) or `2` (both).
/// That is enough for the only question anyone asks of it — "is this a
/// re-release of what I already have?" — and it is the same number on both
/// sides of every comparison, which is what matters.
///
/// One function because three places need the answer and must not drift: the
/// search rank head below, [`crate::canonical_scoring::ScoredRelease::revision`]
/// (so an incumbent's bar carries its revision without a second parse), and
/// [`crate::admission::CandidateFacts`].
pub(crate) fn revision_rank(parsed: &crate::ParsedReleaseMetadata) -> i32 {
    i32::from(parsed.is_proper_upload) + i32::from(parsed.is_repack)
}

/// The part of a rank that is a property of the **release**: whether the profile
/// allows it, its tier, its revision, its score — compared in that order.
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
    /// Higher revision (PROPER/REPACK) wins, so this is stored negated.
    pub negated_revision: i32,
    /// The canonical score, negated so that higher wins.
    pub negated_score: i32,
}

impl RankHead {
    /// Read the head straight off a scored result.
    ///
    /// Everything it needs already travels on the result: the profile decision
    /// carries `allowed`, `preference_score` and (since D11) `tier_index`, and
    /// the parse carries PROPER/REPACK. An unscored result sorts as allowed,
    /// untiered and zero rather than being dropped — ordering must never fail a
    /// search.
    pub(crate) fn from_result(result: &IndexerSearchResult) -> Self {
        let decision = result.quality_profile_decision.as_ref();
        let parsed = result.parsed_release_metadata.as_ref();
        Self {
            blocked: decision.is_some_and(|decision| !decision.allowed),
            tier_index: crate::admission::tier_sort_key(
                decision.and_then(|decision| decision.tier_index),
            ),
            negated_revision: parsed.map_or(0, |parsed| revision_rank(parsed).saturating_neg()),
            negated_score: decision
                .map_or(0, |decision| decision.preference_score.saturating_neg()),
        }
    }

    fn key(&self) -> (bool, usize, i32, i32) {
        (
            self.blocked,
            self.tier_index,
            self.negated_revision,
            self.negated_score,
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
    /// Size, negated so the larger release wins when every stronger signal ties.
    pub negated_size_bytes: i64,
}

type SearchRankKey = (
    (bool, usize, i32, i32),
    bool,
    usize,
    u32,
    i64,
    i64,
    i64,
    i64,
);

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
            self.negated_size_bytes,
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
            negated_size_bytes: 0,
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
            negated_size_bytes: -1,
            ..rank()
        };
        let larger = SearchRank {
            usenet_age_hours: 2,
            negated_size_bytes: -1_000,
            ..rank()
        };
        assert!(fresher.key() < larger.key());

        let larger = SearchRank {
            negated_size_bytes: -1_000,
            ..rank()
        };
        let smaller = SearchRank {
            negated_size_bytes: -1,
            ..rank()
        };
        assert!(larger.key() < smaller.key());
    }
}
