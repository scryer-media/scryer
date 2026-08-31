//! Indexer-asserted category as an identity veto (plan 136 §6, Pillar D).
//!
//! An indexer that files a release under `TV > Anime` has made an explicit,
//! machine-readable identity assertion. Category is only *indexer* evidence
//! (same trust tier as response ids), so it is enforced with an **asymmetric**
//! contradiction rule: a veto fires only when the asserted category is more
//! specific than the subject, never the other way around.
//!
//! This module owns the coarse mapping and the contradiction rule for both
//! insertion points: the pre-submission NZB metadata gate (D1, live) and the
//! newznab/torznab response-attribute lane (D2, later).

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use scryer_domain::MediaFacet;

use crate::{AppError, AppResult};

/// Failure code carried by the release attempt recorded when a submission is
/// vetoed, so operators can see why the release was burned.
pub const CATEGORY_MISMATCH_CODE: &str = "category_mismatch";

/// Upper bound on the NZB prefix inspected for `<head>` metadata. The head
/// precedes the first `<file>` element, so a bounded prefix is enough and an
/// oversized or headless payload never costs more than this.
pub const NZB_HEAD_PROBE_BYTES: usize = 32 * 1024;

/// Coarse family that an indexer category name or newznab id maps onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexerCategoryFamily {
    Anime,
    Movies,
    Tv,
}

impl IndexerCategoryFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anime => "anime",
            Self::Movies => "movies",
            Self::Tv => "tv",
        }
    }
}

/// Map ONE category label (`"TV > Anime"`, `"Movies/HD"`, `"5070"`) onto a
/// coarse family. `>` and `/` are HIERARCHY separators within a single label;
/// comma/pipe LIST separators are handled a level up (`indexer_category_label_families`)
/// because `5000,5070` is two independent assertions, not one path. Unknown or
/// unmappable input yields `None`, which always allows the release through.
pub fn indexer_category_family(raw: &str) -> Option<IndexerCategoryFamily> {
    let mut family: Option<IndexerCategoryFamily> = None;

    for segment in raw.split(['>', '/']) {
        let Some(segment_family) = category_segment_family(segment) else {
            continue;
        };
        // An `anime` segment anywhere is the most specific assertion an
        // indexer can make ("TV > Anime" is anime). For everything else the
        // FIRST mapped segment — the top-level category — wins: newznab paths
        // are parent-first, and sub-labels reuse family words in ways that
        // must not invert the parent ("Movies > HDTV" is a movie category,
        // "TV > TV Movies" is a tv category).
        if segment_family == IndexerCategoryFamily::Anime {
            return Some(IndexerCategoryFamily::Anime);
        }
        if family.is_none() {
            family = Some(segment_family);
        }
    }

    family
}

/// Split a raw category value into independent labels (`,`/`|` are list
/// separators) and map each. `"5000,5070"` yields `[Tv, Anime]` — two
/// assertions the set rule weighs independently, exactly as the same pair
/// arriving as separate response attrs would be.
pub fn indexer_category_label_families(raw: &str) -> Vec<IndexerCategoryFamily> {
    raw.split([',', '|'])
        .filter_map(indexer_category_family)
        .collect()
}

/// The asymmetric contradiction rule from plan 136 §6.
///
/// * anime category + series subject → veto (anime is strictly more specific
///   than a plain series subject)
/// * anime category + movie subject → allow (anime FILMS are legitimately
///   filed under 5070/anime; Scryer itself searches anime categories for
///   them, so anime-vs-movie is not a contradiction)
/// * movies category + episodic subject (series/anime) → veto
/// * tv category + movie subject → veto
/// * generic tv category + anime subject → allow (indexers routinely file
///   anime under plain TV; absence of specificity is not evidence)
/// * anime category + anime subject, movies + movie → allow
pub fn indexer_category_contradicts_facet(
    family: IndexerCategoryFamily,
    facet: &MediaFacet,
) -> bool {
    match (family, facet) {
        (IndexerCategoryFamily::Anime, MediaFacet::Series) => true,
        (IndexerCategoryFamily::Anime, MediaFacet::Anime | MediaFacet::Movie) => false,
        (IndexerCategoryFamily::Movies, MediaFacet::Series | MediaFacet::Anime) => true,
        (IndexerCategoryFamily::Movies, MediaFacet::Movie) => false,
        (IndexerCategoryFamily::Tv, MediaFacet::Movie) => true,
        (IndexerCategoryFamily::Tv, MediaFacet::Series | MediaFacet::Anime) => false,
    }
}

/// Set form of the contradiction rule for the response-attribute lane (D2).
///
/// A newznab/torznab item routinely carries several categories, and each one is
/// an independent assertion. The veto therefore fires only when the indexer said
/// something mappable *and* every mappable thing it said contradicts the
/// subject: a dual-categorized `["5000", "5070"]` item still claims plain TV, so
/// it passes for a series subject, while a `["5070"]`-only item does not. An
/// empty or wholly unmappable set stays permissive, as on the D1 lane.
pub fn indexer_categories_contradict_facet<I, S>(categories: I, facet: &MediaFacet) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut saw_mapped_category = false;

    for category in categories {
        for family in indexer_category_label_families(category.as_ref()) {
            if !indexer_category_contradicts_facet(family, facet) {
                return false;
            }
            saw_mapped_category = true;
        }
    }

    saw_mapped_category
}

/// Read `<head><meta type="category">…</meta></head>` out of an NZB prefix.
///
/// The input may be truncated at any point (it is a bounded probe of a
/// streaming download); parsing stops at the first problem and reports whatever
/// was already recovered. XML entities are resolved, so `TV &gt; Anime` is
/// returned as `TV > Anime`.
pub fn nzb_head_category(nzb_head_bytes: &[u8]) -> Option<String> {
    let head_text = String::from_utf8_lossy(nzb_head_bytes);
    let mut reader = Reader::from_str(&head_text);
    let mut in_head = false;
    let mut capturing = false;
    let mut value = String::new();
    let mut category: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref start)) => {
                match local_name_lowercase(start.name().as_ref()).as_str() {
                    "head" => in_head = true,
                    "meta" if in_head => {
                        capturing = meta_declares_category(start);
                        value.clear();
                    }
                    // The head always precedes the first file segment; anything
                    // else at this point means there is nothing left to find.
                    "file" if !in_head => break,
                    _ => {}
                }
            }
            Ok(Event::Text(ref text)) if capturing => {
                if let Ok(decoded) = quick_xml::escape::unescape(text.as_ref()) {
                    value.push_str(&decoded);
                }
            }
            // CDATA carries the literal category text with no entity escaping;
            // skipping it would let `<![CDATA[TV > Anime]]>` dodge the gate.
            Ok(Event::CData(ref cdata)) if capturing => {
                value.push_str(cdata.as_ref());
            }
            Ok(Event::GeneralRef(ref reference)) if capturing => {
                if let Ok(Some(character)) = reference.resolve_char_ref() {
                    value.push(character);
                } else if let Some(entity) =
                    quick_xml::escape::resolve_predefined_entity(reference.as_ref())
                {
                    value.push_str(entity);
                }
            }
            Ok(Event::End(ref end)) => match local_name_lowercase(end.name().as_ref()).as_str() {
                "meta" if capturing => {
                    let trimmed = value.trim();
                    if category.is_none() && !trimmed.is_empty() {
                        category = Some(trimmed.to_string());
                    }
                    capturing = false;
                    value.clear();
                }
                "head" => break,
                _ => {}
            },
            Ok(Event::Eof) => break,
            // A truncated or malformed probe is not evidence of anything.
            Err(_) => break,
            _ => {}
        }
    }

    category
}

/// Pre-submission gate (D1): reject an NZB whose indexer-asserted category
/// contradicts the subject facet, before the payload reaches a download client.
///
/// Permissive by construction — an absent, unmappable, or unparseable category
/// always passes.
pub fn enforce_nzb_category_gate(nzb_head_bytes: &[u8], facet: &MediaFacet) -> AppResult<()> {
    let Some(category) = nzb_head_category(nzb_head_bytes) else {
        return Ok(());
    };
    // Same set rule as the response-attribute lane: a comma/pipe list is a set
    // of independent assertions, and one compatible member allows the release.
    // `5000,5070` therefore passes a series subject on BOTH lanes, while a
    // hierarchical `TV > Anime` still vetoes it.
    if !indexer_categories_contradict_facet([category.as_str()], facet) {
        return Ok(());
    }

    Err(AppError::Validation(format!(
        "{CATEGORY_MISMATCH_CODE}: indexer category '{category}' contradicts the release's {} \
         subject; the NZB was not handed to the download client",
        facet.as_str()
    )))
}

fn category_segment_family(segment: &str) -> Option<IndexerCategoryFamily> {
    let segment = segment.trim();
    if segment.is_empty() {
        return None;
    }
    if let Ok(newznab_id) = segment.parse::<u32>() {
        return newznab_id_family(newznab_id);
    }

    let lowered = segment.to_ascii_lowercase();
    if lowered.contains("anime") {
        Some(IndexerCategoryFamily::Anime)
    } else if lowered.contains("movie") {
        Some(IndexerCategoryFamily::Movies)
    } else if lowered.contains("series") || lowered.contains("tv") {
        Some(IndexerCategoryFamily::Tv)
    } else {
        None
    }
}

fn newznab_id_family(newznab_id: u32) -> Option<IndexerCategoryFamily> {
    match newznab_id {
        5070 => Some(IndexerCategoryFamily::Anime),
        2000..=2999 => Some(IndexerCategoryFamily::Movies),
        5000..=5999 => Some(IndexerCategoryFamily::Tv),
        _ => None,
    }
}

fn meta_declares_category(start: &BytesStart<'_>) -> bool {
    start
        .attributes()
        .filter_map(|attribute| attribute.ok())
        .any(|attribute| {
            local_name_lowercase(attribute.key.as_ref()) == "type"
                && attribute.value.trim().eq_ignore_ascii_case("category")
        })
}

fn local_name_lowercase(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexer_category_family_maps_names_and_ids() {
        assert_eq!(
            indexer_category_family("TV > Anime"),
            Some(IndexerCategoryFamily::Anime)
        );
        assert_eq!(
            indexer_category_family("Movies > HD"),
            Some(IndexerCategoryFamily::Movies)
        );
        assert_eq!(
            indexer_category_family("TV"),
            Some(IndexerCategoryFamily::Tv)
        );
        assert_eq!(
            indexer_category_family("TV > Web-DL"),
            Some(IndexerCategoryFamily::Tv)
        );
        assert_eq!(
            indexer_category_family("Series"),
            Some(IndexerCategoryFamily::Tv)
        );
        assert_eq!(
            indexer_category_family("5070"),
            Some(IndexerCategoryFamily::Anime)
        );
        assert_eq!(
            indexer_category_family("5040"),
            Some(IndexerCategoryFamily::Tv)
        );
        assert_eq!(
            indexer_category_family("2040"),
            Some(IndexerCategoryFamily::Movies)
        );
        // Comma lists are NOT one hierarchical label: the single-label mapper
        // ignores them, and the label splitter yields independent assertions.
        assert_eq!(indexer_category_family("5000,5070"), None);
        assert_eq!(
            indexer_category_label_families("5000,5070"),
            vec![IndexerCategoryFamily::Tv, IndexerCategoryFamily::Anime]
        );
        assert!(
            !indexer_categories_contradict_facet(["5000,5070"], &MediaFacet::Series),
            "a dual-categorized list still claims plain TV, so a series subject passes on both lanes"
        );
        assert_eq!(
            indexer_category_family("Anime > TV"),
            Some(IndexerCategoryFamily::Anime),
            "an anime segment is never demoted by a shallower generic segment"
        );
    }

    #[test]
    fn indexer_category_family_is_unknown_for_garbage_or_absent_values() {
        assert_eq!(indexer_category_family(""), None);
        assert_eq!(indexer_category_family("   "), None);
        assert_eq!(indexer_category_family("Books > EBook"), None);
        assert_eq!(indexer_category_family("9999"), None);
        assert_eq!(indexer_category_family("¯\\_(ツ)_/¯"), None);
    }

    #[test]
    fn anime_category_vetoes_a_series_subject() {
        assert!(indexer_category_contradicts_facet(
            IndexerCategoryFamily::Anime,
            &MediaFacet::Series
        ));
    }

    #[test]
    fn movies_category_vetoes_an_episodic_subject() {
        assert!(indexer_category_contradicts_facet(
            IndexerCategoryFamily::Movies,
            &MediaFacet::Series
        ));
        assert!(indexer_category_contradicts_facet(
            IndexerCategoryFamily::Movies,
            &MediaFacet::Anime
        ));
    }

    #[test]
    fn tv_category_vetoes_a_movie_subject() {
        assert!(indexer_category_contradicts_facet(
            IndexerCategoryFamily::Tv,
            &MediaFacet::Movie
        ));
    }

    #[test]
    fn generic_tv_category_allows_an_anime_subject() {
        assert!(!indexer_category_contradicts_facet(
            IndexerCategoryFamily::Tv,
            &MediaFacet::Anime
        ));
        assert!(!indexer_category_contradicts_facet(
            IndexerCategoryFamily::Tv,
            &MediaFacet::Series
        ));
    }

    #[test]
    fn matching_categories_allow_their_own_subject() {
        assert!(!indexer_category_contradicts_facet(
            IndexerCategoryFamily::Anime,
            &MediaFacet::Anime
        ));
        assert!(!indexer_category_contradicts_facet(
            IndexerCategoryFamily::Movies,
            &MediaFacet::Movie
        ));
    }

    #[test]
    fn dual_categorized_set_allows_the_subject_its_generic_category_names() {
        assert!(!indexer_categories_contradict_facet(
            ["5000", "5070"],
            &MediaFacet::Series
        ));
        assert!(!indexer_categories_contradict_facet(
            ["TV", "TV > Anime"],
            &MediaFacet::Series
        ));
    }

    #[test]
    fn anime_only_set_vetoes_a_series_subject() {
        assert!(indexer_categories_contradict_facet(
            ["5070"],
            &MediaFacet::Series
        ));
        assert!(indexer_categories_contradict_facet(
            ["5070", "TV > Anime"],
            &MediaFacet::Series
        ));
    }

    #[test]
    fn movies_only_set_vetoes_an_episodic_subject() {
        assert!(indexer_categories_contradict_facet(
            ["2000", "2040"],
            &MediaFacet::Series
        ));
        assert!(indexer_categories_contradict_facet(
            ["Movies > HD"],
            &MediaFacet::Anime
        ));
    }

    #[test]
    fn generic_tv_set_allows_an_anime_subject() {
        assert!(!indexer_categories_contradict_facet(
            ["5000", "5040"],
            &MediaFacet::Anime
        ));
    }

    #[test]
    fn empty_or_unmappable_sets_allow_every_subject() {
        assert!(!indexer_categories_contradict_facet(
            Vec::<String>::new(),
            &MediaFacet::Series
        ));
        assert!(!indexer_categories_contradict_facet(
            ["9999", "Books > EBook", "   "],
            &MediaFacet::Series
        ));
        assert!(!indexer_categories_contradict_facet(
            ["9999", "5070"],
            &MediaFacet::Anime
        ));
    }

    fn nzb_head_with_category(category: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="iso-8859-1" ?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
<head>
 <meta type="name">Tide.Chart.S02.DANiSH.JAPANESE.1080p.WEB.H264</meta>
 <meta type="title">Tide.Chart.S02.DANiSH.JAPANESE.1080p.WEB.H264</meta>
 <meta type="category">{category}</meta>
 <meta type="size">12345678</meta>
</head>
<file poster="poster@example.invalid" date="1700000000" subject="[1/2] - &quot;x.par2&quot;">
</file>
</nzb>"#
        )
    }

    #[test]
    fn nzb_head_category_decodes_xml_entities() {
        assert_eq!(
            nzb_head_category(nzb_head_with_category("TV &gt; Anime").as_bytes()),
            Some("TV > Anime".to_string())
        );
    }

    #[test]
    fn nzb_head_category_is_absent_without_a_category_meta() {
        let nzb = r#"<?xml version="1.0"?>
<nzb><head><meta type="name">Some.Release</meta></head><file></file></nzb>"#;

        assert_eq!(nzb_head_category(nzb.as_bytes()), None);
    }

    #[test]
    fn nzb_head_category_tolerates_a_truncated_probe() {
        let nzb = nzb_head_with_category("TV &gt; Anime");
        let truncated = &nzb.as_bytes()[..nzb.find("<meta type=\"size\"").unwrap()];

        assert_eq!(
            nzb_head_category(truncated),
            Some("TV > Anime".to_string()),
            "a probe cut after the category meta still resolves the category"
        );
    }

    #[test]
    fn nzb_head_category_is_absent_for_a_malformed_head() {
        let nzb = br#"<?xml version="1.0"?><nzb><head><meta type="category">TV > Anime"#;

        assert_eq!(nzb_head_category(nzb), None);
    }

    #[test]
    fn enforce_nzb_category_gate_blocks_an_anime_nzb_for_a_series_subject() {
        let nzb = nzb_head_with_category("TV &gt; Anime");

        let error = enforce_nzb_category_gate(nzb.as_bytes(), &MediaFacet::Series)
            .expect_err("an anime nzb must not be submitted for a series subject");

        assert!(
            error.to_string().contains(CATEGORY_MISMATCH_CODE),
            "gate error must carry the category_mismatch code: {error}"
        );
        assert!(error.to_string().contains("TV > Anime"), "{error}");
    }

    #[test]
    fn enforce_nzb_category_gate_allows_an_anime_nzb_for_an_anime_subject() {
        let nzb = nzb_head_with_category("TV &gt; Anime");

        enforce_nzb_category_gate(nzb.as_bytes(), &MediaFacet::Anime)
            .expect("an anime nzb is exactly right for an anime subject");
    }

    #[test]
    fn enforce_nzb_category_gate_allows_unknown_and_absent_categories() {
        let unknown = nzb_head_with_category("Books &gt; EBook");
        enforce_nzb_category_gate(unknown.as_bytes(), &MediaFacet::Series)
            .expect("an unmappable category is permissive");

        let absent = r#"<?xml version="1.0"?><nzb><head></head><file></file></nzb>"#;
        enforce_nzb_category_gate(absent.as_bytes(), &MediaFacet::Series)
            .expect("an absent category is permissive");

        enforce_nzb_category_gate(b"not xml at all", &MediaFacet::Series)
            .expect("a malformed payload is permissive");
    }

    #[test]
    fn category_mapping_prefers_parent_segment_except_anime() {
        assert_eq!(
            indexer_category_family("Movies > HDTV"),
            Some(IndexerCategoryFamily::Movies)
        );
        assert_eq!(
            indexer_category_family("TV > TV Movies"),
            Some(IndexerCategoryFamily::Tv)
        );
        assert_eq!(
            indexer_category_family("TV > Anime"),
            Some(IndexerCategoryFamily::Anime)
        );
    }

    #[test]
    fn anime_category_does_not_contradict_a_movie_subject() {
        assert!(!indexer_category_contradicts_facet(
            IndexerCategoryFamily::Anime,
            &MediaFacet::Movie
        ));
        assert!(indexer_category_contradicts_facet(
            IndexerCategoryFamily::Anime,
            &MediaFacet::Series
        ));
    }

    #[test]
    fn nzb_head_category_reads_cdata_wrapped_values() {
        let nzb = br#"<?xml version="1.0"?><nzb><head><meta type="category"><![CDATA[TV > Anime]]></meta></head><file/></nzb>"#;
        assert_eq!(nzb_head_category(nzb).as_deref(), Some("TV > Anime"));
    }
}
