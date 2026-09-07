//! Numbering-translation rules, against a synthetic four-cour anime.
//!
//! The fixture layout is the shape this feature exists for: TVDB carries one
//! official season of 60 episodes, while the community carries four seasons of
//! 14 / 12 / 10 / 24. Every show, season and group name here is invented.

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

/// Season packs and series packs are out of scope: they resolve by collection,
/// and moving a whole pack onto another season on the strength of one token is
/// a worse failure than the one being fixed.
#[test]
fn packs_are_never_translated() {
    let mut season_pack = parsed(Some(4), &[]);
    season_pack.full_season = true;
    season_pack.release_type = crate::release_parser::ParsedEpisodeReleaseType::SeasonPack;
    assert_eq!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &official_episodes(true),
            &season_pack,
            &[],
            None
        ),
        NumberingResolution::Unchanged
    );

    let mut series_pack = parsed(Some(4), &[20]);
    series_pack.is_series_pack = true;
    assert_eq!(
        resolve(
            &bridge(),
            &title(SERIES_NAME),
            &official_episodes(true),
            &series_pack,
            &[],
            None
        ),
        NumberingResolution::Unchanged
    );
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

/// An alias of the series is still the series, not a cour.
#[test]
fn a_series_alias_anchors_nothing() {
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
