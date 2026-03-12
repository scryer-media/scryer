use std::collections::HashMap;

use crate::{IndexerSearchResult, ParsedReleaseMetadata, QualityProfile, QualityProfileDecision};

pub(crate) struct ReleaseRuntimeInfo<'a> {
    pub size_bytes: Option<i64>,
    pub published_at: Option<&'a str>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub extra: Option<&'a HashMap<String, serde_json::Value>>,
}

pub(crate) struct RuleContextInfo<'a> {
    pub title_id: Option<&'a str>,
    pub category: Option<&'a str>,
    pub title_tags: &'a [String],
    pub has_existing_file: bool,
    pub existing_score: Option<i32>,
    pub search_mode: &'a str,
    pub runtime_minutes: Option<i32>,
    pub is_filler: bool,
}

pub(crate) fn build_rule_input(
    parsed: &ParsedReleaseMetadata,
    profile: &QualityProfile,
    decision: &QualityProfileDecision,
    release_runtime: ReleaseRuntimeInfo<'_>,
    context: RuleContextInfo<'_>,
    file: Option<scryer_rules::FileDoc>,
) -> scryer_rules::UserRuleInput {
    use scryer_rules::*;

    let category = context.category.unwrap_or("unknown");
    let is_anime = context
        .title_tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case("anime"))
        || category.eq_ignore_ascii_case("anime");

    UserRuleInput {
        release: ReleaseDoc {
            raw_title: parsed.raw_title.clone(),
            quality: parsed.quality.clone(),
            source: parsed.source.clone(),
            video_codec: parsed.video_codec.clone(),
            audio: parsed.audio.clone(),
            audio_codecs: parsed.audio_codecs.clone(),
            audio_channels: parsed.audio_channels.clone(),
            languages_audio: parsed.languages_audio.clone(),
            languages_subtitles: parsed.languages_subtitles.clone(),
            is_dual_audio: parsed.is_dual_audio,
            is_atmos: parsed.is_atmos,
            is_dolby_vision: parsed.is_dolby_vision,
            detected_hdr: parsed.detected_hdr,
            is_remux: parsed.is_remux,
            is_bd_disk: parsed.is_bd_disk,
            is_proper_upload: parsed.is_proper_upload,
            is_repack: parsed.is_repack,
            is_ai_enhanced: parsed.is_ai_enhanced,
            is_hardcoded_subs: parsed.is_hardcoded_subs,
            is_hdr10plus: parsed.is_hdr10plus,
            is_hlg: parsed.is_hlg,
            streaming_service: parsed.streaming_service.clone(),
            edition: parsed.edition.clone(),
            anime_version: parsed.anime_version,
            release_group: parsed.release_group.clone(),
            year: parsed.year,
            parse_confidence: parsed.parse_confidence,
            size_bytes: release_runtime.size_bytes,
            age_days: release_runtime
                .published_at
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| (chrono::Utc::now() - value.with_timezone(&chrono::Utc)).num_days()),
            thumbs_up: release_runtime.thumbs_up,
            thumbs_down: release_runtime.thumbs_down,
            extra: release_runtime.extra.cloned().unwrap_or_default(),
        },
        profile: ProfileDoc {
            id: profile.id.clone(),
            name: profile.name.clone(),
            quality_tiers: profile.criteria.quality_tiers.clone(),
            archival_quality: profile.criteria.archival_quality.clone(),
            allow_unknown_quality: profile.criteria.allow_unknown_quality,
            source_allowlist: profile.criteria.source_allowlist.clone(),
            source_blocklist: profile.criteria.source_blocklist.clone(),
            video_codec_allowlist: profile.criteria.video_codec_allowlist.clone(),
            video_codec_blocklist: profile.criteria.video_codec_blocklist.clone(),
            audio_codec_allowlist: profile.criteria.audio_codec_allowlist.clone(),
            audio_codec_blocklist: profile.criteria.audio_codec_blocklist.clone(),
            atmos_preferred: profile.criteria.atmos_preferred,
            dolby_vision_allowed: profile.criteria.dolby_vision_allowed,
            detected_hdr_allowed: profile.criteria.detected_hdr_allowed,
            prefer_remux: profile.criteria.prefer_remux,
            allow_bd_disk: profile.criteria.allow_bd_disk,
            allow_upgrades: profile.criteria.allow_upgrades,
            prefer_dual_audio: profile.criteria.prefer_dual_audio,
            required_audio_languages: profile.criteria.required_audio_languages.clone(),
        },
        context: ContextDoc {
            title_id: context.title_id.map(str::to_owned),
            media_type: category.to_string(),
            category: category.to_string(),
            tags: context.title_tags.to_vec(),
            has_existing_file: context.has_existing_file,
            existing_score: context.existing_score,
            search_mode: context.search_mode.to_string(),
            runtime_minutes: context.runtime_minutes,
            is_anime,
            is_filler: context.is_filler,
        },
        builtin_score: BuiltinScoreDoc {
            total: decision.release_score,
            blocked: !decision.allowed,
            codes: decision
                .scoring_log
                .iter()
                .map(|entry| entry.code.clone())
                .collect(),
        },
        file,
    }
}

pub(crate) fn build_search_rule_input(
    parsed: &ParsedReleaseMetadata,
    profile: &QualityProfile,
    result: &IndexerSearchResult,
    decision: &QualityProfileDecision,
    category: Option<&str>,
    title_tags: &[String],
    runtime_minutes: Option<i32>,
) -> scryer_rules::UserRuleInput {
    build_rule_input(
        parsed,
        profile,
        decision,
        ReleaseRuntimeInfo {
            size_bytes: result.size_bytes,
            published_at: result.published_at.as_deref(),
            thumbs_up: result.thumbs_up,
            thumbs_down: result.thumbs_down,
            extra: Some(&result.extra),
        },
        RuleContextInfo {
            title_id: None,
            category,
            title_tags,
            has_existing_file: false,
            existing_score: None,
            search_mode: "auto",
            runtime_minutes,
            is_filler: false,
        },
        None,
    )
}

pub(crate) fn build_file_doc(analysis: &scryer_mediainfo::MediaAnalysis) -> scryer_rules::FileDoc {
    scryer_rules::FileDoc {
        video_codec: analysis.video_codec.clone(),
        video_width: analysis.video_width,
        video_height: analysis.video_height,
        video_bitrate_kbps: analysis.video_bitrate_kbps,
        video_bit_depth: analysis.video_bit_depth,
        video_hdr_format: analysis.video_hdr_format.clone(),
        dovi_profile: analysis.dovi_profile,
        dovi_bl_compat_id: analysis.dovi_bl_compat_id,
        video_frame_rate: analysis.video_frame_rate.clone(),
        video_profile: analysis.video_profile.clone(),
        audio_codec: analysis.audio_codec.clone(),
        audio_channels: analysis.audio_channels,
        audio_bitrate_kbps: analysis.audio_bitrate_kbps,
        audio_languages: analysis.audio_languages.clone(),
        audio_streams: analysis
            .audio_streams
            .iter()
            .map(|stream| scryer_rules::AudioStreamDoc {
                codec: stream.codec.clone(),
                channels: stream.channels,
                language: stream.language.clone(),
                bitrate_kbps: stream.bitrate_kbps,
            })
            .collect(),
        subtitle_languages: analysis.subtitle_languages.clone(),
        subtitle_codecs: analysis.subtitle_codecs.clone(),
        subtitle_streams: analysis
            .subtitle_streams
            .iter()
            .map(|stream| scryer_rules::SubtitleStreamDoc {
                codec: stream.codec.clone(),
                language: stream.language.clone(),
                name: stream.name.clone(),
                forced: stream.forced,
                default: stream.default,
            })
            .collect(),
        has_multiaudio: analysis.has_multiaudio,
        duration_seconds: analysis.duration_seconds,
        num_chapters: analysis.num_chapters,
        container_format: analysis.container_format.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{QualityProfileCriteria, ScoringSource};
    use std::collections::HashMap;

    fn test_profile() -> QualityProfile {
        QualityProfile {
            id: "profile".to_string(),
            name: "Profile".to_string(),
            criteria: QualityProfileCriteria {
                quality_tiers: vec!["2160P".to_string(), "1080P".to_string()],
                archival_quality: Some("2160P".to_string()),
                allow_unknown_quality: false,
                source_allowlist: vec![],
                source_blocklist: vec![],
                video_codec_allowlist: vec![],
                video_codec_blocklist: vec![],
                audio_codec_allowlist: vec![],
                audio_codec_blocklist: vec![],
                atmos_preferred: false,
                dolby_vision_allowed: true,
                detected_hdr_allowed: true,
                prefer_remux: false,
                allow_bd_disk: false,
                allow_upgrades: true,
                prefer_dual_audio: false,
                required_audio_languages: vec![],
                scoring_persona: crate::ScoringPersona::Balanced,
                scoring_overrides: crate::ScoringOverrides::default(),
                cutoff_tier: None,
                min_score_to_grab: None,
                facet_persona_overrides: HashMap::new(),
            },
        }
    }

    fn test_decision() -> QualityProfileDecision {
        QualityProfileDecision {
            release_score: 1200,
            scoring_log: vec![crate::ScoringEntry {
                code: "quality_tier_0".to_string(),
                delta: 1200,
                source: ScoringSource::Builtin,
            }],
            allowed: true,
            block_codes: vec![],
            preference_score: 1200,
        }
    }

    fn test_parsed() -> ParsedReleaseMetadata {
        crate::parse_release_metadata("Test.Movie.2024.2160p.WEB-DL.H.265.DDP5.1-Group")
    }

    #[test]
    fn build_search_rule_input_keeps_file_null() {
        let input = build_search_rule_input(
            &test_parsed(),
            &test_profile(),
            &IndexerSearchResult {
                source: "test-indexer".to_string(),
                title: "Test Movie".to_string(),
                link: None,
                download_url: None,
                source_kind: None,
                size_bytes: Some(8_000_000_000),
                published_at: Some("2026-03-10T12:00:00Z".to_string()),
                thumbs_up: Some(5),
                thumbs_down: Some(1),
                nzbgeek_languages: None,
                nzbgeek_subtitles: None,
                nzbgeek_grabs: None,
                nzbgeek_password_protected: None,
                parsed_release_metadata: None,
                quality_profile_decision: None,
                extra: HashMap::from([("indexer".to_string(), serde_json::json!("test"))]),
                guid: None,
                info_url: None,
            },
            &test_decision(),
            Some("movie"),
            &["anime".to_string()],
            Some(120),
        );

        let value = serde_json::to_value(input).unwrap();
        assert!(value["file"].is_null());
        assert_eq!(value["release"]["extra"]["indexer"], "test");
    }

    #[test]
    fn build_rule_input_populates_post_download_file_doc() {
        let analysis = scryer_mediainfo::analyze_file(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("scryer-mediainfo")
                .join("tests")
                .join("media")
                .join("h264_aac.mkv"),
        )
        .unwrap();

        let input = build_rule_input(
            &test_parsed(),
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: Some(1234),
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                extra: None,
            },
            RuleContextInfo {
                title_id: Some("title-1"),
                category: Some("movie"),
                title_tags: &[],
                has_existing_file: true,
                existing_score: Some(900),
                search_mode: "post_download",
                runtime_minutes: Some(120),
                is_filler: false,
            },
            Some(build_file_doc(&analysis)),
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["context"]["search_mode"], "post_download");
        assert_eq!(value["context"]["existing_score"], 900);
        assert_eq!(value["file"]["num_chapters"], 0);
        assert_eq!(value["file"]["audio_streams"][0]["codec"], "aac");
    }
}
