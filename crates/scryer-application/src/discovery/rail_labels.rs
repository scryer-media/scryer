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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    /// The English noun a "browse by" card is titled with. Equal to the web's
    /// `discovery.browseBy.<key>` text, which a test here keeps in step.
    pub(crate) browse_noun: &'static str,
}

const fn genre(
    key: &'static str,
    title: &'static str,
    browse_noun: &'static str,
) -> DiscoveryRailLabel {
    DiscoveryRailLabel {
        key,
        family: DiscoveryRailLabelFamily::Genre,
        signal_slug: key,
        title,
        browse_noun,
    }
}

const fn theme(
    key: &'static str,
    title: &'static str,
    browse_noun: &'static str,
) -> DiscoveryRailLabel {
    theme_with_signal(key, key, title, browse_noun)
}

const fn theme_with_signal(
    key: &'static str,
    signal_slug: &'static str,
    title: &'static str,
    browse_noun: &'static str,
) -> DiscoveryRailLabel {
    DiscoveryRailLabel {
        key,
        family: DiscoveryRailLabelFamily::Theme,
        signal_slug,
        title,
        browse_noun,
    }
}

pub(crate) const DISCOVERY_RAIL_LABELS: &[DiscoveryRailLabel] = &[
    genre("action", "Because you like action", "Action"),
    genre("adventure", "Because you like adventure", "Adventure"),
    genre("comedy", "Because you like comedy", "Comedy"),
    genre("crime", "Because you like crime", "Crime"),
    genre(
        "documentary",
        "Because you like documentaries",
        "Documentaries",
    ),
    genre("drama", "Because you like drama", "Drama"),
    genre("family", "Because you like family favorites", "Family"),
    genre("fantasy", "Because you like fantasy", "Fantasy"),
    genre("history", "Because you like history", "History"),
    genre("horror", "Because you like horror", "Horror"),
    genre("mystery", "Because you like mysteries", "Mystery"),
    genre("romance", "Because you like romance", "Romance"),
    genre(
        "science-fiction",
        "Because you like science fiction",
        "Science Fiction",
    ),
    genre("thriller", "Because you like thrillers", "Thrillers"),
    genre("war", "Because you like war stories", "War"),
    genre("western", "Because you like westerns", "Westerns"),
    genre("sports", "Because you like sports stories", "Sports"),
    genre(
        "supernatural",
        "Because you like the supernatural",
        "Supernatural",
    ),
    theme(
        "coming-of-age",
        "Because you like coming-of-age stories",
        "Coming of Age",
    ),
    theme(
        "found-family",
        "Because you like found families",
        "Found Family",
    ),
    theme("revenge", "Because you like revenge stories", "Revenge"),
    theme("heist", "Because you like heists", "Heists"),
    theme("time-travel", "Because you like time travel", "Time Travel"),
    theme("dystopia", "Because you like dystopias", "Dystopia"),
    theme("cyberpunk", "Because you like cyberpunk", "Cyberpunk"),
    theme("superhero", "Because you like superheroes", "Superheroes"),
    theme("vampire", "Because you like vampires", "Vampires"),
    theme("zombie", "Because you like zombies", "Zombies"),
    theme("survival", "Because you like survival stories", "Survival"),
    theme("space", "Because you like space adventures", "Space"),
    theme(
        "detective",
        "Because you like detective stories",
        "Detectives",
    ),
    theme(
        "organized-crime",
        "Because you like organized crime",
        "Organized Crime",
    ),
    theme(
        "period-drama",
        "Because you like period dramas",
        "Period Dramas",
    ),
    theme(
        "psychological",
        "Because you like psychological stories",
        "Psychological",
    ),
    theme("mythology", "Because you like mythology", "Mythology"),
    theme("isekai", "Because you like isekai", "Isekai"),
    theme("mecha", "Because you like mecha", "Mecha"),
    theme(
        "martial-arts",
        "Because you like martial arts",
        "Martial Arts",
    ),
    theme("music", "Because you like music stories", "Music"),
    theme(
        "workplace",
        "Because you like workplace stories",
        "Workplace",
    ),
    theme("medical", "Because you like medical dramas", "Medical"),
    theme("military", "Because you like military stories", "Military"),
    theme("spy", "Because you like spy stories", "Spies"),
    theme(
        "royal-politics",
        "Because you like royal intrigue",
        "Royal Intrigue",
    ),
    theme(
        "first-love",
        "Because you like first-love stories",
        "First Love",
    ),
    theme(
        "forbidden-love",
        "Because you like forbidden love",
        "Forbidden Love",
    ),
    theme_with_signal("lgbtq", "lgbt", "Because you like LGBTQ+ stories", "LGBTQ+"),
    theme("samurai", "Because you like samurai", "Samurai"),
    theme(
        "magical-girl",
        "Because you like magical girls",
        "Magical Girls",
    ),
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

    #[test]
    fn browse_nouns_match_the_web_browse_strings() {
        let locale = include_str!("../../../../apps/scryer-web/lib/i18n/locales/en.ts");
        for entry in DISCOVERY_RAIL_LABELS {
            let line = format!(
                "\"discovery.browseBy.{}\": \"{}\",",
                entry.key, entry.browse_noun
            );
            assert!(
                locale.lines().any(|candidate| candidate.trim() == line),
                "en.ts must carry {line}"
            );
        }
    }
}
