use std::collections::BTreeMap;

use crate::{ParsedReleaseMetadata, QualityProfile, QualityProfileDecision};

pub(crate) struct ReleaseRuntimeInfo<'a> {
    pub size_bytes: Option<i64>,
    /// Whole days from publish to the lane's anchor. The caller computes it
    /// from the frozen listing snapshot; rule input never reads the clock.
    pub age_days: Option<i64>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub is_password_protected: Option<bool>,
    pub extra: Option<&'a BTreeMap<String, serde_json::Value>>,
    pub indexer_languages: Option<&'a [String]>,
}

impl<'a> ReleaseRuntimeInfo<'a> {
    /// Listing inputs read only from a frozen listing snapshot, with release
    /// age measured at the snapshot's anchor. Without a snapshot every listing
    /// fact is unknown.
    pub(crate) fn from_listing(
        size_bytes: Option<i64>,
        listing: Option<&'a crate::canonical_scoring::ListingFacts>,
    ) -> Self {
        let snapshot = listing.map(|facts| &facts.snapshot);
        Self {
            size_bytes,
            age_days: listing.and_then(|facts| facts.snapshot.age_days(facts.anchor)),
            thumbs_up: snapshot.and_then(|snapshot| snapshot.thumbs_up),
            thumbs_down: snapshot.and_then(|snapshot| snapshot.thumbs_down),
            is_password_protected: snapshot.and_then(|snapshot| snapshot.is_password_protected),
            extra: snapshot.map(|snapshot| &snapshot.extra),
            indexer_languages: snapshot
                .map(|snapshot| snapshot.indexer_languages.as_slice())
                .filter(|languages| !languages.is_empty()),
        }
    }
}

pub(crate) struct RuleContextInfo<'a> {
    pub title_id: Option<&'a str>,
    pub library_name: Option<&'a str>,
    pub category: Option<&'a str>,
    pub original_language: Option<&'a str>,
    pub original_country: Option<&'a str>,
    pub title_tags: &'a [String],
    pub has_existing_file: bool,
    pub existing_score: Option<i32>,
    pub search_mode: &'a str,
    pub runtime_minutes: Option<i32>,
    pub coverage_total_runtime_minutes: Option<i32>,
    pub coverage_member_runtime_minutes: Option<i32>,
    pub coverage_member_count: Option<i32>,
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
    let title_language_context = crate::title_audio_language_context(
        context.original_language,
        context.original_country,
        context.category,
        context.title_tags,
    );
    let is_anime = title_language_context.is_anime;
    let has_release_group = parsed
        .release_group
        .as_ref()
        .is_some_and(|group| !group.trim().is_empty());
    let is_obfuscated = is_obfuscated_release(parsed);
    let is_retagged = is_retagged_release(parsed);
    let (episode_release_type, is_season_pack, is_multi_episode) = release_type_details(parsed);

    let languages_audio = crate::release_audio_language_hints_for_title(
        parsed,
        release_runtime.indexer_languages,
        Some(&title_language_context),
        !profile.criteria.required_audio_languages.is_empty(),
    );

    let mut stereoscopy_tokens = Vec::new();
    if let Some(stereo) = parsed.stereoscopy {
        stereoscopy_tokens.push(format!("stereo:{}", stereo.presentation.as_str()));
        if stereo.has_3d() {
            stereoscopy_tokens.push("stereo:has_3d".into());
        }
        for (kind, value) in [
            ("layout", stereo.layout.map(|value| value.as_str())),
            ("sampling", stereo.sampling.map(|value| value.as_str())),
            ("encoding", stereo.encoding.map(|value| value.as_str())),
        ] {
            if let Some(value) = value {
                stereoscopy_tokens.push(format!("stereo:{kind}:{value}"));
            }
        }
    }
    for hint in &parsed.parse_hints {
        if matches!(
            hint.as_str(),
            "stereo:presentation_conflict"
                | "stereo:layout_conflict"
                | "stereo:sampling_conflict"
                | "stereo:encoding_conflict"
        ) && !stereoscopy_tokens.contains(hint)
        {
            stereoscopy_tokens.push(hint.clone());
        }
    }

    UserRuleInput {
        release: ReleaseDoc {
            stereoscopy_tokens,
            stereoscopy: parsed.stereoscopy.map(|stereo| StereoscopyDoc {
                presentation: stereo.presentation.as_str().into(),
                layout: stereo.layout.map(|value| value.as_str().into()),
                sampling: stereo.sampling.map(|value| value.as_str().into()),
                encoding: stereo.encoding.map(|value| value.as_str().into()),
            }),
            raw_title: parsed.raw_title.clone(),
            normalized_tokens: parsed.normalized_tokens.clone(),
            quality: parsed.quality.clone(),
            source: parsed.source.as_ref().map(ToString::to_string),
            video_codec: parsed.video_codec.as_ref().map(ToString::to_string),
            audio: parsed.audio.as_ref().map(ToString::to_string),
            audio_codecs: parsed
                .audio_codecs
                .iter()
                .map(ToString::to_string)
                .collect(),
            audio_channels: parsed.audio_channels.clone(),
            languages_audio,
            languages_subtitles: parsed.languages_subtitles.clone(),
            is_dual_audio: parsed.is_dual_audio,
            is_atmos: parsed.is_atmos,
            is_dolby_vision: parsed.is_dolby_vision,
            has_hdr_fallback: parsed.has_hdr_fallback,
            detected_hdr: parsed.detected_hdr,
            is_remux: parsed.is_remux,
            is_bd_disk: parsed.is_bd_disk,
            is_proper_upload: parsed.is_proper_upload,
            is_repack: parsed.is_repack,
            revision: crate::acquisition::scoring::revision_rank(parsed),
            is_ai_enhanced: parsed.is_ai_enhanced,
            is_hardcoded_subs: parsed.is_hardcoded_subs,
            is_password_protected: release_runtime.is_password_protected,
            is_hdr10plus: parsed.is_hdr10plus,
            is_hlg: parsed.is_hlg,
            is_10bit: parsed.is_10bit,
            is_uncensored: parsed.is_uncensored,
            is_dubs_only: parsed.is_dubs_only,
            has_release_group,
            is_obfuscated,
            is_retagged,
            streaming_service: parsed.streaming_service.as_ref().map(ToString::to_string),
            edition: parsed.edition.clone(),
            anime_version: parsed.anime_version,
            episode_release_type,
            is_season_pack,
            is_multi_episode,
            release_group: parsed.release_group.clone(),
            year: parsed.year.and_then(|year| u32::try_from(year).ok()),
            parse_confidence: parsed.parse_confidence,
            size_bytes: release_runtime.size_bytes,
            age_days: release_runtime.age_days,
            thumbs_up: release_runtime.thumbs_up,
            thumbs_down: release_runtime.thumbs_down,
            extra: release_runtime
                .extra
                .map(|extra| {
                    extra
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default(),
        },
        profile: ProfileDoc {
            id: profile.id.clone(),
            name: profile.name.clone(),
            quality_tiers: profile.criteria.quality_tiers.clone(),
            archival_quality: profile.criteria.archival_quality.clone(),
            allow_unknown_quality: profile.criteria.allow_unknown_quality,
            source_allowlist: profile
                .criteria
                .source_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            source_blocklist: profile
                .criteria
                .source_blocklist
                .iter()
                .map(ToString::to_string)
                .collect(),
            video_codec_allowlist: profile
                .criteria
                .video_codec_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            video_codec_blocklist: profile
                .criteria
                .video_codec_blocklist
                .iter()
                .map(ToString::to_string)
                .collect(),
            audio_codec_allowlist: profile
                .criteria
                .audio_codec_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            audio_codec_blocklist: profile
                .criteria
                .audio_codec_blocklist
                .iter()
                .map(ToString::to_string)
                .collect(),
            atmos_preferred: profile.criteria.atmos_preferred,
            dolby_vision_allowed: profile.criteria.dolby_vision_allowed,
            detected_hdr_allowed: profile.criteria.detected_hdr_allowed,
            prefer_remux: profile.criteria.prefer_remux,
            allow_bd_disk: profile.criteria.allow_bd_disk,
            allow_upgrades: profile.criteria.allow_upgrades,
            prefer_dual_audio: profile.criteria.prefer_dual_audio,
            required_audio_languages: crate::audio_requirements::normalize_required_audio_languages(
                profile.criteria.required_audio_languages.clone(),
            ),
            scoring_persona: match profile.criteria.resolve_persona(Some(category)) {
                crate::ScoringPersona::Balanced => "balanced",
                crate::ScoringPersona::Audiophile => "audiophile",
                crate::ScoringPersona::Efficient => "efficient",
                crate::ScoringPersona::Compatible => "compatible",
            }
            .to_string(),
            scoring_overrides: ScoringOverridesDoc {
                allow_x265_non4k: profile.criteria.scoring_overrides.allow_x265_non4k,
                block_dv_without_fallback: profile
                    .criteria
                    .scoring_overrides
                    .block_dv_without_fallback,
                prefer_compact_encodes: profile.criteria.scoring_overrides.prefer_compact_encodes,
                prefer_lossless_audio: profile.criteria.scoring_overrides.prefer_lossless_audio,
                block_upscaled: profile.criteria.scoring_overrides.block_upscaled,
            },
        },
        context: ContextDoc {
            title_id: context.title_id.map(str::to_owned),
            library_name: context.library_name.map(str::to_owned),
            media_type: category.to_string(),
            category: category.to_string(),
            original_language: title_language_context.original_language,
            original_country: title_language_context.original_country,
            inferred_original_audio_language: title_language_context
                .inferred_original_audio_language,
            // User-defined labels only. Reserved `scryer:` entries share the
            // storage but are per-title settings, and a release rule reading a
            // quality profile or a monitor type out of a field called `tags`
            // would be reading a setting the profile system already applied.
            // The full bag is still available where it is actually needed —
            // `title_audio_language_context` above reads the anime entries
            // straight from `context.title_tags`.
            tags: context
                .title_tags
                .iter()
                .filter(|tag| !crate::is_reserved_title_tag(tag))
                .cloned()
                .collect(),
            has_existing_file: context.has_existing_file,
            existing_score: context.existing_score,
            search_mode: context.search_mode.to_string(),
            runtime_minutes: context.runtime_minutes,
            coverage_total_runtime_minutes: context.coverage_total_runtime_minutes,
            coverage_member_runtime_minutes: context.coverage_member_runtime_minutes,
            coverage_member_count: context.coverage_member_count,
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

fn release_type_details(parsed: &ParsedReleaseMetadata) -> (Option<String>, bool, bool) {
    let Some(ref episode) = parsed.episode else {
        return (None, false, false);
    };

    let kind = match episode.release_type {
        crate::ParsedEpisodeReleaseType::SingleEpisode => "single_episode",
        crate::ParsedEpisodeReleaseType::MultiEpisode => "multi_episode",
        crate::ParsedEpisodeReleaseType::RangePack => "multi_episode",
        crate::ParsedEpisodeReleaseType::SeasonPack => "season_pack",
        crate::ParsedEpisodeReleaseType::Daily => "single_episode",
        crate::ParsedEpisodeReleaseType::Unknown => "unknown",
    };

    (
        Some(kind.to_string()),
        matches!(
            episode.release_type,
            crate::ParsedEpisodeReleaseType::SeasonPack
        ) || episode.full_season
            || episode.is_partial_season
            || episode.is_multi_season,
        matches!(
            episode.release_type,
            crate::ParsedEpisodeReleaseType::MultiEpisode
                | crate::ParsedEpisodeReleaseType::SeasonPack
        ) || episode.episode_numbers.len() > 1
            || episode.absolute_episode_numbers.len() > 1,
    )
}

fn is_obfuscated_release(parsed: &ParsedReleaseMetadata) -> bool {
    crate::helpers::is_obfuscated_release_name(parsed)
}

fn is_retagged_release(parsed: &ParsedReleaseMetadata) -> bool {
    let lower = parsed.raw_title.to_ascii_lowercase();
    [
        "[rartv]", "rarbg", "[tgx]", "eztvx", "ettv", "yts.mx", "yts",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

pub(crate) fn file_doc_from_analysis(analysis: &crate::MediaFileAnalysis) -> scryer_rules::FileDoc {
    let audio_languages = crate::normalize_detected_audio_languages(
        analysis.audio_languages.iter().map(String::as_str),
    );
    let subtitle_languages = crate::normalize_detected_subtitle_languages(
        analysis.subtitle_languages.iter().map(String::as_str),
    );

    scryer_rules::FileDoc {
        details: analysis.details.clone(),
        video_codec: analysis.video_codec.as_ref().map(ToString::to_string),
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
        audio_profile: analysis.audio_profile.clone(),
        audio_channels: analysis.audio_channels,
        audio_bitrate_kbps: analysis.audio_bitrate_kbps,
        audio_languages,
        audio_streams: analysis
            .audio_streams
            .iter()
            .map(|stream| scryer_rules::AudioStreamDoc {
                codec: stream.codec.clone(),
                profile: stream.profile.clone(),
                channels: stream.channels,
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_audio_language_code),
                name: stream.name.clone(),
                bitrate_kbps: stream.bitrate_kbps,
            })
            .collect(),
        subtitle_languages,
        subtitle_codecs: analysis.subtitle_codecs.clone(),
        subtitle_streams: analysis
            .subtitle_streams
            .iter()
            .map(|stream| scryer_rules::SubtitleStreamDoc {
                codec: stream.codec.clone(),
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_subtitle_language_code),
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
                ignore_proper_repack: false,
                prefer_dual_audio: false,
                required_audio_languages: vec![],
                scoring_persona: crate::ScoringPersona::Balanced,
                scoring_overrides: crate::ScoringOverrides::default(),
                cutoff_tier: None,
                min_score_to_grab: None,
                cutoff_score: None,
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
                kind: crate::quality_profile::ScoringEntryKind::ScoreContribution,
            }],
            allowed: true,
            block_codes: vec![],
            preference_score: 1200,
            tier_index: Some(0),
            size_fit_penalty: 0,
            revision_preference: 0,
            language: crate::AudioLanguageRank::default(),
        }
    }

    fn test_parsed() -> ParsedReleaseMetadata {
        crate::parse_release_metadata("Test.Movie.2024.2160p.WEB-DL.H.265.DDP5.1-Group")
    }

    #[test]
    fn stereo_rule_input_and_neutral_parser_preserve_optional_metadata() {
        for (suffix, expected, tokens, score) in [
            ("", serde_json::Value::Null, vec![], 0),
            (
                "3D.HSBS",
                serde_json::json!({"presentation":"three_d", "layout":"side_by_side", "sampling":"half", "encoding":null}),
                vec![
                    "stereo:three_d",
                    "stereo:has_3d",
                    "stereo:layout:side_by_side",
                    "stereo:sampling:half",
                ],
                100,
            ),
            (
                "2D",
                serde_json::json!({"presentation":"two_d", "layout":null, "sampling":null, "encoding":null}),
                vec!["stereo:two_d"],
                0,
            ),
            (
                "3D.FSBS",
                serde_json::json!({"presentation":"three_d", "layout":"side_by_side", "sampling":"full", "encoding":null}),
                vec![
                    "stereo:three_d",
                    "stereo:has_3d",
                    "stereo:layout:side_by_side",
                    "stereo:sampling:full",
                ],
                25,
            ),
            (
                "3D.MVC",
                serde_json::json!({"presentation":"three_d", "layout":null, "sampling":null, "encoding":"mvc"}),
                vec!["stereo:three_d", "stereo:has_3d", "stereo:encoding:mvc"],
                75,
            ),
            (
                "2D+3D",
                serde_json::json!({"presentation":"mixed_2d_3d", "layout":null, "sampling":null, "encoding":null}),
                vec!["stereo:mixed_2d_3d", "stereo:has_3d"],
                scryer_rules::BLOCK_SCORE,
            ),
            (
                "2D.3D",
                serde_json::Value::Null,
                vec!["stereo:presentation_conflict"],
                scryer_rules::BLOCK_SCORE,
            ),
        ] {
            let parsed = crate::parse_release_metadata(&format!(
                "Test.Movie.2024.1080p.BluRay.{suffix}.x264-Group"
            ));
            let input = build_rule_input(
                &parsed,
                &test_profile(),
                &test_decision(),
                ReleaseRuntimeInfo {
                    size_bytes: None,
                    age_days: None,
                    thumbs_up: None,
                    thumbs_down: None,
                    is_password_protected: None,
                    extra: None,
                    indexer_languages: None,
                },
                RuleContextInfo {
                    title_id: None,
                    library_name: None,
                    category: Some("movie"),
                    original_language: None,
                    original_country: None,
                    title_tags: &[],
                    has_existing_file: false,
                    existing_score: None,
                    search_mode: "interactive",
                    runtime_minutes: None,
                    coverage_total_runtime_minutes: None,
                    coverage_member_runtime_minutes: None,
                    coverage_member_count: None,
                    is_filler: false,
                },
                None,
            );
            assert_eq!(
                serde_json::to_value(&input).unwrap()["release"]["stereoscopy"],
                expected,
                "{suffix}"
            );
            assert_eq!(input.release.stereoscopy_tokens, tokens, "{suffix}");
            assert_eq!(input.release.normalized_tokens, parsed.normalized_tokens);
            for origin in [
                scryer_rules::PolicyOrigin::User,
                scryer_rules::PolicyOrigin::System,
            ] {
                let policy = scryer_rules::UserPolicy {
                    id: "stereo_tokens".into(),
                    name: "Stereo token evaluation".into(),
                    origin,
                    applied_facets: vec![],
                    rego_source: scryer_rules::rewrite_package_declaration(
                        r#"
                        import rego.v1
                        score_entry["three_d"] := 25 if {
                            "stereo:three_d" in input.release.stereoscopy_tokens
                        }
                        score_entry["half_sbs"] := 75 if {
                            "stereo:layout:side_by_side" in input.release.stereoscopy_tokens
                            "stereo:sampling:half" in input.release.stereoscopy_tokens
                        }
                        score_entry["mvc"] := 50 if {
                            "stereo:encoding:mvc" in input.release.stereoscopy_tokens
                        }
                        score_entry["mixed"] := scryer.block_score() if {
                            "stereo:mixed_2d_3d" in input.release.stereoscopy_tokens
                        }
                        score_entry["conflict"] := scryer.block_score() if {
                            "stereo:presentation_conflict" in input.release.stereoscopy_tokens
                        }
                    "#,
                        "stereo_tokens",
                    ),
                };
                let validation =
                    scryer_rules::validation::validate_user_rule(&policy.rego_source, &policy.id)
                        .unwrap();
                assert!(validation.valid, "{:?}", validation.errors);
                let result = scryer_rules::UserRulesEngine::build(&[policy])
                    .unwrap()
                    .evaluator()
                    .evaluate(&input, "movie")
                    .unwrap();
                assert!(result.errors.is_empty(), "{suffix} {origin:?}: {result:?}");
                assert_eq!(
                    result.entries.iter().map(|entry| entry.delta).sum::<i32>(),
                    score,
                    "{suffix} {origin:?}"
                );
            }
        }
        assert!(
            crate::parse_release_metadata("3D.Title.2024.1080p.x264")
                .stereoscopy
                .is_none()
        );
    }

    #[cfg(feature = "runtime-media-analysis")]
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
                age_days: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: Some("title-1"),
                library_name: Some("Movies"),
                category: Some("movie"),
                original_language: None,
                original_country: None,
                title_tags: &[],
                has_existing_file: true,
                existing_score: Some(900),
                search_mode: "post_download",
                runtime_minutes: Some(120),
                coverage_total_runtime_minutes: Some(120),
                coverage_member_runtime_minutes: Some(120),
                coverage_member_count: Some(1),
                is_filler: false,
            },
            Some(file_doc_from_analysis(
                &crate::post_download_gate::build_media_file_analysis(&analysis),
            )),
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["context"]["search_mode"], "post_download");
        assert_eq!(value["context"]["existing_score"], 900);
        assert_eq!(value["context"]["coverage_total_runtime_minutes"], 120);
        assert_eq!(value["context"]["coverage_member_runtime_minutes"], 120);
        assert_eq!(value["context"]["coverage_member_count"], 1);
        assert!(value["release"]["is_password_protected"].is_null());
        assert_eq!(value["release"]["normalized_tokens"][0], "TEST");
        assert_eq!(value["profile"]["scoring_persona"], "balanced");
        assert!(value["profile"]["scoring_overrides"]["block_upscaled"].is_null());
        assert!(
            value["file"]["num_chapters"].is_null(),
            "lightweight probing leaves chapter inventory unknown"
        );
        assert_eq!(value["file"]["audio_profile"], "LC");
        assert_eq!(value["file"]["audio_streams"][0]["codec"], "aac");
        assert_eq!(value["file"]["audio_streams"][0]["profile"], "LC");
    }

    #[test]
    fn indexer_languages_merged_into_languages_audio() {
        let input = build_rule_input(
            &test_parsed(),
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                age_days: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: Some(&["English".to_string(), "French".to_string()]),
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                original_language: None,
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                coverage_total_runtime_minutes: None,
                coverage_member_runtime_minutes: None,
                coverage_member_count: None,
                is_filler: false,
            },
            None,
        );

        let langs = &input.release.languages_audio;
        assert!(
            langs.contains(&"eng".to_string()),
            "should contain eng from indexer"
        );
        assert!(
            langs.contains(&"fra".to_string()),
            "should contain fra from indexer"
        );
    }

    #[test]
    fn indexer_languages_deduplicates_with_parsed() {
        // The test title parses no languages, but let's verify dedup with a title that does
        let parsed = crate::parse_release_metadata("Test.Movie.2024.FRENCH.2160p.WEB-DL");
        let input = build_rule_input(
            &parsed,
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                age_days: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: Some(&["French".to_string()]),
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                original_language: None,
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                coverage_total_runtime_minutes: None,
                coverage_member_runtime_minutes: None,
                coverage_member_count: None,
                is_filler: false,
            },
            None,
        );

        let fra_count = input
            .release
            .languages_audio
            .iter()
            .filter(|l| *l == "fra")
            .count();
        assert_eq!(fra_count, 1, "French should not be duplicated");
    }

    #[test]
    fn indexer_languages_support_full_iso_language_names() {
        let input = build_rule_input(
            &test_parsed(),
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                age_days: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: Some(&[
                    "Filipino".to_string(),
                    "English, Middle (1100-1500)".to_string(),
                ]),
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                original_language: None,
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                coverage_total_runtime_minutes: None,
                coverage_member_runtime_minutes: None,
                coverage_member_count: None,
                is_filler: false,
            },
            None,
        );

        assert!(input.release.languages_audio.contains(&"fil".to_string()));
        assert!(input.release.languages_audio.contains(&"enm".to_string()));
    }

    #[test]
    fn build_rule_input_exposes_episode_release_type_fields() {
        let parsed = crate::parse_release_metadata("Test.Show.S01.COMPLETE.1080p.WEB-DL-Group");
        let input = build_rule_input(
            &parsed,
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                age_days: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: Some("series-1"),
                library_name: Some("Series"),
                category: Some("series"),
                original_language: None,
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                coverage_total_runtime_minutes: None,
                coverage_member_runtime_minutes: None,
                coverage_member_count: None,
                is_filler: false,
            },
            None,
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["release"]["episode_release_type"], "season_pack");
        assert_eq!(value["release"]["is_season_pack"], true);
        assert_eq!(value["release"]["is_multi_episode"], true);
    }

    #[test]
    fn build_rule_input_exposes_release_provenance_flags() {
        let mut parsed = test_parsed();
        parsed.raw_title = "Test.Movie.2024.1080p.WEB-DL.A1B2C3D4E5.RARBG".to_string();
        parsed.release_group = None;

        let input = build_rule_input(
            &parsed,
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                age_days: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                original_language: None,
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                coverage_total_runtime_minutes: None,
                coverage_member_runtime_minutes: None,
                coverage_member_count: None,
                is_filler: false,
            },
            None,
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["release"]["has_release_group"], false);
        assert_eq!(value["release"]["is_obfuscated"], true);
        assert_eq!(value["release"]["is_retagged"], true);
    }
    /// `title.tags` on the release-rule context is the admin registry's
    /// vocabulary, not the whole storage bag. A reserved entry reaching a rule
    /// here would let a release rule branch on a per-title setting the profile
    /// system already applied.
    #[test]
    fn structured_settings_entries_never_reach_the_context_tags() {
        let title_tags = [
            "scryer:quality-profile:profile-one".to_string(),
            "keep".to_string(),
            "scryer:anime-media-type:tv".to_string(),
            "anime-hd".to_string(),
            "needs review".to_string(),
        ];
        let input = build_rule_input(
            &test_parsed(),
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                age_days: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: Some("title-1"),
                library_name: Some("Movies"),
                category: Some("movie"),
                original_language: None,
                original_country: None,
                title_tags: &title_tags,
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                coverage_total_runtime_minutes: None,
                coverage_member_runtime_minutes: None,
                coverage_member_count: None,
                is_filler: false,
            },
            None,
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(
            value["context"]["tags"],
            serde_json::json!(["keep", "anime-hd", "needs review"])
        );
        // The unfiltered bag is still read where it matters: it drives the
        // language context, which is why the filter lives here and not at the
        // caller.
        assert_eq!(value["context"]["is_anime"], true);
    }
}
