use super::*;


#[test]
fn parse_bdmv_release() {
    let parsed = parse_release_metadata(
        "Cosmic.Warriors.2012.ULTRAHD.Blu-ray.2160p.BDMV.Atmos.TrueHD.7.1-xHDGRP-Anonfinhel",
    );
    assert_eq!(parsed.year, Some(2012));
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
    assert_eq!(parsed.source.as_deref(), Some("BluRay"));
    assert_eq!(parsed.audio.as_deref(), Some("TRUEHD"));
    assert!(parsed.is_bd_disk);
    assert!(parsed.is_atmos);
}
#[test]
fn parse_web_dl_movie() {
    let parsed = parse_release_metadata("Crimson.Horizon.2019.1080p.WEB-DL.DDP2.0.H.264-Group.REPACK");
    assert_eq!(parsed.year, Some(2019));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.264"));
    assert_eq!(parsed.audio.as_deref(), Some("DDP"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("2.0"));
    assert!(parsed.is_proper_upload);
}

#[test]
fn parse_movie_dolby_vision_with_language() {
    let parsed = parse_release_metadata(
        "Crimson.Horizon.2019.2160p.MA.WEB-DL.Hybrid.H265.DV.HDR.DDP.Atmos.5.1.English.HONE",
    );

    assert_eq!(parsed.year, Some(2019));
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
    assert_eq!(parsed.audio.as_deref(), Some("DDP"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("5.1"));
    assert!(parsed.is_dolby_vision);
    assert!(parsed.detected_hdr);
    assert!(parsed.is_atmos);
    assert_eq!(parsed.languages_audio, vec!["eng"]);
}

#[test]
fn parse_movie_hdr10_metadata() {
    let parsed = parse_release_metadata(
        "Starfall.Beyond.Distant.Skies.S01E02.1080p.WEB-DL.H.265.HDR10.6ch.AAC",
    );

    assert!(parsed.detected_hdr);
    assert!(!parsed.is_dolby_vision);
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_movie_hlg_is_hdr() {
    let parsed = parse_release_metadata("Movie.Name.2023.2160p.BluRay.HLG.DDPlus.HEVC");

    assert!(parsed.detected_hdr);
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_movie_hdr_inferred_from_uhd_h265_10bit() {
    let parsed = parse_release_metadata("Movie.Name.2023.2160p.BluRay.H265.10BIT.DD5.1");

    assert!(!parsed.detected_hdr);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_series_with_dual_in_brackets_and_spaces() {
    let parsed = parse_release_metadata(
        "[Subeteka] Starfall Beyond Distant Skies-S02E02 [1080p WEB DUAL DDP2.0 H.265] [B263C5D8]",
    );

    assert_eq!(parsed.release_group.as_deref(), Some("Subeteka"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.audio.as_deref(), Some("DDP"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("2.0"));
    assert!(parsed.is_dual_audio);
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![2],
            absolute_episode: None,
            raw: Some("S02E02".to_string()),
        })
    );
}

#[test]
fn parse_series_multi_subs_only() {
    let parsed = parse_release_metadata(
        "[FNS] Tabibito no Yume-S02E05 [1080p][HEVC x265 10bit][Multi-Subs][75E5FCE7]",
    );

    assert_eq!(parsed.release_group.as_deref(), Some("FNS"));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
    assert_eq!(parsed.audio.as_deref(), None);
    assert!(!parsed.is_dual_audio);
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![5],
            absolute_episode: None,
            raw: Some("S02E05".to_string()),
        })
    );
}

#[test]
fn parse_series_vostfr_marks_subtitles() {
    let parsed = parse_release_metadata("Tabibito.No.Yume.S02E05.VOSTFR.1080p.WEBRiP.x265-RLS");

    assert_eq!(parsed.release_group.as_deref(), Some("RLS"));
    assert_eq!(parsed.languages_subtitles, vec!["fre".to_string()]);
    assert!(parsed.episode.is_some());
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
}

#[test]
fn parse_series_episode_and_dual() {
    let parsed = parse_release_metadata(
        "Starfall-Beyond.Distant.Skies.S02E03.Somewhere.Shed.Like.1080p.CR.WEB-DL.DUAL.DDP2.0.H.265",
    );

    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![3],
            absolute_episode: None,
            raw: Some("S02E03".to_string()),
        })
    );
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.audio.as_deref(), Some("DDP"));
    assert!(parsed.is_dual_audio);
    assert_eq!(parsed.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn parse_series_dual_default_fallback() {
    let parsed = parse_release_metadata(
        "Starfall-Beyond.Distant.Skies.S02E03.Somewhere.Shed.Like.1080p.WEB-DL.DUAL.DDP2.0.H.265",
    );

    assert!(parsed.is_dual_audio);
    assert_eq!(
        parsed.languages_audio,
        vec!["eng".to_string(), "jpn".to_string()]
    );
    assert!(parsed.languages_subtitles.is_empty());
}

#[test]
fn parse_language_subtitle_marker() {
    let parsed = parse_release_metadata(
            "Starfall-Beyond.Distant.Skies.S02E03.Somewhere.Shed.Like.1080p.WEB-DL.MULTISUBS.ITA.AAC2.0.H.265",
        );

    assert_eq!(parsed.languages_subtitles, vec!["ita".to_string()]);
    assert!(parsed.languages_audio.is_empty());
    assert!(parsed.episode.is_some());
}

#[test]
fn parse_dotted_language_markers() {
    let parsed = parse_release_metadata(
        "Crimson.Horizon.2019.2160p.H265.10.bit.DV.HDR10ita.eng.AC-3.5.1.sub.ita.eng.Licdom",
    );

    assert_eq!(parsed.languages_audio, vec!["eng".to_string()]);
    assert_eq!(
        parsed.languages_subtitles,
        vec!["ita".to_string(), "eng".to_string()]
    );
    assert_eq!(parsed.audio.as_deref(), Some("AC3"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("5.1"));
    assert!(parsed.is_dolby_vision);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_movie_release_group_after_metadata_tokens() {
    let parsed = parse_release_metadata("Crimson.Horizon.2019.1080p.WEB-DL.DDP2.0.H.264-Group.REPACK");

    assert_eq!(parsed.release_group.as_deref(), Some("Group"));
    assert!(parsed.is_proper_upload);
}

#[test]
fn parse_movie_release_group_with_hash_bracket() {
    let parsed = parse_release_metadata(
        "Starfall.Beyond.Distant.Skies.S01.1080p.WEB-DL.H.265-Licdom[75E5FCE8]",
    );

    assert_eq!(parsed.release_group.as_deref(), Some("Licdom"));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_multi_not_dual_audio() {
    let parsed = parse_release_metadata("Crimson.Horizon.2019.MULTi.VF2.1080p.HDLight.AC-3.5.1.H264-LiGHT");

    assert!(!parsed.is_dual_audio);
    assert_eq!(parsed.audio.as_deref(), Some("AC3"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn parse_multiple_audio_codecs() {
    let parsed = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-HD.TrueHD.7.1.H.265-GRP");

    assert_eq!(parsed.audio.as_deref(), Some("DTSHD"));
    assert_eq!(
        parsed.audio_codecs,
        vec!["DTSHD".to_string(), "TRUEHD".to_string()]
    );
}

#[test]
fn parse_language_order_variation() {
    let parsed =
        parse_release_metadata("Crimson.Horizon.2019.WEB.DL.DDP2.0.1080p.AMZN.DUAL.DOLBY.VISION.HEVC");
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert!(parsed.is_dolby_vision);
    assert!(parsed.is_dual_audio);
    assert_eq!(parsed.audio.as_deref(), Some("DDP"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn parse_fps_like_value() {
    let parsed =
        parse_release_metadata("[Fanz] Tabibito no Yume S2-05 x265 10bit 1080p 143.8561fps");
    assert_eq!(parsed.fps, Some(143.8561));
    assert_eq!(parsed.quality, Some("1080p".to_string()));
    assert_eq!(parsed.video_codec, Some("H.265".to_string()));
}

#[test]
fn parse_short_s_season_episode_pattern() {
    let parsed = parse_release_metadata("[Fanz] Tabibito no Yume S2-05 x265 1080p");
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![5],
            absolute_episode: None,
            raw: Some("S2 05".to_string()),
        })
    );
}

#[test]
fn parse_x_pattern() {
    let parsed = parse_release_metadata("Drama.Name.01x22.1080p.BluRay.x264.REMUX");
    assert_eq!(parsed.video_codec.as_deref(), Some("H.264"));
    assert!(parsed.is_remux);
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(1),
            episode_numbers: vec![22],
            absolute_episode: None,
            raw: Some("01x22".to_string()),
        })
    );
}

#[test]
fn parse_season_episode_compound() {
    let parsed =
        parse_release_metadata("Starfall-Beyond.Distant.Skies.S02E03E04E05.1080p.WEB-DL.H.265");
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![3, 4, 5],
            absolute_episode: None,
            raw: Some("S02E03E04E05".to_string()),
        })
    );
}

#[test]
fn parse_season_episode_range_in_one_token() {
    let parsed = parse_release_metadata("Starfall-S01E03-05.1080p.WEB-DL.x264");
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(1),
            episode_numbers: vec![3, 4, 5],
            absolute_episode: None,
            raw: Some("S01E03-05".to_string()),
        })
    );
}

#[test]
fn parse_x_range_and_multi_episode() {
    let parsed = parse_release_metadata("Show.Name.01x03-04x05.1080p.BluRay.x264");
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(1),
            episode_numbers: vec![3, 4, 5],
            absolute_episode: None,
            raw: Some("01x03-04x05".to_string()),
        })
    );
}

#[test]
fn parse_season_and_delayed_episode_tokens() {
    let parsed = parse_release_metadata("Starfall-S2 EP03 1080p.WEB-DL.H.264");
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![3],
            absolute_episode: None,
            raw: Some("S2 EP03".to_string()),
        })
    );
}

#[test]
fn parse_delayed_season_phrase() {
    let parsed = parse_release_metadata("Starfall.S02.EPISODE.03.1080p.WEB-DL.H.265");
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![3],
            absolute_episode: None,
            raw: Some("S02 EPISODE 03".to_string()),
        })
    );
}

#[test]
fn parse_separated_season_number() {
    let parsed = parse_release_metadata("Starfall.S 2 EP03 [1080p][WEBDL][H.264]");
    assert_eq!(
        parsed.episode,
        Some(ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![3],
            absolute_episode: None,
            raw: Some("S 2 EP03".to_string()),
        })
    );
}

// ── Phase B: DTS-X detection ─────────────────────────────────────────────

#[test]
fn parse_dtsx_codec() {
    let parsed = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-X.7.1.H.265-GRP");
    assert_eq!(parsed.audio.as_deref(), Some("DTSX"));
    assert_eq!(parsed.audio_codecs, vec!["DTSX".to_string()]);
    assert_eq!(parsed.audio_channels.as_deref(), Some("7.1"));
}

#[test]
fn parse_dtsx_no_hyphen() {
    let parsed = parse_release_metadata("Movie.2024.2160p.BluRay.DTSX.7.1.H.265-GRP");
    assert_eq!(parsed.audio.as_deref(), Some("DTSX"));
}

// ── Phase B: DTS-MA split from DTS-HD ────────────────────────────────────

#[test]
fn parse_dts_ma_codec() {
    let parsed = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-MA.5.1.H.265-GRP");
    assert_eq!(parsed.audio.as_deref(), Some("DTSMA"));
    assert_eq!(parsed.audio_codecs, vec!["DTSMA".to_string()]);
}

#[test]
fn parse_dtsma_no_hyphen() {
    let parsed = parse_release_metadata("Movie.2024.2160p.BluRay.DTSMA.5.1.H.265-GRP");
    assert_eq!(parsed.audio.as_deref(), Some("DTSMA"));
}

#[test]
fn parse_dtshd_stays_dtshd() {
    let parsed = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-HD.5.1.H.265-GRP");
    assert_eq!(parsed.audio.as_deref(), Some("DTSHD"));
}

// ── AI Enhanced detection ───────────────────────────────────────────────────

#[test]
fn parse_ai_enhanced_dot_separated() {
    let parsed = parse_release_metadata(
        "Predator.Badlands.2025.1080p.AI.Enhanced.WEB-DL.LINE.AUDIO.DDP.5.1.H265-ZAX",
    );
    assert!(parsed.is_ai_enhanced);
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
}

#[test]
fn parse_ai_enhanced_hyphenated() {
    let parsed = parse_release_metadata(
        "The.Martian.2015.EXTENDED.2160p.DV.HDR10.Ai-Enhanced.H265.TrueHD.7.1.Atmos.MULTI-RIFE.4.15-60fps-DirtyHippie",
    );
    assert!(parsed.is_ai_enhanced);
    assert!(parsed.is_dolby_vision);
    assert!(parsed.is_atmos);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
}

#[test]
fn parse_rife_triggers_ai_enhanced() {
    let parsed = parse_release_metadata(
        "The.Crow.2024.2160p.DV.HDR10+Ai-Enhanced.HEVC.DDP.5.1.Atmos-RIFE.4.18v2-60fps-DirtyHippie",
    );
    assert!(parsed.is_ai_enhanced);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
}

#[test]
fn parse_ai_alone_does_not_trigger() {
    // "A.I." the movie should not trigger is_ai_enhanced
    let parsed = parse_release_metadata(
        "A.I.Artificial.Intelligence.2001.1080p.BluRay.H.264.DTS-GRP",
    );
    assert!(!parsed.is_ai_enhanced);
}

#[test]
fn parse_hfr_does_not_trigger_ai_enhanced() {
    // HFR is a legitimate WEB-DL attribute (Netflix, iPlayer etc.)
    let parsed = parse_release_metadata(
        "Formula.1.Drive.to.Survive.S07E01.Business.as.Usual.2160p.NF.WEB-DL.DDP5.1.Atmos.DV.HDR.HFR.H.265-KAE",
    );
    assert!(!parsed.is_ai_enhanced);
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert!(parsed.is_dolby_vision);
    assert!(parsed.is_atmos);
}

#[test]
fn parse_high_fps_triggers_ai_enhanced() {
    let parsed = parse_release_metadata(
        "[Raze] Phantom-Academy S2-09 x265 10bit 1080p 143.8561fps",
    );
    assert!(parsed.is_ai_enhanced);
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert!(parsed.fps.is_some_and(|f| f > 140.0));
}

#[test]
fn parse_normal_fps_does_not_trigger_ai_enhanced() {
    let parsed = parse_release_metadata(
        "Movie.2024.1080p.BluRay.H.264.DTS-GRP",
    );
    assert!(!parsed.is_ai_enhanced);
}

// ── Real-world release title tests ──────────────────────────────────────────

#[test]
fn parse_ai_enhanced_bluray_zax() {
    let parsed = parse_release_metadata(
        "Ghost.in.the.Machine.2012.PROPER.BluRay.1080p.AI.Enhanced.DTS-HD.MA.5.1.10Bit.x265-ZAX",
    );
    assert!(parsed.is_ai_enhanced);
    assert!(parsed.is_proper_upload);
    assert_eq!(parsed.source.as_deref(), Some("BluRay"));
    assert_eq!(parsed.audio.as_deref(), Some("DTSHD"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_ai_enhanced_rife_heavy_variant() {
    // Some DirtyHippie releases use "RIFE.4.25v2.Heavy" suffix
    let parsed = parse_release_metadata(
        "Night.Crawlers.2021.2160p.DV.HDR10Plus.Ai-Enhanced.HEVC.TrueHD.7.1.Atmos.MULTi3-RIFE.4.25v2.Heavy-60fps-DirtyHippie",
    );
    assert!(parsed.is_ai_enhanced);
    assert!(parsed.is_dolby_vision);
    assert!(parsed.is_atmos);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
    assert_eq!(parsed.audio.as_deref(), Some("TRUEHD"));
}

#[test]
fn parse_ai_enhanced_uhd_bluray_dv() {
    let parsed = parse_release_metadata(
        "The.Forgotten.Warrior.2016.BluRay.2160p.UHD.AI.Enhanced.DDP.Atmos.7.1.DV.HDR10.10Bit.x265-ZAX",
    );
    assert!(parsed.is_ai_enhanced);
    assert!(parsed.is_dolby_vision);
    assert!(parsed.is_atmos);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
    assert_eq!(parsed.source.as_deref(), Some("BluRay"));
}

#[test]
fn parse_ai_enhanced_imax_rife() {
    let parsed = parse_release_metadata(
        "Galaxy.Guardians.2014.IMAX.2160p.DV.HDR10+Ai-Enhanced.HEVC.TrueHD.7.1.Atmos.MULTI-RIFE.4.18v2-60fps-DirtyHippie",
    );
    assert!(parsed.is_ai_enhanced);
    assert!(parsed.is_dolby_vision);
    assert!(parsed.is_atmos);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
}

#[test]
fn parse_cr_webdl_anime() {
    let parsed = parse_release_metadata(
        "Gnosia.S01E20.World.of.Stars.1080p.CR.WEB-DL.AAC2.0.H.264-playWEB",
    );
    assert!(!parsed.is_ai_enhanced);
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.264"));
    assert_eq!(parsed.audio.as_deref(), Some("AAC"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("2.0"));
    assert!(parsed.episode.is_some());
}

#[test]
fn parse_netflix_complete_season() {
    let parsed = parse_release_metadata(
        "Fermats.Kitchen.S01.2025.Complete.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
    );
    assert_eq!(parsed.year, Some(2025));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.audio.as_deref(), Some("AAC"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn parse_bracket_anime_hevc() {
    let parsed = parse_release_metadata(
        "[ASW] Release that Witch-02 [1080p HEVC][D80C2AF2]",
    );
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_iplayer_hfr_webdl() {
    let parsed = parse_release_metadata(
        "Call.the.Midwife.S15E08.1080p.IP.WEB-DL.AAC2.0.HFR.H.264-SNAKE",
    );
    assert!(!parsed.is_ai_enhanced);
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.264"));
    assert!(parsed.episode.is_some());
}

#[test]
fn parse_av1_webrip() {
    let parsed = parse_release_metadata(
        "[Onalrie] Release that Witch-S01E02 [1080p WEBRip AV1]",
    );
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.video_codec.as_deref(), Some("AV1"));
}

#[test]
fn parse_atvp_hfr_atmos_dv() {
    let parsed = parse_release_metadata(
        "Formula.1.Drive.to.Survive.S08E07.What.Happens.In.Vegas.2160p.ATVP.WEB-DL.DDP5.1.Atmos.HFR.H.265-FLUX",
    );
    assert!(!parsed.is_ai_enhanced);
    assert_eq!(parsed.quality.as_deref(), Some("2160p"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert!(parsed.is_atmos);
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
    assert_eq!(parsed.audio.as_deref(), Some("DDP"));
    assert_eq!(parsed.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn bulk_rss_feed_titles_parse_without_panic() {
    // 525 real-world NZB release titles from NZBGeek RSS feeds (anime 5070, movies 2000, TV 5000).
    // This test validates that every title parses without panicking and that the parser
    // produces reasonable results across diverse real-world input.
    let titles = [
        "2000.meters.to.andriivka.2025.720p.web.h264-jff",
        "2000.meters.to.andriivka.2025.web.h264-rbb",
        "A.Friend.A.Murderer.S01E02.1080p.HEVC.x265-MeGusta",
        "A.Gatherers.Adventure.in.Isekai.S01.2025.Complete.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E01.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E02.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E03.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E04.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E05.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E06.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E07.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E08.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E09.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E10.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E11.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "A.Gatherers.Adventure.in.Isekai.S01E12.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "American.Yakuza.1993.1080p.BluRay.REMUX.PCM2.AVC-d3g",
        "American.Yakuza.[1993].br.remux.avc-d3g",
        "Atlantis.Milos.Return.2003.1080p.DSNP.WEB-DL.DDP5.1.H.264-BLOOM",
        "Avengement.2019.2160p.BluRay.x265.10bit.DTS-WiKi",
        "Bargain.Hunt.S73E20-Stafford.14.1080p.WEB-DL.AAC2.0.H.264-7VFr33104D",
        "Bargain.Hunt.S73E20-Stafford.14.WEB-DL.AAC2.0.H.264-7VFr33104D",
        "Bargain.Hunt.S73E20.720p.WEB.H264-JFF",
        "Bargain.Hunt.S73E20.WEB.H264-RBB",
        "Bargain.Loving.Brits.In.The.Sun.S14E73.HDTV.x264-NGP",
        "Baskin.2015.UHD.BluRay.2160p.DTS-HD.MA.5.1.HEVC.REMUX-FraMeSToR",
        "Black.Panther.Wakanda.Forever.2022.IMAX.Hybrid.1080p.BluRay.DDP7.1.x264-ZoroSenpai",
        "Boxcar.Bertha.[1972].br.remux.avc-d3g",
        "Bridget.Jones.Mad.About.the.Boy.2025.2160p.60fps.WEB-DL.HEVC.10bit.AV3A5.1-QHstudIo",
        "Celebrity.Puzzling.S02E11.HDTV.x264-NGP",
        "Chateau.Christmas.2020.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Cheaper.by.the.Dozen.2022.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Checkin.It.Twice.2023.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Chiquita.2025.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Chris.Jansing.Reports.12PM.2026.03.09.720p.WEB.H.264-NGP",
        "Chris.Jansing.Reports.12PM.2026.03.09.720p.WEB.H264-NGP",
        "Chris.Jansing.Reports.12PM.2026.03.09.WEB.H.264-NGP",
        "Chris.Jansing.Reports.12PM.2026.03.09.WEB.H264-NGP",
        "Chris.Jansing.Reports.1PM.2026.03.09.720p.WEB.H.264-NGP",
        "Chris.Jansing.Reports.1PM.2026.03.09.720p.WEB.H264-NGP",
        "Chris.Jansing.Reports.1PM.2026.03.09.WEB.H.264-NGP",
        "Chris.Jansing.Reports.1PM.2026.03.09.WEB.H264-NGP",
        "Christmas.Around.The.USA.2022.1080p.WEB.h264-BAE",
        "Christmas.Bells.Are.Ringing.2018.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.CEO.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.Cookies.2016.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.Getaway.2017.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.Incorporated.2015.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.Karma.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.Land.2015.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.Town.2019.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.Under.the.Lights.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.at.Cartwrights.2014.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.at.Dollywood.2019.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.at.Graceland.Home.for.the.Holidays.2019.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.at.the.Catnip.Cafe.2025.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.at.the.Golden.Dragon.2022.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.by.Design.2023.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.for.Keeps.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.Angel.Falls.2017.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.Canaan.2009.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.Conway.2013.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.Harmony.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.Homestead.2016.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.in.Montana.2019.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.in.My.Heart.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.Notting.Hill.2023.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.Tahoe.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.in.the.Friendly.Skies.2024.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.on.Duty.2025.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.on.Honeysuckle.Lane.2018.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.on.Mistletoe.Farm.2022.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.on.My.Mind.2019.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.with.Tucker.2013.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christmas.with.a.Kiss.2023.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.with.the.Campbells.2022.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Christmas.with.the.Singhs.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Christopher.Robin.2018.1080p.BluRay.DDP.5.1.10bit.H.265-iVy",
        "Christy.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Chuck.Billy.and.The.Marvelous.Guava.Tree.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "City.on.Fire.1979.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Class.2010.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Code.3.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Cold.Meat.2024.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Cold.Road.2024.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Collateral.Damage.2002.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Coming.Home.for.Christmas.2017.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Commando.2.2017.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Compulsion.2024.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Concrete.Evidence.A.Fixer.Upper.Mystery.2017.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Confessions.of.a.Christmas.Letter.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Cooking.with.Love.2018.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Cool.Kids.Dont.Cry.2012.1080p.BluRay.DDP.5.1.10bit.H.265-iVy",
        "Counterattack.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Coyotes.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Crashing.Through.the.Snow.2021.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "CrimeTime.Freefall.2024.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Crimewatch.Live.S23E06-Who.Killed.Diane.Sindall.1080p.WEB-DL.AAC2.0.H.264-P147YPU5",
        "Crimewatch.Live.S23E06.720p.WEB.H.264-JFF",
        "Crimewatch.Live.S23E06.720p.WEB.H264-JFF",
        "Crimewatch.Live.S23E06.WEB.H264-RBB",
        "Croma.Kid.2023.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Cross.Country.Christmas.2020.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Crossword.Mysteries.Riddle.Me.Dead.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Crossword.Mysteries.Terminal.Descent.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Curious.Caterer.Dying.for.Chocolate.2022.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Curious.Caterer.Fatal.Vows.2023.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Curious.Caterer.Foiled.Plans.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Curious.Caterer.Forbidden.Fruit.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Curtain.Up.Class.2026.S01E03.1080p.VIU.WEB-DL.AAC2.0.H.264-DUSKLiGHT",
        "Cut.Color.Murder.2022.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Cycling.UCI.World.Tour.2026.Tirreno.Adriatico.Men.Elite.Stage.01.1080p.WEB.h264-EPOWORKS",
        "D-Day.2013.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "DD.Returns.2023.1080p.WEBRip.DDP.2.0.8bit.H.265-iVy",
        "Dabangg.2010.1080p.BluRay.DDP.5.1.10bit.H.265-iVy",
        "Dangerous.Animals.2025.1080p.BluRay.DDP.5.1.10bit.H.265-iVy",
        "Dangerous.Animals.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Dani.Rovira.Vale.la.pena.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Dark.Seeker.the.Silent.Whispers.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Date.with.Love.2016.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Daters.Handbook.2016.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Dave.Chappelle.The.Unstoppable.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Dawshom.Awbotaar.2023.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Day.Is.Done.2011.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "De.Rugzak.S01.FLEMISH.1080p.WEB.H.264-TRIPEL",
        "Dead.Girl.Summer.2025.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Dead.of.Winter.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Deadly.Deed.A.Fixer.Upper.Mystery.2018.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Dear.Dumb.Diary.2013.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Death.Becomes.Her.1992.1080p.PCOK.WEB-DL.AAC.2.0.H.264-PiRaTeS",
        "Death.Race.2008.1080p.PCOK.WEB-DL.DDP.5.1.H.264-PiRaTeS",
        "Death.al.Dente.A.Gourmet.Detective.Mystery.2016.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Debbie.Macomber.s.Joyful.Mrs.Miracle.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Debbie.Macombers.A.Mrs.Miracle.Christmas.2021.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Debbie.Macombers.Dashing.Through.the.Snow.2015.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Deck.the.Halls.on.Cherry.Lane.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Deer.Camp.86.2022.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Den.of.Thieves.2.Pantera.2025.1080p.BluRay.DDP.5.1.10bit.H.265-iVy",
        "Den.of.Thieves.2.Pantera.2025.1080p.BluRay.TrueHD.7.1.Atmos.10bit.H.265-iVy",
        "Desk.Set.1957.1080p.BluRay.DDP.1.0.10bit.H.265-iVy",
        "Detectives.These.Days.Are.Crazy.S01.2025.Complete.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E01.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E02.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E03.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E04.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E05.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E06.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E07.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E08.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E09.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E10.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E11.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Detectives.These.Days.Are.Crazy.S01E12.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Devils.Knight.2024.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Dharam.Sankat.1991.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Diablo.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Diary.of.Fireflies.2016.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Diary.of.a.Wimpy.Kid.The.Last.Straw.2025.1080p.WEBRip.DDP.5.1.Atmos.10bit.H.265-iVy",
        "Die.My.Love.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Do.You.Know.Your.Place.S01E11.HDTV.x264-NGP",
        "Dolphins.Up.Close.with.Bertie.Gregory.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Don.Q.2023.1080p.BluRay.REMUX.AVC.DTS-HD-MA.5.1-UnKn0wn",
        "Dont.Come.Upstairs.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Dont.Move.2024.1080p.WEBRip.DDP.5.1.Atmos.10bit.H.265-iVy",
        "Doomsday.2008.2160p.BluRayRIP.DTS-HD-MA.5.1-UnKn0wn",
        "Doomsday.2008.UNRATED.1080p.BluRayRIP.x265.10bit.DTS-HA-5.1-UnKn0wn",
        "Double.Blind.2023.1080p.BluRay.REMUX.AVC.DD.5.1-UnKn0wn",
        "Double.Exposure.2024.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Double.Jeopardy.1999.1080p.BluRay.DDP.5.1.10bit.H.265-iVy",
        "Downton.Abbey.The.Grand.Finale.2025.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Dr.Seusss.The.Sneetches.2025.1080p.WEBRip.DDP.5.1.Atmos.10bit.H.265-iVy",
        "Dracula.2025.1080p.BluRay.REMUX.AVC.DTS-HD-MA.5.1-UnKn0wn",
        "Dracula.A.D.1972.1972.1080p.BluRay.DDP.2.0.10bit.H.265-iVy",
        "Dracula.A.Love.Tale.2025.1080p.THEATER.DDP.5.1.10bit.H.265-iVy",
        "Dracula.The.Original.Living.Vampire.2022.1080p.WEBRip.DDP.5.1.10bit.H.265-iVy",
        "Draculaw.2023.1080p.WEBRip.DDP.2.0.10bit.H.265-iVy",
        "Dreamscape.1984.1080p.BluRay.REMUX.AVC.DTS-HD-MA.5.1-UnKn0wn",
        "Dungeons.of.Ecstasy.2026.1080p.BluRay.REMUX.AVC.DD.5.1-UnKn0wn",
        "Eagle.Eye.2008.1080p.PCOK.WEB-DL.DDP.5.1.H.264-PiRaTeS",
        "Escape.To.The.Country.S26E33.HDTV.x264-NGP",
        "Escape.from.the.Outland.2025.1080p.WEB-DL.H.264.AAC-UBWEB",
        "Escape.from.the.Outland.2025.2160p.WEB-DL.DoVi.H.265.10bit.DDP5.1.Atmos-UBWEB",
        "Escape.from.the.Outland.2025.2160p.WEB-DL.HDRVivid.H.265.10bit.DDP5.1.Atmos-UBWEB",
        "Evil.Dead.Rise.2023.1080p.BluRayRIP.x265.10bit.TrueHD.7.1.Atmos-UnKn0wn",
        "Evil.Eye.2020.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Excalibur.1981.UHD.BluRay.DV-HDR10.10bit.2160p.Dts-HDMa5.1.HEVC-d3g",
        "Excalibur.1981.UHD.br.dv-hdr.hevc-d3g",
        "FPJs.Batang.Quiapo.2023.S03E278.Bagong.Maynila.1080p.iW.WEB-DL.AAC2.0.H.264-DUSKLiGHT",
        "Family.Guy.S24E07.1080p.x265-ELiTE",
        "Family.Guy.S24E07.720p.x264-FENiX",
        "Fermat.Kitchen.S01.2025.Complete.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E01.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E02.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E03.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E04.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E05.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E06.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E07.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E08.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E09.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E10.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E11.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Fermat.Kitchen.S01E12.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01.2025.Complete.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E01.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E02.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E03.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E04.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E05.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E06.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E07.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E08.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E09.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E10.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E11.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "From.Old.Country.Bumpkin.to.Master.Swordsman.S01E12.2025.1080p.Amazon.WEB-DL.AVC.DDP.2.0-DBTV",
        "Fuyu.no.Nankasa.Haru.no.Nankane.S01E06.1080p.NF.WEB-DL.AAC2.0.H.264-playWEB",
        "Girl.Slaves.of.Morgana.Le.Fay.1971.UHD.BluRay.2160p.x265.SDR.FLAC.mUHD-FRDS",
        "Gnosia.S01E19.Epilogue.1080p.CR.WEB-DL.AAC2.0.H.264-playWEB",
        "Gnosia.S01E19.Epilogue.720p.CR.WEB-DL.AAC2.0.H.264-playWEB",
        "Gnosia.S01E20.World.of.Stars.1080p.CR.WEB-DL.AAC2.0.H.264-playWEB",
        "Gnosia.S01E20.World.of.Stars.720p.CR.WEB-DL.AAC2.0.H.264-playWEB",
        "Golden.Kamuy.S05E08.Tokyo.Love.Story.1080p.CR.WEB-DL.DUAL.AAC2.0.H.264-VARYG",
        "Golden.Kamuy.S05E08.Tokyo.Love.Story.1080p.CR.WEB-DL.DUAL.AAC2.0.H.264.MSubs-ToonsHub",
        "Golden.Kamuy.S05E10.1080p.AMZN.WEB-DL.JPN.DDP2.0.H.264-ToonsHub",
        "Golden.Kamuy.S05E10.1080p.CR.WEB-DL.AAC2.0.H.264-OldT",
        "Golden.Kamuy.S05E10.Our.Kamuy.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
        "Golden.Kamuy.S05E10.Our.Kamuy.1080p.CR.WEB-DL.JPN.AAC2.0.H.264.MSubs-ToonsHub",
        "Gone.2026.S01E05.1080p.HEVC.x265-MeGusta",
        "Gone.2026.S01E05.720p.HEVC.x265-MeGusta",
        "Gone.2026.S01E06.720p.HEVC.x265-MeGusta",
        "Good.Songs.and.Daughters.S01.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E01.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E02.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E03.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E04.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E05.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E06.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E07.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E08.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Good.Songs.and.Daughters.S01E09.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB",
        "Goosebumps.2.Haunted.Halloween.2018.BluRay.1080p.DDP.5.1.x264-hallowed",
        "Goosebumps.2015.BluRay.1080p.DDP.Atmos.5.1.x264-hallowed",
        "Green.Card.1990.1080p.BDRip.x264.DUAL.DD5.1.TSRG",
        "Gulizar.2004.Yerli.1080p.WEB-DL.x264.AAC.TSRG",
        "H.G.P.w.M.W.S01E03.1080p.WEB.h264-EDITH",
        "H.G.P.w.M.W.S01E03.720p.WEB.H264-JFF",
        "H.G.P.w.M.W.S01E03.WEB.H264-RBB",
        "Hanna.2011.1080p.PCOK.WEB-DL.DDP.5.1.H.264-PiRaTeS",
        "Harry.Styles.One.Night.In.Manchester.2026.1080p.NF.WEB-DL.DD+5.1.H.264-playWEB",
        "Harry.Styles.One.Night.in.Manchester.2026.1080p.WEB.h264-EDITH",
        "Himesama.Goumon.no.Jikan.desu.S02E09.1080p.ABEMA.WEB-DL.JPN.AAC2.0.H.264-ToonsHub",
        "Instant.Family.2018.BluRay.1080p.DDP.5.1.x264-hallowed",
        "Jungle Emperor-Symphonic Poem [Blu-Flash][990DCDF2]",
        "Kantara.A.Legend.Chapter.1.2025.1080p.AMZN.WEB-DL.H.265.DDP5.1.HIN+ENG+KAN+TEL+TAM+MAL+SPA.ESUB-SHB931",
        "Kantara.A.Legend.Chapter.1.2025.ENG.1080p.AMZN.WEB-DL.H.264.DDP5.1.ESUB-SHB931",
        "Kate.A.Life.In.10.Dresses.2026.1080p.WEB.H264-CBFM",
        "Killers.of.the.Flower.Moon.2023.2160p.UHD.Blu-ray.Remux.DV.HDR.HEVC.TrueHD.Atmos.7.1-CiNEPHiLES",
        "L.Amour Est un Crime Parfait 2013 BRRip XVID DD5.1 NL Subs",
        "Left-Handed.Girl.2025.1080p.BluRay.REMUX.AVC.DTS-HD.MA.5.1-RainSunny",
        "Legend.2015.2160p.BluRayRIP.TrueHD.7.1.Atmos-UnKn0wn",
        "Logan.Lucky.2017.UHD.BluRay.2160p.DDP.5.1.HDR.x265-hallowed",
        "Love.Is.the.Perfect.Crime.2013.720p.BluRay.x264-RedBlade",
        "Love.on.the.Turquoise.Land.S01.2025.Complete.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E01.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E02.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E03.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E04.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E05.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E06.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E07.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E08.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E09.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E10.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E11.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E12.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E13.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E14.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E15.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E16.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E17.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E18.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E19.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E20.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E21.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E22.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E23.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E24.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E25.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E26.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E27.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E28.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E29.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E30.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E31.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Love.on.the.Turquoise.Land.S01E32.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Marshals.A.Yellowstone.Story.S01E02.Zone.of.Death.2160p.AMZN.WEB-DL.DDP5.1.H.265-NTb",
        "Marshals.S01E02.iNTERNAL.1080p.HEVC.x265-MeGusta",
        "Marshals.S01E02.iNTERNAL.1080p.WEB.h264-EDITH",
        "Marshals.S01E02.iNTERNAL.720p.HEVC.x265-MeGusta",
        "Matori.&amp;.Kyoken.S01E05.The.Mad.Dogs.Rampage.1080p.NF.WEB-DL.AAC2.0.H.264-playWEB",
        "Minority.Report.2002.2160p.BluRayRIP.DTS-HD-MA.5.1-UnKn0wn",
        "Mission.Against.Drugs.2026.2160p.WEB-DL.60Fps.H.265.10bit.AAC-UBWEB",
        "Mission.Against.Drugs.2026.2160p.WEB-DL.H.265.10bit.AAC-UBWEB",
        "Mission.Against.Drugs.2026.2160p.WEB-DL.HDRVivid.H.265.10bit.AAC-UBWEB",
        "Moonstruck.1987.1080p.PCOK.WEB-DL.DDP.5.1.H.264-PiRaTeS",
        "Morbius.2022.1080p.HMAX.WEB-DL.DDP5.1.H.264-BLOOM",
        "Mortal.Kombat.2001.1080p.BluRayRIP.x265.10bit.TrueHD.7.1-UnKn0wn",
        "Murder.Trial.The.Suffolk.Strangler.2026.1080p.WEB.H264-CBFM",
        "Muzzle.City.Of.Wolves.2025.1080p.BluRayRIP.x265.10bit.DTS-HD-MA.5.1-UnKn0wn",
        "My.Hero.Academia.Vigilantes.S02E10.Zero.Hour.1080p.BILI.WEB-DL.JPN.AAC2.0.H.265.MSubs-ToonsHub",
        "My.Hero.Academia.Vigilantes.S02E10.Zero.Hour.1080p.CR.WEB-DL.DUAL.AAC2.0.H.264-VARYG",
        "My.Hero.Academia.Vigilantes.S02E10.Zero.Hour.1080p.CR.WEB-DL.DUAL.AAC2.0.H.264.MSubs-ToonsHub",
        "My.Hero.Academia.Vigilantes.S02E10.Zero.Hour.1080p.CR.WEB-DL.MULTi.AAC2.0.H.264-VARYG",
        "My.Hero.Academia.Vigilantes.S02E10.Zero.Hour.1080p.NF.WEB-DL.AAC2.0.H.264-VARYG",
        "My.Hero.Academia.Vigilantes.S02E10.Zero.Hour.1080p.NF.WEB-DL.JPN.AAC2.0.H.264.MSubs-ToonsHub",
        "Onmyo.Kaiten.ReBirth.Verse.S01.2025.Complete.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E01.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E02.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E03.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E04.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E05.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E06.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E07.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E08.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E09.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E10.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E11.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Onmyo.Kaiten.ReBirth.Verse.S01E12.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Poseidon.2006.2160p.BluRayRIP.DTS-HD-MA.5.1-UnKn0wn",
        "Prehistoric.Planet.2022.S02E01.Islands.1080p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E01.Islands.720p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E02.Badlands.1080p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E03.Swamps.1080p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E03.Swamps.720p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E04.Oceans.1080p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E04.Oceans.720p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E05.North.America.1080p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S02E05.North.America.720p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S03E01.The.Big.Freeze.1080p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S03E01.The.Big.Freeze.720p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S03E02.New.Lands.720p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S03E04.Grass.Lands.1080p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S03E04.Grass.Lands.720p.HEVC.x265-MeGusta",
        "Prehistoric.Planet.2022.S03E05.The.Big.Melt.720p.HEVC.x265-MeGusta",
        "Pump.Up.the.Volume.1990.1080p.BluRay.x264.DUAL.DD5.1.TSRG",
        "Red.2.2013.4K.DSNP.WEB-DL.DE-EN-TR.DDP5.1.Atmos.HDR10.H.265-TURG",
        "Red.2010.4K.DSNP.WEB-DL.DE-EN-TR.DDP5.1.Atmos.HDR10.H.265-TURG",
        "Red.Sparrow.2018.BluRay.1080p.DDP.5.1.x264-hallowed",
        "Release.that.Witch.S01E02.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
        "Release.that.Witch.S01E02.1080p.CR.WEB-DL.CMN.AAC2.0.H.264.MSubs-ToonsHub",
        "Rock.Is.a.Ladys.Modesty.S01.2025.Complete.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E01.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E02.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E03.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E04.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E05.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E06.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E07.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E08.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E09.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E10.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E11.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E12.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Rock.Is.a.Ladys.Modesty.S01E13.2025.1080p.Netflix.WEB-DL.AVC.AAC.2.0-DBTV",
        "Roja.2025.S01E77.Secrets.and.Plans.1080p.iW.WEB-DL.AAC2.0.H.264-DUSKLiGHT",
        "Roja.2025.S01E78.Clash.of.Fathers.1080p.iW.WEB-DL.AAC2.0.H.264-DUSKLiGHT",
        "She.Who.Dares.Road.To.Milano.Cortina.2026.1080p.WEB.H264-CBFM",
        "Sirens.Kiss.S01E03.1080p.AMZN.WEB-DL.DUAL.DDP2.0.H.264-TURG",
        "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E01.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E02.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E03.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E04.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E05.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E06.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E07.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E08.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E09.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S02E10.2022.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03.2023.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E01.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E02.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E03.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E04.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E05.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E06.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E07.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E08.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E09.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Star.Trek.Picard.S03E10.2023.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        "Stolen.Face.1952.COMPLETE.UHD.BLURAY-LWRTD",
        "Stonehenge.Secrets.Of.The.New.Stone.2026.1080p.WEB.H264-CBFM",
        "Syd.2026.1080p.WEB.H264-CBFM",
        "Teen.Titans.Go.S09E39.WEB.H264-RBB",
        "The.Beast.in.Me.S01.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E01.Sick.Puppy.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E02.Just.Dont.Want.to.Be.Lonely.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E03.Elephant.in.the.Room.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E04.Thanatos.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E05.Bacchanal.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E06.The.Beast.and.Me.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E07.Ghosts.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Beast.in.Me.S01E08.The.Last.Word.2160p.NF.WEB-DL.DDP.5.1.Atmos.H.265-CHDWEB",
        "The.Conjuring.Last.Rites.2025.COMPLETE.UHD.BLURAY-FR0MH3LL",
        "The.Cook.Up.with.Adam.Liaw.S09E11.720p.WEB-DL.AAC2.0.H.264-7VFr33104D",
        "The.Cook.Up.with.Adam.Liaw.S09E11.WEB.H264-RBB",
        "The.Fire.Raven.2025.2160p.WEB-DL.HDRVivid.H.265.10bit.DDP5.1-UBWEB",
        "The.Killers.Game.2024.UHD.BluRay.2160p.TrueHD.Atmos.7.1.DV.HEVC.REMUX-FraMeSToR",
        "The.Pool.2024.2160p.WEB.H265-CBFM",
        "The.Secrets.of.Hotel.88.2026.S01E11.Shared.Spaces.1080p.iW.WEB-DL.AAC2.0.H.264-DUSKLiGHT",
        "The.SpongeBob.Movie.Search.for.SquarePants.2025.1080p.WEB-DL.HebDub.DD5.1.H.264-HBRW",
        "The.Usual.Suspects.1995.REPACK.1080p.AMZN.WEB-DL.DDP.5.1.H.264-YUTeamHD",
        "The.Wild.Robot.2024.1080p.BluRayRIP.x265.10bit.TrueHD.7.1-UnKn0wn",
        "Tis.Time.for.Torture.Princess.S02E09.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
        "Tis.Time.for.Torture.Princess.S02E09.1080p.CR.WEB-DL.JPN.AAC2.0.H.264.MSubs-ToonsHub",
        "Trouble.Every.Day.2001.1080p.BluRay.Remux.AVC.DTS-HD.MA.5.1-ADE",
        "Trouble.Every.Day.2001.1080p.BluRay.x264.DTS-ADE",
        "Trouble.Every.Day.2001.1080p.BluRay.x265.10bit.DTS-ADE",
        "Vigilante.Boku.no.Hero.Academia.ILLEGALS.S02E10.1080p.AMZN.WEB-DL.JPN.DDP2.0.H.264-ToonsHub",
        "Vigilante.Boku.no.Hero.Academia.Illegals.S02E10.MULTi.1080p.WEBRiP.x265-KAF",
        "Vigilante.Boku.no.Hero.Academia.Illegals.S02E10.VOSTFR.1080p.WEBRiP.x265-KAF",
        "Vladimir.S01E07.1080p.HEVC.x265-MeGusta",
        "Vladimir.S01E07.480p.x264-mSD",
        "Wash.It.All.Away.S01E10.1080p.CR.WEB-DL.AAC2.0.H.264-OldT",
        "Wash.It.All.Away.S01E10.A.Top-Class.Challenge.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
        "Wash.It.All.Away.S01E10.A.Top-Class.Challenge.1080p.CR.WEB-DL.JPN.AAC2.0.H.264.MSubs-ToonsHub",
        "What.Lies.Beneath.2025.S01E102.Edge.of.Revelation.1080p.iW.WEB-DL.AAC2.0.H.264-DUSKLiGHT",
        "What.Lies.Beneath.2025.S01E103.Vacant.Mercy.1080p.iW.WEB-DL.AAC2.0.H.264-DUSKLiGHT",
        "Yoroi.2025.FRENCH.COMPLETE.BLURAY-HiBOU",
        "You.Cant.Be.In.a.Rom-Com.with.Your.Childhood.Friends.S01E08.1080p.CR.WEB-DL.DUAL.AAC2.0.H.264.MSubs-ToonsHub",
        "You.Cant.Be.In.a.Rom-Com.with.Your.Childhood.Friends.S01E10.1080p.CR.WEB-DL.AAC2.0.H.264-OldT",
        "You.Cant.Be.In.a.Rom-Com.with.Your.Childhood.Friends.S01E10.1080p.CR.WEB-DL.JPN.AAC2.0.H.264.MSubs-ToonsHub",
        "You.Cant.Be.In.a.Rom.Com.with.Your.Childhood.Friends.S01E08.1080p.CR.WEB-DL.DUAL.AAC2.0.H.264-VARYG",
        "You.Cant.Be.In.a.Rom.Com.with.Your.Childhood.Friends.S01E10.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
        "You.Cant.Be.In.a.with.Your.Childhood.Friends.S01E08.1080p.CR.WEB-DL.DUAL.AAC2.0.H.264-VARYG",
        "You.Cant.Be.In.a.with.Your.Childhood.Friends.S01E10.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
        "You.Cant.Be.in.a.Rom-Com.With.Your.Childhood.Friends.2026.S01E10.1080p.CR.WEB-DL.H.264.AAC.2.0-AnoZu",
        "Zootopia.2.[2025].br.remux.avc-d3g",
        "Zuopiezi.nuhai.2025.1080p.BluRay.DD+5.1.x264-playHD",
        "Zuopiezi.nuhai.2025.720p.BluRay.DD+5.1.x264-playHD",
        "[ASW] Golden Kamuy-59 [1080p HEVC][867DEE0B]",
        "[ASW] Hime-sama Goumon no Jikan desu-21 [1080p HEVC][51926BC0]",
        "[ASW] Kirei ni Shite Moraemasu ka.-10 [1080p HEVC][594964A4]",
        "[ASW] Osananajimi to wa Love Comedy ni Naranai-10 [1080p HEVC][B1B53689]",
        "[ASW] Release that Witch-02 [1080p HEVC][D80C2AF2]",
        "[ASW] Vigilante-Boku no Hero Academia Illegals S2-10 [1080p HEVC][4389D8F4]",
        "[AnimeBefreiung] Lycoris Recoil [1080p] [Multi-Audio] [Multi-Subs]",
        "[DKB] Golden Kamuy-S05E10 [1080p][HEVC x265 10bit][Multi-Subs][8E03F257]",
        "[DKB] Vigilante-Boku no Hero Academia Illegals-S02E10 [1080p][HEVC x265 10bit][Multi-Subs][A781E23F]",
        "[Erai-raws] Golden Kamuy Final Season-10 [1080p CR WEB-DL AVC AAC][MultiSub][413FA2DC]",
        "[Erai-raws] Golden Kamuy Final Season-10 [480p CR WEB-DL AVC AAC][MultiSub][DC8CE481]",
        "[Erai-raws] Golden Kamuy Final Season-10 [720p CR WEB-DL AVC AAC][MultiSub][6B98C538]",
        "[Erai-raws] Hime-sama Goumon no Jikan desu 2nd Season-09 [1080p CR WEB-DL AVC AAC][MultiSub][522E2AB9]",
        "[Erai-raws] Hime-sama Goumon no Jikan desu 2nd Season-09 [480p CR WEB-DL AVC AAC][MultiSub][E18946D4]",
        "[Erai-raws] Hime-sama Goumon no Jikan desu 2nd Season-09 [720p CR WEB-DL AVC AAC][MultiSub][BCFC9C32]",
        "[Erai-raws] Kirei ni Shitemoraemasu ka-10 [1080p CR WEB-DL AVC AAC][MultiSub][37C34491]",
        "[Erai-raws] Kirei ni Shitemoraemasu ka-10 [480p CR WEB-DL AVC AAC][MultiSub][FBC934C8]",
        "[Erai-raws] Osananajimi to wa LoveCom ni Naranai-10 [1080p CR WEB-DL AVC AAC][MultiSub][B636D204]",
        "[Erai-raws] Osananajimi to wa LoveCom ni Naranai-10 [480p CR WEB-DL AVC AAC][MultiSub][CF812B37]",
        "[Erai-raws] Osananajimi to wa LoveCom ni Naranai-10 [720p CR WEB-DL AVC AAC][MultiSub][0C044D2B]",
        "[Erai-raws] Vigilante-Boku no Hero Academia Illegals 2nd Season-10 [1080p CR WEB-DL AVC AAC][MultiSub][6527D8AD]",
        "[Erai-raws] Vigilante-Boku no Hero Academia Illegals 2nd Season-10 [480p CR WEB-DL AVC AAC][MultiSub][096AB432]",
        "[Erai-raws] Vigilante-Boku no Hero Academia Illegals 2nd Season-10 [720p CR WEB-DL AVC AAC][MultiSub][2AB3CFC1]",
        "[Erai-raws].Golden.Kamuy.Final.Season-10.[1080p.CR.WEB-DL.AVC.AAC][MultiSub][413FA2DC]",
        "[Erai-raws].Golden.Kamuy.Final.Season-10.[480p.CR.WEB-DL.AVC.AAC][MultiSub][DC8CE481]",
        "[Erai-raws].Golden.Kamuy.Final.Season-10.[720p.CR.WEB-DL.AVC.AAC][MultiSub][6B98C538]",
        "[Erai-raws].Hime-sama.Goumon.no.Jikan.desu.2nd.Season-09.[1080p.CR.WEB-DL.AVC.AAC][MultiSub][522E2AB9]",
        "[Erai-raws].Hime-sama.Goumon.no.Jikan.desu.2nd.Season-09.[480p.CR.WEB-DL.AVC.AAC][MultiSub][E18946D4]",
        "[Erai-raws].Hime-sama.Goumon.no.Jikan.desu.2nd.Season-09.[720p.CR.WEB-DL.AVC.AAC][MultiSub][BCFC9C32]",
        "[Erai-raws].Osananajimi.to.wa.LoveCom.ni.Naranai-10.[1080p.CR.WEB-DL.AVC.AAC][MultiSub][B636D204]",
        "[Erai-raws].Osananajimi.to.wa.LoveCom.ni.Naranai-10.[480p.CR.WEB-DL.AVC.AAC][MultiSub][CF812B37]",
        "[Erai-raws].Osananajimi.to.wa.LoveCom.ni.Naranai-10.[720p.CR.WEB-DL.AVC.AAC][MultiSub][0C044D2B]",
        "[Erai-raws].Vigilante-Boku.no.Hero.Academia.Illegals.2nd.Season-10.[480p.CR.WEB-DL.AVC.AAC][MultiSub][096AB432]",
        "[Ironclad] Golden Kamuy-Saishuushou-S05E10 [WEB.1080p.AV1]",
        "[Judas] Golden Kamuy-S05E10",
        "[Judas] Goumon Baito-kun-S01E10",
        "[Judas] Hime-sama-S02E09",
        "[Judas] Vigilantes-S02E10",
        "[Onalrie] Golden Kamuy-S05E10 [1080p WEBRip AV1]",
        "[Onalrie] Hime-sama Goumon no Jikan desu-S02E09 [1080p WEBRip AV1]",
        "[Onalrie] Kirei ni Shite Moraemasu ka.-S01E10 [1080p WEBRip AV1]",
        "[Onalrie] Osananajimi to wa Love Comedy ni Naranai-S01E10 [1080p WEBRip AV1]",
        "[Onalrie] Release that Witch-S01E02 [1080p WEBRip AV1]",
        "[Onalrie] Vigilante-Boku no Hero Academia Illegals-S02E10 [1080p WEBRip AV1]",
        "[Raze] Vigilante-Boku no Hero Academia Illegals S2-10 x265 10bit 1080p 143.8561fps",
        "[Salchow] Medalist-S02E06 [WEB 1080p x264 AAC] [A0C3D267]",
        "[SubsPlease] Golden Kamuy-59 [1080p] [DF189C77]",
        "[SubsPlease] Golden Kamuy-59 [480p] [2AA01433]",
        "[SubsPlease] Golden Kamuy-59 [720p] [73053F12]",
        "[SubsPlease] Hime-sama Goumon no Jikan desu-21 [1080p] [710887A6]",
        "[SubsPlease] Hime-sama Goumon no Jikan desu-21 [480p] [34A7E88C]",
        "[SubsPlease] Hime-sama Goumon no Jikan desu-21 [720p] [8734BED2]",
        "[SubsPlease] Kirei ni Shite Moraemasu ka.-10 [1080p] [248F955E]",
        "[SubsPlease] Kirei ni Shite Moraemasu ka.-10 [480p] [BB12C2DF]",
        "[SubsPlease] Kirei ni Shite Moraemasu ka.-10 [720p] [3D6B83E6]",
        "[SubsPlease] Osananajimi to wa Love Comedy ni Naranai-10 [1080p] [2DDC1610]",
        "[SubsPlease] Osananajimi to wa Love Comedy ni Naranai-10 [480p] [A86B7BEB]",
        "[SubsPlease] Osananajimi to wa Love Comedy ni Naranai-10 [720p] [1F9DDDA0]",
        "[SubsPlease] Release that Witch-02 [1080p] [F3289096]",
        "[SubsPlease] Release that Witch-02 [480p] [A68753FD]",
        "[SubsPlease] Release that Witch-02 [720p] [461AF692]",
        "[SubsPlease] Vigilante-Boku no Hero Academia Illegals S2-10 [1080p] [F0FC5B24]",
        "[SubsPlease] Vigilante-Boku no Hero Academia Illegals S2-10 [480p] [3DC8B155]",
        "[SubsPlease] Vigilante-Boku no Hero Academia Illegals S2-10 [720p] [5D51B997]",
        "[TokekHutan] Ill Live a Long Life to Dote on My Favorite Stepbrother-S01E09 [AMZN.WEB-DL 1080P AVC, EAC3, MULTi][60FA9FDB]",
        "[Trix] Wash It All Away S01E10 [WEBRip 1080p AV1] [Multi Subs]",
        "[Yameii] Golden Kamuy-S05E08 [English Dub] [CR WEB-DL 1080p H264 AAC] [09A41A61]",
        "[Yameii] Golden Kamuy-S05E08 [English Dub] [CR WEB-DL 720p H264 AAC] [A09A2124]",
        "[Yameii] My Hero Academia-Vigilantes-S02E10 [English Dub] [CR WEB-DL 1080p H264 AAC] [49D122DA]",
        "[Yameii] My Hero Academia-Vigilantes-S02E10 [English Dub] [CR WEB-DL 720p H264 AAC] [6E11B198]",
        "[Yameii] You Cant Be In a Rom-Com with Your Childhood Friends-S01E08 [English Dub] [CR WEB-DL 1080p H264 AAC] [FBF2CFCF]",
        "[Yameii] You Cant Be In a Rom-Com with Your Childhood Friends-S01E08 [English Dub] [CR WEB-DL 720p H264 AAC] [0C272C40]",
        "[shincaps] Osananajimi to wa Love Comedy ni Naranai-10 [ANIPLUS 1920x1080 H264 AAC]",
        "bone.lake.2024.bdrip.h264-rbb",
        "brides.2025.weh264-rbb",
        "clairtone.2025.1080p.amzn.web-dl.ddp5.1.h.264-kitsune",
        "clairtone.2025.1080p.webrip.x264.aac5.1-yts.bz",
        "clairtone.2025.2160p.crav.web-dl.ddp5.1.h.265-kitsune",
        "clairtone.2025.720p.amzn.web-dl.ddp5.1.h.264-kitsune",
        "clairtone.2025.720p.web.h264-jff",
        "clairtone.2025.720p.webrip.x264.aac-yts.bz",
        "clairtone.2025.web.h264-rbb",
        "harry.styles.one.night.in.manchester.2026.1080p.webrip.x264.aac5.1-yts.bz",
        "harry.styles.one.night.in.manchester.2026.720p.webrip.x264.aac-yts.bz",
    ];

    let mut stats = BulkParseStats::default();

    for title in &titles {
        let parsed = parse_release_metadata(title);

        if parsed.quality.is_some() { stats.has_quality += 1; }
        if parsed.source.is_some() { stats.has_source += 1; }
        if parsed.video_codec.is_some() { stats.has_codec += 1; }
        if parsed.audio.is_some() { stats.has_audio += 1; }
        if parsed.release_group.is_some() { stats.has_group += 1; }
        if parsed.year.is_some() { stats.has_year += 1; }
        if parsed.is_ai_enhanced { stats.ai_enhanced += 1; }
        if parsed.parse_confidence < 0.1 { stats.very_low_confidence += 1; }
        stats.total += 1;
    }

    // Sanity thresholds — if these fail, something fundamental broke in the parser.
    // At least 70% of real NZB titles should have a detectable quality.
    assert!(
        stats.has_quality * 100 / stats.total >= 70,
        "Quality detection rate too low: {}/{}", stats.has_quality, stats.total
    );
    // At least 60% should have a detectable source.
    assert!(
        stats.has_source * 100 / stats.total >= 60,
        "Source detection rate too low: {}/{}", stats.has_source, stats.total
    );
    // At least 60% should have a detectable video codec.
    assert!(
        stats.has_codec * 100 / stats.total >= 60,
        "Codec detection rate too low: {}/{}", stats.has_codec, stats.total
    );
    // At least 50% should have a detectable audio codec.
    assert!(
        stats.has_audio * 100 / stats.total >= 50,
        "Audio detection rate too low: {}/{}", stats.has_audio, stats.total
    );
    // Very few titles should have near-zero confidence.
    assert!(
        stats.very_low_confidence * 100 / stats.total <= 10,
        "Too many titles with very low confidence: {}/{}", stats.very_low_confidence, stats.total
    );
    // AI enhanced should be rare in normal feeds — only the 143fps Raze title should trigger.
    assert!(
        stats.ai_enhanced <= 5,
        "Too many AI enhanced detections (false positives?): {}/{}", stats.ai_enhanced, stats.total
    );
}

#[derive(Default)]
struct BulkParseStats {
    total: usize,
    has_quality: usize,
    has_source: usize,
    has_codec: usize,
    has_audio: usize,
    has_group: usize,
    has_year: usize,
    ai_enhanced: usize,
    very_low_confidence: usize,
}

#[test]
fn verify_bluray_source_variants() {
    // BluRayRIP
    let p = parse_release_metadata("Doomsday.2008.2160p.BluRayRIP.DTS-HD-MA.5.1-UnKn0wn");
    assert_eq!(p.source.as_deref(), Some("BluRay"));
    assert_eq!(p.audio.as_deref(), Some("DTSHD"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));

    // BDRip
    let p = parse_release_metadata("Green.Card.1990.1080p.BDRip.x264.DUAL.DD5.1.TSRG");
    assert_eq!(p.source.as_deref(), Some("BluRay"));
    assert!(p.is_dual_audio);

    // BRRip
    let p = parse_release_metadata("L.Amour Est un Crime Parfait 2013 BRRip XVID DD5.1 NL Subs");
    assert_eq!(p.source.as_deref(), Some("BluRay"));

    // br.remux (lowercase)
    let p = parse_release_metadata("Zootopia.2.[2025].br.remux.avc-d3g");
    assert_eq!(p.source.as_deref(), Some("BluRay"));
    assert!(p.is_remux);
}

#[test]
fn verify_dd_vs_ddp_distinction() {
    // DD 5.1 = Dolby Digital
    let p = parse_release_metadata("Double.Blind.2023.1080p.BluRay.REMUX.AVC.DD.5.1-UnKn0wn");
    assert_eq!(p.audio.as_deref(), Some("DD"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));

    // DD5.1 = Dolby Digital (concatenated)
    let p = parse_release_metadata("The.SpongeBob.Movie.2025.1080p.WEB-DL.DD5.1.H.264-HBRW");
    assert_eq!(p.audio.as_deref(), Some("DD"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));

    // DDP5.1 = Dolby Digital Plus
    let p = parse_release_metadata("Red.2.2013.4K.DSNP.WEB-DL.DDP5.1.Atmos.HDR10.H.265-TURG");
    assert_eq!(p.audio.as_deref(), Some("DDP"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
    assert!(p.is_atmos);

    // DDP.5.1 = Dolby Digital Plus (dot-separated)
    let p = parse_release_metadata("Hanna.2011.1080p.PCOK.WEB-DL.DDP.5.1.H.264-PiRaTeS");
    assert_eq!(p.audio.as_deref(), Some("DDP"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn verify_hevc_x265_encoding_coexistence() {
    let p = parse_release_metadata("Gone.2026.S01E05.1080p.HEVC.x265-MeGusta");
    assert_eq!(p.video_codec.as_deref(), Some("H.265"));
    assert_eq!(p.video_encoding.as_deref(), Some("x265"));

    let p = parse_release_metadata("[DKB] Golden Kamuy-S05E10 [1080p][HEVC x265 10bit][Multi-Subs][8E03F257]");
    assert_eq!(p.video_codec.as_deref(), Some("H.265"));
    assert_eq!(p.video_encoding.as_deref(), Some("x265"));
}

#[test]
fn verify_hdrvivid_detection() {
    let p = parse_release_metadata("Escape.from.the.Outland.2025.2160p.WEB-DL.HDRVivid.H.265.10bit.DDP5.1.Atmos-UBWEB");
    assert!(p.detected_hdr);
    assert!(p.is_atmos);
    assert_eq!(p.audio.as_deref(), Some("DDP"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn verify_complete_bluray_bd_flag() {
    let p = parse_release_metadata("Yoroi.2025.FRENCH.COMPLETE.BLURAY-HiBOU");
    assert!(p.is_bd_disk);
    assert_eq!(p.source.as_deref(), Some("BluRay"));

    let p = parse_release_metadata("Stolen.Face.1952.COMPLETE.UHD.BLURAY-LWRTD");
    assert!(p.is_bd_disk);
}

#[test]
fn verify_60fps_dotted_titles() {
    let p = parse_release_metadata("Good.Songs.and.Daughters.S01.1989.2160p.WEB-DL.AAC.H.265.60fps-HDSWEB");
    assert_eq!(p.fps, Some(60.0));
}

#[test]
fn verify_pcm_audio_detection() {
    let p = parse_release_metadata("American.Yakuza.1993.1080p.BluRay.REMUX.PCM2.AVC-d3g");
    assert_eq!(p.audio.as_deref(), Some("PCM"));
    assert!(p.is_remux);
}

// ── Phase D: streaming service, edition, repack, HDR10+, hardcoded subs, anime version ──

#[test]
fn streaming_service_amazon() {
    let p = parse_release_metadata("The.Grand.Tour.S05E03.2160p.AMZN.WEB-DL.DDP5.1.H.265-NTb");
    assert_eq!(p.source.as_deref(), Some("WEB-DL"));
    assert_eq!(p.streaming_service.as_deref(), Some("Amazon"));
}

#[test]
fn streaming_service_netflix() {
    let p = parse_release_metadata("Stranger.Things.S04E01.2160p.NF.WEB-DL.DDP5.1.Atmos.H.265-FLUX");
    assert_eq!(p.streaming_service.as_deref(), Some("Netflix"));
}

#[test]
fn streaming_service_apple_tv() {
    let p = parse_release_metadata("Severance.S02E01.2160p.ATVP.WEB-DL.DDP5.1.H.265-NTb");
    assert_eq!(p.streaming_service.as_deref(), Some("Apple TV+"));
}

#[test]
fn streaming_service_disney_plus() {
    let p = parse_release_metadata("Andor.S01E01.2160p.DSNP.WEB-DL.DDP5.1.H.265-NTb");
    assert_eq!(p.streaming_service.as_deref(), Some("Disney+"));
}

#[test]
fn streaming_service_hbo_max() {
    let p = parse_release_metadata("The.Last.of.Us.S01E01.1080p.HMAX.WEB-DL.DDP5.1.Atmos.H.265-FLUX");
    assert_eq!(p.streaming_service.as_deref(), Some("HBO Max"));
}

#[test]
fn streaming_service_crunchyroll() {
    let p = parse_release_metadata("One.Piece.E1100.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG");
    assert_eq!(p.streaming_service.as_deref(), Some("Crunchyroll"));
}

#[test]
fn no_streaming_service_for_bluray() {
    let p = parse_release_metadata("Movie.2024.2160p.BluRay.REMUX.H.265.DTS-HD.MA.7.1-FraMeSToR");
    assert!(p.streaming_service.is_none());
}

#[test]
fn edition_imax() {
    let p = parse_release_metadata("Dune.Part.Two.2024.IMAX.2160p.WEB-DL.DDP5.1.Atmos.H.265-FLUX");
    assert_eq!(p.edition.as_deref(), Some("IMAX"));
}

#[test]
fn edition_imax_enhanced() {
    let p = parse_release_metadata("Dune.2021.IMAX.Enhanced.2160p.WEB-DL.DDP5.1.H.265-NTb");
    assert_eq!(p.edition.as_deref(), Some("IMAX Enhanced"));
}

#[test]
fn edition_extended() {
    let p = parse_release_metadata("Lord.of.the.Rings.2001.Extended.1080p.BluRay.H.264.DTS-DON");
    assert_eq!(p.edition.as_deref(), Some("Extended"));
}

#[test]
fn edition_directors_cut() {
    let p = parse_release_metadata("Blade.Runner.1982.Directors.Cut.1080p.BluRay.H.264-DON");
    assert_eq!(p.edition.as_deref(), Some("Director's Cut"));
}

#[test]
fn edition_remastered() {
    let p = parse_release_metadata("Heat.1995.Remastered.2160p.UHD.BluRay.H.265.DTS-HD.MA.5.1-DON");
    assert_eq!(p.edition.as_deref(), Some("Remaster"));
}

#[test]
fn edition_criterion() {
    let p = parse_release_metadata("Paris.Texas.1984.Criterion.1080p.BluRay.FLAC.H.264-DON");
    assert_eq!(p.edition.as_deref(), Some("Criterion"));
}

#[test]
fn repack_detected() {
    let p = parse_release_metadata("Show.S01E01.1080p.WEB-DL.DDP5.1.H.264-GROUP.REPACK");
    assert!(p.is_proper_upload);
    assert!(p.is_repack);
}

#[test]
fn proper_is_not_repack() {
    let p = parse_release_metadata("Show.S01E01.1080p.WEB-DL.DDP5.1.H.264-GROUP.PROPER");
    assert!(p.is_proper_upload);
    assert!(!p.is_repack);
}

#[test]
fn hdr10plus_detected() {
    let p = parse_release_metadata("Movie.2024.2160p.WEB-DL.HDR10Plus.DDP5.1.H.265-NTb");
    assert!(p.detected_hdr);
    assert!(p.is_hdr10plus);
    assert!(!p.is_hlg);
}

#[test]
fn hlg_detected() {
    let p = parse_release_metadata("Movie.2024.2160p.BluRay.HLG.H.265.DTS-HD.MA.5.1-DON");
    assert!(p.detected_hdr);
    assert!(p.is_hlg);
    assert!(!p.is_hdr10plus);
}

#[test]
fn hdr10_not_hdr10plus() {
    let p = parse_release_metadata("Movie.2024.2160p.WEB-DL.HDR10.DDP5.1.H.265-NTb");
    assert!(p.detected_hdr);
    assert!(!p.is_hdr10plus);
}

#[test]
fn hardcoded_subs_detected() {
    let p = parse_release_metadata("Movie.2024.1080p.HC.WEB-DL.AAC2.0.H.264-GROUP");
    assert!(p.is_hardcoded_subs);
}

#[test]
fn hardsubbed_detected() {
    let p = parse_release_metadata("Movie.2024.1080p.HARDSUBBED.WEB-DL.AAC2.0.H.264-GROUP");
    assert!(p.is_hardcoded_subs);
}

#[test]
fn anime_version_v2() {
    let p = parse_release_metadata("[SubGroup] Anime Title - 01v2 [1080p] [HEVC]");
    assert_eq!(p.anime_version, Some(2));
    assert!(p.is_proper_upload);
}

#[test]
fn anime_version_v3() {
    let p = parse_release_metadata("[SubGroup] Anime Title - 05v3 [720p]");
    assert_eq!(p.anime_version, Some(3));
}

#[test]
fn no_anime_version_for_normal_release() {
    let p = parse_release_metadata("Movie.2024.2160p.WEB-DL.DDP5.1.H.265-NTb");
    assert!(p.anime_version.is_none());
}

// ── Bug fix: RED streaming service false positive ─────────────────────
#[test]
fn verify_red_sparrow_bluray_not_webdl() {
    let p = parse_release_metadata("Red.Sparrow.2018.BluRay.1080p.DDP.5.1.x264-hallowed");
    assert_eq!(p.source.as_deref(), Some("BluRay"), "RED should not trigger YouTube/WEB-DL");
    assert_eq!(p.quality.as_deref(), Some("1080p"));
    assert_eq!(p.audio.as_deref(), Some("DDP"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
    assert_eq!(p.video_encoding.as_deref(), Some("x264"));
    assert!(p.streaming_service.is_none());
}

#[test]
fn verify_red_notice_bluray_not_webdl() {
    let p = parse_release_metadata("Red.Notice.2021.2160p.BluRay.REMUX.HEVC.DTS-HD.MA.7.1-FGT");
    assert_eq!(p.source.as_deref(), Some("BluRay"));
    assert!(p.streaming_service.is_none());
    assert!(p.is_remux);
}

#[test]
fn verify_youtube_still_detected() {
    let p = parse_release_metadata("Documentary.2024.1080p.YOUTUBE.WEB-DL.AAC2.0.H.264-GROUP");
    assert_eq!(p.source.as_deref(), Some("WEB-DL"));
    assert_eq!(p.streaming_service.as_deref(), Some("YouTube"));
}

// ── Bug fix: bare DD false audio match ────────────────────────────────
#[test]
fn verify_bare_dd_in_title_not_audio() {
    // "DD" is a movie title, not Dolby Digital — real audio is DDP later
    let p = parse_release_metadata("DD.Returns.2024.1080p.WEB-DL.DDP.5.1.H.264-GROUP");
    assert_eq!(p.audio.as_deref(), Some("DDP"), "bare DD should not consume audio slot");
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn verify_dd_with_channels_still_detected() {
    let p = parse_release_metadata("Movie.2024.1080p.WEB-DL.DD5.1.H.264-GROUP");
    assert_eq!(p.audio.as_deref(), Some("DD"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn verify_dd_dot_channels_still_detected() {
    let p = parse_release_metadata("Movie.2024.1080p.WEB-DL.DD.5.1.H.264-GROUP");
    assert_eq!(p.audio.as_deref(), Some("DD"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
}

// ── Bug fix: DDP5.1-GROUP channel extraction ──────────────────────────
#[test]
fn verify_ddp_channels_with_hyphen_group() {
    let p = parse_release_metadata("Movie.2024.1080p.WEB-DL.DDP5.1-GROUP");
    assert_eq!(p.audio.as_deref(), Some("DDP"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
    assert_eq!(p.release_group.as_deref(), Some("GROUP"));
}

#[test]
fn verify_aac_channels_with_hyphen_group() {
    let p = parse_release_metadata("Movie.2024.1080p.WEB-DL.AAC2.0-DBTV");
    assert_eq!(p.audio.as_deref(), Some("AAC"));
    assert_eq!(p.audio_channels.as_deref(), Some("2.0"));
    assert_eq!(p.release_group.as_deref(), Some("DBTV"));
}

#[test]
fn verify_dd_channels_with_hyphen_group() {
    let p = parse_release_metadata("Movie.2024.1080p.BluRay.DD5.1-GROUP");
    assert_eq!(p.audio.as_deref(), Some("DD"));
    assert_eq!(p.audio_channels.as_deref(), Some("5.1"));
}
