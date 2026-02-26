use super::*;

#[test]
fn parse_web_dl_movie() {
    let parsed = parse_release_metadata("1917.2019.1080p.WEB-DL.DDP2.0.H.264-Group.REPACK");
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
        "1917.2019.2160p.MA.WEB-DL.Hybrid.H265.DV.HDR.DDP.Atmos.5.1.English.HONE",
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
        "Frieren.Beyond.Journeys.End.S01E02.1080p.WEB-DL.H.265.HDR10.6ch.AAC",
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
        "[Subeteka] Frieren Beyond Journeys End-S02E02 [1080p WEB DUAL DDP2.0 H.265] [B263C5D8]",
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
        "[DKB] Sousou no Frieren-S02E05 [1080p][HEVC x265 10bit][Multi-Subs][75E5FCE7]",
    );

    assert_eq!(parsed.release_group.as_deref(), Some("DKB"));
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
    let parsed = parse_release_metadata("Sousou.No.Frieren.S02E05.VOSTFR.1080p.WEBRiP.x265-KAF");

    assert_eq!(parsed.release_group.as_deref(), Some("KAF"));
    assert_eq!(parsed.languages_subtitles, vec!["fre".to_string()]);
    assert!(parsed.episode.is_some());
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
    assert_eq!(parsed.source.as_deref(), Some("WEB-DL"));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
}

#[test]
fn parse_series_episode_and_dual() {
    let parsed = parse_release_metadata(
        "Frieren-Beyond.Journeys.End.S02E03.Somewhere.Shed.Like.1080p.CR.WEB-DL.DUAL.DDP2.0.H.265",
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
        "Frieren-Beyond.Journeys.End.S02E03.Somewhere.Shed.Like.1080p.WEB-DL.DUAL.DDP2.0.H.265",
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
            "Frieren-Beyond.Journeys.End.S02E03.Somewhere.Shed.Like.1080p.WEB-DL.MULTISUBS.ITA.AAC2.0.H.265",
        );

    assert_eq!(parsed.languages_subtitles, vec!["ita".to_string()]);
    assert!(parsed.languages_audio.is_empty());
    assert!(parsed.episode.is_some());
}

#[test]
fn parse_dotted_language_markers() {
    let parsed = parse_release_metadata(
        "1917.2019.2160p.H265.10.bit.DV.HDR10ita.eng.AC-3.5.1.sub.ita.eng.Licdom",
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
    let parsed = parse_release_metadata("1917.2019.1080p.WEB-DL.DDP2.0.H.264-Group.REPACK");

    assert_eq!(parsed.release_group.as_deref(), Some("Group"));
    assert!(parsed.is_proper_upload);
}

#[test]
fn parse_movie_release_group_with_hash_bracket() {
    let parsed = parse_release_metadata(
        "Frieren.Beyond.Journeys.End.S01.1080p.WEB-DL.H.265-Licdom[75E5FCE8]",
    );

    assert_eq!(parsed.release_group.as_deref(), Some("Licdom"));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.video_codec.as_deref(), Some("H.265"));
}

#[test]
fn parse_multi_not_dual_audio() {
    let parsed = parse_release_metadata("1917.2019.MULTi.VF2.1080p.HDLight.AC-3.5.1.H264-LiHDL");

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
    assert_eq!(parsed.audio_channels.as_deref(), Some("7.1"));
}

#[test]
fn parse_language_order_variation() {
    let parsed =
        parse_release_metadata("1917.2019.WEB.DL.DDP2.0.1080p.AMZN.DUAL.DOLBY.VISION.HEVC");
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
        parse_release_metadata("[Raze] Sousou no Frieren S2-05 x265 10bit 1080p 143.8561fps");
    assert_eq!(parsed.fps, Some(143.8561));
    assert_eq!(parsed.quality, Some("1080p".to_string()));
    assert_eq!(parsed.video_codec, Some("H.265".to_string()));
}

#[test]
fn parse_short_s_season_episode_pattern() {
    let parsed = parse_release_metadata("[Raze] Sousou no Frieren S2-05 x265 1080p");
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
        parse_release_metadata("Frieren-Beyond.Journeys.End.S02E03E04E05.1080p.WEB-DL.H.265");
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
    let parsed = parse_release_metadata("Frieren-S01E03-05.1080p.WEB-DL.x264");
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
    let parsed = parse_release_metadata("Frieren-S2 EP03 1080p.WEB-DL.H.264");
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
    let parsed = parse_release_metadata("Frieren.S02.EPISODE.03.1080p.WEB-DL.H.265");
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
    let parsed = parse_release_metadata("Frieren.S 2 EP03 [1080p][WEBDL][H.264]");
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
