use chrono::NaiveDate;

use crate::enrichment::{enrich_candidate, project_final_metadata};
use crate::{
    AudioCodec, ContextAlias, ContextEpisode, ContextFacetHint, ContextTitle,
    ContextTitleMatchKind, ExternalIdSource, ParseFamily, ParsedEpisodeReleaseType,
    ReleaseParseContext, ReleaseSource, StreamingService, VideoCodec,
    analyze_release_against_targets, analyze_release_for_target,
};

fn source_label(source: Option<&ReleaseSource>) -> Option<&str> {
    source.map(ReleaseSource::as_str)
}

fn audio_label(codec: Option<&AudioCodec>) -> Option<&str> {
    codec.map(AudioCodec::as_str)
}

fn audio_codec_labels(codecs: &[AudioCodec]) -> Vec<&str> {
    codecs.iter().map(AudioCodec::as_str).collect()
}

fn streaming_service_label(service: Option<&StreamingService>) -> Option<&str> {
    service.map(StreamingService::as_str)
}

fn context(facet_hint: ContextFacetHint, title: &str) -> ReleaseParseContext {
    ReleaseParseContext {
        facet_hint,
        title: ContextTitle {
            name: title.to_string(),
        },
        aliases: Vec::new(),
        known_years: Vec::new(),
        imdb_ids: Vec::new(),
        episodes: Vec::new(),
    }
}

fn context_with_episode_title(title: &str, episode_title: &str) -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Series, title);
    target.episodes.push(ContextEpisode {
        season: Some(2),
        episode: Some(4),
        title: Some(episode_title.to_string()),
        ..Default::default()
    });
    target
}

fn context_with_episode_alias(title: &str, episode_alias: &str) -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Series, title);
    target.episodes.push(ContextEpisode {
        season: Some(2),
        episode: Some(4),
        title_aliases: vec![episode_alias.to_string()],
        ..Default::default()
    });
    target
}

#[test]
fn lex_and_parse_standard_episode_release() {
    let analysis = analyze_release_for_target(
        "Show.Name.S01E02.1080p.WEB-DL.H264-Group",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(candidate.projected.normalized_title, "SHOW NAME");
    assert_eq!(candidate.projected.quality.as_deref(), Some("1080p"));
    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEB-DL")
    );
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::SingleEpisode)
    );
}

#[test]
fn parses_8k_av1_movie_release() {
    let analysis = analyze_release_for_target(
        "Movie.Title.2026.4320p.WEB-DL.AV1.AAC-GRP",
        &context(ContextFacetHint::Movie, "Movie Title"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.quality.as_deref(), Some("4320p"));
    assert_eq!(candidate.projected.video_codec, Some(VideoCodec::Av1));
}

#[test]
fn parses_sonarr_style_x_episode_release() {
    let analysis = analyze_release_for_target(
        "Show Name - 01x02 - The Episode WEBDL-1080p",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![2]);
}

#[test]
fn parses_numeric_series_title_with_time_window_episode_name() {
    let mut target = context(ContextFacetHint::Series, "13");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "13 (2024) - S02E01 - Day 2 800 A.M. 900 A.M. [WEBDL-1080p] [EAC3 5.1] [h265]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(candidate.projected.normalized_title, "13");
    assert_eq!(episode.season, Some(2));
    assert_eq!(episode.episode_numbers, vec![1]);
}

#[test]
fn parses_daily_release_with_part_marker() {
    let mut target = context(ContextFacetHint::Series, "Series Title");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Series.Title.2026.04.22.Part.2.720p.HULU.WEBRip.AAC2.0.H264-Group",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(
        episode.air_date,
        Some(NaiveDate::from_ymd_opt(2026, 4, 22).unwrap())
    );
    assert_eq!(episode.daily_part, Some(2));
}

#[test]
fn parser_accepts_unicode_separator_release() {
    let analysis = analyze_release_for_target(
        "Show–Name.S01E02.1080p.WEB-DL.H264-Group",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(candidate.projected.normalized_title, "SHOW NAME");
}

#[test]
fn context_episode_title_tokens_are_not_source_metadata() {
    let target = context_with_episode_title("Fixture Series", "Camera Token");
    let analysis = analyze_release_for_target(
        "Fixture.Series.S02E04.Camera.Token.1080p.WEB-DL.H264-Group",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEB-DL")
    );
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|evidence| evidence == "context:episode_title_hit")
    );
}

#[test]
fn real_cam_source_still_parses_as_cam() {
    let target = context_with_episode_title("Fixture Series", "Neutral Token");
    let analysis = analyze_release_for_target(
        "Fixture.Series.S02E04.Neutral.Token.1080p.CAM.H264-Group",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("CAM")
    );
}

#[test]
fn metadata_like_episode_alias_tokens_are_protected() {
    let target = context_with_episode_alias("Fixture Series", "CAM Token");
    let analysis = analyze_release_for_target(
        "Fixture.Series.S02E04.CAM.Token.1080p.WEB-DL.H264-Group",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEB-DL")
    );
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|evidence| evidence == "context:episode_title_hit")
    );
}

#[test]
fn single_token_source_like_alias_is_not_globally_protected() {
    let target = context_with_episode_alias("Fixture Series", "CAM");
    let analysis =
        analyze_release_for_target("Fixture.Series.S02E04.1080p.CAM.H264-Group", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("CAM")
    );
}

#[test]
fn parser_preserves_sanitize_hints_for_entities_and_controls() {
    let analysis = analyze_release_for_target(
        "AT&amp;T\u{200B}.Show.S01E02.1080p.WEB-DL.H264-Group",
        &context(ContextFacetHint::Series, "AT and T Show"),
    );

    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "html_entity_decoded")
    );
    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "zero_width_stripped")
    );
}

#[test]
fn bounds_token_role_hypotheses_and_marks_pruning() {
    let analysis = analyze_release_for_target(
        "S01E01.2024.1080p.MULTI.AVC",
        &context(ContextFacetHint::Series, "Placeholder"),
    );
    assert!(
        analysis
            .annotations
            .iter()
            .all(|annotation| annotation.alternate_roles.len() <= 2)
    );
    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "annotation:role_pruned")
            || analysis
                .annotations
                .iter()
                .all(|annotation| !annotation.role_pruned)
    );
}

#[test]
fn context_keeps_stacked_anime_aliases_as_title_variants() {
    let mut target = context(
        ContextFacetHint::Anime,
        "Silver Horizon Beyond Harbor's End",
    );
    target.aliases = vec![
        ContextAlias {
            name: "Sora no Vale".to_string(),
        },
        ContextAlias {
            name: "Silver Horizon Beyond the Vale".to_string(),
        },
    ];
    target.known_years.push(2023);

    let analysis = analyze_release_for_target(
        "[SubsPlease] Sora no Vale Silver Horizon Beyond the Vale - 01 [1080p] [HEVC]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert!(
        candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title: &String| title.contains("SORA NO VALE"))
    );
    assert!(
        candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title: &String| title.contains("SILVER HORIZON BEYOND THE VALE"))
    );
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|code| code == "context:title_alias_hit")
    );
}

#[test]
fn context_does_not_invent_absent_titles() {
    let mut target = context(ContextFacetHint::Series, "Completely Different Show");
    target.aliases = vec![ContextAlias {
        name: "Different Alias".to_string(),
    }];

    let analysis = analyze_release_for_target("Farwander.S08E05.1080p.WEB-DL", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "FARWANDER");
    assert!(
        !candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "COMPLETELY DIFFERENT SHOW")
    );
}

#[test]
fn context_match_retains_pre_projection_span_for_identity_proof() {
    let mut target = context(
        ContextFacetHint::Series,
        "The Quiet Meadow Blooms with Splendor",
    );
    target.aliases = vec![ContextAlias {
        name: "BLOOM".to_string(),
    }];

    let analysis = analyze_release_for_target("Electric.Bloom.S01E09.1080p.DSNP.WEB-DL", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let alias_match = candidate
        .context_title_matches
        .iter()
        .find(|context_match| context_match.kind == ContextTitleMatchKind::TitleAlias)
        .expect("BLOOM context match");

    assert_eq!(alias_match.normalized, "BLOOM");
    assert_eq!(alias_match.token_range.start_token, 1);
    assert_eq!(alias_match.token_range.end_token, 2);
    assert_eq!(candidate.zones.title_zones[0].start_token, 0);
    assert_eq!(candidate.zones.title_zones[0].end_token, 2);
}

#[test]
fn resource_limits_emit_hints() {
    let huge = "A".repeat(5000);
    let analysis = analyze_release_for_target(&huge, &context(ContextFacetHint::Movie, "Huge"));
    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "input_truncated")
    );
}

#[test]
fn targeted_single_context_parse_is_not_title_ambiguous() {
    let mut target = context(ContextFacetHint::Movie, "Neon Cipher");
    target.known_years.push(2010);

    let analysis =
        analyze_release_against_targets("Neon.Cipher.2010.1080p.BluRay.x264-GRP", &[target]);

    assert_eq!(analysis.best_target_index, Some(0));
    assert_eq!(analysis.ambiguity_margin(), i32::MAX);
    assert!(!analysis.is_ambiguous());
}

#[test]
fn targeted_empty_context_bank_has_no_best_target() {
    let analysis = analyze_release_against_targets("Neon.Cipher.2010.1080p.BluRay.x264-GRP", &[]);

    assert_eq!(analysis.best_target_index, None);
    assert!(analysis.is_ambiguous());
}

#[test]
fn episode_context_supplies_soft_absolute_signal() {
    let mut target = context(ContextFacetHint::Anime, "Emberfall");
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(330),
        title: Some("Emberfall".to_string()),
        ..Default::default()
    }];

    let analysis = analyze_release_for_target("[SubsPlease] Emberfall - 330 [1080p]", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|code| code == "context:absolute_mapping_hit")
    );
}

#[test]
fn movie_parser_extracts_year_and_source_from_compound_tokens() {
    let mut target = context(ContextFacetHint::Movie, "protector");
    target.known_years.push(2025);

    let analysis =
        analyze_release_for_target("protector.2025.108010bit.webri6ch.x265.hevc-psa", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::Movie);
    assert_eq!(candidate.projected.normalized_title, "PROTECTOR");
    assert_eq!(candidate.projected.year, Some(2025));
    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEBRip")
    );
}

#[test]
fn daily_parser_projects_air_date_year_and_normalizes_web_source() {
    let mut target = context(ContextFacetHint::Series, "The 9th Bell With Marlow Reed");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "The.9th.Bell.With.Marlow.Reed.2026.04.21.720p.WEB.x264-NGP",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(candidate.projected.year, Some(2026));
    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEB-DL")
    );
}

#[test]
fn range_pack_parser_handles_anime_batch_release() {
    let analysis = analyze_release_for_target(
        "Emberfall 1-366",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(candidate.projected.normalized_title, "EMBERFALL");
    assert_eq!(candidate.projected.audio, None);
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::RangePack)
    );
}

#[test]
fn anime_context_prefers_season_pack_with_trailing_episode_range() {
    let analysis = analyze_release_for_target(
        "Emberfall Season 12 - (213 - 229) [Typis]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::SeasonPack)
    );
}

#[test]
fn movie_context_avoids_daily_misparse_for_numeric_movie_title() {
    let mut target = context(
        ContextFacetHint::Movie,
        "Orbit 7 1 2 A Quiet Age Childhood Orbit 7 1 2 Sessiz Cagda Cocuk Olmak",
    );
    target.known_years.push(2022);

    let analysis = analyze_release_for_target(
        "Orbit.7.1.2.A.Quiet.Age.Childhood-Orbit.7.1.2.Sessiz.Cagda.Cocuk.Olmak.2022.Animasyon.1080p.NF.WEB-DL",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::Movie);
    assert_eq!(candidate.projected.year, Some(2022));
}

#[test]
fn series_context_supports_split_day_month_year_daily_release() {
    let mut target = context(ContextFacetHint::Series, "Yalimkan");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Yalimkan.29.Blm.21.04.2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(candidate.projected.year, Some(2026));
}

#[test]
fn series_context_supports_hyphenated_day_month_year_daily_release() {
    let mut target = context(ContextFacetHint::Series, "Yalimkan");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Yalimkan.29.Blm.21-04-2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(candidate.projected.year, Some(2026));
}

#[test]
fn split_season_episode_tokens_parse_as_standard_episode() {
    let analysis = analyze_release_for_target(
        "[Erai-raws] Irasshai Tsukikage Nagisa Shuurei no Koushitsu e S4-07 [1080p]",
        &context(
            ContextFacetHint::Anime,
            "Irasshai Tsukikage Nagisa Shuurei no Koushitsu e",
        ),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(4));
    assert_eq!(episode.episode_numbers, vec![7]);
}

#[test]
fn standard_episode_range_token_projects_multi_episode() {
    let analysis = analyze_release_for_target(
        "Umbra.Vector.S01E01-11.BDRemux.1080p",
        &context(ContextFacetHint::Anime, "Umbra Vector"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, (1..=11).collect::<Vec<_>>());
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::MultiEpisode);
}

#[test]
fn e_prefixed_episode_token_maps_to_season_one_episode() {
    let analysis = analyze_release_for_target(
        "Tide.Chart.E1158.1080p.WEB.H264",
        &context(ContextFacetHint::Series, "Tidebreaker"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1158]);
}

#[test]
fn season_pack_projection_preserves_single_season_number() {
    let analysis = analyze_release_for_target(
        "Clashing.Lanterns.S02.1080p.WEB-DL",
        &context(ContextFacetHint::Series, "Clashing Lanterns"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.season, Some(2));
}

#[test]
fn movie_title_keeps_numeric_tokens_that_are_part_of_the_name() {
    let mut target = context(ContextFacetHint::Movie, "Volt 30");
    target.known_years.push(2023);

    let analysis = analyze_release_for_target("Volt.30.[2023].720p.WEBRip-LAMA", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "VOLT 30");
}

#[test]
fn movie_title_keeps_hyphenated_words_before_metadata_boundary() {
    let mut target = context(ContextFacetHint::Movie, "Wellensang Veil Of Dusk");
    target.known_years.push(2024);

    let analysis =
        analyze_release_for_target("Wellensang-Veil.Of.Dusk.[2024].720p.WEBRip-LAMA", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "WELLENSANG VEIL OF DUSK"
    );
}

#[test]
fn bracketed_prefix_group_can_be_captured_as_release_group() {
    let analysis = analyze_release_for_target(
        "[SubsPlease] Emberfall - 330 [1080p]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("SubsPlease")
    );
}

#[test]
fn title_word_web_does_not_force_metadata_source_in_title_zone() {
    let mut target = context(ContextFacetHint::Movie, "The Web of Lies");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target("The.Web.of.Lies.2021.1080p.WEB-DL", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "THE WEB OF LIES");
}

#[test]
fn html_entity_and_ampersand_normalize_into_title_words() {
    let mut target = context(ContextFacetHint::Movie, "Echoes Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Echoes.Heard.&amp;.Seen.2021.2160p.NF.WEB-DL.DD+5.1.Atmos.H.265-playWEB",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "ECHOES HEARD AND SEEN"
    );
}

#[test]
fn connector_title_variants_split_aka_and_slash_titles() {
    let mut target = context(ContextFacetHint::Movie, "Mon Phare My Lighthouse");
    target.known_years.push(2020);

    let analysis = analyze_release_for_target(
        "Mon Phare / My Lighthouse 2020 1080p BluRay x264-GRP",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert!(
        projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "MON PHARE"),
        "{:?}",
        projected.normalized_title_variants
    );
    assert!(
        projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "MY LIGHTHOUSE"),
        "{:?}",
        projected.normalized_title_variants
    );

    let mut aka_target = context(ContextFacetHint::Movie, "Portmere A K A Hard Nine");
    aka_target.known_years.push(1996);
    let aka_analysis = analyze_release_for_target(
        "Portmere.A.K.A.Hard.Nine.1996.1080p.WEB-DL.H.264",
        &aka_target,
    );
    let aka_projected = &aka_analysis
        .best_candidate()
        .expect("best candidate")
        .projected;

    assert_eq!(aka_projected.normalized_title, "PORTMERE AKA HARD NINE");
    assert!(
        aka_projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "PORTMERE")
    );
    assert!(
        aka_projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "HARD NINE")
    );
}

#[test]
fn double_encoded_html_entity_normalizes_into_title_words() {
    let mut target = context(ContextFacetHint::Movie, "Echoes Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Echoes.Heard.&amp;amp;.Seen.2021.2160p.NF.WEB-DL.DD+5.1.Atmos.H.265-playWEB",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "ECHOES HEARD AND SEEN"
    );
}

#[test]
fn numeric_html_entity_normalizes_into_title_words() {
    let mut target = context(ContextFacetHint::Movie, "Echoes Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Echoes.Heard.&#x26;.Seen.2021.2160p.NF.WEB-DL.DD+5.1.Atmos.H.265-playWEB",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "ECHOES HEARD AND SEEN"
    );
}

#[test]
fn colon_separated_prefix_preserves_spaced_title_form() {
    let analysis = analyze_release_for_target(
        "[Judas] Ka:Nova kara Meguru Isekai Kikou - S04E03 [1080p]",
        &context(ContextFacetHint::Anime, "Ka Nova kara Meguru Isekai Kikou"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert!(
        candidate
            .projected
            .normalized_title
            .contains("KA NOVA KARA MEGURU ISEKAI KIKOU")
    );
}

#[test]
fn service_tagged_webrip_normalizes_to_webdl() {
    let analysis = analyze_release_for_target(
        "AWL.NXG.2026.04.21.NF.iNT.720p.WEBRip.H.264-HEEL",
        &context(ContextFacetHint::Series, "AWL NXG"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEB-DL")
    );
}

/// A season-only token under an episode-scoped search is a season pack.
///
/// This test used to pin the opposite: the context hint projected the searched
/// episode (`S04E03`) onto a name that only says `S04`. That inferred numbering
/// outranking explicit evidence is the defect of issue #170 — coverage
/// resolution maps a pack onto the searched episode downstream, so the parse
/// must report what the name says. The inferred reading survives as a
/// penalized, recorded fallback for names where nothing explicit parses.
#[test]
fn season_only_token_is_a_season_pack_even_under_an_episode_scoped_search() {
    let mut target = context(ContextFacetHint::Series, "Ironbound");
    target.known_years.push(2021);
    target.episodes = vec![ContextEpisode {
        season: Some(4),
        episode: Some(3),
        ..Default::default()
    }];

    let analysis = analyze_release_for_target(
        "Ironbound.2021.S04.1080p.AMZN.Webrip.AV1.10bit.EAC3.5.1-Goki.TAoE",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.season_numbers, vec![4]);
    assert!(episode.full_season);
    assert!(
        episode.episode_numbers.is_empty(),
        "the searched episode number appears nowhere in the name"
    );
    assert!(
        analysis.candidates.iter().any(|candidate| candidate
            .reasons
            .iter()
            .any(|reason| reason.code == "identity:inferred_from_context")),
        "the inferred reading is still recorded"
    );
}

#[test]
fn parenthesized_cjk_alt_title_and_prefix_group_do_not_pollute_primary_title() {
    let analysis = analyze_release_for_target(
        "[H3LL] Silver Horizon (銀界の地平線 第2期) - Beyond the Vale - S02E01 [1080p][x264 10bits][AAC][Multiple Subtitles].mkv",
        &context(ContextFacetHint::Anime, "Silver Horizon Beyond the Vale"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "SILVER HORIZON BEYOND THE VALE"
    );
    assert!(
        candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title| title.contains("SILVER HORIZON"))
    );
}

#[test]
fn empty_movie_title_falls_back_to_required_context_match_before_metadata_boundary() {
    let mut target = context(ContextFacetHint::Movie, "Denizhan");
    target.known_years.push(2019);

    let analysis =
        analyze_release_for_target("Denizhan.2019.Yerli.1080p.WEB-DL.x264.AAC-TSRG", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "DENIZHAN");
}

#[test]
fn fused_standard_episode_quality_suffix_is_split_correctly() {
    let analysis = analyze_release_for_target(
        "Barley.Works.S01E101080p.NF.WEB-DL.DDP5.1.H.264-SPWEB",
        &context(ContextFacetHint::Series, "Barley Works"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![10]);
}

#[test]
fn merged_alias_pattern_can_project_canonical_split_title() {
    let mut target = context(
        ContextFacetHint::Anime,
        "Ka Nova Drifting Through A Distant World",
    );
    target.aliases = vec![
        ContextAlias {
            name: "Ka Nova".to_string(),
        },
        ContextAlias {
            name: "KaNOVA".to_string(),
        },
    ];

    let analysis = analyze_release_for_target(
        "KaNOVA.Drifting.Through.A.Distant.World.S04E03.1080p.AMZN.WEB-DL",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "KA NOVA DRIFTING THROUGH A DISTANT WORLD"
    );
}

#[test]
fn enrichment_extracts_legacy_audio_and_hdr_fields_without_inventing_languages() {
    let mut target = context(ContextFacetHint::Movie, "Echoes Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Echoes.Heard.And.Seen.2021.2160p.NF.WEB-DL.DDP5.1.Atmos.DV.HDR10+.DUAL-AUDIO.MULTISUB.10bit.x265",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(audio_label(projected.audio.as_ref()), Some("DDP"));
    assert_eq!(audio_codec_labels(&projected.audio_codecs), vec!["DDP"]);
    assert_eq!(projected.audio_channels.as_deref(), Some("5.1"),);
    assert!(projected.is_atmos);
    assert!(projected.is_dolby_vision);
    assert!(projected.detected_hdr);
    assert!(projected.has_hdr_fallback);
    assert!(projected.is_hdr10plus);
    assert!(projected.is_10bit);
    assert!(projected.is_dual_audio);
    assert!(projected.languages_audio.is_empty());
}

#[test]
fn enrichment_does_not_treat_plain_hdr_or_hlg_as_hdr_fallback() {
    let mut target = context(ContextFacetHint::Movie, "Movie");
    target.known_years.push(2024);

    let hdr = analyze_release_for_target("Movie.2024.1080p.WEB-DL.HDR.x264", &target);
    let hdr_candidate = hdr.best_candidate().expect("best candidate");
    let hdr_enrichment = enrich_candidate(&hdr.tokens, hdr_candidate, &hdr.raw_input);
    let hdr_projected = project_final_metadata(hdr_candidate.projected.clone(), &hdr_enrichment);

    assert!(hdr_projected.detected_hdr);
    assert!(!hdr_projected.has_hdr_fallback);

    let hlg = analyze_release_for_target("Movie.2024.1080p.WEB-DL.HLG.x264", &target);
    let hlg_candidate = hlg.best_candidate().expect("best candidate");
    let hlg_enrichment = enrich_candidate(&hlg.tokens, hlg_candidate, &hlg.raw_input);
    let hlg_projected = project_final_metadata(hlg_candidate.projected.clone(), &hlg_enrichment);

    assert!(hlg_projected.is_hlg);
    assert!(!hlg_projected.has_hdr_fallback);
}

#[test]
fn enrichment_does_not_treat_atmosphere_title_word_as_atmos() {
    let mut target = context(ContextFacetHint::Movie, "Atmosphere");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target("Atmosphere.2024.1080p.WEB-DL.HDR.x264", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.normalized_title, "ATMOSPHERE");
    assert!(!projected.is_atmos);
}

#[test]
fn enrichment_extracts_split_dts_x_audio() {
    let mut target = context(ContextFacetHint::Movie, "Movie");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target("Movie.2024.2160p.BluRay.DTS-X.7.1.H.265", &target);
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(audio_label(projected.audio.as_ref()), Some("DTSX"));
    assert_eq!(projected.audio_channels.as_deref(), Some("7.1"));
}

#[test]
fn enrichment_extracts_split_eac3_without_matching_title_substrings() {
    let analysis = analyze_release_for_target(
        "[YukiSubs] Sora no Vale - 29 (S02E01) (WEB 1080p HEVC EAC-3).mkv",
        &context(ContextFacetHint::Anime, "Silver Horizon Beyond the Vale"),
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(audio_label(projected.audio.as_ref()), Some("EAC3"));

    let emberfall = analyze_release_for_target(
        "Emberfall 1-366",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let emberfall_projected = &emberfall
        .best_candidate()
        .expect("best candidate")
        .projected;

    assert_eq!(emberfall_projected.normalized_title, "EMBERFALL");
    assert_eq!(emberfall_projected.audio, None);
}

#[test]
fn enrichment_canonicalizes_french_language_codes_to_fra() {
    let mut target = context(ContextFacetHint::Movie, "Colette Marin");
    target.known_years.push(2000);

    let analysis = analyze_release_for_target(
        "Colette.Marin.2000.REMASTERED.VOSTFR.1080p.FRA.BluRay.REMUX.AVC.DTS-HD.MA.5.1-MAD",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["fra".to_string()]);
    assert_eq!(projected.languages_subtitles, vec!["fra".to_string()]);
}

#[test]
fn enrichment_scopes_language_before_subtitle_marker_to_subtitles() {
    let mut target = context(ContextFacetHint::Anime, "Clockwork Cat");
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(911),
        ..Default::default()
    }];

    let analysis = analyze_release_for_target(
        "[Ommex] Clockwork Cat - 911 [ENG-Sub][1080p x265 AAC]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_subtitles, vec!["eng".to_string()]);
    assert!(
        projected.languages_audio.is_empty(),
        "an English-subbed release must not claim English audio, got {:?}",
        projected.languages_audio
    );
}

#[test]
fn enrichment_extracts_affixed_language_before_video_anchor() {
    let analysis = analyze_release_for_target(
        "Rooftop.Neon.Shell.Squad.S05E07.HebDub.XviD",
        &context(ContextFacetHint::Series, "Rooftop Neon Shell Squad"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["heb".to_string()]);
    assert_eq!(projected.video_codec.as_ref(), Some(&VideoCodec::Xvid));
}

#[test]
fn enrichment_extracts_short_language_code_inside_metadata_zone() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-GRP",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["tur".to_string()]);
    assert_eq!(
        streaming_service_label(projected.streaming_service.as_ref()),
        Some("Disney+")
    );
    assert_eq!(audio_label(projected.audio.as_ref()), Some("AAC"));
    assert_eq!(projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn enrichment_extracts_named_language_before_release_group_suffix() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.1080p.BluRay.x265.DDP.5.1.English-GRP",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["eng".to_string()]);
    assert_eq!(projected.release_group.as_deref(), Some("GRP"));
}

#[test]
fn enrichment_extracts_english_dub_gap_group_after_episode_identity() {
    let analysis = analyze_release_for_target(
        "[Yameii] Hang In There, Matsuda-kun!! - S01E05 [English Dub] [CR WEB-DL 1080p H264 AAC] [994F6EBD] (Faito! Matsuda-kun!!)",
        &context(ContextFacetHint::Anime, "Hang In There Matsuda-kun"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["eng".to_string()]);
    assert!(projected.is_dubs_only);
}

#[test]
fn enrichment_extracts_release_flags_before_quality_anchor() {
    let analysis = analyze_release_for_target(
        "V33LO.New.Horizon.S01E17.DV.2160p.WEB.h265-EDITH",
        &context(ContextFacetHint::Series, "V33LO New Horizon"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert!(projected.is_dolby_vision);
    assert!(projected.detected_hdr);
}

#[test]
fn enrichment_extracts_fused_and_plural_10bit_markers() {
    let mut target = context(ContextFacetHint::Movie, "Protector");
    target.known_years.push(2025);

    let analysis =
        analyze_release_for_target("protector.2025.108010bit.webri6ch.x265.hevc-psa", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert!(projected.is_10bit);
}

#[test]
fn remux_is_projected_as_structural_flag_not_edition() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.2160p.BluRay.REMUX.HEVC.TrueHD.7.1",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert!(candidate.projected.is_remux);
    assert_ne!(candidate.projected.edition.as_deref(), Some("REMUX"));
}

#[test]
fn remux_survives_when_proper_appears_first() {
    let mut target = context(ContextFacetHint::Movie, "13 Bells");
    target.known_years.push(2014);

    let analysis = analyze_release_for_target(
        "13.Bells.2014.PROPER.BluRay.1080p.DTS-HD.MA.5.1.AVC.HYBRID.REMUX-FraMeSToR",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert!(projected.is_remux);
    assert!(projected.is_proper_upload);
}

#[test]
fn leading_bdmv_group_is_source_not_release_group() {
    let analysis = analyze_release_for_target(
        "[BDMV] Emberfall [BD-BOX] [SET 1- 9]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert!(projected.is_bd_disk);
    assert_ne!(projected.release_group.as_deref(), Some("BDMV"));
}

#[test]
fn split_dvd_rip_source_projects_as_dvd() {
    let analysis = analyze_release_for_target(
        "[EDG] EMBERFALL EP 1-30 [DVD RIP X264 Hi10]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("DVD")
    );
}

#[test]
fn range_pack_projects_absolute_range_without_doubling_episode_numbers() {
    let analysis = analyze_release_for_target(
        "[HorribleSubs] Harbor Wraith [01-12] [720p] [Batch]",
        &context(ContextFacetHint::Anime, "Harbor Wraith"),
    );
    let episode = analysis
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .as_ref()
        .expect("episode");

    assert!(episode.episode_numbers.is_empty());
    assert_eq!(
        episode.absolute_episode_numbers,
        (1..=12).collect::<Vec<_>>()
    );
}

#[test]
fn numbered_ova_projects_special_absolute_episode_number() {
    let analysis = analyze_release_for_target(
        "[DeadFish] Another Anime Show - 01 - OVA [BD][720p][AAC]",
        &context(ContextFacetHint::Anime, "Another Anime Show"),
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;
    let episode = projected.episode.as_ref().expect("episode");

    assert_eq!(projected.parse_family, ParseFamily::Special);
    assert_eq!(episode.special_kind, Some(crate::ParsedSpecialKind::Ova));
    assert_eq!(episode.special_absolute_episode_numbers, vec![1]);
}

#[test]
fn season_pack_range_sets_multi_season_contract_flag() {
    let analysis = analyze_release_for_target(
        "The.Regent.S01-S03.NORDiC.1080p.MAX.WEB-DL.H.265-NORViNE",
        &context(ContextFacetHint::Series, "The Regent"),
    );
    let episode = analysis
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .as_ref()
        .expect("episode");

    assert_eq!(episode.season, None);
    assert!(episode.full_season);
    assert!(episode.is_multi_season);
}

#[test]
fn series_pack_markers_cover_complete_and_multi_season_release_names() {
    let complete = analyze_release_for_target(
        "[GroupTag] Quiet Meridian Complete Series + Specials & Extras",
        &context(ContextFacetHint::Series, "Quiet Meridian"),
    );
    let complete_episode = complete
        .best_candidate()
        .and_then(|candidate| candidate.projected.episode.as_ref())
        .expect("complete series marker should project an episodic pack");
    assert!(complete_episode.is_series_pack);
    assert!(complete_episode.season_numbers.is_empty());

    let complete_with_extras = analyze_release_for_target(
        "[GT] No Map No Meridian Complete Series+OVAs+Movie [BD 1080p]",
        &context(ContextFacetHint::Series, "No Map No Meridian"),
    );
    assert!(
        complete_with_extras
            .best_candidate()
            .and_then(|candidate| candidate.projected.episode.as_ref())
            .is_some_and(|episode| episode.is_series_pack)
    );

    for (release, title, expected_seasons) in [
        (
            "[GroupTag] The Corners of My Study S01+S02+OVAs [BD 1080p]",
            "The Corners of My Study",
            vec![1, 2],
        ),
        (
            "[GroupTag] Salt and Signal (Seasons 01-02) [BD 1080p]",
            "Salt and Signal",
            vec![1, 2],
        ),
    ] {
        let analysis =
            analyze_release_for_target(release, &context(ContextFacetHint::Series, title));
        let episode = analysis
            .best_candidate()
            .and_then(|candidate| candidate.projected.episode.as_ref())
            .unwrap_or_else(|| panic!("{release}: {:?}", analysis.tokens));
        assert!(episode.is_series_pack, "{release}");
        assert_eq!(episode.season_numbers, expected_seasons, "{release}");
    }
}

#[test]
fn explicit_single_episode_markers_beat_series_pack_markers() {
    let analysis = analyze_release_for_target(
        "Show.S05E12.The.Complete.Series.720p",
        &context(ContextFacetHint::Series, "Show"),
    );
    let episode = analysis
        .best_candidate()
        .and_then(|candidate| candidate.projected.episode.as_ref())
        .expect("single episode should win over complete-series marker");
    assert_eq!(
        episode.release_type,
        crate::ParsedEpisodeReleaseType::SingleEpisode
    );
    assert_eq!(episode.season, Some(5));
    assert_eq!(episode.episode_numbers, vec![12]);

    for release in [
        "Show.S01-S03.S02E05.mkv",
        "Show.S01+S02.S02E05.1080p.WEB-DL",
    ] {
        let analysis =
            analyze_release_for_target(release, &context(ContextFacetHint::Series, "Show"));
        assert!(
            !analysis
                .best_candidate()
                .and_then(|candidate| candidate.projected.episode.as_ref())
                .is_some_and(|episode| episode.is_series_pack),
            "{release}: {:?}",
            analysis.tokens
        );
    }
}

#[test]
fn movie_ova_and_bare_complete_markers_are_not_series_packs() {
    for (release, title) in [
        (
            "The Harbor of Signals Complete Movie Series [BD 1080p]",
            "The Harbor of Signals",
        ),
        (
            "Meridian Marmalade Complete OVA Series [BD 1080p]",
            "Meridian Marmalade",
        ),
        ("Show Complete Collection [BD 1080p]", "Show"),
        ("Show Complete Original Series [BD 1080p]", "Show"),
        ("Show Complete Subbed Collection [BD 1080p]", "Show"),
        ("Show All Seasons [BD 1080p]", "Show"),
        (
            "Quiet Meridian Season 01 to 09 [BD 1080p]",
            "Quiet Meridian",
        ),
        ("Quietfall Complete [BD 1080p]", "Quietfall"),
        ("Show 01-24 [BD 1080p]", "Show"),
    ] {
        let analysis =
            analyze_release_for_target(release, &context(ContextFacetHint::Series, title));
        assert!(
            !analysis
                .best_candidate()
                .and_then(|candidate| candidate.projected.episode.as_ref())
                .is_some_and(|episode| episode.is_series_pack),
            "{release}"
        );
    }
}

#[test]
fn parenthetical_standard_identity_beats_prefixed_absolute_number() {
    let mut target = context(
        ContextFacetHint::Anime,
        "Silver Horizon Beyond Harbor's End",
    );
    target.aliases.push(ContextAlias {
        name: "Sora no Vale".to_string(),
    });

    let analysis = analyze_release_for_target(
        "[YukiSubs] Sora no Vale - 29 (S02E01) (WEB 1080p HEVC EAC-3).mkv",
        &target,
    );
    let episode = analysis
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .as_ref()
        .expect("episode");

    assert_eq!(episode.season, Some(2));
    assert_eq!(episode.episode_numbers, vec![1]);
}

#[test]
fn series_facet_with_episode_context_can_recover_absolute_episode() {
    let mut target = context(ContextFacetHint::Series, "Yalimkan");
    target.known_years.push(2026);
    target.episodes.push(ContextEpisode {
        season: None,
        episode: None,
        absolute_number: Some(29),
        air_date: None,
        title: None,
        title_aliases: Vec::new(),
    });

    let analysis = analyze_release_for_target(
        "Yalimkan.29.Blm.21.04.2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(episode.absolute_episode, Some(29));
    assert_eq!(candidate.projected.quality.as_deref(), Some("1080p"));
    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEB-DL")
    );
}

#[test]
fn movie_facet_with_episode_context_can_recover_absolute_episode() {
    let mut target = context(ContextFacetHint::Movie, "Yalimkan");
    target.known_years.push(2026);
    target.episodes.push(ContextEpisode {
        season: None,
        episode: None,
        absolute_number: Some(29),
        air_date: None,
        title: None,
        title_aliases: Vec::new(),
    });

    let analysis = analyze_release_for_target(
        "Yalimkan.29.Blm.21.04.2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(episode.absolute_episode, Some(29));
}

#[test]
fn service_tokens_project_to_canonical_service_names() {
    let mut target = context(ContextFacetHint::Movie, "Askari");
    target.known_years.push(2001);

    let analysis =
        analyze_release_for_target("askari.2001.amzn.web-dl.dd.2.0.h.264-playweb", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        streaming_service_label(candidate.projected.streaming_service.as_ref()),
        Some("Amazon")
    );
}

#[test]
fn distilled_service_tokens_beyond_the_legacy_list_project_to_their_service() {
    let mut series = context(ContextFacetHint::Series, "Umibe Signal");
    series.episodes.push(ContextEpisode {
        season: Some(1),
        episode: Some(1),
        ..Default::default()
    });
    let analysis = analyze_release_for_target("Umibe.Signal.S01.ABEMA.WEB-DL", &series);
    let candidate = analysis.best_candidate().expect("best candidate");
    assert_eq!(
        streaming_service_label(candidate.projected.streaming_service.as_ref()),
        Some("ABEMA")
    );

    let mut movie = context(ContextFacetHint::Movie, "Copper Kettle");
    movie.known_years.push(2019);
    let analysis = analyze_release_for_target("Copper.Kettle.2019.ITVX.WEB-DL", &movie);
    let candidate = analysis.best_candidate().expect("best candidate");
    assert_eq!(
        streaming_service_label(candidate.projected.streaming_service.as_ref()),
        Some("ITVX")
    );
}

#[test]
fn web_adjacent_service_tokens_need_the_web_marker_to_count() {
    // Upstream's own pattern for NOW is `\b(now)\b[ ._-]web[ ._-]?(dl|rip)?\b`,
    // so the token names the service only when a WEB marker follows it.
    let mut series = context(ContextFacetHint::Series, "Glass Harbor");
    series.episodes.push(ContextEpisode {
        season: Some(1),
        episode: Some(3),
        ..Default::default()
    });
    let analysis = analyze_release_for_target("Glass.Harbor.S01E03.NOW.WEB-DL.1080p", &series);
    let candidate = analysis.best_candidate().expect("best candidate");
    assert_eq!(
        streaming_service_label(candidate.projected.streaming_service.as_ref()),
        Some("NOW")
    );

    // The same word inside a title, with no WEB marker after it, is a title word.
    let mut movie = context(ContextFacetHint::Movie, "Now You Return");
    movie.known_years.push(2024);
    let analysis = analyze_release_for_target("Now.You.Return.2024.1080p.BluRay", &movie);
    let candidate = analysis.best_candidate().expect("best candidate");
    assert_eq!(
        streaming_service_label(candidate.projected.streaming_service.as_ref()),
        None
    );
}

#[test]
fn web_adjacent_service_token_inside_a_title_does_not_claim_the_release() {
    // `IT` is upstream's iTunes tag, but only next to a WEB marker. Here the
    // release *is* a WEB-DL, so a policy-free lookup would tag it iTunes on the
    // strength of a title word four tokens earlier.
    let mut movie = context(ContextFacetHint::Movie, "It Rains In Portmere");
    movie.known_years.push(2024);
    let analysis = analyze_release_for_target("It.Rains.In.Portmere.2024.1080p.WEB-DL", &movie);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        streaming_service_label(candidate.projected.streaming_service.as_ref()),
        None
    );
    assert_eq!(candidate.projected.normalized_title, "IT RAINS IN PORTMERE");
}

#[test]
fn unicode_case_and_out_of_corpus_metadata_parse_without_fixture_bias() {
    let mut target = context(ContextFacetHint::Movie, "Éclair Monstra");
    target.known_years.push(2024);

    let analysis =
        analyze_release_for_target("éclair.monstra.2024.576p.WEB-DL.VVC.OPUS.2.0-GRP", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    // Normalized titles are accent-folded matching keys; raw display fields
    // keep their accents.
    assert_eq!(candidate.projected.normalized_title, "ECLAIR MONSTRA");
    assert_eq!(candidate.projected.quality.as_deref(), Some("576p"));
    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::Vvc)
    );
    assert_eq!(
        audio_label(candidate.projected.audio.as_ref()),
        Some("OPUS")
    );
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn fused_standard_episode_suffix_accepts_unseen_resolution() {
    let analysis = analyze_release_for_target(
        "Out.Show.S01E01576p.WEB-DL.VVC-Group",
        &context(ContextFacetHint::Series, "Out Show"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_eq!(candidate.projected.quality.as_deref(), Some("576p"));
}

#[test]
fn generic_external_ids_parse_beyond_imdb() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.1080p.WEB-DL.TMDB.12345.TVDB.67890.IMDB.tt7654321",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.imdb_id.as_deref(), Some("tt7654321"));
    assert_eq!(projected.tmdb_id.as_deref(), Some("12345"));
    assert_eq!(projected.tvdb_id.as_deref(), Some("67890"));
    assert!(
        projected
            .external_ids
            .iter()
            .any(|id| { id.source == ExternalIdSource::Tmdb && id.value == "12345" })
    );
    assert!(
        projected
            .external_ids
            .iter()
            .any(|id| { id.source == ExternalIdSource::Tvdb && id.value == "67890" })
    );
}

#[test]
fn numeric_anime_title_uses_context_before_absolute_episode() {
    let mut target = context(ContextFacetHint::Anime, "77");
    target.episodes.push(ContextEpisode {
        absolute_number: Some(11),
        ..Default::default()
    });

    let analysis = analyze_release_for_target("77 - 11 [1080p]", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.projected.normalized_title, "77");
    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert_eq!(episode.absolute_episode_numbers, vec![11]);
}

#[test]
fn short_service_alias_can_remain_a_title_word() {
    let mut target = context(ContextFacetHint::Movie, "Max Lantern");
    target.known_years.push(1985);

    let analysis = analyze_release_for_target("Max.Lantern.1985.480p.DVD.MPEG2.MP3-GRP", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "MAX LANTERN");
    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::Mpeg2)
    );
    assert_eq!(audio_label(candidate.projected.audio.as_ref()), Some("MP3"));
}

#[test]
fn release_group_preserves_dotted_and_hyphenated_suffixes() {
    let mut target = context(ContextFacetHint::Movie, "The Moment");
    target.known_years.push(2026);

    let analysis =
        analyze_release_for_target("the.moment.2026.1080p.bluray.x264.aac5.1-yts.bz", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("yts.bz"));
}

#[test]
fn leading_fansub_group_beats_trailing_batch_group() {
    let analysis = analyze_release_for_target(
        "[HorribleSubs] Harbor Wraith [01-12] [720p] [Batch]",
        &context(ContextFacetHint::Anime, "Harbor Wraith"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("HorribleSubs")
    );
}

#[test]
fn suffix_group_after_episode_subtitle_beats_earlier_hyphen_text() {
    let analysis = analyze_release_for_target(
        "Barns.Beneath.the.Gavel.S28E75-One.Man.and.His.Cart.WEB-DL.H.264-W45Ps",
        &context(ContextFacetHint::Series, "Barns Beneath the Gavel"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("W45Ps"));
}

#[test]
fn suffix_group_ignores_parenthetical_alt_title_and_language_markers() {
    let analysis = analyze_release_for_target(
        "Silver Horizon.Beyond.Journeys.End.S02E01.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG.(Sousou.no.Silver Horizon.Multi-Subs)",
        &context(ContextFacetHint::Anime, "Silver Horizon Beyond the Vale"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("VARYG"));
}

#[test]
fn suffix_group_does_not_capture_hyphenated_words_in_trailing_title() {
    let analysis = analyze_release_for_target(
        "Emberfall.S17E11.720p.DSNP.WEB-DL.AAC2.0.H.264-PiroRips.mkv (Emberfall - Iron Eclipse)",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("PiroRips")
    );
}

#[test]
fn release_group_skips_embedded_suffix_inside_large_metadata_bracket() {
    let mut target = context(ContextFacetHint::Movie, "The Fieldhands");
    target.known_years.push(2023);

    let analysis = analyze_release_for_target(
        "The.Fieldhands.[2023].[1080p.BluRay.x265.SDR.DDP.5.1.Dual-DarQ.HONE]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group, None);
}

#[test]
fn release_group_preserves_short_hyphenated_prefix() {
    let mut target = context(ContextFacetHint::Movie, "Nilavu Maalai Ulagam");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Nilavu.Maalai.Ulagam.2026.Tamil.2160p.SNXT.WEB-DL.DDP5.1.H.265-PMi-XDMovies",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("PMi-XDMovies")
    );
}

#[test]
fn release_group_uses_terminal_component_for_two_part_p2p_suffix() {
    let mut target = context(ContextFacetHint::Series, "Ironbound");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Ironbound.2021.S04E07.DONT.DO.ANYTHING.RASH.1080p.AMZN.Webrip.AV1.10bit.EAC3.5.1-Goki-TAoE",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("TAoE"));
}

#[test]
fn bracketed_short_hyphenated_release_group_preserves_both_parts() {
    let analysis = analyze_release_for_target(
        "Emberfall - 224 - 3 vs 1 Battle! Rangiku's Crisis [C-W].avi",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("C-W"));
}

#[test]
fn terminal_language_adjacent_token_can_be_release_group() {
    let mut target = context(
        ContextFacetHint::Movie,
        "Yarindan Kalan Izler Echoes of Him",
    );
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Yarindan.Kalan.Izler.Echoes.of.Him.2026.WEBDLRip.m1080p.X265.10bit.AAC.5.1.Turkce.TurkSeeD",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("TurkSeeD")
    );
}

#[test]
fn compound_source_suffix_is_not_captured_as_release_group() {
    let mut target = context(ContextFacetHint::Movie, "Askari");
    target.known_years.push(2001);

    let analysis =
        analyze_release_for_target("askari.2001.amzn.web-dl.dd.2.0.h.264-playWEB", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("playWEB")
    );
}

#[test]
fn enrichment_fills_split_video_codec_and_audio_channels() {
    let mut target = context(ContextFacetHint::Movie, "Kestrel of the Reeds");
    target.known_years.push(1998);

    let analysis = analyze_release_for_target(
        "Kestrel.of.the.Reeds.1998.DVDRip.HebDub.AAC2.0.H.264-T00LBAR",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::H264)
    );
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn parser_canonicalizes_h264_family_video_codec_tokens() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    for raw in [
        "Movie.Title.2024.1080p.BluRay.h264-GRP",
        "Movie.Title.2024.1080p.BluRay.x264-GRP",
        "Movie.Title.2024.1080p.BluRay.AVC-GRP",
    ] {
        let analysis = analyze_release_for_target(raw, &target);
        let candidate = analysis.best_candidate().expect("best candidate");

        assert_eq!(
            candidate.projected.video_codec.as_ref(),
            Some(&VideoCodec::H264),
            "{raw}"
        );
    }
}

#[test]
fn parser_canonicalizes_h265_family_video_codec_tokens() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    for raw in [
        "Movie.Title.2024.2160p.WEB-DL.hevc-GRP",
        "Movie.Title.2024.2160p.WEB-DL.h265-GRP",
        "Movie.Title.2024.2160p.WEB-DL.x265-GRP",
    ] {
        let analysis = analyze_release_for_target(raw, &target);
        let candidate = analysis.best_candidate().expect("best candidate");

        assert_eq!(
            candidate.projected.video_codec.as_ref(),
            Some(&VideoCodec::H265),
            "{raw}"
        );
    }
}

#[test]
fn enrichment_extracts_split_dts_ma_audio() {
    let mut target = context(ContextFacetHint::Movie, "The Warden of Ash");
    target.known_years.push(1939);

    let analysis = analyze_release_for_target(
        "The.Warden.of.Ash.1939.2160p.MA.WEB-DL.DTS-HD.MA.5.1.H.265-FLUX",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        audio_label(candidate.projected.audio.as_ref()),
        Some("DTSMA")
    );
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn enrichment_extracts_fused_dts_hd_audio() {
    let mut target = context(ContextFacetHint::Movie, "Boiler Shift");
    target.known_years.push(1990);

    let analysis = analyze_release_for_target(
        "Boiler.Shift.1990.REMASTERED.1080p.BluRay.REMUX.Dts-HDMa5.1.AVC-d3g",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        audio_label(candidate.projected.audio.as_ref()),
        Some("DTSHD")
    );
}

#[test]
fn enrichment_extracts_bare_dd_with_split_channels() {
    let mut target = context(ContextFacetHint::Movie, "Askari");
    target.known_years.push(2001);

    let analysis =
        analyze_release_for_target("askari.2001.amzn.web-dl.dd.2.0.h.264-playweb", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(audio_label(candidate.projected.audio.as_ref()), Some("AC3"));
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn standalone_channel_count_without_audio_codec_is_not_projected() {
    let mut target = context(ContextFacetHint::Movie, "Tow");
    target.known_years.push(2025);

    let analysis =
        analyze_release_for_target("tow.2025.1080p.10biwebrip.6ch.x265.hevc-psa", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.audio_channels, None);
}

#[test]
fn labeled_episode_range_stays_multi_episode() {
    let analysis = analyze_release_for_target(
        "[EDG] EMBERFALL EP 1-30 [DVD R2 X264 Hi10]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::RangePack)
    );
}

#[test]
fn labeled_single_episode_does_not_become_range_pack() {
    let mut target = context(ContextFacetHint::Anime, "Clockwork Cat");
    target.known_years.push(2005);
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(911),
        ..Default::default()
    }];

    let analysis = analyze_release_for_target(
        "[Ommex] Clockwork Cat (2005) Episode 911 [ENG SUB][1080p x265 AAC]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::SingleEpisode)
    );
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.absolute_episode_numbers.clone()),
        Some(vec![911])
    );
}

fn midnight_alloy_context() -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Anime, "Midnight Alloy Dark Signal");
    target.aliases = vec![
        ContextAlias {
            name: "Midnight Alloy Dark".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy Dark Signal".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy Kage Requiem".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy".to_string(),
        },
    ];
    target.known_years.push(2022);
    target
}

#[test]
fn midnight_alloy_part_one_and_two_release_projects_full_season_pack() {
    let analysis = analyze_release_for_target(
        "[Studio Nova] MIDNIGHT ALLOY Dark Signal (Season 1) [Part 1 + Part 2] [Dual Audio] [1080p][HEVC 10bit x265][AAC][Multi Sub] [Batch]",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::SeasonPack);
    assert_eq!(episode.season, Some(1));
    assert!(episode.full_season);
    assert!(!episode.is_partial_season);
}

#[test]
fn midnight_alloy_standalone_part_two_release_projects_partial_season_pack() {
    let analysis = analyze_release_for_target(
        "[EMBER] MIDNIGHT ALLOY‼ Dark Signal (2022) (Season 1 | Part 02) [1080p] [Dual Audio HEVC 10 bits WEBRip AAC] (Midnight Alloy Kage Requiem) (Batch)",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::SeasonPack);
    assert_eq!(episode.season, Some(1));
    assert!(episode.is_partial_season);
    assert_eq!(episode.season_part, Some(2));
}

#[test]
fn midnight_alloy_tilde_absolute_range_projects_absolute_episode_numbers() {
    let analysis = analyze_release_for_target(
        "[Erai-raws] Midnight Alloy Kage no Requiem (2022) - 01 ~ 13 [1080p][Multiple Subtitle]",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(episode.season, None);
    assert!(episode.episode_numbers.is_empty());
    assert_eq!(
        episode.absolute_episode_numbers,
        (1..=13).collect::<Vec<_>>()
    );
}

#[test]
fn midnight_alloy_labeled_absolute_range_projects_absolute_episode_numbers() {
    let analysis = analyze_release_for_target(
        "MIDNIGHT ALLOY -Dark Signal- Episodes 14-24 | Midnight Alloy Kage no Requiem [Dual][1080p] - E.N.D (English Dub | Japanese Dub)",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(episode.season, None);
    assert_eq!(
        episode.absolute_episode_numbers,
        (14..=24).collect::<Vec<_>>()
    );
}

#[test]
fn midnight_alloy_season_scoped_labeled_range_projects_episode_numbers() {
    let analysis = analyze_release_for_target(
        "[Anime Chap] MIDNIGHT ALLOY‼ Dark Signal 2022 - Season 1 (ONA) [WEB 1080p] {OP & ED Lyrics} Improved Subs (Episode 1 - 13) {Batch}",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, (1..=13).collect::<Vec<_>>());
    assert!(episode.absolute_episode_numbers.is_empty());
}

fn starfall_iron_eclipse_context() -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Anime, "Starfall Iron Eclipse");
    target.aliases = vec![
        ContextAlias {
            name: "Starfall".to_string(),
        },
        ContextAlias {
            name: "Starfall - Iron Eclipse".to_string(),
        },
        ContextAlias {
            name: "Starfall: Iron Eclipse".to_string(),
        },
    ];
    target.known_years.push(2022);
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(14),
        title: Some("The Last 9 Signals".to_string()),
        ..Default::default()
    }];
    target
}

#[test]
fn anime_absolute_release_keeps_release_group_and_metadata_boundaries() {
    let analysis = analyze_release_for_target(
        "[Studio Nova] Starfall - Iron Eclipse - 014 - The Last 9 Signals [BD][1080p][HEVC 10bit x265][AAC] [Dual Audio][ENG Subs]",
        &starfall_iron_eclipse_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert_eq!(episode.absolute_episode, Some(14));
    assert_eq!(episode.absolute_episode_numbers, vec![14]);
    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("Studio Nova")
    );
    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("BluRay")
    );
    assert_eq!(candidate.projected.quality.as_deref(), Some("1080p"));
    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::H265)
    );
    assert!(
        candidate
            .projected
            .audio_codecs
            .iter()
            .any(|codec| codec.as_str() == "AAC")
    );
}

#[test]
fn dotted_hyphen_split_season_episode_parses_as_standard_episode() {
    let analysis = analyze_release_for_target(
        "[SubsPlease] Emberfall S3.-.01.(1080p).[F00DBABE].mkv",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(3));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_eq!(
        episode.release_type,
        ParsedEpisodeReleaseType::SingleEpisode
    );
    assert!(!episode.full_season);
    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("SubsPlease")
    );
    assert_eq!(candidate.projected.quality.as_deref(), Some("1080p"));
}

#[test]
fn dot_split_season_episode_parses_as_standard_episode() {
    let analysis = analyze_release_for_target(
        "Show.Name.S3.01.1080p.WEB-DL.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(3));
    assert_eq!(episode.episode_numbers, vec![1]);
}

#[test]
fn hyphen_split_season_episode_still_parses() {
    let analysis = analyze_release_for_target(
        "Show.Name.S3-01.1080p.WEB-DL.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(3));
    assert_eq!(episode.episode_numbers, vec![1]);
}

#[test]
fn dot_split_guard_keeps_true_season_packs_and_years() {
    let pack = analyze_release_for_target(
        "Clashing.Lanterns.S02.1080p.WEB-DL",
        &context(ContextFacetHint::Series, "Clashing Lanterns"),
    );
    let pack_candidate = pack.best_candidate().expect("best candidate");
    assert_eq!(pack_candidate.family, ParseFamily::SeasonPack);

    let mut year_target = context(ContextFacetHint::Series, "Show Name");
    year_target.known_years.push(2024);
    let year = analyze_release_for_target("Show.Name.S02.2024.1080p.WEB-DL.x264-GRP", &year_target);
    let year_candidate = year.best_candidate().expect("best candidate");
    let year_episode = year_candidate.projected.episode.as_ref().expect("episode");
    assert_eq!(year_candidate.family, ParseFamily::SeasonPack);
    assert_eq!(year_episode.episode_numbers, Vec::<u32>::new());
    assert_eq!(year_candidate.projected.year, Some(2024));

    let resolution = analyze_release_for_target(
        "Show.Name.S02.720.WEB.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let resolution_candidate = resolution.best_candidate().expect("best candidate");
    assert_eq!(resolution_candidate.family, ParseFamily::SeasonPack);
}

#[test]
fn season_keyword_with_dotted_episode_parses_as_standard_episode() {
    let analysis = analyze_release_for_target(
        "Show.Name.Season.3.-.01.1080p.WEB-DL.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(3));
    assert_eq!(episode.episode_numbers, vec![1]);
}

#[test]
fn fused_season_token_parses_as_season_pack() {
    let analysis = analyze_release_for_target(
        "Show.Name.Season1.1080p.WEB-DL.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.season, Some(1));
    assert!(episode.full_season);
}

#[test]
fn season_like_title_words_do_not_mint_seasons() {
    let mut target = context(ContextFacetHint::Movie, "Seven Seasons");
    target.known_years.push(2024);
    let analysis = analyze_release_for_target("Seven.Seasons.2024.1080p.WEB-DL.x264-GRP", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::Movie);
    assert_eq!(candidate.projected.normalized_title, "SEVEN SEASONS");

    let fused_year = analyze_release_for_target(
        "Racing.League.Season2024.1080p.WEB-DL.x264-GRP",
        &context(ContextFacetHint::Series, "Racing League"),
    );
    let fused_candidate = fused_year.best_candidate().expect("best candidate");
    assert_ne!(
        fused_candidate
            .projected
            .episode
            .as_ref()
            .and_then(|episode| episode.season),
        Some(2024)
    );
}

#[test]
fn month_name_daily_dates_parse_in_both_orders() {
    let mut target = context(ContextFacetHint::Series, "Night Signal");
    target.known_years.push(2026);

    let year_first =
        analyze_release_for_target("Night.Signal.2026.Jan.05.720p.WEB.x264-GRP", &target);
    let year_first_episode = year_first
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(
        year_first_episode.air_date,
        Some(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap())
    );

    let day_first =
        analyze_release_for_target("Night.Signal.05.Jan.2026.720p.WEB.x264-GRP", &target);
    let day_first_episode = day_first
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(
        day_first_episode.air_date,
        Some(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap())
    );
}

#[test]
fn fused_eight_digit_air_date_parses_as_daily() {
    let mut target = context(ContextFacetHint::Series, "Show Name");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target("Show.Name.20260105.720p.WEB.x264-GRP", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(
        episode.air_date,
        Some(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap())
    );
}

#[test]
fn eight_digit_tokens_that_are_not_dates_stay_non_daily() {
    let analysis = analyze_release_for_target(
        "Show.Name.12345678.720p.WEB.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .and_then(|episode| episode.air_date),
        None
    );
}

#[test]
fn dot_split_episode_with_hyphen_range_projects_multi_episode() {
    let analysis = analyze_release_for_target(
        "Show.Name.S3.01-02.1080p.WEB.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(3));
    assert_eq!(episode.episode_numbers, vec![1, 2]);
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::MultiEpisode);
}

#[test]
fn season_dash_episode_with_range_tail_projects_full_span() {
    let analysis = analyze_release_for_target(
        "[Grp] Emberfall - Season 1 - 001-020 (1080p x264 AAC)",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, (1..=20).collect::<Vec<_>>());
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::MultiEpisode);
}

#[test]
fn hyphen_split_episode_with_range_tail_projects_full_span() {
    let analysis = analyze_release_for_target(
        "Show.Name.S3-01-02.1080p.WEB.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(3));
    assert_eq!(episode.episode_numbers, vec![1, 2]);
}

#[test]
fn bracketed_decimal_hash_that_forms_a_date_stays_a_checksum() {
    let analysis = analyze_release_for_target(
        "Show.Name.S01E05.1080p.WEB.x264.[20261204]",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.episode_numbers, vec![5]);
    assert_eq!(episode.air_date, None);
}

#[test]
fn separated_fps_word_requires_a_standard_frame_rate() {
    let mut target = context(ContextFacetHint::Movie, "Top 10 FPS Games");
    target.known_years.push(2024);
    let analysis = analyze_release_for_target("Top.10.FPS.Games.2024.1080p.WEB.x264-GRP", &target);
    let projected = &analysis.best_candidate().expect("best candidate").projected;
    assert_eq!(projected.fps, None);

    let mut rate_target = context(ContextFacetHint::Movie, "Movie Title");
    rate_target.known_years.push(2024);
    let decimal = analyze_release_for_target(
        "Movie.Title.2024.1080p.23.976.fps.WEB-DL.x264-GRP",
        &rate_target,
    );
    let decimal_projected = &decimal.best_candidate().expect("best candidate").projected;
    assert_eq!(decimal_projected.fps, Some(23.976));

    let separated = analyze_release_for_target(
        "Movie.Title.2024.1080p.60.fps.WEB-DL.x264-GRP",
        &rate_target,
    );
    let separated_projected = &separated
        .best_candidate()
        .expect("best candidate")
        .projected;
    assert_eq!(separated_projected.fps, Some(60.0));
}

#[test]
fn title_words_containing_cam_or_web_are_not_sources() {
    let analysis = analyze_release_for_target(
        "Show.Name.S01E06.I.Became.Human.and.Got.My.Butt.Kicked.1080p.HIDIVE.WEB-DL.DUAL.AAC2.0.H.264.ESub-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    assert_eq!(
        source_label(candidate.projected.source.as_ref()),
        Some("WEB-DL")
    );

    let cobweb = analyze_release_for_target(
        "Show.Name.S01E05.Cobweb.Theory.1080p.HDTV.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let cobweb_candidate = cobweb.best_candidate().expect("best candidate");
    assert_eq!(
        source_label(cobweb_candidate.projected.source.as_ref()),
        Some("HDTV")
    );

    let camera = analyze_release_for_target(
        "Show.Name.S01E05.Camera.Shy.1080p.HDTV.x264-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let camera_candidate = camera.best_candidate().expect("best candidate");
    assert_eq!(
        source_label(camera_candidate.projected.source.as_ref()),
        Some("HDTV")
    );
}

#[test]
fn explicit_cam_variants_still_parse_as_cam_sources() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    for raw in [
        "Movie.Title.2024.CAMRip.x264-GRP",
        "Movie.Title.2024.HDCAM.x264-GRP",
        "Movie.Title.2024.HQCAM.x264-GRP",
        "Movie.Title.2024.1080p.CAM.x264-GRP",
    ] {
        let analysis = analyze_release_for_target(raw, &target);
        let candidate = analysis.best_candidate().expect("best candidate");
        assert_eq!(
            source_label(candidate.projected.source.as_ref()),
            Some("CAM"),
            "{raw}"
        );
    }
}

#[test]
fn dolby_vision_alone_is_not_an_hdr_fallback() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2021);

    let dv_only = analyze_release_for_target(
        "Movie.Title.2021.2160p.DSNP.WEB-DL.DDP5.1.DV.H.265-GRP",
        &target,
    );
    let dv_projected = &dv_only.best_candidate().expect("best candidate").projected;
    assert!(dv_projected.is_dolby_vision);
    assert!(dv_projected.detected_hdr);
    assert!(!dv_projected.has_hdr_fallback);

    let hdr10 = analyze_release_for_target(
        "Movie.Title.2021.2160p.WEB-DL.DTS-HD.MA.7.1.HDR10.x265.10bit-GRP",
        &target,
    );
    let hdr10_projected = &hdr10.best_candidate().expect("best candidate").projected;
    assert!(hdr10_projected.detected_hdr);
    assert!(hdr10_projected.has_hdr_fallback);
    assert!(!hdr10_projected.is_dolby_vision);

    let dv_with_fallback =
        analyze_release_for_target("Movie.Title.2021.2160p.WEB-DL.DV.HDR10.H.265-GRP", &target);
    let both_projected = &dv_with_fallback
        .best_candidate()
        .expect("best candidate")
        .projected;
    assert!(both_projected.is_dolby_vision);
    assert!(both_projected.has_hdr_fallback);

    // "DoVi HDR" (either order) declares an HDR10 base under DV.
    for raw in [
        "Movie.Title.2021.2160p.NF.WEB-DL.DD+5.1.Atmos.DoVi.HDR.H.265-GRP",
        "Movie.Title.2021.4K.HDR.DV.2160p.BDRemux.x265-GRP",
    ] {
        let analysis = analyze_release_for_target(raw, &target);
        let projected = &analysis.best_candidate().expect("best candidate").projected;
        assert!(projected.is_dolby_vision, "{raw}");
        assert!(projected.has_hdr_fallback, "{raw}");
    }
}

#[test]
fn fully_hyphenated_dts_hd_ma_extracts_codec_and_channels() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2025);

    let analysis = analyze_release_for_target(
        "Movie.Title.2025.DUAL.1080p.BluRay.REMUX.AVC.DTS-HD-MA.5.1-GRP",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(audio_label(projected.audio.as_ref()), Some("DTSMA"));
    assert_eq!(projected.audio_channels.as_deref(), Some("5.1"));
    assert_eq!(projected.release_group.as_deref(), Some("GRP"));
}

#[test]
fn accented_context_title_matches_ascii_release_without_aliases() {
    let analysis = analyze_release_for_target(
        "Kelune.S20E15.1080p.WEB-DL.AAC2.0.H.264-GRP",
        &context(ContextFacetHint::Anime, "Kelúne"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.projected.normalized_title, "KELUNE");
    assert_eq!(episode.season, Some(20));
    assert_eq!(episode.episode_numbers, vec![15]);
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|code| code == "context:title_canonical_hit"),
        "accent-folded canonical title should match without aliases: {:?}",
        candidate.context_evidence
    );
}

#[test]
fn accented_release_matches_ascii_context_title() {
    let analysis = analyze_release_for_target(
        "Kelúne.S20E15.1080p.WEB-DL.AAC2.0.H.264-GRP",
        &context(ContextFacetHint::Anime, "Kelune"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "KELUNE");
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|code| code == "context:title_canonical_hit")
    );
}

#[test]
fn non_decomposable_letters_fold_for_matching() {
    let analysis = analyze_release_for_target(
        "Vorndby.Stories.2024.1080p.WEB-DL.x264-GRP",
        &context(ContextFacetHint::Movie, "Vørndby Stories"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "VORNDBY STORIES");
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|code| code == "context:title_canonical_hit")
    );
}

#[test]
fn proper_is_a_revision_flag_not_an_edition() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis =
        analyze_release_for_target("Movie.Title.2024.1080p.PROPER.WEB-DL.x264-GRP", &target);
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.edition, None);
    assert!(projected.is_proper_upload);
    assert!(!projected.is_repack);

    let repack =
        analyze_release_for_target("Movie.Title.2024.1080p.REPACK.WEB-DL.x264-GRP", &target);
    let repack_projected = &repack.best_candidate().expect("best candidate").projected;

    assert_eq!(repack_projected.edition, None);
    assert!(repack_projected.is_proper_upload);
    assert!(repack_projected.is_repack);
}

#[test]
fn proper_does_not_shadow_a_real_edition() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.PROPER.EXTENDED.1080p.WEB-DL.x264-GRP",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.edition.as_deref(), Some("Extended"));
    assert!(projected.is_proper_upload);
}

#[test]
fn beam_editions_project_with_canonical_casing() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis =
        analyze_release_for_target("Movie.Title.2024.UNCUT.1080p.WEB-DL.x264-GRP", &target);
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.edition.as_deref(), Some("Uncut"));
}

#[test]
fn fps_is_detected_in_dot_separated_names() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis =
        analyze_release_for_target("Movie.Title.2024.1080p.60fps.WEB-DL.x264-GRP", &target);
    let projected = &analysis.best_candidate().expect("best candidate").projected;
    assert_eq!(projected.fps, Some(60.0));

    let upscaled =
        analyze_release_for_target("Movie.Title.2024.2160p.144fps.WEB-DL.x264-GRP", &target);
    let upscaled_projected = &upscaled.best_candidate().expect("best candidate").projected;
    assert_eq!(upscaled_projected.fps, Some(144.0));
    assert!(upscaled_projected.is_ai_enhanced);
}

#[test]
fn bare_fps_word_without_adjacent_rate_is_not_fps_metadata() {
    let mut target = context(ContextFacetHint::Movie, "Silent Signal Shooter");
    target.known_years.push(1997);

    let analysis = analyze_release_for_target(
        "Silent.Signal.Shooter.1997.FPS.1080p.WEB-DL.x264-GRP",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.fps, None);
}

#[test]
fn episode_raw_uses_stable_renderings() {
    let absolute = analyze_release_for_target(
        "[Grp] Show Name - 05v2 [1080p]",
        &context(ContextFacetHint::Anime, "Show Name"),
    );
    let absolute_episode = absolute
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(absolute_episode.raw.as_deref(), Some("05v2"));

    let mut daily_target = context(ContextFacetHint::Series, "Series Title");
    daily_target.known_years.push(2026);
    let daily = analyze_release_for_target(
        "Series.Title.2026.04.22.720p.HULU.WEBRip.AAC2.0.H264-Group",
        &daily_target,
    );
    let daily_episode = daily
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(daily_episode.raw.as_deref(), Some("2026-04-22"));

    let pack = analyze_release_for_target(
        "Clashing.Lanterns.S02.1080p.WEB-DL",
        &context(ContextFacetHint::Series, "Clashing Lanterns"),
    );
    let pack_episode = pack
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(pack_episode.raw.as_deref(), Some("S02"));

    let multi_pack = analyze_release_for_target(
        "The.Regent.S01-S03.NORDiC.1080p.MAX.WEB-DL.H.265-NORViNE",
        &context(ContextFacetHint::Series, "The Regent"),
    );
    let multi_pack_episode = multi_pack
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(multi_pack_episode.raw.as_deref(), Some("S01-S03"));

    let special = analyze_release_for_target(
        "[DeadFish] Another Anime Show - 01 - OVA [BD][720p][AAC]",
        &context(ContextFacetHint::Anime, "Another Anime Show"),
    );
    let special_episode = special
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(special_episode.raw.as_deref(), Some("OVA"));

    let mut part_target = context(ContextFacetHint::Series, "Series Title");
    part_target.known_years.push(2026);
    let daily_part = analyze_release_for_target(
        "Series.Title.2026.04.22.Part.2.720p.HULU.WEBRip.AAC2.0.H264-Group",
        &part_target,
    );
    let daily_part_episode = daily_part
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .clone()
        .expect("episode");
    assert_eq!(daily_part_episode.raw.as_deref(), Some("2026-04-22 Part 2"));
    assert_eq!(daily_part_episode.daily_part, Some(2));
}

#[test]
fn split_color_depth_after_season_is_not_an_episode() {
    let analysis = analyze_release_for_target(
        "Show.Name.S02.10.bit.x265.1080p.WEB-DL-GRP",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.episode_numbers, Vec::<u32>::new());
}

#[test]
fn target_bank_prefers_specific_title_when_alias_and_episode_title_align() {
    let mut classic_starfall = context(ContextFacetHint::Anime, "Starfall");
    classic_starfall.aliases = vec![ContextAlias {
        name: "Starfall".to_string(),
    }];

    let analysis = analyze_release_against_targets(
        "[Studio Nova] Starfall - 014 - The Last 9 Signals [BD][1080p][HEVC 10bit x265][AAC]",
        &[classic_starfall, starfall_iron_eclipse_context()],
    );
    let candidate = analysis
        .best_target()
        .and_then(|target| target.analysis.best_candidate())
        .expect("best candidate");

    assert_eq!(analysis.best_target_index, Some(1));
    assert!(analysis.ambiguity_margin() >= 0);
    assert_eq!(
        candidate.projected.normalized_title,
        "STARFALL IRON ECLIPSE"
    );
}

/// Issue #170: a release whose name says `S01` and nothing more is a season
/// pack. The episode number a context hint can infer is what the search asked
/// for, not what the name says, so the explicit pack must win — unambiguously,
/// with full enrichment, so a required-language rule sees `iTALiAN`.
#[test]
fn explicit_season_pack_outranks_context_inferred_episode_numbering() {
    let mut target = context(ContextFacetHint::Series, "Quiet Meridian");
    target.episodes = vec![ContextEpisode {
        season: Some(1),
        episode: Some(1),
        absolute_number: None,
        air_date: None,
        title: None,
        title_aliases: Vec::new(),
    }];

    let analysis = analyze_release_for_target(
        "Quiet.Meridian.S01.iTALiAN.MULTi.1080p.DSNP.WEB-DL.DDP5.1.H.264-GRP",
        &target,
    );

    assert!(
        !analysis.is_ambiguous,
        "explicit evidence must not be rendered ambiguous by an inferred fallback"
    );
    let best = analysis.best_candidate().expect("best candidate");
    let episode = best.projected.episode.as_ref().expect("episode metadata");
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::SeasonPack);
    assert_eq!(episode.season_numbers, vec![1]);
    assert!(episode.full_season);
    assert!(
        episode.episode_numbers.is_empty(),
        "no episode number appears in the name"
    );
    assert_eq!(best.projected.languages_audio, vec!["ita"]);
    assert!(
        !best
            .reasons
            .iter()
            .any(|reason| reason.code == "identity:inferred_from_context"),
        "the winner's identity is explicit"
    );
    // The inferred reading still exists — penalized and recorded, available
    // when nothing explicit parses.
    assert!(
        analysis.candidates.iter().any(|candidate| candidate
            .reasons
            .iter()
            .any(|reason| reason.code == "identity:inferred_from_context")),
        "context-inferred identity must be recorded as inferred"
    );
}

/// Issue #170: an ambiguous parse keeps title/season/episode unresolved, but
/// structure-independent facts — languages above all — are read from the
/// tokens outside every contender's title zone, so a required-language rule
/// does not reject a release whose language is plainly in the name.
#[test]
fn ambiguous_parse_still_extracts_structure_independent_metadata() {
    let target = context(ContextFacetHint::Series, "Quiet Meridian");

    let analysis = analyze_release_for_target(
        "Quiet.Meridian.S01E05E06.iTALiAN.MULTi.1080p.WEB-DL-GRP",
        &target,
    );

    assert!(
        analysis.is_ambiguous,
        "fixture must stay a genuinely ambiguous parse"
    );
    let best = analysis.best_candidate().expect("best candidate");
    assert_eq!(best.projected.languages_audio, vec!["ita"]);
    assert!(
        best.projected
            .parse_hints
            .iter()
            .any(|hint| hint == "enrichment:ambiguous_structure_independent")
    );
}
