//! The labels allowed to name a "because you like" rail, and the English
//! sentence each one reads as.
//!
//! A rail label is a sentence a person would say. Labels come from this
//! explicit allowlist, never from the raw canonical tag vocabulary: medium
//! nouns (`animation`, `anime`), provider-register tags (`school`, `cgdct`,
//! `harem`, ...), demographic genres (`boys-love`, `girls-love`) and jargon
//! (`slice-of-life`, `gourmet`, `avant-garde`, `iyashikei`) are left out on
//! purpose. Medium is owned by the facet chips and the medium-mix gate, so no
//! medium can ever name a rail.
//!
//! The metadata gateway serves the same vocabulary decision as
//! `rail_label_eligible` on canonical tags and affinity signals. This list is
//! the mirror that protects a release already in the field when the gateway
//! widens its list: a label must pass both. Today only this mirror is applied;
//! reading the gateway's `rail_label_eligible` flag is a follow-up once the
//! gateway release that serves it is deployed, and until then no gateway
//! document may select that field.
//!
//! The web resolves `labelKey` through `discovery.rail.becauseYouLike.<key>`
//! and keeps its key list in `apps/scryer-web/lib/contracts/discovery-rail-labels.json`;
//! a test here keeps that contract equal to this table.

use super::normalize_discovery_affinity_key;

/// The canonical family a rail label belongs to. A key is only a label in its
/// own family: a theme tag that happens to be named "Horror" is not the
/// `horror` genre.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiscoveryRailLabelFamily {
    Genre,
    Theme,
}

impl DiscoveryRailLabelFamily {
    pub(crate) fn from_canonical_kind(kind: &str) -> Option<Self> {
        if kind.eq_ignore_ascii_case("genre") {
            Some(Self::Genre)
        } else if kind.eq_ignore_ascii_case("theme") {
            Some(Self::Theme)
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Genre => "genre",
            Self::Theme => "theme",
        }
    }
}

/// One allowlisted rail label.
#[derive(Debug)]
pub(crate) struct DiscoveryRailLabel {
    /// Canonical hyphenated key, served to clients as `labelKey`. Stable API:
    /// never rename or repurpose a released key.
    pub(crate) key: &'static str,
    pub(crate) family: DiscoveryRailLabelFamily,
    /// The slug of the affinity signal the gateway attaches to this tag
    /// (`affinity:<family>:<signal_slug>`). It equals `key` except where the
    /// gateway's registry folds a tag into an older signal key: the `lgbtq`
    /// theme signals as `affinity:theme:lgbt`.
    pub(crate) signal_slug: &'static str,
    /// The English rail title. Clients that know the key translate it; the
    /// rest read this.
    pub(crate) title: &'static str,
}

const fn genre(key: &'static str, title: &'static str) -> DiscoveryRailLabel {
    DiscoveryRailLabel {
        key,
        family: DiscoveryRailLabelFamily::Genre,
        signal_slug: key,
        title,
    }
}

const fn theme(key: &'static str, title: &'static str) -> DiscoveryRailLabel {
    theme_with_signal(key, key, title)
}

const fn theme_with_signal(
    key: &'static str,
    signal_slug: &'static str,
    title: &'static str,
) -> DiscoveryRailLabel {
    DiscoveryRailLabel {
        key,
        family: DiscoveryRailLabelFamily::Theme,
        signal_slug,
        title,
    }
}

pub(crate) const DISCOVERY_RAIL_LABELS: &[DiscoveryRailLabel] = &[
    genre("action", "Because you like action"),
    genre("adventure", "Because you like adventure"),
    genre("comedy", "Because you like comedy"),
    genre("crime", "Because you like crime"),
    genre("documentary", "Because you like documentaries"),
    genre("drama", "Because you like drama"),
    genre("family", "Because you like family favorites"),
    genre("fantasy", "Because you like fantasy"),
    genre("history", "Because you like history"),
    genre("horror", "Because you like horror"),
    genre("mystery", "Because you like mysteries"),
    genre("romance", "Because you like romance"),
    genre("science-fiction", "Because you like science fiction"),
    genre("thriller", "Because you like thrillers"),
    genre("war", "Because you like war stories"),
    genre("western", "Because you like westerns"),
    genre("sports", "Because you like sports stories"),
    genre("supernatural", "Because you like the supernatural"),
    theme("coming-of-age", "Because you like coming-of-age stories"),
    theme("found-family", "Because you like found families"),
    theme("revenge", "Because you like revenge stories"),
    theme("heist", "Because you like heists"),
    theme("time-travel", "Because you like time travel"),
    theme("dystopia", "Because you like dystopias"),
    theme("cyberpunk", "Because you like cyberpunk"),
    theme("superhero", "Because you like superheroes"),
    theme("vampire", "Because you like vampires"),
    theme("zombie", "Because you like zombies"),
    theme("survival", "Because you like survival stories"),
    theme("space", "Because you like space adventures"),
    theme("detective", "Because you like detective stories"),
    theme("organized-crime", "Because you like organized crime"),
    theme("period-drama", "Because you like period dramas"),
    theme("psychological", "Because you like psychological stories"),
    theme("mythology", "Because you like mythology"),
    theme("isekai", "Because you like isekai"),
    theme("mecha", "Because you like mecha"),
    theme("martial-arts", "Because you like martial arts"),
    theme("music", "Because you like music stories"),
    theme("workplace", "Because you like workplace stories"),
    theme("medical", "Because you like medical dramas"),
    theme("military", "Because you like military stories"),
    theme("spy", "Because you like spy stories"),
    theme("royal-politics", "Because you like royal intrigue"),
    theme("first-love", "Because you like first-love stories"),
    theme("forbidden-love", "Because you like forbidden love"),
    theme_with_signal("lgbtq", "lgbt", "Because you like LGBTQ+ stories"),
    theme("samurai", "Because you like samurai"),
    theme("magical-girl", "Because you like magical girls"),
];

/// The allowlisted rail label a canonical label names in `family`, if any.
///
/// The comparison is on the normalized form, so "coming-of-age",
/// "Coming of Age" and "coming of age" all resolve to `coming-of-age`.
pub(crate) fn discovery_rail_label(
    family: DiscoveryRailLabelFamily,
    label: &str,
) -> Option<&'static DiscoveryRailLabel> {
    let normalized = normalize_discovery_affinity_key(label);
    if normalized.is_empty() {
        return None;
    }
    DISCOVERY_RAIL_LABELS.iter().find(|entry| {
        entry.family == family && normalize_discovery_affinity_key(entry.key) == normalized
    })
}

/// Whether an affinity signal slug (the tail of `affinity:<family>:<slug>`) is
/// the signal that backs `label` in `family`. A label outside the allowlist
/// can only be backed by a signal that spells it the same way; an allowlisted
/// label is also backed by the gateway's own signal key for its tag.
pub(crate) fn discovery_rail_label_signal_matches(
    family: DiscoveryRailLabelFamily,
    label: &str,
    signal_slug: &str,
) -> bool {
    let signal = normalize_discovery_affinity_key(signal_slug);
    if signal.is_empty() {
        return false;
    }
    normalize_discovery_affinity_key(label) == signal
        || discovery_rail_label(family, label)
            .is_some_and(|entry| normalize_discovery_affinity_key(entry.signal_slug) == signal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn every_rail_label_key_is_canonical_unique_and_phrased() {
        let mut keys = BTreeSet::new();
        for entry in DISCOVERY_RAIL_LABELS {
            assert!(keys.insert(entry.key), "duplicate rail label {}", entry.key);
            assert!(
                entry
                    .key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "rail label key {} must be lowercase and hyphenated",
                entry.key
            );
            assert!(
                entry.title.starts_with("Because you like ")
                    && entry.title.len() > "Because you like ".len(),
                "rail label {} needs a sentence",
                entry.key
            );
            assert_eq!(
                discovery_rail_label(entry.family, entry.key).map(|found| found.key),
                Some(entry.key)
            );
        }
    }

    #[test]
    fn rail_labels_resolve_from_every_spelling_and_only_in_their_family() {
        for spelling in [
            "coming-of-age",
            "Coming of Age",
            "coming of age",
            "COMING_OF_AGE",
        ] {
            assert_eq!(
                discovery_rail_label(DiscoveryRailLabelFamily::Theme, spelling)
                    .map(|entry| entry.key),
                Some("coming-of-age"),
                "{spelling}"
            );
        }
        assert_eq!(
            discovery_rail_label(DiscoveryRailLabelFamily::Genre, "Science Fiction")
                .map(|entry| entry.key),
            Some("science-fiction")
        );
        assert!(discovery_rail_label(DiscoveryRailLabelFamily::Theme, "Horror").is_none());
        assert!(discovery_rail_label(DiscoveryRailLabelFamily::Genre, "Heist").is_none());
    }

    #[test]
    fn only_lgbtq_signals_under_a_different_slug() {
        for entry in DISCOVERY_RAIL_LABELS {
            if entry.key == "lgbtq" {
                assert_eq!(entry.signal_slug, "lgbt");
            } else {
                assert_eq!(entry.signal_slug, entry.key);
            }
        }
        let theme = DiscoveryRailLabelFamily::Theme;
        assert!(discovery_rail_label_signal_matches(theme, "LGBTQ", "lgbt"));
        assert!(discovery_rail_label_signal_matches(theme, "LGBTQ", "lgbtq"));
        assert!(discovery_rail_label_signal_matches(theme, "Heist", "heist"));
        assert!(!discovery_rail_label_signal_matches(theme, "Heist", "lgbt"));
        assert!(!discovery_rail_label_signal_matches(
            theme,
            "LGBTQ",
            "coming-out"
        ));
        assert!(!discovery_rail_label_signal_matches(
            DiscoveryRailLabelFamily::Genre,
            "LGBTQ",
            "lgbt"
        ));
        assert!(!discovery_rail_label_signal_matches(theme, "LGBTQ", ""));
    }

    #[test]
    fn mediums_jargon_and_demographics_never_name_a_rail() {
        for family in [
            DiscoveryRailLabelFamily::Genre,
            DiscoveryRailLabelFamily::Theme,
        ] {
            for excluded in [
                "anime",
                "Animation",
                "School",
                "CGDCT",
                "Harem",
                "Adult Cast",
                "Delinquents",
                "Otaku Culture",
                "Gag Humor",
                "Parody",
                "Crossdressing",
                "Magical Sex Shift",
                "Love Polygon",
                "Love Status Quo",
                "Reverse Harem",
                "Villainess",
                "Boys Love",
                "Girls Love",
                "Slice of Life",
                "Gourmet",
                "Avant Garde",
                "Iyashikei",
                "movies",
                "tmdb",
                "",
            ] {
                assert!(
                    discovery_rail_label(family, excluded).is_none(),
                    "{excluded:?} must not name a {} rail",
                    family.as_str()
                );
            }
        }
    }

    /// The web translates `labelKey` from the shared contract; the contract,
    /// and so every locale, must carry exactly this table's keys.
    #[test]
    fn web_rail_label_contract_matches_the_allowlist() {
        let contract: BTreeMap<String, BTreeSet<String>> = serde_json::from_str(include_str!(
            "../../../../apps/scryer-web/lib/contracts/discovery-rail-labels.json"
        ))
        .expect("rail label contract is JSON");
        let mut expected = BTreeMap::<String, BTreeSet<String>>::new();
        for entry in DISCOVERY_RAIL_LABELS {
            expected
                .entry(entry.family.as_str().to_string())
                .or_default()
                .insert(entry.key.to_string());
        }
        assert_eq!(contract, expected);
    }
}
