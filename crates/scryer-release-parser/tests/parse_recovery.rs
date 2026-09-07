use scryer_release_parser::{
    ContextAlias, ContextFacetHint, ContextTitle, ParseDisposition, ReleaseParseContext,
    ReleaseSource, VideoCodec, analyze_release_for_target,
};

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

#[test]
fn recovers_technical_facts_for_an_unresolved_release() {
    let analysis = analyze_release_for_target(
        "Unresolved Collection [1080p] [WEB-DL] [x265] [10bit]",
        &context(ContextFacetHint::Anime, "Unresolved Collection"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert_eq!(analysis.disposition, ParseDisposition::Unparseable);
    assert!(projected.episode.is_none());
    assert_eq!(projected.parse_confidence, 0.0);
    assert_eq!(projected.ambiguity_margin, 0);
    assert_eq!(projected.quality.as_deref(), Some("1080p"));
    assert_eq!(projected.source, Some(ReleaseSource::WebDl));
    assert_eq!(projected.video_codec, Some(VideoCodec::H265));
    assert!(projected.is_10bit);
}

#[test]
fn short_numeric_suffix_is_a_release_group() {
    let analysis = analyze_release_for_target(
        "Known.Series.S10E16.1080p.WEB.H264-77",
        &context(ContextFacetHint::Series, "Known Series"),
    );
    assert_eq!(
        analysis
            .best_candidate()
            .expect("candidate")
            .projected
            .release_group
            .as_deref(),
        Some("77")
    );
}

#[test]
fn numeric_range_endpoints_are_not_release_groups() {
    let analysis = analyze_release_for_target(
        "Known Anime 01-52 Complete",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    assert_ne!(
        analysis
            .best_candidate()
            .expect("candidate")
            .projected
            .release_group
            .as_deref(),
        Some("52")
    );
}

#[test]
fn bounded_leading_numeric_and_compound_groups_are_preserved() {
    let numeric = analyze_release_for_target(
        "[777] Known Anime S01-S03 [1080p]",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    assert_eq!(
        numeric
            .best_candidate()
            .expect("candidate")
            .projected
            .release_group
            .as_deref(),
        Some("777")
    );

    let compound = analyze_release_for_target(
        "[Groupa_&_Groupb-Groupc] Known Anime S01E01 [720p]",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    assert_eq!(
        compound
            .best_candidate()
            .expect("candidate")
            .projected
            .release_group
            .as_deref(),
        Some("Groupa_&_Groupb-Groupc")
    );
}

#[test]
fn exact_context_alias_can_follow_a_symbolic_title_delimiter() {
    let mut target = context(ContextFacetHint::Anime, "Known Anime");
    target.aliases.push(ContextAlias {
        name: "Alternate Title".to_string(),
    });

    for delimiter in ["|", "•"] {
        let analysis = analyze_release_for_target(
            &format!("[Group] Known Anime {delimiter} Alternate Title S01-S02 [1080p]"),
            &target,
        );
        let projected = &analysis.best_candidate().expect("candidate").projected;
        assert_eq!(
            projected
                .episode
                .as_ref()
                .map(|episode| episode.season_numbers.as_slice()),
            Some([1, 2].as_slice())
        );
        assert!(
            projected
                .parse_hints
                .iter()
                .any(|hint| hint == "beam:context_alias_delimiter")
        );
    }

    let unmatched = analyze_release_for_target(
        "[Group] Known Anime | Unrelated Words S01-S02 [1080p]",
        &target,
    );
    assert!(
        !unmatched
            .best_candidate()
            .expect("candidate")
            .projected
            .parse_hints
            .iter()
            .any(|hint| hint == "beam:context_alias_delimiter")
    );
}

#[test]
fn bare_absolute_before_technical_group_requires_a_context_title_anchor() {
    let target = context(ContextFacetHint::Anime, "Known Anime");
    let analysis = analyze_release_for_target("[Group] Known Anime 42 (WEB 1080p x264)", &target);
    assert_eq!(
        analysis
            .best_candidate()
            .expect("candidate")
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.absolute_episode),
        Some(Some(42))
    );

    let no_technical_anchor =
        analyze_release_for_target("[Group] Known Anime 42 (presentation copy)", &target);
    assert_ne!(
        no_technical_anchor
            .best_candidate()
            .expect("candidate")
            .projected
            .episode
            .as_ref()
            .and_then(|episode| episode.absolute_episode),
        Some(42)
    );
}

#[test]
fn every_unknown_episodic_candidate_is_projected_as_unparseable() {
    let analysis = analyze_release_for_target(
        "Unresolved Collection [1080p]",
        &context(ContextFacetHint::Anime, "Unresolved Collection"),
    );
    let unknown_candidates = analysis
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.identity,
                scryer_release_parser::ReleaseIdentity::Unknown
            )
        })
        .collect::<Vec<_>>();

    assert!(!unknown_candidates.is_empty());
    for candidate in unknown_candidates {
        assert_eq!(
            candidate.projected.disposition,
            ParseDisposition::Unparseable
        );
        assert_eq!(candidate.projected.parse_confidence, 0.0);
        assert_eq!(candidate.projected.ambiguity_margin, 0);
        assert!(!candidate.projected.is_ambiguous);
    }

    let movie = analyze_release_for_target(
        "Known Movie 2024 [1080p]",
        &context(ContextFacetHint::Movie, "Known Movie"),
    );
    let movie_projected = &movie.best_candidate().expect("candidate").projected;
    assert_ne!(movie_projected.disposition, ParseDisposition::Unparseable);
    assert!(movie_projected.parse_confidence > 0.0);
}

#[test]
fn unresolved_pack_scope_is_reported_without_inventing_seasons() {
    let analysis = analyze_release_for_target(
        "Known Anime Ultimate Complete Pack (3 Seasons) [1080p] [WEB-DL]",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
    assert!(
        projected
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
}

#[test]
fn rejected_oversized_season_ranges_are_hinted_without_inventing_coverage() {
    let analysis = analyze_release_for_target(
        "Known Anime S01-S4294967295+OVA",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
    assert!(
        projected
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
}

#[test]
fn supported_episode_titles_with_completion_words_do_not_get_pack_hints() {
    let analysis = analyze_release_for_target(
        "Known.Anime.S01E01.The.Complete.Series.1080p.WEB-DL",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert_eq!(analysis.disposition, ParseDisposition::Parsed);
    assert_eq!(
        projected
            .episode
            .as_ref()
            .and_then(|episode| episode.season),
        Some(1)
    );
    assert_eq!(
        projected
            .episode
            .as_ref()
            .map(|episode| episode.episode_numbers.as_slice()),
        Some([1].as_slice())
    );
    assert!(
        !analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
    assert!(
        !projected
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
}

#[test]
fn bare_unknown_pack_words_are_hinted_outside_protected_titles() {
    let analysis = analyze_release_for_target(
        "Known Anime [WEB-DL] Collection",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );

    for title in ["Known Anime Batch", "Known Anime Collection"] {
        let protected = analyze_release_for_target(title, &context(ContextFacetHint::Anime, title));
        assert!(
            !protected
                .parse_hints
                .iter()
                .any(|hint| hint == "identity:unresolved_pack_scope")
        );
    }
}

#[test]
fn complete_batch_with_counted_seasons_and_extras_remains_unresolved() {
    let analysis = analyze_release_for_target(
        "Known Anime Complete Batch All 3 Seasons+Movie",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert_eq!(analysis.disposition, ParseDisposition::Unparseable);
    assert_eq!(projected.disposition, ParseDisposition::Unparseable);
    assert!(projected.episode.is_none());
    assert_eq!(projected.parse_confidence, 0.0);
    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
    assert!(
        projected
            .parse_hints
            .iter()
            .any(|hint| hint == "identity:unresolved_pack_scope")
    );
}

#[test]
fn supported_pack_coverage_does_not_receive_an_unresolved_scope_hint() {
    for release in [
        "Known Anime Complete Series [1080p] [WEB-DL]",
        "Known Anime All Seasons [1080p] [WEB-DL]",
        "Known Anime S01-S03 (3 Seasons) [1080p] [WEB-DL]",
    ] {
        let analysis =
            analyze_release_for_target(release, &context(ContextFacetHint::Anime, "Known Anime"));
        assert!(
            !analysis
                .parse_hints
                .iter()
                .any(|hint| hint == "identity:unresolved_pack_scope"),
            "unexpected unresolved scope hint for {release}",
        );
    }
}

#[test]
fn title_and_group_technical_words_do_not_recover_metadata() {
    let analysis = analyze_release_for_target(
        "1080p WEB-DL x265 10bit",
        &context(ContextFacetHint::Movie, "1080p WEB-DL x265 10bit"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert!(projected.quality.is_none());
    assert!(projected.source.is_none());
    assert!(projected.video_codec.is_none());
    assert!(!projected.is_10bit);
}

#[test]
fn conflicting_recovery_quality_and_source_stay_unset() {
    let analysis = analyze_release_for_target(
        "Unresolved Collection [720p] [1080p] [WEB-DL] [BluRay] [x264] [x265]",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert!(projected.quality.is_none());
    assert!(projected.source.is_none());
    assert!(projected.video_codec.is_none());
}

#[test]
fn conflicting_recovery_split_codec_stays_unset() {
    let analysis = analyze_release_for_target(
        "Unresolved Collection [H.264] [x265]",
        &context(ContextFacetHint::Anime, "Unresolved Collection"),
    );

    assert!(
        analysis
            .best_candidate()
            .expect("candidate")
            .projected
            .video_codec
            .is_none()
    );
}

#[test]
fn conflicting_recovery_bit_depth_stays_unset_for_unresolved_releases() {
    let analysis = analyze_release_for_target(
        "Unresolved Collection [WEB-DL] [8bit] [10bit]",
        &context(ContextFacetHint::Anime, "Unresolved Collection"),
    );
    assert!(
        !analysis
            .best_candidate()
            .expect("candidate")
            .projected
            .is_10bit
    );
}

#[test]
fn conflicting_recovery_clears_metadata_without_changing_parsed_identity() {
    let analysis = analyze_release_for_target(
        "Known Anime S01E01 720p WEB-DL [1080p] [BluRay] [x264] [x265] [8bit] [10bit]",
        &context(ContextFacetHint::Series, "Known Anime"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert_eq!(analysis.disposition, ParseDisposition::Parsed);
    assert_eq!(projected.disposition, ParseDisposition::Parsed);
    assert!(projected.episode.is_some());
    assert!(projected.parse_confidence > 0.0);
    assert!(projected.quality.is_none());
    assert!(projected.source.is_none());
    assert!(projected.video_codec.is_none());
    assert!(!projected.is_10bit);
}

#[test]
fn recovers_split_codec_only_within_one_technical_bracket_scope() {
    let analysis = analyze_release_for_target(
        "Unresolved Collection [H.264] [10bit]",
        &context(ContextFacetHint::Anime, "Unresolved Collection"),
    );
    let projected = &analysis.best_candidate().expect("candidate").projected;

    assert_eq!(projected.video_codec, Some(VideoCodec::H264));
    assert!(projected.is_10bit);
}

#[test]
fn title_alias_group_and_separate_brackets_do_not_supply_technical_facts() {
    let mut alias_target = context(ContextFacetHint::Series, "Known Series");
    alias_target.aliases = vec![ContextAlias {
        name: "1080p WEB-DL x265 10bit".to_string(),
    }];
    let alias_analysis =
        analyze_release_for_target("1080p WEB-DL x265 10bit S01E01", &alias_target);
    let alias_projected = &alias_analysis
        .best_candidate()
        .expect("candidate")
        .projected;
    assert!(alias_projected.quality.is_none());
    assert!(alias_projected.source.is_none());
    assert!(alias_projected.video_codec.is_none());
    assert!(!alias_projected.is_10bit);

    let group_analysis = analyze_release_for_target(
        "Known Series S01E01 - 10bit",
        &context(ContextFacetHint::Series, "Known Series"),
    );
    assert!(
        !group_analysis
            .best_candidate()
            .expect("candidate")
            .projected
            .is_10bit
    );

    let split_analysis = analyze_release_for_target(
        "Known Series S01E01 [10] [bit]",
        &context(ContextFacetHint::Series, "Known Series"),
    );
    assert!(
        !split_analysis
            .best_candidate()
            .expect("candidate")
            .projected
            .is_10bit
    );
}

#[test]
fn fused_bd_resolution_requires_a_valid_suffix_and_an_unprotected_scope() {
    let recovered = analyze_release_for_target(
        "Unresolved Collection [BD720p]",
        &context(ContextFacetHint::Anime, "Unresolved Collection"),
    );
    let recovered_projected = &recovered.best_candidate().expect("candidate").projected;
    assert_eq!(recovered_projected.quality.as_deref(), Some("720p"));
    assert_eq!(recovered_projected.source, Some(ReleaseSource::BluRay));

    let arbitrary_prefix = analyze_release_for_target(
        "Known Movie BDTITLE",
        &context(ContextFacetHint::Movie, "Known Movie"),
    );
    let arbitrary_projected = &arbitrary_prefix
        .best_candidate()
        .expect("candidate")
        .projected;
    assert!(arbitrary_projected.quality.is_none());
    assert!(arbitrary_projected.source.is_none());
}

#[test]
fn multi_season_global_range_retains_season_annotations_without_remapping_bounds() {
    let analysis = analyze_release_for_target(
        "[Group] Known Anime S01-S03 001-068 [1080p]",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    let episode = analysis
        .best_candidate()
        .expect("candidate")
        .projected
        .episode
        .as_ref()
        .expect("range pack");

    assert_eq!(episode.season, None);
    assert_eq!(episode.season_numbers, vec![1, 2, 3]);
    assert!(episode.episode_numbers.is_empty());
    assert_eq!(
        episode.absolute_episode_numbers,
        (1..=68).collect::<Vec<_>>()
    );
    assert!(episode.is_multi_season);
    assert!(!episode.is_series_pack);
    assert!(!episode.full_season);
}

#[test]
fn localized_context_probe_can_resolve_after_explicit_range_support() {
    let mut second = context(ContextFacetHint::Anime, "Titlec");
    second.aliases = vec![
        ContextAlias {
            name: "Titled Titlee: Titlef Titleg Titleh".to_string(),
        },
        ContextAlias {
            name: "Titled Titlee Titlei".to_string(),
        },
        ContextAlias {
            name: "Titlej".to_string(),
        },
    ];

    let analysis = analyze_release_for_target(
        "[Groupa-Groupb][Titlec/Titled Titlee: Titlef Titleg Titleh][01-13TV全集][美版/USA.Ver][1080P][BDRip][HEVC-10bit][FLACx2][MKV](Titled Titlee Titlei/Titlej)",
        &second,
    );
    assert_eq!(analysis.disposition, ParseDisposition::Parsed);
}

#[test]
fn late_batch_bounds_require_delimited_fields_or_an_adjacent_known_alias() {
    let mut target = context(ContextFacetHint::Anime, "Known Anime");
    target.aliases.push(ContextAlias {
        name: "Other Name".into(),
    });
    for (release, end) in [
        ("Known Anime S01 (WEB 1080p) | Sub ENG | 01-13 | Batch", 13),
        (
            "Known Anime (Complete Season 01) Other Name 01-24 [Batch]",
            24,
        ),
    ] {
        let analysis = analyze_release_for_target(release, &target);
        let episode = analysis
            .best_candidate()
            .unwrap()
            .projected
            .episode
            .as_ref()
            .unwrap();
        assert_eq!(episode.season, Some(1), "{release}");
        assert_eq!(
            episode.episode_numbers,
            (1..=end).collect::<Vec<_>>(),
            "{release}"
        );
        assert!(!episode.full_season, "{release}");
    }
    for release in [
        "Known Anime S01 Interlude 01-13 Batch",
        "Known Anime (Complete Season 01) Other Name Interlude 01-24 [Batch]",
        "Known Anime Other Name (Complete Season 01) Interlude 01-24 [Batch]",
    ] {
        let analysis = analyze_release_for_target(release, &target);
        assert!(
            !analysis
                .best_candidate()
                .unwrap()
                .projected
                .episode
                .as_ref()
                .is_some_and(|episode| !episode.episode_numbers.is_empty()),
            "{release}"
        );
    }
    target.aliases.push(ContextAlias {
        name: "01-13".into(),
    });
    let protected =
        analyze_release_for_target("Known Anime S01 (WEB 1080p) | 01-13 | Batch", &target);
    assert!(
        protected
            .best_candidate()
            .unwrap()
            .projected
            .episode
            .as_ref()
            .unwrap()
            .episode_numbers
            .is_empty()
    );
}

#[test]
fn late_batch_bounds_cannot_cross_another_explicit_identity() {
    for scope in ["S02", "S02E05", "02-05"] {
        let release = format!("Known Anime S01 (WEB 1080p) | {scope} | 01-13 | Batch");
        let analysis =
            analyze_release_for_target(&release, &context(ContextFacetHint::Anime, "Known Anime"));
        let projected = &analysis.best_candidate().unwrap().projected;
        let episode = projected.episode.as_ref().unwrap();
        assert!(episode.episode_numbers.is_empty(), "{release}");
        assert!(!episode.full_season, "{release}");
        assert!(
            projected
                .parse_hints
                .iter()
                .any(|hint| hint == "identity:unresolved_pack_scope"),
            "{release}"
        );
    }
    let numbered = analyze_release_for_target(
        "Known Anime S01E01 | 01-13 | Batch",
        &context(ContextFacetHint::Anime, "Known Anime"),
    );
    assert_eq!(
        numbered
            .best_candidate()
            .unwrap()
            .projected
            .episode
            .as_ref()
            .unwrap()
            .episode_numbers,
        vec![1]
    );
}

#[test]
fn mixed_scope_episode_annotations_cannot_be_release_groups() {
    for release in [
        "Known Anime S01-S03(E117)",
        "Known Anime Seasons 1-2 and 4 Episodes of S3 Uncut Episodes 1-104",
        "Known Anime S01E001-S02E156 Complete",
    ] {
        let analysis =
            analyze_release_for_target(release, &context(ContextFacetHint::Anime, "Known Anime"));
        let projected = &analysis.best_candidate().unwrap().projected;
        assert!(
            projected.release_group.is_none(),
            "{release}: {:?}",
            projected.release_group
        );
        assert!(
            projected
                .parse_hints
                .iter()
                .any(|hint| hint == "identity:unresolved_pack_scope"),
            "{release}"
        );
    }
}
