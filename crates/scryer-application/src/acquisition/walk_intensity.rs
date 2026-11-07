//! How hard the background acquisition walk works: the share of wall time it
//! may take, how many titles it walks at once, how deep it pages a cold
//! scope's indexer feeds, and how far one walk follows scopes that cost
//! nothing.
//!
//! The walk rests for the walk interval after every walk. Its slice is sized
//! from that rest so that walking and resting together give exactly the
//! intensity's time share: a slice of `rest * share / (1 - share)`.

use std::time::Duration;

/// Above this many cached targets, `Normal` walks as `Quiet`.
///
/// Sized from the library-scale bench: about one hundred thousand media files
/// (thirty thousand movies, fifteen hundred series) left about thirteen
/// thousand scopes missing or below cutoff. A rotation that long is a
/// permanent backlog: a walk always has work, so it always spends its whole
/// slice. Ten thousand puts the switch below that library, where the backlog
/// no longer drains between walks and the walk is better kept to a tenth of
/// the machine. `Eager` is the operator saying they want the backlog drained
/// anyway, and is never lowered.
pub const AUTO_QUIET_TARGET_THRESHOLD: usize = 10_000;

/// The share of a slice the cold long tail may use. Hot scopes take the rest
/// of the slice first; the cold tail always gets this much after them.
pub(crate) const COLD_SUB_SLICE_PERCENT: u32 = 25;

/// Indexer pages a hot scope reads per pass, whatever the intensity.
pub(crate) const HOT_PAGES_PER_PASS: u32 = 5;

/// The shortest slice any intensity gets, so a very short walk interval
/// still lets a walk reach its first title's first search.
const MIN_WALK_SLICE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AcquisitionWalkIntensity {
    Quiet,
    #[default]
    Normal,
    Eager,
}

impl AcquisitionWalkIntensity {
    pub const ALL: [Self; 3] = [Self::Quiet, Self::Normal, Self::Eager];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Normal => "normal",
            Self::Eager => "eager",
        }
    }

    /// The stored form back to the value. Anything else, a row written by
    /// hand included, reads as the default.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|intensity| intensity.as_str() == value.trim())
    }

    /// A compact form for an atomic: the position in [`Self::ALL`].
    pub(crate) fn to_index(self) -> u8 {
        match self {
            Self::Quiet => 0,
            Self::Normal => 1,
            Self::Eager => 2,
        }
    }

    /// [`Self::to_index`] back to the value; anything out of range is the
    /// default.
    pub(crate) fn from_index(index: u8) -> Self {
        Self::ALL
            .get(usize::from(index))
            .copied()
            .unwrap_or_default()
    }

    /// The intensity a walk over `cached_targets` targets actually runs at.
    pub(crate) fn effective(self, cached_targets: usize) -> Self {
        match self {
            Self::Normal if cached_targets > AUTO_QUIET_TARGET_THRESHOLD => Self::Quiet,
            other => other,
        }
    }

    /// Percent of wall time walking may take.
    pub(crate) fn time_share_percent(self) -> u32 {
        match self {
            Self::Quiet => 10,
            Self::Normal => 20,
            Self::Eager => 50,
        }
    }

    /// Titles walked at once.
    pub(crate) fn title_concurrency(self) -> usize {
        match self {
            Self::Quiet => 1,
            Self::Normal => 2,
            Self::Eager => 4,
        }
    }

    /// Indexer pages a cold scope reads per pass. Hot scopes keep
    /// [`HOT_PAGES_PER_PASS`].
    pub(crate) fn cold_pages_per_pass(self) -> u32 {
        match self {
            Self::Quiet => 1,
            Self::Normal => 2,
            Self::Eager => HOT_PAGES_PER_PASS,
        }
    }

    /// How many scopes one walk may consider, as a multiple of its batch.
    /// Scopes whose walk spent nothing are replaced from further along the
    /// rotation; this bounds how far one walk follows them.
    pub(crate) fn scan_top_up_factor(self) -> usize {
        match self {
            Self::Quiet => 1,
            Self::Normal => 2,
            Self::Eager => 4,
        }
    }

    /// Search results one cold-scope search scores. The cap is taken a round
    /// at a time across indexers, each in the order it returned them, after
    /// duplicates and blocklisted releases are dropped and before anything is
    /// parsed. Hot scopes are scored in full.
    pub(crate) fn evaluated_results_per_search(self) -> usize {
        match self {
            Self::Quiet => 150,
            Self::Normal => 300,
            Self::Eager => 1_000,
        }
    }

    /// The walk's slice when it rests `rest` after every walk: the length
    /// that makes walking `share` of walking plus resting.
    pub(crate) fn walk_slice(self, rest: Duration) -> Duration {
        let share = u64::from(self.time_share_percent());
        let rest_ms = rest.as_millis().min(u128::from(u64::MAX)) as u64;
        let slice_ms = rest_ms.saturating_mul(share) / (100 - share).max(1);
        Duration::from_millis(slice_ms).max(MIN_WALK_SLICE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slice_and_the_rest_give_the_intensity_its_share() {
        let rest = Duration::from_secs(300);
        for intensity in AcquisitionWalkIntensity::ALL {
            let slice = intensity.walk_slice(rest);
            let share = slice.as_secs_f64() / (slice + rest).as_secs_f64();
            assert!(
                (share - f64::from(intensity.time_share_percent()) / 100.0).abs() < 1e-6,
                "{intensity:?}: share {share}"
            );
        }
        assert_eq!(
            AcquisitionWalkIntensity::Normal.walk_slice(rest),
            Duration::from_secs(75)
        );
        assert_eq!(
            AcquisitionWalkIntensity::Quiet.walk_slice(Duration::from_secs(1)),
            MIN_WALK_SLICE
        );
    }

    #[test]
    fn only_normal_turns_quiet_on_a_large_backlog() {
        use AcquisitionWalkIntensity::{Eager, Normal, Quiet};
        assert_eq!(Normal.effective(AUTO_QUIET_TARGET_THRESHOLD), Normal);
        assert_eq!(Normal.effective(AUTO_QUIET_TARGET_THRESHOLD + 1), Quiet);
        assert_eq!(Eager.effective(usize::MAX), Eager);
        assert_eq!(Quiet.effective(0), Quiet);
    }

    #[test]
    fn every_intensity_round_trips_through_its_stored_form() {
        for intensity in AcquisitionWalkIntensity::ALL {
            assert_eq!(
                AcquisitionWalkIntensity::parse(intensity.as_str()),
                Some(intensity)
            );
            assert_eq!(
                AcquisitionWalkIntensity::from_index(intensity.to_index()),
                intensity
            );
        }
        assert_eq!(AcquisitionWalkIntensity::parse("loud"), None);
    }

    #[test]
    fn heavier_intensities_never_do_less() {
        let [quiet, normal, eager] = AcquisitionWalkIntensity::ALL;
        for (lighter, heavier) in [(quiet, normal), (normal, eager)] {
            assert!(lighter.time_share_percent() < heavier.time_share_percent());
            assert!(lighter.title_concurrency() < heavier.title_concurrency());
            assert!(lighter.cold_pages_per_pass() < heavier.cold_pages_per_pass());
            assert!(lighter.scan_top_up_factor() < heavier.scan_top_up_factor());
            assert!(
                lighter.evaluated_results_per_search() < heavier.evaluated_results_per_search()
            );
        }
        assert!(eager.cold_pages_per_pass() <= HOT_PAGES_PER_PASS);
    }
}
