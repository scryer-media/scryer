//! Medium is an axis, not a genre.
//!
//! Every title belongs to exactly one medium — live action, (western)
//! animation, or anime — and the three are nearly disjoint audiences. A library
//! of Pixar and DreamWorks says nothing positive about anime, and an anime
//! library says nothing positive about Bluey. Kind (movie / series) is a
//! separate axis and is not modelled here.
//!
//! The classifier is the single place that decision is made, and it is reused
//! for owned titles (to derive the library's medium mix) and for pool items (to
//! gate what a rail may contain). Source classification decides, never tags: a
//! shared `canonical:genre:animation` facet must never make an anime look like
//! Western animation, so the anime witness is always checked first.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum DiscoveryMedium {
    LiveAction,
    Animation,
    Anime,
}

/// Counts of owned titles per medium.
///
/// Absence is evidence: a medium whose count is zero has been rejected by the
/// library, and no genre, theme, edge, co-watch or semantic signal outranks
/// that rejection. Above zero the share is the ceiling on a rail's medium mix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiscoveryLibraryMediumMix {
    pub(crate) live_action: usize,
    pub(crate) animation: usize,
    pub(crate) anime: usize,
}

impl DiscoveryLibraryMediumMix {
    pub(super) fn count(&self, medium: DiscoveryMedium) -> usize {
        match medium {
            DiscoveryMedium::LiveAction => self.live_action,
            DiscoveryMedium::Animation => self.animation,
            DiscoveryMedium::Anime => self.anime,
        }
    }

    pub(super) fn add(&mut self, medium: DiscoveryMedium) {
        match medium {
            DiscoveryMedium::LiveAction => self.live_action += 1,
            DiscoveryMedium::Animation => self.animation += 1,
            DiscoveryMedium::Anime => self.anime += 1,
        }
    }

    pub(super) fn total(&self) -> usize {
        self.live_action + self.animation + self.anime
    }

    pub(super) fn share(&self, medium: DiscoveryMedium) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.count(medium) as f64 / total as f64
    }

    /// Zero means zero. No tolerance, no minimum library size: a library that
    /// owns nothing of a medium has said so, and every personalized rail obeys
    /// it — theme rails, genre rails, FOR_YOU, and the Anime/Animation label
    /// rails themselves.
    ///
    /// An *empty* mix is the one exception, and it is not a tolerance: a
    /// library with no owned titles at all has rejected nothing, it has said
    /// nothing. Absence of evidence is only evidence of absence when there was
    /// something to observe.
    pub(super) fn admits(&self, medium: DiscoveryMedium) -> bool {
        self.total() == 0 || self.count(medium) > 0
    }

    /// Ceiling on how many of a rail's `limit` slots one medium may take.
    ///
    /// A rail's medium share may not exceed [`DISCOVERY_MEDIUM_SHARE_MULTIPLIER`]
    /// times the library's share, with a floor of
    /// [`DISCOVERY_MEDIUM_SHARE_MIN_SLOTS`] slots so a library that owns a
    /// single anime can still be shown a couple. Rails are refilled from the
    /// next candidates rather than truncated.
    pub(super) fn slot_cap(&self, medium: DiscoveryMedium, limit: usize) -> usize {
        if !self.admits(medium) {
            return 0;
        }
        if self.total() == 0 {
            // Nothing observed, nothing to be proportional to.
            return limit;
        }
        let proportional =
            (self.share(medium) * DISCOVERY_MEDIUM_SHARE_MULTIPLIER * limit as f64).ceil();
        let proportional = if proportional.is_finite() && proportional > 0.0 {
            proportional as usize
        } else {
            0
        };
        proportional
            .max(DISCOVERY_MEDIUM_SHARE_MIN_SLOTS)
            .min(limit)
    }

    pub(super) fn from_titles(titles: &[Title]) -> Self {
        let mut mix = Self::default();
        for title in titles {
            mix.add(discovery_title_medium(title));
        }
        mix
    }

    pub(super) fn to_context_input(self) -> DiscoveryContextMediumMixInput {
        DiscoveryContextMediumMixInput {
            live_action: self.live_action.min(i32::MAX as usize) as i32,
            animation: self.animation.min(i32::MAX as usize) as i32,
            anime: self.anime.min(i32::MAX as usize) as i32,
        }
    }
}

/// The medium of a pool item.
///
/// Anime first (media kind, then the canonical `anime` genre facet as a second
/// witness, exactly as the anime/animation boundary guard has always done),
/// then animation, then live action. Scryer's own media kinds are only
/// `movie` / `series` / `anime`, so the animation arm is decided by the
/// canonical `animation` genre facet — which is meaningful only because the
/// anime arm already claimed every anime that shares it.
pub(super) fn discovery_item_medium(item: &DiscoveryItemRecord) -> DiscoveryMedium {
    if discovery_item_is_anime(item) {
        return DiscoveryMedium::Anime;
    }
    if discovery_item_canonical_facet_labels(item, "genre")
        .iter()
        .any(|label| normalize_discovery_affinity_key(label) == "animation")
    {
        return DiscoveryMedium::Animation;
    }
    DiscoveryMedium::LiveAction
}

/// The medium of an owned title. Same ladder as [`discovery_item_medium`], read
/// off the catalog facet and the canonical tags SMG assigned to the title.
pub(super) fn discovery_title_medium(title: &Title) -> DiscoveryMedium {
    if title.facet == MediaFacet::Anime || owned_title_has_genre(title, "anime") {
        return DiscoveryMedium::Anime;
    }
    if owned_title_has_genre(title, "animation") {
        return DiscoveryMedium::Animation;
    }
    DiscoveryMedium::LiveAction
}

fn owned_title_has_genre(title: &Title, genre_key: &str) -> bool {
    title
        .canonical_tags
        .iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
        .any(|tag| normalize_discovery_affinity_key(&tag.name) == genre_key)
}
