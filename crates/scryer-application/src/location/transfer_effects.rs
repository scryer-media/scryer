//! What a cross-library transfer does to a title beyond moving its bytes:
//! the series↔anime facet conversion and the settings it invalidates
//! (FR-057–FR-058), and the dispositions a transfer owes for series-movie
//! links, media kinds, and collections (FR-060–FR-062).
//!
//! # Why the facet conversion is not optional
//!
//! A title's facet and its library's facet are one invariant, not two
//! independent values: every root resolution goes through
//! `AppUseCase::title_root_folder_path`, which refuses a title whose facet does
//! not match its library's
//! (`crates/scryer-application/src/catalog/workflow/roots.rs:194`). So a series
//! transferred into an anime library without converting would not merely read
//! oddly — it would be a catalog state that fails every subsequent path lookup.
//! FR-057's "converts the facet automatically" is the only correct behavior,
//! and the conversion has to land in the same transaction as the library flip.
//!
//! # Facet-sensitivity is a property of the *reader*, not the tag
//!
//! Every reserved `scryer:*` tag survives the transfer — FR-056 preserves the
//! title's own settings, and dropping a value because it is momentarily inert
//! would make a transfer back to the source library lossy. What changes is
//! whether anything reads it. Four of the reserved settings are read inside an
//! `if title.facet == MediaFacet::Anime` branch, so crossing the boundary turns
//! them on or off wholesale:
//!
//! | Setting | Reader gated on anime |
//! |---|---|
//! | `scryer:filler-policy:` | `catalog/workflow/collections.rs:531`, `catalog/workflow/metadata.rs:229` |
//! | `scryer:recap-policy:` | `catalog/workflow/collections.rs:548`, `catalog/workflow/metadata.rs:261` |
//! | `scryer:monitor-specials:` | `catalog/workflow/collections.rs:371` |
//! | `scryer:inter-season-movies:` | `catalog/workflow/collections.rs:389` |
//!
//! Two more change meaning without changing validity.
//! `scryer:monitor-type:` keeps deciding episode monitoring either way, but only
//! on an anime title does it also decide whether linked series movies are
//! monitored (`catalog/workflow/monitoring.rs:55`), so it is a meaning change
//! exactly when the title has links. `scryer:season-folder:` is honored for both
//! episodic facets, but when the title carries no explicit value the default is
//! resolved *per facet*
//! (`catalog/workflow/metadata.rs`, `resolve_use_season_folders`), so the absence
//! of the tag is what the conversion changes.
//!
//! The three metadata-derived anime tags are the one group that is genuinely
//! reset rather than re-interpreted: they are hydrated from anime mappings
//! (`catalog/workflow/hydration.rs:1155`) and are stripped on conversion the way
//! a re-match strips them (`REMATCH_DERIVED_TAG_PREFIXES`,
//! `catalog/workflow/metadata.rs:4`).
//!
//! # Folders, not files (FR-058)
//!
//! The conversion recalculates the *folder* name, because the destination
//! library's folder template is resolved per facet
//! (`AppUseCase::title_folder_template_for`) and the planner already asks for
//! the destination library's facet — which is the post-conversion facet by
//! definition. File names are untouched, and [`FILES_KEEP_THEIR_NAMES`] is the
//! sentence the preview says so with.

use serde::{Deserialize, Serialize};

use scryer_domain::MediaFacet;

/// The reserved tag prefixes whose values are derived from anime metadata and
/// therefore cannot survive a facet conversion.
///
/// The same list a re-match strips (`REMATCH_DERIVED_TAG_PREFIXES` in
/// `crate::catalog::workflow::metadata`). Re-declared here rather than shared
/// because the two are the same set for the same reason, not by coupling: both
/// drop values that were hydrated under an identity that no longer applies.
pub const FACET_DERIVED_TAG_PREFIXES: &[&str] = &[
    "scryer:mal-score:",
    "scryer:anime-media-type:",
    "scryer:anime-status:",
];

/// FR-058's required statement, phrased once so the preview and any later
/// summary cannot word it differently.
pub const FILES_KEEP_THEIR_NAMES: &str =
    "files keep their names; aligning file names with the destination library's policy is a separate rename";

/// What a facet conversion does to one title-level setting (FR-057).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SettingDisposition {
    /// The value stays on the title but nothing reads it under the new facet.
    BecomesInvalid,
    /// The value does not survive the conversion at all.
    Resets,
    /// The value is still read, and decides something different than it did.
    ChangesMeaning,
}

impl SettingDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BecomesInvalid => "becomes_invalid",
            Self::Resets => "resets",
            Self::ChangesMeaning => "changes_meaning",
        }
    }
}

/// One title-level setting the conversion affects, named individually because
/// FR-057 asks for "every setting", not a blanket sentence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConvertedSetting {
    /// Stable machine key, for grouping and translation.
    pub setting: String,
    /// The reserved tag prefix this setting lives under.
    pub tag_prefix: String,
    /// Human-readable name of the setting.
    pub label: String,
    /// The value the title carries today, when it carries one explicitly.
    pub value: Option<String>,
    pub disposition: SettingDisposition,
    /// The sentence the preview shows (C3: never a bare code).
    pub detail: String,
}

/// Title-level associations a transfer has to state a disposition for
/// (FR-060–FR-062). Counts rather than rows: the preview needs to say how many
/// and what happens to them, never to enumerate every episode.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleAssociationFacts {
    /// `series_movie_links` rows whose `series_title_id` is this title.
    pub series_movie_links: i64,
    /// `collections` rows owned by this title (its seasons).
    pub collections: i64,
    /// `episodes` rows owned by this title.
    pub episodes: i64,
}

impl TitleAssociationFacts {
    pub fn new(series_movie_links: i64, collections: i64, episodes: i64) -> Self {
        Self {
            series_movie_links,
            collections,
            episodes,
        }
    }

    pub fn has_links(&self) -> bool {
        self.series_movie_links > 0
    }

    pub fn has_collections(&self) -> bool {
        self.collections > 0
    }
}

/// The facet change a transfer performs, with every setting it touches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FacetConversion {
    pub from: MediaFacet,
    pub to: MediaFacet,
    /// Every affected setting, in a stable order.
    pub settings: Vec<ConvertedSetting>,
    /// Reserved tag prefixes the conversion strips from the title row.
    pub dropped_tag_prefixes: Vec<String>,
}

impl FacetConversion {
    /// The one-line statement of the conversion itself, folder scope included
    /// (FR-057 + FR-058).
    pub fn headline(&self, title_name: &str) -> String {
        format!(
            "\"{title_name}\" converts from {} to {}: its folder name is recalculated from the destination library's {} naming policy, and {FILES_KEEP_THEIR_NAMES}",
            self.from.as_str(),
            self.to.as_str(),
            self.to.as_str(),
        )
    }

    /// Tags the title carries after the conversion. Only the metadata-derived
    /// prefixes go; everything the user set explicitly survives, because a
    /// setting that is merely inert under the new facet is still the user's
    /// (FR-056).
    pub fn converted_tags(&self, tags: &[String]) -> Vec<String> {
        tags.iter()
            .filter(|tag| {
                !self
                    .dropped_tag_prefixes
                    .iter()
                    .any(|prefix| tag.starts_with(prefix.as_str()))
            })
            .cloned()
            .collect()
    }

    pub fn settings_with(&self, disposition: SettingDisposition) -> Vec<&ConvertedSetting> {
        self.settings
            .iter()
            .filter(|setting| setting.disposition == disposition)
            .collect()
    }
}

/// Whether `source` → `destination` is the series↔anime crossover FR-057
/// converts. Movie is not episodic, so it is never one half of a conversion —
/// the classifier has already refused that pairing (FR-017).
pub fn converts_facet(source: &MediaFacet, destination: &MediaFacet) -> bool {
    matches!(
        (source, destination),
        (MediaFacet::Series, MediaFacet::Anime) | (MediaFacet::Anime, MediaFacet::Series)
    )
}

/// One reserved setting's identity, so the table below reads as data.
struct ReservedSetting {
    key: &'static str,
    prefix: &'static str,
    label: &'static str,
}

/// The four settings only anime titles read.
const ANIME_ONLY_SETTINGS: &[ReservedSetting] = &[
    ReservedSetting {
        key: "filler_policy",
        prefix: "scryer:filler-policy:",
        label: "filler handling",
    },
    ReservedSetting {
        key: "recap_policy",
        prefix: "scryer:recap-policy:",
        label: "recap handling",
    },
    ReservedSetting {
        key: "monitor_specials",
        prefix: "scryer:monitor-specials:",
        label: "specials monitoring",
    },
    ReservedSetting {
        key: "inter_season_movies",
        prefix: "scryer:inter-season-movies:",
        label: "inter-season movie inclusion",
    },
];

const MONITOR_TYPE: ReservedSetting = ReservedSetting {
    key: "monitor_type",
    prefix: "scryer:monitor-type:",
    label: "monitoring mode",
};

const SEASON_FOLDER: ReservedSetting = ReservedSetting {
    key: "season_folder",
    prefix: "scryer:season-folder:",
    label: "season-folder layout",
};

const DERIVED_SETTINGS: &[ReservedSetting] = &[
    ReservedSetting {
        key: "mal_score",
        prefix: "scryer:mal-score:",
        label: "MyAnimeList score",
    },
    ReservedSetting {
        key: "anime_media_type",
        prefix: "scryer:anime-media-type:",
        label: "anime media type",
    },
    ReservedSetting {
        key: "anime_status",
        prefix: "scryer:anime-status:",
        label: "anime status",
    },
];

fn tag_value<'a>(tags: &'a [String], prefix: &str) -> Option<&'a str> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Plan the facet conversion for one title, or `None` when the destination
/// library's facet is the title's own.
///
/// Pure: tags and association counts in, dispositions out, so the FR-057
/// enumeration is testable from literals and the preview, the execution plan,
/// and the GraphQL payload all read the same list.
pub fn plan_facet_conversion(
    source: &MediaFacet,
    destination: &MediaFacet,
    tags: &[String],
    associations: TitleAssociationFacts,
) -> Option<FacetConversion> {
    if !converts_facet(source, destination) {
        return None;
    }
    let into_anime = *destination == MediaFacet::Anime;
    let mut settings = Vec::new();

    // The four anime-gated settings. Entering anime they start being read;
    // leaving it they stop. Either way the stored value stays on the title.
    for setting in ANIME_ONLY_SETTINGS {
        let Some(value) = tag_value(tags, setting.prefix) else {
            continue;
        };
        let (disposition, detail) = if into_anime {
            (
                SettingDisposition::ChangesMeaning,
                format!(
                    "{} is set to \"{value}\" but is inert on a series title; after the conversion it takes effect",
                    setting.label
                ),
            )
        } else {
            (
                SettingDisposition::BecomesInvalid,
                format!(
                    "{} (\"{value}\") is only read for anime titles; the value is kept on the title but stops taking effect",
                    setting.label
                ),
            )
        };
        settings.push(ConvertedSetting {
            setting: setting.key.to_string(),
            tag_prefix: setting.prefix.to_string(),
            label: setting.label.to_string(),
            value: Some(value.to_string()),
            disposition,
            detail,
        });
    }

    // FR-060 seam: the monitoring mode keeps deciding episode monitoring, but
    // its authority over linked series movies exists only on an anime title, so
    // it is only a meaning change for a title that actually has links.
    if associations.has_links()
        && let Some(value) = tag_value(tags, MONITOR_TYPE.prefix)
    {
        let detail = if into_anime {
            format!(
                "{} (\"{value}\") starts deciding whether this title's {} linked series {} monitored, in addition to its episodes",
                MONITOR_TYPE.label,
                associations.series_movie_links,
                if associations.series_movie_links == 1 {
                    "movie is"
                } else {
                    "movies are"
                },
            )
        } else {
            format!(
                "{} (\"{value}\") stops deciding whether this title's {} linked series {} monitored; each link keeps the monitored state it has now",
                MONITOR_TYPE.label,
                associations.series_movie_links,
                if associations.series_movie_links == 1 {
                    "movie is"
                } else {
                    "movies are"
                },
            )
        };
        settings.push(ConvertedSetting {
            setting: MONITOR_TYPE.key.to_string(),
            tag_prefix: MONITOR_TYPE.prefix.to_string(),
            label: MONITOR_TYPE.label.to_string(),
            value: Some(value.to_string()),
            disposition: SettingDisposition::ChangesMeaning,
            detail,
        });
    }

    // FR-058's other half: with no explicit override, whether season folders are
    // used is resolved from a per-facet default, so the conversion is what moves
    // it. With an override the title decides either way and nothing changes.
    if tag_value(tags, SEASON_FOLDER.prefix).is_none() {
        settings.push(ConvertedSetting {
            setting: SEASON_FOLDER.key.to_string(),
            tag_prefix: SEASON_FOLDER.prefix.to_string(),
            label: SEASON_FOLDER.label.to_string(),
            value: None,
            disposition: SettingDisposition::ChangesMeaning,
            detail: format!(
                "{} is not set on this title, so it follows the {} default instead of the {} one after the conversion; folders already on disk are not renamed by this operation",
                SEASON_FOLDER.label,
                destination.as_str(),
                source.as_str(),
            ),
        });
    }

    // Metadata-derived anime values: hydrated under the old facet's identity,
    // so they go the way a re-match drops them.
    for setting in DERIVED_SETTINGS {
        let Some(value) = tag_value(tags, setting.prefix) else {
            continue;
        };
        settings.push(ConvertedSetting {
            setting: setting.key.to_string(),
            tag_prefix: setting.prefix.to_string(),
            label: setting.label.to_string(),
            value: Some(value.to_string()),
            disposition: SettingDisposition::Resets,
            detail: format!(
                "{} (\"{value}\") is derived from anime metadata and is removed by the conversion; it is repopulated only if the destination facet's metadata provides it",
                setting.label
            ),
        });
    }

    Some(FacetConversion {
        from: source.clone(),
        to: destination.clone(),
        settings,
        dropped_tag_prefixes: FACET_DERIVED_TAG_PREFIXES
            .iter()
            .map(|prefix| (*prefix).to_string())
            .collect(),
    })
}

/// FR-060: what happens to this title's series-movie links, or `None` when it
/// has none.
///
/// The disposition is **move together**, and it is structural rather than a
/// choice: `series_movie_links.series_title_id` references the title id, which a
/// transfer does not change, and the other end (`movie_entity_id`) points at
/// `movie_entities` — shared metadata with no library or root column of its own
/// (`crates/scryer/src/db/migrations/0130_series_movie_links.sql:1`). Nothing in
/// the link row is library-scoped, so there is nothing to orphan and nothing to
/// rewrite. The preview says so anyway, because "we checked and nothing happens"
/// is the statement FR-060 is asking for.
pub fn series_movie_link_statement(
    title_name: &str,
    associations: TitleAssociationFacts,
    destination_library_id: &str,
) -> Option<String> {
    if !associations.has_links() {
        return None;
    }
    let count = associations.series_movie_links;
    Some(format!(
        "\"{title_name}\" carries {count} series-movie {}; {} into {destination_library_id} with the series and {} linked movie metadata, which no library owns",
        if count == 1 { "link" } else { "links" },
        if count == 1 { "it moves" } else { "they move" },
        if count == 1 { "keeps its" } else { "keep their" },
    ))
}

/// FR-062: the collection consequence of a transfer, or `None` when there is
/// none worth a line.
///
/// A transfer repoints the title row; `collections.title_id` and
/// `episodes.collection_id` are keyed on ids the transfer does not reissue
/// (`crates/scryer/src/db/migrations/0004_catalog_core.sql:36`), so membership
/// is preserved with no remap at all. That is not worth saying on every
/// transfer. It *is* worth saying when the facet also converts, because the
/// settings that decide how seasons and specials are treated change with it —
/// which is exactly the "cross-library collection consequence" FR-062 wants
/// noted.
pub fn collection_statement(
    title_name: &str,
    associations: TitleAssociationFacts,
    conversion: Option<&FacetConversion>,
) -> Option<String> {
    let conversion = conversion?;
    if !associations.has_collections() {
        return None;
    }
    Some(format!(
        "\"{title_name}\" keeps all {} of its seasons and {} of its episodes with the same season membership; the {}→{} conversion changes how seasons and specials are treated, not which episodes belong to them",
        associations.collections,
        associations.episodes,
        conversion.from.as_str(),
        conversion.to.as_str(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn settings_named<'a>(conversion: &'a FacetConversion, key: &str) -> Option<&'a ConvertedSetting> {
        conversion
            .settings
            .iter()
            .find(|setting| setting.setting == key)
    }

    #[test]
    fn a_same_facet_destination_converts_nothing() {
        assert!(
            plan_facet_conversion(
                &MediaFacet::Series,
                &MediaFacet::Series,
                &tags(&["scryer:filler-policy:skip_filler"]),
                TitleAssociationFacts::default(),
            )
            .is_none()
        );
    }

    #[test]
    fn the_movie_boundary_is_never_a_conversion() {
        // FR-017 refuses these before the planner sees them; this asserts the
        // conversion planner does not quietly invent one behind that refusal.
        for (source, destination) in [
            (MediaFacet::Movie, MediaFacet::Series),
            (MediaFacet::Series, MediaFacet::Movie),
            (MediaFacet::Movie, MediaFacet::Anime),
            (MediaFacet::Anime, MediaFacet::Movie),
        ] {
            assert!(!converts_facet(&source, &destination));
            assert!(
                plan_facet_conversion(
                    &source,
                    &destination,
                    &tags(&[]),
                    TitleAssociationFacts::default()
                )
                .is_none()
            );
        }
    }

    #[test]
    fn leaving_anime_invalidates_every_anime_gated_setting() {
        let conversion = plan_facet_conversion(
            &MediaFacet::Anime,
            &MediaFacet::Series,
            &tags(&[
                "scryer:filler-policy:skip_filler",
                "scryer:recap-policy:skip_recap",
                "scryer:monitor-specials:true",
                "scryer:inter-season-movies:false",
                "scryer:season-folder:enabled",
                "favourites",
            ]),
            TitleAssociationFacts::default(),
        )
        .expect("anime → series converts");

        for key in [
            "filler_policy",
            "recap_policy",
            "monitor_specials",
            "inter_season_movies",
        ] {
            let setting = settings_named(&conversion, key)
                .unwrap_or_else(|| panic!("{key} is enumerated"));
            assert_eq!(
                setting.disposition,
                SettingDisposition::BecomesInvalid,
                "{key} stops being read outside anime"
            );
            assert!(
                setting.detail.contains("anime"),
                "{key} says why: {}",
                setting.detail
            );
        }
        // An explicit season-folder override is honoured on both episodic
        // facets, so it is not a consequence of the conversion.
        assert!(settings_named(&conversion, "season_folder").is_none());
    }

    #[test]
    fn entering_anime_turns_the_same_settings_on() {
        let conversion = plan_facet_conversion(
            &MediaFacet::Series,
            &MediaFacet::Anime,
            &tags(&[
                "scryer:filler-policy:skip_filler",
                "scryer:monitor-specials:true",
                "scryer:season-folder:disabled",
            ]),
            TitleAssociationFacts::default(),
        )
        .expect("series → anime converts");

        for key in ["filler_policy", "monitor_specials"] {
            let setting = settings_named(&conversion, key)
                .unwrap_or_else(|| panic!("{key} is enumerated"));
            assert_eq!(setting.disposition, SettingDisposition::ChangesMeaning);
            assert!(
                setting.detail.contains("takes effect"),
                "{key}: {}",
                setting.detail
            );
        }
        assert!(settings_named(&conversion, "recap_policy").is_none());
    }

    #[test]
    fn an_absent_season_folder_override_follows_the_new_facet_default() {
        let conversion = plan_facet_conversion(
            &MediaFacet::Series,
            &MediaFacet::Anime,
            &tags(&[]),
            TitleAssociationFacts::default(),
        )
        .expect("series → anime converts");
        let setting = settings_named(&conversion, "season_folder").expect("enumerated");
        assert_eq!(setting.disposition, SettingDisposition::ChangesMeaning);
        assert_eq!(setting.value, None);
        assert!(setting.detail.contains("anime"), "{}", setting.detail);
    }

    #[test]
    fn monitoring_mode_is_a_meaning_change_only_for_a_title_with_links() {
        let with_links = plan_facet_conversion(
            &MediaFacet::Anime,
            &MediaFacet::Series,
            &tags(&["scryer:monitor-type:allepisodes"]),
            TitleAssociationFacts::new(2, 0, 0),
        )
        .expect("converts");
        let setting = settings_named(&with_links, "monitor_type").expect("enumerated");
        assert_eq!(setting.disposition, SettingDisposition::ChangesMeaning);
        assert!(setting.detail.contains("2 linked"), "{}", setting.detail);

        let without_links = plan_facet_conversion(
            &MediaFacet::Anime,
            &MediaFacet::Series,
            &tags(&["scryer:monitor-type:allepisodes"]),
            TitleAssociationFacts::default(),
        )
        .expect("converts");
        assert!(
            settings_named(&without_links, "monitor_type").is_none(),
            "no links, no meaning change, no noise"
        );
    }

    #[test]
    fn metadata_derived_anime_tags_reset_and_are_stripped() {
        let source_tags = tags(&[
            "scryer:mal-score:8.4",
            "scryer:anime-media-type:tv",
            "scryer:anime-status:finished",
            "scryer:quality-profile:1080p",
            "favourites",
        ]);
        let conversion = plan_facet_conversion(
            &MediaFacet::Anime,
            &MediaFacet::Series,
            &source_tags,
            TitleAssociationFacts::default(),
        )
        .expect("converts");

        for key in ["mal_score", "anime_media_type", "anime_status"] {
            let setting = settings_named(&conversion, key)
                .unwrap_or_else(|| panic!("{key} is enumerated"));
            assert_eq!(setting.disposition, SettingDisposition::Resets);
        }

        let converted = conversion.converted_tags(&source_tags);
        assert_eq!(
            converted,
            tags(&["scryer:quality-profile:1080p", "favourites"]),
            "derived anime values go; everything the user set stays"
        );
    }

    #[test]
    fn the_headline_states_the_conversion_and_that_files_keep_their_names() {
        let conversion = plan_facet_conversion(
            &MediaFacet::Series,
            &MediaFacet::Anime,
            &tags(&[]),
            TitleAssociationFacts::default(),
        )
        .expect("converts");
        let headline = conversion.headline("Fullmetal Alchemist");
        assert!(headline.contains("series"));
        assert!(headline.contains("anime"));
        assert!(headline.contains("folder name is recalculated"));
        assert!(
            headline.contains(FILES_KEEP_THEIR_NAMES),
            "FR-058's statement is not optional: {headline}"
        );
    }

    #[test]
    fn a_title_with_no_reserved_settings_still_converts() {
        let conversion = plan_facet_conversion(
            &MediaFacet::Anime,
            &MediaFacet::Series,
            &tags(&["scryer:season-folder:enabled"]),
            TitleAssociationFacts::default(),
        )
        .expect("converts");
        assert!(
            conversion.settings.is_empty(),
            "nothing invented: {:?}",
            conversion.settings
        );
        assert_eq!(conversion.from, MediaFacet::Anime);
        assert_eq!(conversion.to, MediaFacet::Series);
    }

    #[test]
    fn series_movie_links_are_stated_only_when_the_title_has_them() {
        assert!(
            series_movie_link_statement("Show", TitleAssociationFacts::default(), "anime-library")
                .is_none()
        );
        let statement =
            series_movie_link_statement("Show", TitleAssociationFacts::new(1, 0, 0), "anime-library")
                .expect("stated");
        assert!(statement.contains("1 series-movie link"));
        assert!(statement.contains("anime-library"));
        assert!(statement.contains("it moves"));

        let plural =
            series_movie_link_statement("Show", TitleAssociationFacts::new(3, 0, 0), "anime-library")
                .expect("stated");
        assert!(plural.contains("3 series-movie links"));
        assert!(plural.contains("they move"));
    }

    #[test]
    fn collections_are_noted_only_when_the_facet_converts() {
        let conversion = plan_facet_conversion(
            &MediaFacet::Series,
            &MediaFacet::Anime,
            &tags(&[]),
            TitleAssociationFacts::default(),
        )
        .expect("converts");
        let associations = TitleAssociationFacts::new(0, 4, 62);

        assert!(
            collection_statement("Show", associations, None).is_none(),
            "a same-facet transfer changes nothing about collections, so it says nothing"
        );
        assert!(
            collection_statement("Show", TitleAssociationFacts::default(), Some(&conversion))
                .is_none(),
            "a title with no seasons has no collection consequence"
        );
        let statement =
            collection_statement("Show", associations, Some(&conversion)).expect("noted");
        assert!(statement.contains("4 of its seasons"));
        assert!(statement.contains("62 of its episodes"));
    }
}
