//! Numbering-translation rules, against a synthetic four-cour anime.
//!
//! The fixture layout is the shape this feature exists for: TVDB carries one
//! official season of 60 episodes, while the community carries four seasons of
//! 14 / 12 / 10 / 24. Every show, season and group name here is invented.

use super::super::coverage::{
    ReleaseCoverage, parsed_numbering_contradicts_episode,
    parsed_release_contradicts_requested_episode, resolve_release_coverage,
};
use super::*;
use chrono::NaiveDate;
use scryer_domain::{
    AnimeCommunitySeason, AnimeCommunitySeasonRange, AnimeNumberingBridge, Episode, EpisodeType,
    MediaFacet, TaggedAlias, Title,
};

const SERIES_NAME: &str = "Lantern Verge";
const COUR_TITLES: [&str; 4] = [
    "Lantern Verge",
    "Lantern Verge: Ember Circuit",
    "Lantern Verge: Glass Meridian",
    "Lantern Verge: Final Chorus",
];
const COUR_LENGTHS: [i32; 4] = [14, 12, 10, 24];

fn title(name: &str) -> Title {
    Title {
        id: "title-1".to_string(),
        name: name.to_string(),
        facet: MediaFacet::Anime,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Anime),
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
        monitored: true,
        tags: Vec::new(),
        canonical_tags: Vec::new(),
        external_ids: Vec::new(),
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2025),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: Vec::new(),
        tagged_aliases: Vec::new(),
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn episode(id: &str, season: u32, number: u32, absolute: Option<u32>, aired: &str) -> Episode {
    Episode {
        id: id.to_string(),
        title_id: "title-1".to_string(),
        collection_id: Some(format!("season-{season}")),
        episode_type: EpisodeType::Standard,
        episode_number: Some(number.to_string()),
        season_number: Some(season.to_string()),
        episode_label: None,
        title: None,
        air_date: (!aired.is_empty()).then(|| aired.to_string()),
        duration_seconds: Some(1_440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: absolute.map(|value| value.to_string()),
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    }
}

/// TVDB official order: one season, 60 episodes, weekly from a fixed start,
/// absolute numbers present.
fn official_episodes(with_absolute: bool) -> Vec<Episode> {
    let start = NaiveDate::from_ymd_opt(2025, 7, 6).expect("valid date");
    (1..=60u32)
        .map(|number| {
            let aired = start + chrono::Duration::days(i64::from(number - 1) * 7);
            episode(
                &format!("ep-{number}"),
                1,
                number,
                with_absolute.then_some(number),
                &aired.format("%Y-%m-%d").to_string(),
            )
        })
        .collect()
}

/// Community layout: 1-14, 15-26, 27-36, 37-60 of TVDB season 1.
fn bridge() -> AnimeNumberingBridge {
    let mut seasons = Vec::new();
    let mut tvdb_start = 1;
    for (offset, length) in COUR_LENGTHS.iter().enumerate() {
        let index = i32::try_from(offset).expect("small index") + 1;
        seasons.push(AnimeCommunitySeason {
            index,
            anidb_id: Some(90_000 + i64::from(index)),
            anilist_id: None,
            mal_id: None,
            titles: vec![COUR_TITLES[offset].to_string()],
            ranges: vec![AnimeCommunitySeasonRange {
                community_episode_start: 1,
                community_episode_end: Some(*length),
                tvdb_season: 1,
                tvdb_episode_start: tvdb_start,
                tvdb_episode_end: Some(tvdb_start + length - 1),
            }],
            absolute_start: Some(tvdb_start),
            episode_count: Some(*length),
        });
        tvdb_start += length;
    }
    AnimeNumberingBridge {
        generated_on: "2026-08-30".to_string(),
        corroborating_order: Some("dvd".to_string()),
        seasons,
    }
}

fn parsed(season: Option<u32>, episodes: &[u32]) -> ParsedEpisodeMetadata {
    ParsedEpisodeMetadata {
        season,
        episode_numbers: episodes.to_vec(),
        ..Default::default()
    }
}

/// A release that carries only an absolute number — no season, no episode
/// token — which is how a great deal of anime is posted.
fn absolute_only_parse(absolute: u32) -> ParsedEpisodeMetadata {
    ParsedEpisodeMetadata {
        absolute_episode: Some(absolute),
        absolute_episode_numbers: vec![absolute],
        ..Default::default()
    }
}

fn resolve(
    bridge: &AnimeNumberingBridge,
    title: &Title,
    episodes: &[Episode],
    parsed: &ParsedEpisodeMetadata,
    variants: &[String],
    reference_date: Option<NaiveDate>,
) -> NumberingResolution {
    resolve_numbering(&NumberingInput {
        bridge,
        title,
        episodes,
        parsed,
        parsed_title_variants: variants,
        reference_date,
    })
}

// ── community numbering ───────────────────────────────────────────────────

/// The whole point: `S04E20` is community season 4 episode 20, which TVDB
/// records as S01E56. Official order has no season 4, so nothing competes.
#[test]
fn a_community_season_token_maps_onto_the_official_order() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(4), &[20]),
        &[],
        None,
    );

    let candidate = resolution.resolved().expect("community candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Community);
    assert_eq!(candidate.season, 1);
    assert_eq!(candidate.episode_numbers, vec![56]);
    assert_eq!(candidate.episode_ids, vec!["ep-56".to_string()]);
    assert!(candidate.covers_episode_id("ep-56"));
}

#[test]
fn translated_parser_metadata_resolves_coverage_and_vetoes_against_official_coordinates() {
    let series = title(SERIES_NAME);
    let episodes = official_episodes(true);
    let mut release = crate::parse_release_metadata("Lantern.Verge.S04E20.1080p.WEB-DL-GROUP");
    let raw_title = release.raw_title.clone();
    let original = release.episode.as_ref().expect("parser-produced episode");
    let raw_episode = original.raw.clone();
    assert_eq!(original.season, Some(4));
    assert_eq!(original.episode_numbers, vec![20]);

    let resolution =
        translate_release_numbering(Some(&bridge()), &series, &episodes, &mut release, None);

    assert_eq!(
        resolution
            .resolved()
            .expect("community candidate")
            .episode_ids,
        vec!["ep-56".to_string()]
    );
    assert_eq!(release.raw_title, raw_title);
    let translated = release.episode.as_ref().expect("translated episode");
    assert_eq!(translated.raw, raw_episode);
    assert_eq!(translated.season, Some(1));
    assert_eq!(translated.season_numbers, vec![1]);
    assert_eq!(translated.episode_numbers, vec![56]);
    assert_eq!(translated.absolute_episode, Some(56));
    assert_eq!(translated.absolute_episode_numbers, vec![56]);
    assert_eq!(
        resolve_release_coverage(&release, &episodes, &[], Some(&episodes[55])),
        ReleaseCoverage::SingleEpisode("ep-56".to_string())
    );
    assert!(!parsed_numbering_contradicts_episode(
        Some(1),
        Some(56),
        Some(56),
        translated,
    ));
    assert!(parsed_numbering_contradicts_episode(
        Some(1),
        Some(55),
        Some(55),
        translated,
    ));
    assert!(!parsed_release_contradicts_requested_episode(
        &release,
        &episodes[55],
    ));
    assert!(parsed_release_contradicts_requested_episode(
        &release,
        &episodes[54],
    ));
}

#[test]
fn translation_rebuilds_complete_absolute_coordinates() {
    let episodes = official_episodes(true);
    let mut parsed = parsed(Some(4), &[20, 19]);

    translate_parsed_episode_numbering(
        &bridge(),
        &title(SERIES_NAME),
        &episodes,
        &mut parsed,
        &[],
        None,
    );

    assert_eq!(parsed.season, Some(1));
    assert_eq!(parsed.season_numbers, vec![1]);
    assert_eq!(parsed.episode_numbers, vec![55, 56]);
    assert_eq!(parsed.absolute_episode, Some(55));
    assert_eq!(parsed.absolute_episode_numbers, vec![55, 56]);
    assert!(parsed.special_absolute_episode_numbers.is_empty());
}

#[test]
fn translation_clears_incomplete_absolute_coordinates() {
    let mut episodes = official_episodes(true);
    episodes[55].absolute_number = None;
    let mut parsed = parsed(Some(4), &[20, 19]);
    parsed.absolute_episode = Some(20);
    parsed.absolute_episode_numbers = vec![20, 19];
    parsed.special_absolute_episode_numbers = vec![20];

    translate_parsed_episode_numbering(
        &bridge(),
        &title(SERIES_NAME),
        &episodes,
        &mut parsed,
        &[],
        None,
    );

    assert_eq!(parsed.season, Some(1));
    assert_eq!(parsed.season_numbers, vec![1]);
    assert_eq!(parsed.episode_numbers, vec![55, 56]);
    assert_eq!(parsed.absolute_episode, None);
    assert!(parsed.absolute_episode_numbers.is_empty());
    assert!(parsed.special_absolute_episode_numbers.is_empty());
}

#[test]
fn translation_clears_inconsistent_absolute_coordinates() {
    let mut episodes = official_episodes(true);
    episodes[55].absolute_number = Some("55".to_string());
    let mut parsed = parsed(Some(4), &[20, 19]);

    translate_parsed_episode_numbering(
        &bridge(),
        &title(SERIES_NAME),
        &episodes,
        &mut parsed,
        &[],
        None,
    );

    assert_eq!(parsed.season, Some(1));
    assert_eq!(parsed.season_numbers, vec![1]);
    assert_eq!(parsed.episode_numbers, vec![55, 56]);
    assert_eq!(parsed.absolute_episode, None);
    assert!(parsed.absolute_episode_numbers.is_empty());
    assert!(parsed.special_absolute_episode_numbers.is_empty());
}

/// Community season 1 is TVDB season 1 episodes 1-14, so an `S01E05` release
/// reads the same either way and the literal reading stands untouched.
#[test]
fn an_agreeing_reading_leaves_the_literal_numbering_alone() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(1), &[5]),
        &[],
        None,
    );

    assert_eq!(resolution, NumberingResolution::Unchanged);
}

/// A community season boundary is respected: episode 13 of a 12-episode cour
/// belongs to no range, so that reading is simply not offered.
#[test]
fn an_episode_past_a_cour_boundary_is_not_translated() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(2), &[13]),
        &[],
        None,
    );

    assert_eq!(resolution, NumberingResolution::Unchanged);
}

/// A multi-episode release inside one cour translates as a block.
#[test]
fn a_multi_episode_release_translates_as_a_block() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(3), &[4, 5]),
        &[],
        None,
    );

    let candidate = resolution.resolved().expect("community candidate");
    assert_eq!(candidate.episode_numbers, vec![30, 31]);
    assert_eq!(
        candidate.episode_ids,
        vec!["ep-30".to_string(), "ep-31".to_string()]
    );
}

/// A complete community cour can become a bounded official range, while an
/// explicit member bound remains just that member instead of widening again.
#[test]
fn complete_cour_packs_translate_only_when_their_mapping_is_closed() {
    let mut season_pack = parsed(Some(4), &[]);
    season_pack.full_season = true;
    season_pack.release_type = crate::release_parser::ParsedEpisodeReleaseType::SeasonPack;
    let season_candidate = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &season_pack,
        &[],
        None,
    )
    .resolved()
    .cloned()
    .expect("complete cour candidate");
    assert_eq!(
        season_candidate.episode_numbers,
        (37..=60).collect::<Vec<_>>()
    );

    let mut series_pack = parsed(Some(4), &[20]);
    series_pack.is_series_pack = true;
    let bounded_candidate = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &series_pack,
        &[],
        None,
    )
    .resolved()
    .cloned()
    .expect("bounded cour member");
    assert_eq!(bounded_candidate.episode_numbers, vec![56]);
}

// ── absolute numbering ────────────────────────────────────────────────────

/// Absolute numbering already worked through `Episode.absolute_number`, and it
/// still does: an absolute-only release resolves without any bridge help.
#[test]
fn absolute_numbering_still_resolves_through_the_catalog() {
    let absolute_only = absolute_only_parse(56);

    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only,
        &[],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_ids, vec!["ep-56".to_string()]);
}

/// When the catalog carries no absolute numbers at all, the bridge's
/// `absolute_start` places the release instead.
#[test]
fn an_absolute_release_falls_back_to_the_bridge_absolute_start() {
    let absolute_only = absolute_only_parse(56);

    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(false),
        &absolute_only,
        &[],
        None,
    );

    let candidate = resolution.resolved().expect("community candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Community);
    assert_eq!(candidate.episode_numbers, vec![56]);
}

// ── title anchoring ───────────────────────────────────────────────────────

/// A release named after a cour pins that cour even when its season token says
/// something else — `S01E08` of "Glass Meridian" is TVDB S01E34.
#[test]
fn a_cour_title_pins_the_season_over_the_season_token() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(1), &[8]),
        &["Lantern Verge Glass Meridian".to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("title-anchored candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::TitleAnchored);
    assert_eq!(candidate.episode_numbers, vec![34]);
}

/// A release that names the plain series title carries no season evidence, so
/// nothing is pinned and the literal reading stands.
#[test]
fn the_plain_series_title_anchors_nothing() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(1), &[8]),
        &["Lantern Verge".to_string()],
        None,
    );

    assert_eq!(resolution, NumberingResolution::Unchanged);
}

/// A cour's own name is normally a series alias too — a metadata provider's
/// alias list for a long-running anime is the union of every cour's names — so
/// an alias match is not evidence that the release named the series rather
/// than a season within it. The cour still anchors.
#[test]
fn a_cour_name_that_is_also_a_series_alias_still_anchors() {
    let mut series = title(SERIES_NAME);
    series.aliases = vec!["Lantern Verge: Ember Circuit".to_string()];
    series.tagged_aliases = vec![TaggedAlias {
        name: "Lantern Verge: Ember Circuit".to_string(),
        language: "eng".to_string(),
    }];

    let resolution = resolve(
        &bridge(),
        &series,
        &official_episodes(true),
        &parsed(Some(1), &[8]),
        &["Lantern Verge Ember Circuit".to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("title-anchored candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::TitleAnchored);
    assert_eq!(candidate.episode_numbers, vec![22]);
}

/// The parser projects every release onto the title it matched, so the series'
/// canonical name is always among the variants. Anchoring has to look past it
/// to the variant that says something the series name does not — otherwise no
/// release ever anchors.
#[test]
fn the_projected_series_name_does_not_block_a_cour_variant() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(1), &[8]),
        &[
            SERIES_NAME.to_string(),
            "Lantern Verge Glass Meridian".to_string(),
        ],
        None,
    );

    let candidate = resolution.resolved().expect("title-anchored candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::TitleAnchored);
    assert_eq!(candidate.episode_numbers, vec![34]);
}

/// A cour catalogued under the series' own name is the franchise, not a season
/// within it. Without this guard the first cour — which is normally named for
/// the series — would swallow every release that names the franchise.
#[test]
fn a_cour_named_for_the_series_anchors_nothing() {
    let mut franchise_named = bridge();
    franchise_named.seasons[0]
        .titles
        .push("Opening Movement".to_string());

    let resolution = resolve(
        &franchise_named,
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(1), &[8]),
        &[SERIES_NAME.to_string(), "Opening Movement".to_string()],
        None,
    );

    assert_eq!(resolution, NumberingResolution::Unchanged);
}

/// A name shared by two community seasons pins neither.
#[test]
fn a_name_shared_by_two_cours_anchors_nothing() {
    let mut shared = bridge();
    shared.seasons[1]
        .titles
        .push("Shared Cour Name".to_string());
    shared.seasons[2]
        .titles
        .push("Shared Cour Name".to_string());

    let resolution = resolve(
        &shared,
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(1), &[8]),
        &["Shared Cour Name".to_string()],
        None,
    );

    assert_eq!(resolution, NumberingResolution::Unchanged);
}

/// The shape this rule exists for. Groups that title a release after its cour
/// number it within that cour, so the bare number behind the cour name is the
/// cour's episode 20 — official S01E56 — not series-wide absolute 20. The
/// catalog carrying its own absolute numbers does not change that: it is what
/// makes the wrong reading available in the first place.
#[test]
fn a_bare_number_behind_a_cour_name_counts_within_that_cour() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(20),
        &[
            SERIES_NAME.to_string(),
            "Lantern Verge Final Chorus".to_string(),
        ],
        None,
    );

    let candidate = resolution.resolved().expect("title-anchored candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::TitleAnchored);
    assert_eq!(candidate.season, 1);
    assert_eq!(candidate.episode_numbers, vec![56]);
    assert_eq!(candidate.episode_ids, vec!["ep-56".to_string()]);
}

/// A group that names the cour but numbers absolutely is still read correctly:
/// 45 is past the end of a 10-episode cour, so that reading maps to nothing
/// and the absolute one stands.
#[test]
fn a_number_past_the_named_cour_falls_back_to_absolute_numbering() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(45),
        &[
            SERIES_NAME.to_string(),
            "Lantern Verge Glass Meridian".to_string(),
        ],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![45]);
}

// ── precedence, ambiguity and tie-breaking ────────────────────────────────

/// With a multi-season official order, the literal and the community readings
/// both land somewhere — and the community reading wins the disagreement.
#[test]
fn community_numbering_outranks_the_literal_reading_when_they_disagree() {
    // Official order here has four seasons of 14/12/10/24, but the bridge maps
    // the community cours onto season 1 as a single 60-episode run.
    let mut episodes = official_episodes(true);
    let mut absolute = 61;
    for (offset, length) in COUR_LENGTHS.iter().enumerate() {
        let season = u32::try_from(offset).expect("small index") + 2;
        for number in 1..=u32::try_from(*length).expect("small length") {
            episodes.push(episode(
                &format!("s{season}-e{number}"),
                season,
                number,
                Some(absolute),
                "",
            ));
            absolute += 1;
        }
    }

    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &episodes,
        &parsed(Some(4), &[20]),
        &[],
        None,
    );

    let candidate = resolution.resolved().expect("community candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Community);
    assert_eq!(candidate.episode_ids, vec!["ep-56".to_string()]);
}

/// Two community entries claiming the same index both answer to `S02`, and
/// they land on different episodes. Nothing ranks one above the other, so the
/// release is unplaceable and must be reported rather than guessed at. (The
/// contract makes `index` unique, so this is the defensive case: Scryer never
/// silently takes whichever entry came first.)
#[test]
fn two_equally_ranked_readings_are_ambiguous() {
    let mut conflicted = bridge();
    conflicted.seasons[2].index = 2;

    let resolution = resolve(
        &conflicted,
        &title(SERIES_NAME),
        &official_episodes(false),
        &parsed(Some(2), &[3]),
        &[],
        None,
    );

    assert!(
        resolution.is_ambiguous(),
        "expected ambiguity: {resolution:?}"
    );
    let summary = resolution.ambiguity_summary().expect("summary");
    assert!(summary.contains("community season 2"), "{summary}");
    assert!(summary.contains("S01E17"), "{summary}");
    assert!(summary.contains("S01E29"), "{summary}");
}

/// A reference date near one reading's air date settles an otherwise equal tie.
#[test]
fn a_reference_date_settles_an_otherwise_equal_tie() {
    let mut conflicted = bridge();
    conflicted.seasons[2].index = 2;
    let episodes = official_episodes(false);

    // Community season 2 episode 3 is TVDB episode 17 (aired 2025-10-26); the
    // conflicting entry's episode 3 is TVDB episode 29 (aired 2026-01-18).
    assert!(
        resolve(
            &conflicted,
            &title(SERIES_NAME),
            &episodes,
            &parsed(Some(2), &[3]),
            &[],
            None
        )
        .is_ambiguous()
    );

    let settled = resolve(
        &conflicted,
        &title(SERIES_NAME),
        &episodes,
        &parsed(Some(2), &[3]),
        &[],
        NaiveDate::from_ymd_opt(2025, 10, 27),
    );
    let candidate = settled.resolved().expect("date-settled candidate");
    assert_eq!(candidate.episode_numbers, vec![17]);

    // A date far from either reading leaves the ambiguity standing.
    assert!(
        resolve(
            &conflicted,
            &title(SERIES_NAME),
            &episodes,
            &parsed(Some(2), &[3]),
            &[],
            NaiveDate::from_ymd_opt(2027, 3, 1),
        )
        .is_ambiguous()
    );
}

/// A bridge with no seasons is the same as no bridge at all.
#[test]
fn an_empty_bridge_changes_nothing() {
    let resolution = resolve(
        &AnimeNumberingBridge::default(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(4), &[20]),
        &[],
        None,
    );

    assert_eq!(resolution, NumberingResolution::Unchanged);
}

/// A reading that lands on no catalog episode is discarded rather than offered.
#[test]
fn a_reading_with_no_catalog_episode_is_discarded() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true)[..10],
        &parsed(Some(4), &[20]),
        &[],
        None,
    );

    assert_eq!(resolution, NumberingResolution::Unchanged);
}

// ── search-side translation ───────────────────────────────────────────────

/// The inverse translation used to build community-numbered search queries.
#[test]
fn a_wanted_tvdb_episode_translates_back_into_community_numbering() {
    let bridge = bridge();

    let coordinates =
        community_coordinates_for_tvdb_episode(&bridge, 1, 56).expect("community coordinates");
    assert_eq!(coordinates.season, 4);
    assert_eq!(coordinates.episode, 20);
    assert_eq!(
        coordinates.season_title.as_deref(),
        Some("Lantern Verge: Final Chorus")
    );

    assert_eq!(
        community_coordinates_for_tvdb_episode(&bridge, 1, 1)
            .expect("first cour")
            .season,
        1
    );
    assert!(community_coordinates_for_tvdb_episode(&bridge, 2, 1).is_none());
    assert!(community_coordinates_for_tvdb_episode(&bridge, 1, 99).is_none());
}

// ── fuzzy title anchoring ─────────────────────────────────────────────────

/// The bridge's own romanisation of each cour, long enough that a group's
/// spelling of the same name is a handful of letters away rather than a
/// different string.
const LONG_COUR_TITLES: [&str; 4] = [
    "Rantan Kyoukai Monogatari Hajimari no Akari wo Motomete Tabi no Uta",
    "Rantan Kyoukai Monogatari Kagayaku Yoru no Kioku to Tomo ni Ayumu Uta",
    "Rantan Kyoukai Monogatari Garasu no Shigosen wo Koete Susumu Michi no Uta",
    "Rantan Kyoukai Monogatari Saigo no Gasshou wo Utau Toki no Hikari to Kage no Uta",
];

fn bridge_with_cour_titles(titles: &[&str]) -> AnimeNumberingBridge {
    let mut bridge = bridge();
    for (season, title) in bridge.seasons.iter_mut().zip(titles) {
        season.titles = vec![(*title).to_string()];
    }
    bridge
}

/// Long enough to be fuzzed at all, short enough that the tolerance it earns
/// is two edits rather than the ceiling.
const MID_LENGTH_COUR_TITLE: &str = "Rantan Kyoukai Monogatari Hikari no Utagoe";
/// Past the point where length alone would buy a fifth edit of tolerance.
const OVERLONG_COUR_TITLE: &str = concat!(
    "Rantan Kyoukai Monogatari Saigo no Gasshou wo Utau Toki no Hikari to Kage no Uta ",
    "Owari naki Yoake no Shou"
);
/// The series' own long romanisation, and a cour catalogued one letter away
/// from it — the drift a written-out macron leaves behind.
const LONG_SERIES_NAME: &str = "Rantan Kyoukai Monogatari Honzuki no Gekokujou";
const COUR_TITLED_ALMOST_LIKE_THE_SERIES: &str = "Rantan Kyoukai Monogatari Honzuki no Gekokujo";

fn bridge_with_cour_title_lists(titles: &[&[&str]]) -> AnimeNumberingBridge {
    let mut bridge = bridge();
    for (season, season_titles) in bridge.seasons.iter_mut().zip(titles) {
        season.titles = season_titles
            .iter()
            .map(|title| (*title).to_string())
            .collect();
    }
    bridge
}

/// The case the whole cour-aware chain exists for: the group romanises the
/// cour's long title its own way — three letters apart from the bridge's — and
/// numbers the release within that cour. Nothing else names it, so the near
/// miss is allowed to anchor.
#[test]
fn a_romanised_cour_title_anchors_within_tolerance() {
    let resolution = resolve(
        &bridge_with_cour_titles(&LONG_COUR_TITLES),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        &[
            "Rantan Kyokai Monogatari Saigo no Gassho o Utau Toki no Hikari to Kage no Uta"
                .to_string(),
        ],
        None,
    );

    let candidate = resolution.resolved().expect("title-anchored candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::TitleAnchored);
    // Community episode 5 of cour 4, which starts at TVDB 37.
    assert_eq!(candidate.episode_numbers, vec![41]);
}

/// Sibling cours differ by a single character when they are numbered, so
/// letter distance alone would anchor a release to the cour next door. The
/// numbers a title carries have to agree before any of its letters are
/// compared.
#[test]
fn an_adjacent_cour_number_anchors_nothing() {
    let numbered: Vec<String> = LONG_COUR_TITLES
        .iter()
        .enumerate()
        .map(|(offset, title)| format!("{title} S{}", offset + 1))
        .collect();
    let numbered: Vec<&str> = numbered.iter().map(String::as_str).collect();

    let resolution = resolve(
        &bridge_with_cour_titles(&numbered),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        // One character from cour 4's title, and that character is its number.
        &[format!("{} S3", LONG_COUR_TITLES[3])],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

/// A release naming a cour but leaving its number off cannot be read as that
/// cour: carrying no number where the catalogued title carries one is itself a
/// disagreement, whatever the letters do. This is what stops the series' own
/// romanisation — which never carries a cour number — from anchoring anywhere.
#[test]
fn an_unnumbered_title_never_matches_a_numbered_cour() {
    let numbered: Vec<String> = LONG_COUR_TITLES
        .iter()
        .enumerate()
        .map(|(offset, title)| format!("{title} S{}", offset + 1))
        .collect();
    let numbered: Vec<&str> = numbered.iter().map(String::as_str).collect();

    let resolution = resolve(
        &bridge_with_cour_titles(&numbered),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        &[LONG_COUR_TITLES[3].to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

/// A release that already says which season it belongs to needs no anchoring,
/// so the near-miss tier never runs for it: the community reading of its own
/// season token stands.
#[test]
fn a_season_numbered_parse_never_reaches_the_fuzzy_tier() {
    let resolution = resolve(
        &bridge_with_cour_titles(&LONG_COUR_TITLES),
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(4), &[20]),
        &[
            "Rantan Kyokai Monogatari Saigo no Gassho o Utau Toki no Hikari to Kage no Uta"
                .to_string(),
        ],
        None,
    );

    let candidate = resolution.resolved().expect("community candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Community);
    assert_eq!(candidate.episode_numbers, vec![56]);
}

/// Two cours near enough to the same name pin nothing. A romanisation that
/// could plausibly be either is evidence for neither.
#[test]
fn two_cours_within_tolerance_anchor_nothing() {
    let mut near_twins: Vec<&str> = LONG_COUR_TITLES.to_vec();
    // Cour 3 restated as a one-letter variation of cour 4's title.
    near_twins[2] =
        "Rantan Kyoukai Monogatari Saigo no Gasshou wo Utau Toki no Hikari to Kage no Ute";

    let resolution = resolve(
        &bridge_with_cour_titles(&near_twins),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        &[
            "Rantan Kyokai Monogatari Saigo no Gassho o Utau Toki no Hikari to Kage no Uta"
                .to_string(),
        ],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

/// Below the length floor a single edit is most of the difference between two
/// cours, so nothing fuzzes at all — the short catalogued forms stay exact-only.
#[test]
fn a_cour_title_below_the_length_floor_anchors_nothing() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        // One letter from cour 4's catalogued title, which is 26 characters.
        &["Lantern Verge: Final Chorum".to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

/// Tolerance is proportional to length, so a 42-character title allows two
/// edits and a third is one too many.
#[test]
fn a_romanisation_past_the_tolerance_anchors_nothing() {
    let resolution = resolve(
        &bridge_with_cour_titles(&[
            LONG_COUR_TITLES[0],
            LONG_COUR_TITLES[1],
            LONG_COUR_TITLES[2],
            MID_LENGTH_COUR_TITLE,
        ]),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        // Three edits from cour 4's title, which earns two.
        &["Rantan Kyokai Monogatari Hikaru no Utagoo".to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

/// The tolerance belongs to the two names actually being compared. A cour that
/// also answers to a much longer name must not lend that name's larger
/// allowance to its shorter one.
#[test]
fn the_tolerance_is_earned_by_the_pair_not_by_the_cours_longest_title() {
    let resolution = resolve(
        &bridge_with_cour_title_lists(&[
            &[LONG_COUR_TITLES[0]],
            &[LONG_COUR_TITLES[1]],
            &[LONG_COUR_TITLES[2]],
            // The long form would earn four edits; the short one earns two.
            &[MID_LENGTH_COUR_TITLE, OVERLONG_COUR_TITLE],
        ]),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        &["Rantan Kyokai Monogatari Hikaru no Utagoo".to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

/// Length buys tolerance only up to a ceiling: a 105-character title would
/// otherwise earn five edits, and five edits is a different name.
#[test]
fn a_very_long_title_still_caps_its_tolerance() {
    let resolution = resolve(
        &bridge_with_cour_titles(&[
            LONG_COUR_TITLES[0],
            LONG_COUR_TITLES[1],
            LONG_COUR_TITLES[2],
            OVERLONG_COUR_TITLE,
        ]),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        &[
            "Rantan Kyokai Monogatari Saigo no Gassho o Utau Toki no Hikari to Kaje no Uta \
             Owari naki Yoake no Sho"
                .to_string(),
        ],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

/// The series' own name competes with the cours. A cour catalogued one letter
/// from the franchise name slips past the exact test that drops franchise-named
/// cours, and would otherwise collect every release that names the series.
#[test]
fn a_cour_titled_almost_like_the_series_anchors_nothing() {
    let resolution = resolve(
        &bridge_with_cour_titles(&[
            COUR_TITLED_ALMOST_LIKE_THE_SERIES,
            LONG_COUR_TITLES[1],
            LONG_COUR_TITLES[2],
            LONG_COUR_TITLES[3],
        ]),
        &title(LONG_SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        // A group's romanisation of the series name: one edit from the series,
        // two from the cour catalogued under almost the same name.
        &["Rantan Kyokai Monogatari Honzuki no Gekokujou".to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("absolute candidate");
    assert_eq!(candidate.kind, NumberingCandidateKind::Absolute);
    assert_eq!(candidate.episode_numbers, vec![5]);
}

fn whole_pack(season: Option<u32>, episode_numbers: &[u32]) -> ParsedEpisodeMetadata {
    ParsedEpisodeMetadata {
        season,
        season_numbers: season.into_iter().collect(),
        episode_numbers: episode_numbers.to_vec(),
        full_season: true,
        release_type: ParsedEpisodeReleaseType::SeasonPack,
        ..Default::default()
    }
}

#[test]
fn an_exact_cour_pack_projects_only_its_closed_official_episode_set() {
    let resolution = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &whole_pack(Some(1), &[]),
        &[COUR_TITLES[2].to_string()],
        None,
    );

    let candidate = resolution.resolved().expect("bounded cour candidate");
    assert_eq!(candidate.episode_numbers, (27..=36).collect::<Vec<_>>());
    assert_eq!(
        candidate.episode_ids.first().map(String::as_str),
        Some("ep-27")
    );
    assert_eq!(
        candidate.episode_ids.last().map(String::as_str),
        Some("ep-36")
    );
}

#[test]
fn unqualified_complete_pack_with_distinct_official_and_community_readings_is_unresolved() {
    assert!(matches!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &official_episodes(true),
            &whole_pack(Some(1), &[]),
            &[SERIES_NAME.to_string()],
            None,
        ),
        NumberingResolution::UnresolvedPack
    ));
}

#[test]
fn incomplete_cour_mapping_cannot_fall_back_to_official_collection_scope() {
    let mut incomplete = bridge();
    incomplete.seasons[2].episode_count = None;
    assert!(matches!(
        resolve(
            &incomplete,
            &title(SERIES_NAME),
            &official_episodes(true),
            &whole_pack(Some(3), &[]),
            &[],
            None,
        ),
        NumberingResolution::UnresolvedPack
    ));
}

#[test]
fn incomplete_absolute_range_is_not_reduced_to_its_present_catalog_endpoint() {
    let episodes = official_episodes(true)
        .into_iter()
        .filter(|episode| episode.absolute_number.as_deref() == Some("55"))
        .collect::<Vec<_>>();
    let parsed = ParsedEpisodeMetadata {
        absolute_episode: Some(55),
        absolute_episode_numbers: vec![55, 56],
        ..Default::default()
    };
    assert_eq!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &episodes,
            &parsed,
            &[],
            None
        ),
        NumberingResolution::Unchanged
    );
}

#[test]
fn explicit_bounds_outrank_completion_flags() {
    let mut relative = whole_pack(Some(3), &[1]);
    relative.is_series_pack = true;
    let relative = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &relative,
        &[],
        None,
    );
    assert_eq!(
        relative
            .resolved()
            .expect("bounded relative candidate")
            .episode_numbers,
        vec![27]
    );

    let mut absolute = whole_pack(None, &[]);
    absolute.absolute_episode = Some(27);
    absolute.absolute_episode_numbers = vec![27];
    let absolute = resolve(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute,
        &[],
        None,
    );
    assert_eq!(
        absolute
            .resolved()
            .expect("bounded absolute candidate")
            .episode_numbers,
        vec![27]
    );

    let mut translated = whole_pack(Some(3), &[1, 2]);
    translated.release_type = ParsedEpisodeReleaseType::MultiEpisode;
    translate_parsed_episode_numbering(
        &bridge(),
        &title(SERIES_NAME),
        &official_episodes(true),
        &mut translated,
        &[],
        None,
    );
    assert_eq!(translated.episode_numbers, vec![27, 28]);
    assert!(!translated.full_season);
    assert_eq!(
        translated.release_type,
        ParsedEpisodeReleaseType::MultiEpisode
    );
}

#[test]
fn parser_complete_cour_with_local_s01_projects_only_the_named_cour() {
    let mut release = crate::parse_release_metadata(
        "Lantern.Verge.Glass.Meridian.S01.Complete.1080p.WEB-DL-GROUP",
    );
    let resolution = translate_release_numbering(
        Some(&bridge()),
        &title(SERIES_NAME),
        &official_episodes(true),
        &mut release,
        None,
    );
    assert_eq!(
        resolution
            .resolved()
            .expect("cour three candidate")
            .episode_numbers,
        (27..=36).collect::<Vec<_>>()
    );
}

#[test]
fn ambiguous_exact_titles_block_fuzzy_and_whole_pack_widening() {
    let mut ambiguous = bridge();
    ambiguous.seasons[1]
        .titles
        .push("Shared Exact Name".to_string());
    ambiguous.seasons[2]
        .titles
        .push("Shared Exact Name".to_string());
    ambiguous.seasons[3]
        .titles
        .push(LONG_COUR_TITLES[3].to_string());

    let mut pack = whole_pack(None, &[]);
    pack.season_numbers.clear();
    pack.season = None;
    assert_eq!(
        resolve(
            &ambiguous,
            &title(SERIES_NAME),
            &official_episodes(true),
            &pack,
            &["Shared Exact Name".to_string()],
            None,
        ),
        NumberingResolution::UnresolvedPack
    );

    let episodic = resolve(
        &ambiguous,
        &title(SERIES_NAME),
        &official_episodes(true),
        &absolute_only_parse(5),
        &[
            "Shared Exact Name".to_string(),
            LONG_COUR_TITLES[3].to_string(),
        ],
        None,
    );
    assert_eq!(
        episodic
            .resolved()
            .expect("catalog absolute candidate")
            .kind,
        NumberingCandidateKind::Absolute
    );
}

#[test]
fn exact_entry_remains_authoritative_when_indexes_duplicate() {
    let mut duplicated = bridge();
    duplicated.seasons[2].index = 2;
    let resolution = resolve(
        &duplicated,
        &title(SERIES_NAME),
        &official_episodes(true),
        &parsed(Some(2), &[8]),
        &["Lantern Verge Glass Meridian".to_string()],
        None,
    );
    assert_eq!(
        resolution
            .resolved()
            .expect("exact cour candidate")
            .episode_numbers,
        vec![34]
    );
    assert!(matches!(
        exact_cour_title_match(
            SERIES_NAME,
            &duplicated,
            &["Lantern Verge Glass Meridian".to_string()],
        ),
        ExactCourTitleMatch::Unique(cour) if std::ptr::eq(cour, &duplicated.seasons[2])
    ));
}

#[test]
fn bounded_cour_packs_do_not_discard_conflicting_season_restrictions() {
    let mut pack = whole_pack(Some(2), &[1, 2]);
    pack.season_numbers = vec![2, 3];
    assert!(matches!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &official_episodes(true),
            &pack,
            &[],
            None
        ),
        NumberingResolution::UnresolvedPack
    ));
    pack.season_numbers = vec![2];
    assert!(matches!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &official_episodes(true),
            &pack,
            &[COUR_TITLES[2].to_string()],
            None
        ),
        NumberingResolution::UnresolvedPack
    ));
}

#[test]
fn explicit_cour_lists_keep_exact_coverage_and_reject_reused_catalog_ids() {
    let mut episodes = official_episodes(true);
    let mut pack = whole_pack(Some(2), &[]);
    pack.season_numbers = vec![2, 3];
    pack.is_multi_season = true;
    let resolution = resolve(&bridge(), &title(SERIES_NAME), &episodes, &pack, &[], None);
    let candidate = resolution.resolved().expect("closed cours two and three");
    assert_eq!(candidate.episode_numbers, (15..=36).collect::<Vec<_>>());
    episodes[26].id = episodes[14].id.clone();
    assert!(matches!(
        resolve(&bridge(), &title(SERIES_NAME), &episodes, &pack, &[], None),
        NumberingResolution::UnresolvedPack
    ));
}

#[test]
fn identical_official_multi_season_packs_need_no_community_projection() {
    let mut bridge = bridge();
    bridge.seasons.truncate(2);
    let mut episodes = Vec::new();
    for cour in &mut bridge.seasons {
        cour.ranges[0].tvdb_season = cour.index;
        cour.ranges[0].tvdb_episode_start = 1;
        cour.ranges[0].tvdb_episode_end = cour.episode_count;
        for number in 1..=cour.episode_count.unwrap() {
            episodes.push(episode(
                &format!("s{}e{number}", cour.index),
                cour.index as u32,
                number as u32,
                None,
                "",
            ));
        }
    }
    let mut pack = whole_pack(Some(1), &[]);
    pack.season_numbers = vec![1, 2];
    pack.is_multi_season = true;
    assert_eq!(
        resolve(&bridge, &title(SERIES_NAME), &episodes, &pack, &[], None),
        NumberingResolution::Unchanged
    );
}

#[test]
fn multi_season_official_pack_disagreement_is_unresolved() {
    let mut episodes = official_episodes(true);
    episodes.extend((1..=3).map(|number| episode(&format!("s2-{number}"), 2, number, None, "")));
    let mut pack = whole_pack(Some(1), &[]);
    pack.season_numbers = vec![1, 2];
    pack.is_multi_season = true;
    assert!(matches!(
        resolve(&bridge(), &title(SERIES_NAME), &episodes, &pack, &[], None),
        NumberingResolution::UnresolvedPack
    ));
}

#[test]
fn only_explicit_series_packs_preserve_unqualified_scope() {
    let mut series = whole_pack(None, &[]);
    series.season_numbers.clear();
    series.is_series_pack = true;
    assert_eq!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &official_episodes(true),
            &series,
            &[],
            None
        ),
        NumberingResolution::Unchanged
    );

    let mut extra = whole_pack(Some(3), &[]);
    extra.is_season_extra = true;
    assert!(matches!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &official_episodes(true),
            &extra,
            &[],
            None
        ),
        NumberingResolution::UnresolvedPack
    ));
}

#[test]
fn malformed_community_ranges_are_rejected_before_projection() {
    let source = bridge().seasons[2].clone();
    let cases = [
        (
            "gap",
            vec![AnimeCommunitySeasonRange {
                community_episode_start: 2,
                ..source.ranges[0].clone()
            }],
        ),
        (
            "overlap",
            vec![
                source.ranges[0].clone(),
                AnimeCommunitySeasonRange {
                    community_episode_start: 2,
                    community_episode_end: Some(3),
                    tvdb_episode_start: 28,
                    tvdb_episode_end: Some(29),
                    ..source.ranges[0].clone()
                },
            ],
        ),
        (
            "outside count",
            vec![AnimeCommunitySeasonRange {
                community_episode_end: Some(11),
                tvdb_episode_end: Some(37),
                ..source.ranges[0].clone()
            }],
        ),
        (
            "open source",
            vec![AnimeCommunitySeasonRange {
                community_episode_end: None,
                ..source.ranges[0].clone()
            }],
        ),
        (
            "open destination",
            vec![AnimeCommunitySeasonRange {
                tvdb_episode_end: None,
                ..source.ranges[0].clone()
            }],
        ),
        (
            "duplicate destination",
            vec![
                AnimeCommunitySeasonRange {
                    community_episode_end: Some(5),
                    tvdb_episode_end: Some(31),
                    ..source.ranges[0].clone()
                },
                AnimeCommunitySeasonRange {
                    community_episode_start: 6,
                    tvdb_episode_start: 27,
                    ..source.ranges[0].clone()
                },
            ],
        ),
        (
            "mismatched span",
            vec![AnimeCommunitySeasonRange {
                tvdb_episode_start: i32::MAX,
                tvdb_episode_end: Some(i32::MAX),
                ..source.ranges[0].clone()
            }],
        ),
    ];
    for (name, ranges) in cases {
        let mut season = source.clone();
        season.ranges = ranges;
        assert!(complete_community_projection(&season).is_none(), "{name}");
    }
}

#[test]
fn whole_cour_packs_reject_missing_duplicate_and_cross_season_catalog_rows() {
    let pack = whole_pack(Some(3), &[]);
    let missing = official_episodes(true)
        .into_iter()
        .filter(|episode| episode.id != "ep-28")
        .collect::<Vec<_>>();
    assert!(matches!(
        resolve(&bridge(), &title(SERIES_NAME), &missing, &pack, &[], None),
        NumberingResolution::UnresolvedPack
    ));

    let mut duplicate = official_episodes(true);
    duplicate.push(duplicate[26].clone());
    assert!(matches!(
        resolve(&bridge(), &title(SERIES_NAME), &duplicate, &pack, &[], None),
        NumberingResolution::UnresolvedPack
    ));

    let mut split = bridge();
    split.seasons[2].ranges = vec![
        AnimeCommunitySeasonRange {
            community_episode_start: 1,
            community_episode_end: Some(5),
            tvdb_season: 1,
            tvdb_episode_start: 27,
            tvdb_episode_end: Some(31),
        },
        AnimeCommunitySeasonRange {
            community_episode_start: 6,
            community_episode_end: Some(10),
            tvdb_season: 2,
            tvdb_episode_start: 1,
            tvdb_episode_end: Some(5),
        },
    ];
    let mut multi_season = official_episodes(true);
    multi_season
        .extend((1..=5).map(|number| episode(&format!("split-{number}"), 2, number, None, "")));
    assert!(matches!(
        resolve(&split, &title(SERIES_NAME), &multi_season, &pack, &[], None),
        NumberingResolution::UnresolvedPack
    ));
}
