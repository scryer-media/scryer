//! Canonicality invariants.
//!
//! These are the tests the whole design exists to make expressible. If a term
//! ever leaks back into one evidence level and not the other, or incumbent
//! state creeps back into scoring, these fail rather than the behaviour
//! silently drifting the way it did before.

use super::canonical::*;
use crate::import::post_download_gate::build_stream_pointer_media_file_analysis;
use crate::quality_profile::{BLOCK_SCORE, CoverageSizeBasis, SIZE_PACK_MEMBER_BASIS_CODE};
use crate::release_parser::{VideoCodec, parse_release_metadata};
use crate::scoring_weights::balanced_weights;
use crate::{MediaFileAnalysis, ParsedReleaseMetadata, QualityProfile};

const GIB: i64 = 1024 * 1024 * 1024;

fn profile(json: &str) -> QualityProfile {
    QualityProfile::parse(json).expect("profile fixture should parse")
}

fn movie_profile() -> QualityProfile {
    profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    )
}

fn ctx<'a>(
    profile: &'a QualityProfile,
    weights: &'a crate::scoring_weights::ScoringWeights,
    tags: &'a [String],
) -> ScoringContext<'a> {
    ScoringContext {
        profile,
        weights,
        required_audio_languages: &[],
        category: "movie",
        size_basis: CoverageSizeBasis::default(),
        rules: None,
        title_id: None,
        library_name: None,
        original_language: None,
        original_country: None,
        title_tags: tags,
        is_filler: false,
    }
}

fn announced(size_gib: f64) -> ReleaseEvidence {
    ReleaseEvidence::announced(
        parse_release_metadata("Movie.2024.1080p.WEB-DL.H.264"),
        Some((size_gib * GIB as f64) as i64),
    )
}

fn analysis_reporting(codec: Option<&str>) -> MediaFileAnalysis {
    let mut analysis = build_stream_pointer_media_file_analysis();
    if let Some(codec) = codec {
        analysis.video_codec = VideoCodec::parse(codec);
    }
    analysis
}

fn analyzed(size_gib: f64, codec: Option<&str>) -> AnalyzedFacts {
    let analysis = analysis_reporting(codec);
    let rule_file_doc = crate::user_rule_input::file_doc_from_analysis(&analysis);
    AnalyzedFacts {
        analysis,
        actual_size_bytes: (size_gib * GIB as f64) as i64,
        // Populated, not `None`: import scores with a file-rule document, so a
        // fixture without one cannot detect a re-derivation that drops it.
        rule_file_doc: Some(rule_file_doc),
    }
}

/// **The canonicality invariant.** Adding analyzed evidence must never move the
/// announced half of the score. The announced number is what a grab decision
/// used; if it changes at import, grab and import can disagree about the same
/// release, which is the original defect.
#[test]
fn announced_half_is_unchanged_by_analysis() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    for (announced_gib, actual_gib) in [(8.0, 8.0), (8.0, 6.0), (8.0, 3.0), (3.0, 8.0)] {
        let without = score_release(&announced(announced_gib), &context);
        let with = score_release(
            &announced(announced_gib).with_analysis(analyzed(actual_gib, None)),
            &context,
        );

        assert_eq!(
            without.release_score, with.release_score,
            "release_score drifted when analysis was added ({announced_gib} GiB → {actual_gib} GiB)"
        );
    }
}

/// A file we could not measure must never be scored as though it measured
/// badly. No analysis ⇒ variance is exactly zero, and the bar equals the
/// announced score.
#[test]
fn variance_is_zero_without_analysis() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    let scored = score_release(&announced(8.0), &context);

    assert_eq!(scored.truth_variance, 0);
    assert_eq!(scored.total, scored.release_score);
    assert!(scored.truth_verdict.is_consistent());
}

/// **The intrinsic invariant.** A release's score must not depend on the
/// library's state. `allow_upgrades` is an admission policy; flipping it used to
/// inject a `BLOCK_SCORE` into the score itself.
#[test]
fn incumbent_policy_cannot_change_the_score() {
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();

    let upgrades_on = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    );
    let upgrades_off = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":false}}"#,
    );

    let on = score_release(&announced(8.0), &ctx(&upgrades_on, &weights, &tags));
    let off = score_release(&announced(8.0), &ctx(&upgrades_off, &weights, &tags));

    assert_eq!(
        on.total, off.total,
        "allow_upgrades leaked into the intrinsic score; it belongs to admission"
    );
    assert!(on.announced_decision.allowed && off.announced_decision.allowed);
}

/// Release age is listing metadata and must not be scored. A freshness bonus
/// makes a same-size re-grab read as an upgrade — pure bandwidth churn.
#[test]
fn listing_age_is_never_scored() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();

    let scored = score_release(&announced(8.0), &ctx(&profile, &weights, &tags));

    assert!(
        !scored
            .announced_decision
            .scoring_log
            .iter()
            .any(|entry| entry.code.starts_with("age_")),
        "age term leaked into the canonical score"
    );
}

/// A hard block discovered only by the analyzed pass is a verdict, never a
/// bounded number. Collapsing it into arithmetic would turn a veto into a
/// survivable penalty — and, worse, a *persisted* one: the bar would sit at
/// −10 000, every later candidate would read as a huge upgrade, and a structural
/// block would make its replacement score −10 000 too.
///
/// The old assertion was `total > BLOCK_SCORE`, which −10_000 + any positive
/// term satisfies; it passed for years while the block sat inside the number.
#[test]
fn analyzed_hard_block_is_a_verdict_not_a_number() {
    // Announced as H.264, which the profile permits; the file is really H.265,
    // which it blocks.
    let blocking = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"video_codec_blocklist":["H.265"]}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&blocking, &weights, &tags);

    let evidence = announced(8.0).with_analysis(analyzed(8.0, Some("h265")));
    let scored = score_release(&evidence, &context);

    assert!(
        matches!(scored.truth_verdict, TruthVerdict::Blocked { .. }),
        "expected Blocked, got {:?}",
        scored.truth_verdict
    );
    assert_eq!(
        scored.truth_variance, 0,
        "a block must not be expressed as a variance"
    );
    assert!(
        !scored
            .analyzed_decision
            .as_ref()
            .expect("the analyzed pass ran")
            .allowed,
        "the veto must still be reported as a veto"
    );
    // The number is the preference score the file honestly earned: within one
    // size bucket of the announced score, nowhere near BLOCK_SCORE.
    assert!(
        scored.total >= scored.release_score - TRUTH_VARIANCE_BOUND,
        "block score leaked into the persisted bar: total {} vs release_score {}",
        scored.total,
        scored.release_score
    );
    assert!(
        scored.total > BLOCK_SCORE / 2,
        "total {} still carries a hard block",
        scored.total
    );
}

/// The same rule for a block the *announced* pass found. `min_score_to_grab`
/// above everything the title can score used to bury the bar at −10 000 for
/// every file imported under that profile.
#[test]
fn announced_hard_block_is_a_verdict_not_a_number() {
    let unreachable_floor = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"min_score_to_grab":100000}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&unreachable_floor, &weights, &tags);

    let permissive = movie_profile();
    let unblocked = score_release(&announced(8.0), &ctx(&permissive, &weights, &tags));
    let scored = score_release(&announced(8.0), &context);

    assert!(
        !scored.announced_decision.allowed,
        "the floor must still veto the release"
    );
    assert!(
        scored
            .announced_decision
            .block_codes
            .iter()
            .any(|code| code == "score_below_minimum"),
        "the veto must name itself: {:?}",
        scored.announced_decision.block_codes
    );
    assert_eq!(
        scored.total, unblocked.total,
        "the floor is a verdict; it must not move the number"
    );
    assert!(
        scored.total > BLOCK_SCORE / 2,
        "total {} still carries a hard block",
        scored.total
    );
}

/// **The `Blocked` boundary.** A veto the profile would have raised against the
/// release's *name* is not evidence that the release lied, so it is not
/// `Blocked` — however loudly the analyzed pass complains about it.
///
/// This matters because `Blocked` is expensive: the import gate refuses the
/// file, blocklists the release for the title and reopens the search. If a
/// name-and-profile veto counted, the reopened search would grab the next
/// release, block it identically and burn that one too, until every release for
/// the title was blocklisted — with a provably correct file already on disk.
#[test]
fn a_veto_that_fires_on_the_name_alone_is_not_the_release_lying() {
    // An unreachable floor blocks both passes with `score_below_minimum`. It is
    // Sonarr's `MinFormatScore`: a grab floor, never an import specification.
    let unreachable_floor = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"min_score_to_grab":100000}}"#,
    );
    // A codec blocklist the *announcement* already admits to: the name says
    // H.264 and so does the file, but the profile blocks H.264.
    let blocked_codec = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"video_codec_blocklist":["H.264"]}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();

    for blocking in [unreachable_floor, blocked_codec] {
        let context = ctx(&blocking, &weights, &tags);
        let evidence = announced(8.0).with_analysis(analyzed(8.0, Some("h264")));
        let scored = score_release(&evidence, &context);

        assert!(
            !scored.announced_decision.allowed,
            "fixture precondition: the announced pass must be blocked too"
        );
        assert!(
            !scored
                .analyzed_decision
                .as_ref()
                .expect("the analyzed pass ran")
                .allowed,
            "fixture precondition: the analyzed pass must be blocked too"
        );
        assert!(
            !matches!(scored.truth_verdict, TruthVerdict::Blocked { .. }),
            "a block on both passes is the profile refusing the name, not the \
             release lying: {:?}",
            scored.truth_verdict
        );
        // The veto is still reported where it belongs — on the decisions.
        assert!(!scored.announced_decision.block_codes.is_empty());
        assert!(
            scored.total > BLOCK_SCORE / 2,
            "total {} still carries a hard block",
            scored.total
        );
    }
}

/// The other side of the boundary: a veto the **file** introduced. Nothing in
/// the announcement predicted it, so the release was mis-advertised and the
/// import gate is right to burn it.
#[test]
fn a_veto_only_the_file_could_raise_is_the_release_lying() {
    let blocking = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"video_codec_blocklist":["H.265"]}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&blocking, &weights, &tags);

    // Announced as H.264 (permitted); the file is really H.265 (blocked).
    let evidence = announced(8.0).with_analysis(analyzed(8.0, Some("h265")));
    let scored = score_release(&evidence, &context);

    assert!(
        scored.announced_decision.allowed,
        "fixture precondition: the announcement must pass"
    );
    let TruthVerdict::Blocked { codes } = &scored.truth_verdict else {
        panic!("expected Blocked, got {:?}", scored.truth_verdict);
    };
    assert_eq!(codes, &["video_codec_in_profile_blocklist".to_string()]);
}

/// A file that lands far from its announcement was mis-advertised. That is a
/// contradiction for admission to act on, and the numeric part stays clamped so
/// one bad probe cannot bury an episode's bar.
#[test]
fn large_analyzed_swing_is_contradicted_and_clamped() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    // 8 GiB announced sits in `size_expected`; 1 GiB actual lands in
    // `size_tiny` — a swing well past the bound.
    let evidence = announced(8.0).with_analysis(analyzed(1.0, None));
    let scored = score_release(&evidence, &context);

    assert!(
        matches!(scored.truth_verdict, TruthVerdict::Contradicted { .. }),
        "expected Contradicted, got {:?}",
        scored.truth_verdict
    );
    assert_eq!(scored.truth_variance, -TRUTH_VARIANCE_BOUND);
    assert!(
        !scored.truth_verdict.codes().is_empty(),
        "a contradiction should name the terms that moved"
    );
}

/// A small, honest difference is a variance, not a contradiction, and it lands
/// in the persisted bar unclamped.
#[test]
fn small_analyzed_swing_is_a_consistent_variance() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    // 8 GiB → 6 GiB drops one bucket (`size_expected` → `size_slightly_small`).
    let evidence = announced(8.0).with_analysis(analyzed(6.0, None));
    let scored = score_release(&evidence, &context);

    assert!(
        scored.truth_verdict.is_consistent(),
        "expected Consistent, got {:?}",
        scored.truth_verdict
    );
    assert!(scored.truth_variance.abs() <= TRUTH_VARIANCE_BOUND);
    assert_eq!(scored.total, scored.release_score + scored.truth_variance);
}

/// The same evidence must always produce the same number — scoring is pure.
#[test]
fn scoring_is_deterministic() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);
    let evidence = announced(8.0).with_analysis(analyzed(6.0, None));

    let first = score_release(&evidence, &context);
    let second = score_release(&evidence, &context);

    assert_eq!(first.total, second.total);
    assert_eq!(first.release_score, second.release_score);
    assert_eq!(first.truth_variance, second.truth_variance);
}

/// Guards the helper above: the fixture really does parse as H.264, so the
/// hard-block test is exercising an analyzed-only block rather than one both
/// passes would have found.
#[test]
fn announced_fixture_parses_as_h264() {
    let parsed: ParsedReleaseMetadata = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.264");
    assert_eq!(parsed.video_codec, VideoCodec::parse("h264"));
}

/// Build the row an import would have written for a release: the stored parse
/// columns come from the *rescored* parse, which is what the import persists.
fn media_row_as_import_would_write(
    evidence: &ReleaseEvidence,
    score: &ScoredRelease,
) -> crate::TitleMediaFile {
    let analyzed = evidence
        .analyzed
        .as_ref()
        .expect("round-trip fixture is an imported file");
    let stored_parse = crate::post_download_gate::rescore_parsed_from_analysis(
        &evidence.parsed,
        Some(&analyzed.analysis),
    )
    .0;

    crate::TitleMediaFile {
        id: "file-1".into(),
        title_id: "title-1".into(),
        episode_id: None,
        series_movie_link_ids: Vec::new(),
        file_path: "/data/Movies/Movie (2024)/Movie (2024) WEBDL-1080p.mkv".into(),
        size_bytes: analyzed.actual_size_bytes,
        announced_size_bytes: None,
        role: crate::MediaFileRole::Primary,
        source_signature_scheme: None,
        source_signature_value: None,
        content_hashes: None,
        quality_label: stored_parse.quality.clone(),
        scan_status: "imported".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        // Every analysis field, carried through — import serializes the whole
        // analysis onto the row, so a fixture that drops fields would let a
        // re-derivation gap pass unnoticed. That is the failure this models.
        video_codec: analyzed.analysis.video_codec,
        video_width: analyzed.analysis.video_width,
        video_height: analyzed.analysis.video_height,
        video_bitrate_kbps: analyzed.analysis.video_bitrate_kbps,
        video_bit_depth: analyzed.analysis.video_bit_depth,
        video_hdr_format: analyzed.analysis.video_hdr_format.clone(),
        dovi_profile: analyzed.analysis.dovi_profile,
        dovi_bl_compat_id: analyzed.analysis.dovi_bl_compat_id,
        video_frame_rate: analyzed.analysis.video_frame_rate.clone(),
        video_profile: analyzed.analysis.video_profile.clone(),
        audio_codec: analyzed.analysis.audio_codec.clone(),
        audio_profile: analyzed.analysis.audio_profile.clone(),
        audio_channels: analyzed.analysis.audio_channels,
        audio_bitrate_kbps: analyzed.analysis.audio_bitrate_kbps,
        audio_languages: analyzed.analysis.audio_languages.clone(),
        audio_streams: analyzed.analysis.audio_streams.clone(),
        subtitle_languages: analyzed.analysis.subtitle_languages.clone(),
        subtitle_codecs: analyzed.analysis.subtitle_codecs.clone(),
        subtitle_streams: analyzed.analysis.subtitle_streams.clone(),
        has_multiaudio: analyzed.analysis.has_multiaudio,
        duration_seconds: analyzed.analysis.duration_seconds,
        num_chapters: analyzed.analysis.num_chapters,
        container_format: analyzed.analysis.container_format.clone(),
        scene_name: Some(evidence.parsed.raw_title.clone()),
        release_group: stored_parse.release_group.clone(),
        source_type: None,
        resolution: stored_parse.quality.clone(),
        video_codec_parsed: stored_parse.video_codec,
        audio_codec_parsed: stored_parse.audio.as_ref().map(ToString::to_string),
        audio_channels_parsed: stored_parse.audio_channels.clone(),
        acquisition_score: Some(score.total),
        scoring_log: None,
        indexer_source: None,
        grabbed_release_title: None,
        grabbed_at: None,
        edition: stored_parse.edition.clone(),
        original_file_path: None,
        release_hash: None,
    }
}

/// **The round-trip invariant.** Re-deriving an incumbent's bar from its row
/// must reproduce the number the import computed. Every comparison re-derives,
/// so any evidence the row cannot reconstruct becomes a permanent gap between
/// the bar and the file that set it — the original bug wearing a different hat.
#[test]
fn re_deriving_a_stored_file_reproduces_the_imported_score() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    // Announced 8 GiB, landed 7.2 GiB — a real but honest difference.
    let evidence = announced(8.0).with_analysis(analyzed(7.2, None));
    let at_import = score_release(&evidence, &context);

    let row = media_row_as_import_would_write(&evidence, &at_import);
    let re_derived = score_media_file(&row, &context);

    assert_eq!(
        re_derived.total, at_import.total,
        "re-deriving the bar from the row moved it: import wrote {}, row yields {}",
        at_import.total, re_derived.total
    );
}

/// The bar equals what the file actually is. Once analyzed evidence exists, the
/// total collapses to the analyzed score — which is why a row that no longer
/// remembers its announcement can still reproduce the number.
#[test]
fn the_bar_is_the_analyzed_score() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    let evidence = announced(8.0).with_analysis(analyzed(6.0, None));
    let scored = score_release(&evidence, &context);

    // Scoring the landed facts as though they had been announced gives the same
    // number, because total = announced + (analyzed - announced).
    let as_announced_landed = score_release(&announced(6.0), &context);

    assert!(scored.truth_verdict.is_consistent());
    assert_eq!(scored.total, as_announced_landed.release_score);
}

/// A row with no stored analysis is still scorable — it simply carries no
/// variance. Scanned-in files land here, and there are a lot of them.
#[test]
fn a_row_without_analysis_still_yields_a_bar() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    let evidence = announced(8.0).with_analysis(analyzed(8.0, None));
    let scored = score_release(&evidence, &context);
    let mut row = media_row_as_import_would_write(&evidence, &scored);
    // Strip everything a media scan would not have populated.
    row.video_codec = None;
    row.video_width = None;
    row.video_height = None;
    row.audio_codec = None;
    row.audio_channels = None;

    let re_derived = score_media_file(&row, &context);
    assert!(
        re_derived.total > 0,
        "a scanned row must still produce a bar"
    );
    assert!(re_derived.truth_verdict.is_consistent());
}

/// The file-rule document survives the round trip.
///
/// `file.*` rules contribute to the score import writes. If re-deriving from the
/// row rebuilt a different document — or none — every incumbent bar would sit
/// below the score its own file was written with, and every candidate would look
/// like an upgrade. Compared structurally rather than through a score so a
/// failure names the field that drifted.
#[test]
fn a_stored_row_rebuilds_the_file_rule_document_import_scored_with() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    let evidence = announced(8.0).with_analysis(analyzed(7.2, None));
    let at_import = score_release(&evidence, &context);
    let imported_doc = evidence
        .analyzed
        .as_ref()
        .and_then(|analyzed| analyzed.rule_file_doc.as_ref())
        .expect("import evidence carries a file-rule document");

    let row = media_row_as_import_would_write(&evidence, &at_import);
    let rebuilt = analyzed_facts_from_media_file(&row)
        .rule_file_doc
        .expect("a stored row must rebuild its file-rule document");

    assert_eq!(
        serde_json::to_value(&rebuilt).expect("file doc serializes"),
        serde_json::to_value(imported_doc).expect("file doc serializes"),
        "the rebuilt file-rule document differs from the one import scored with"
    );
}

// ── D4: one runtime basis per scope ─────────────────────────────────────────

fn series_profile() -> QualityProfile {
    profile(
        r#"{"id":"s","name":"S","criteria":{"quality_tiers":["2160P","1080P","720P"],"allow_upgrades":true}}"#,
    )
}

fn episode_ctx<'a>(
    profile: &'a QualityProfile,
    weights: &'a crate::scoring_weights::ScoringWeights,
    tags: &'a [String],
    size_basis: CoverageSizeBasis,
) -> ScoringContext<'a> {
    ScoringContext {
        profile,
        weights,
        required_audio_languages: &[],
        category: "series",
        size_basis,
        rules: None,
        title_id: None,
        library_name: None,
        original_language: None,
        original_country: None,
        title_tags: tags,
        is_filler: false,
    }
}

fn feature_length_episode(id: &str, duration_seconds: i64) -> scryer_domain::Episode {
    scryer_domain::Episode {
        id: id.to_string(),
        title_id: "title-1".to_string(),
        collection_id: Some("season-1".to_string()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: None,
        title: None,
        air_date: None,
        duration_seconds: Some(duration_seconds),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    }
}

/// **The D4 invariant.** For one episode and one file, the grab candidate score,
/// the import score and the re-derived incumbent bar are the same number.
///
/// Size scoring is runtime-derived, so the three sites have to agree about how
/// long the thing is. They did not: grab used the *episode's* runtime
/// (`coverage_runtime_minutes`), import used `title.runtime_minutes`, and the
/// incumbent bar used the title default too. A 90-minute premiere in a
/// 25-minute series was scored against three different expected sizes, which is
/// several size bands apart — enough to admit at grab and refuse at import.
#[test]
fn one_episode_scores_the_same_at_grab_at_import_and_as_a_bar() {
    use crate::acquisition_coverage::{
        ReleaseCoverage, coverage_size_basis, episode_span_size_basis,
    };

    let profile = series_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();

    // A 90-minute premiere in a series whose nominal runtime is 25 minutes.
    const TITLE_RUNTIME_MINUTES: Option<i32> = Some(25);
    let episodes = vec![feature_length_episode("ep-01", 90 * 60)];
    let episode_ids = vec!["ep-01".to_string()];

    let parsed = parse_release_metadata("Glass.Harbor.S01E01.1080p.WEB-DL.H.264-GRP");
    let size_bytes = (5.5 * GIB as f64) as i64;

    // 1. Grab: the coverage's basis.
    let grab_basis = coverage_size_basis(
        &ReleaseCoverage::SingleEpisode("ep-01".to_string()),
        &parsed,
        &episodes,
        TITLE_RUNTIME_MINUTES,
    );
    // 2. Import: the target episodes' basis.
    let import_basis = episode_span_size_basis(&episodes, &episode_ids, TITLE_RUNTIME_MINUTES);
    // 3. Incumbent bar: the file's `covers`.
    let bar_basis = episode_span_size_basis(&episodes, &episode_ids, TITLE_RUNTIME_MINUTES);

    // One episode is one member, so the total and the member runtime are the
    // same number and no aggregate reinterpretation is even reachable.
    assert_eq!(grab_basis, CoverageSizeBasis::single(Some(90)));
    assert_eq!(grab_basis, import_basis);
    assert_eq!(grab_basis, bar_basis);

    let evidence = ReleaseEvidence::announced(parsed.clone(), Some(size_bytes));
    let grab_score = score_release(
        &evidence,
        &episode_ctx(&profile, &weights, &tags, grab_basis),
    )
    .release_score;
    let import_score = score_release(
        &evidence,
        &episode_ctx(&profile, &weights, &tags, import_basis),
    )
    .release_score;
    let bar_score = score_release(
        &evidence,
        &episode_ctx(&profile, &weights, &tags, bar_basis),
    )
    .release_score;

    assert_eq!(grab_score, import_score);
    assert_eq!(grab_score, bar_score);

    // And the fix is load-bearing: scoring the same file against the series
    // average gives a different number, which is the bug this pins.
    let title_basis = score_release(
        &evidence,
        &episode_ctx(
            &profile,
            &weights,
            &tags,
            CoverageSizeBasis::single(TITLE_RUNTIME_MINUTES),
        ),
    )
    .release_score;
    assert_ne!(
        grab_score, title_basis,
        "fixture precondition: the episode runtime must actually move the score"
    );
}

/// The multi-episode shape: a two-episode file is scored against the sum of its
/// episodes, at all three sites.
#[test]
fn a_multi_episode_file_scores_against_the_sum_of_its_episodes() {
    use crate::acquisition_coverage::{
        ReleaseCoverage, coverage_size_basis, episode_span_size_basis,
    };

    let mut first = feature_length_episode("ep-01", 24 * 60);
    first.episode_number = Some("1".to_string());
    let mut second = feature_length_episode("ep-02", 24 * 60);
    second.episode_number = Some("2".to_string());
    let episodes = vec![first, second];
    let episode_ids = vec!["ep-01".to_string(), "ep-02".to_string()];

    let parsed = parse_release_metadata("Glass.Harbor.S01E01E02.1080p.WEB-DL.H.264-GRP");
    let grab_basis = coverage_size_basis(
        &ReleaseCoverage::EpisodeSet(episode_ids.clone()),
        &parsed,
        &episodes,
        Some(24),
    );
    let import_basis = episode_span_size_basis(&episodes, &episode_ids, Some(24));

    assert_eq!(grab_basis.total_runtime_minutes, Some(48));
    // Two members of 24 minutes each: the basis carries both the sum and the
    // member, so an aggregate whose listed size is one episode's can be read
    // that way at grab and at import alike.
    assert_eq!(grab_basis.member_runtime_minutes, Some(24));
    assert_eq!(grab_basis.member_count, 2);
    assert_eq!(grab_basis, import_basis);
}

/// **The pack shape, at every site.** A season pack listed under one episode's
/// byte count is read as that member — and it has to be read that way by the
/// grab lane, by the import rescore and by a re-derived incumbent bar, or the
/// three disagree about the same release again.
///
/// The evidence is identical at all three; only the caller differs. So is the
/// number.
#[test]
fn a_pack_listed_at_one_members_size_scores_the_same_everywhere() {
    use crate::acquisition_coverage::{
        ReleaseCoverage, coverage_size_basis, episode_span_size_basis,
    };

    let profile = series_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();

    // A twelve-episode season of 45-minute episodes.
    let episodes = (1..=12)
        .map(|number| {
            let mut episode = feature_length_episode(&format!("ep-{number:02}"), 45 * 60);
            episode.episode_number = Some(number.to_string());
            episode
        })
        .collect::<Vec<_>>();
    let episode_ids = episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect::<Vec<_>>();

    let parsed = parse_release_metadata("Quiet.Meridian.S01.1080p.WEB-DL.H.264-GroupTag");
    // What the indexer reported: one episode, not twelve.
    let size_bytes = (2.5 * GIB as f64) as i64;

    let grab_basis = coverage_size_basis(
        &ReleaseCoverage::Collection("season-1".to_string()),
        &parsed,
        &episodes,
        Some(45),
    );
    let import_basis = episode_span_size_basis(&episodes, &episode_ids, Some(45));
    let bar_basis = episode_span_size_basis(&episodes, &episode_ids, Some(45));

    assert_eq!(grab_basis, import_basis);
    assert_eq!(grab_basis, bar_basis);
    assert_eq!(grab_basis.total_runtime_minutes, Some(540));
    assert_eq!(grab_basis.member_runtime_minutes, Some(45));
    assert_eq!(grab_basis.member_count, 12);

    let evidence = ReleaseEvidence::announced(parsed.clone(), Some(size_bytes));
    let scored_with =
        |basis| score_release(&evidence, &episode_ctx(&profile, &weights, &tags, basis));
    let grab = scored_with(grab_basis);
    let import = scored_with(import_basis);
    let bar = scored_with(bar_basis);

    assert_eq!(grab.release_score, import.release_score);
    assert_eq!(grab.release_score, bar.release_score);
    assert_eq!(grab.total, import.total);
    assert_eq!(grab.total, bar.total);

    // And the release survives, on the member reading, with the interpretation
    // named in the log.
    assert!(
        grab.announced_decision.allowed,
        "{:?}",
        grab.announced_decision.block_codes
    );
    assert!(
        grab.announced_decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == SIZE_PACK_MEMBER_BASIS_CODE),
        "{:?}",
        grab.announced_decision.scoring_log
    );

    // Scored against the season's total runtime with no member to fall back on
    // — the reading before this change — the same bytes take the tiny penalty.
    let total_only = scored_with(CoverageSizeBasis::single(Some(540)));
    assert!(
        total_only
            .announced_decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == "size_tiny_for_quality"),
        "fixture precondition: the member reading must be doing the work"
    );
    assert!(grab.release_score > total_only.release_score);
}

// ── I2 / I4: grab and import agree ──────────────────────────────────────────

/// **The grab-vs-import equality test.** For a table of releases against a sweep
/// of incumbent bars: whatever the grab policy admits, the import policy admits
/// too, on the same release once it has landed.
///
/// This is invariants I2 and I4 made executable. The candidate number on the
/// grab side is announced evidence (the advertised byte count); on the import
/// side it is landed evidence (the real file, a few percent smaller). Before D3
/// that few percent could cross a size bucket and move the number by 700, so a
/// release admitted at grab by +250 was refused at import as a downgrade — the
/// download completed and was thrown away.
#[test]
fn whatever_grab_admits_import_admits() {
    use crate::admission::{
        AdmissionPolicy, AdmissionScope, AdmissionSubject, CandidateFacts, Incumbent,
        evaluate_admission,
    };

    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    // Each release with sizes that are plausible for what it claims (a
    // 120-minute movie, the category default). An implausible pairing would test
    // the flat ends of the size curve, not the drift the invariant is about.
    let corpus: &[(&str, &[f64])] = &[
        (
            "Portmere.2024.2160p.WEB-DL.DDP5.1.H.265-GRP",
            &[24.0, 30.0, 38.0],
        ),
        ("Portmere.2024.1080p.WEB-DL.H.264-GRP", &[6.0, 7.5, 9.5]),
        ("Portmere.2024.1080p.BluRay.x265-GRP", &[7.0, 9.0, 11.0]),
        ("Portmere.2024.1080p.WEB-DL.AV1-GRP", &[3.0, 4.5, 6.0]),
        (
            "Portmere.2024.2160p.BluRay.REMUX.HDR-GRP",
            &[85.0, 105.0, 135.0],
        ),
    ];

    let grab_policy = AdmissionPolicy {
        allow_upgrades: true,
        min_delta: 200,
        cutoff_score: None,
        manual_override: false,
        applies_to_queue: false,
    };
    let import_policy = AdmissionPolicy::not_a_downgrade();

    for (release, advertised_gib) in corpus {
        let parsed = parse_release_metadata(release);
        for gib in advertised_gib.iter().copied() {
            let advertised_bytes = (gib * GIB as f64) as i64;
            // Landed 10% short: par2 and RAR overhead are in the NZB's size and
            // not in the video file.
            let landed_bytes = (advertised_bytes as f64 * 0.9) as i64;

            let announced_evidence =
                ReleaseEvidence::announced(parsed.clone(), Some(advertised_bytes));
            let landed_evidence = ReleaseEvidence::announced(parsed.clone(), Some(landed_bytes))
                .with_analysis(analyzed(gib * 0.9, None));

            let at_grab = score_release(&announced_evidence, &context);
            let at_import = score_release(&landed_evidence, &context);

            // D3: the drift moves the number by a bounded amount rather than a
            // whole bucket. 300 rather than 100 because this corpus deliberately
            // includes sizes that straddle the `small`/`slightly_small`
            // boundary, where the Balanced weights step 700 across a 1.35× band
            // — the steepest place on the curve, and the one where grab and
            // import used to disagree outright. Away from it the movement is
            // tens of points; `realistic_landed_drift_moves_the_size_term_only_slightly`
            // pins that at 100.
            assert!(
                (at_grab.release_score - at_import.total).abs() <= 300,
                "`{release}` at {gib} GiB: grab scored {} and import {} \
                 ({} apart)",
                at_grab.release_score,
                at_import.total,
                (at_grab.release_score - at_import.total).abs()
            );

            let tier = crate::quality_profile::quality_tier_index(
                &profile.criteria,
                at_grab.parsed_quality.as_deref(),
            );

            for bar in [-500, 0, 250, 500, 900, 1_500, 3_000] {
                for incumbent_tier in [Some(0), Some(1), None] {
                    let subject = AdmissionSubject::new(
                        AdmissionScope::Title,
                        [(
                            Incumbent {
                                tier_index: incumbent_tier,
                                revision: 0,
                                file_id: "file-1".into(),
                                file_path: "/data/Movies/Portmere (2024).mkv".into(),
                                release_group: None,
                                score: bar,
                                covers: Vec::new(),
                                created_at: "2026-01-01T00:00:00Z".into(),
                            },
                            true,
                        )],
                    );

                    let grabbed = evaluate_admission(
                        &subject,
                        CandidateFacts::new(tier, at_grab.revision, at_grab.release_score),
                        &grab_policy,
                    );
                    if !grabbed.is_admitted() {
                        continue;
                    }
                    let imported = evaluate_admission(
                        &subject,
                        CandidateFacts::new(tier, at_import.revision, at_import.total),
                        &import_policy,
                    );
                    assert!(
                        imported.is_admitted(),
                        "`{release}` at {gib} GiB was grabbed against a bar of {bar} \
                         (tier {incumbent_tier:?}) and then refused at import: \
                         grab {} vs import {} -> {imported:?}",
                        at_grab.release_score,
                        at_import.total,
                    );
                }
            }
        }
    }
}

// ── D5: one FileDoc constructor ─────────────────────────────────────────────

/// **The FileDoc round trip**, rooted in a `scryer_mediainfo::MediaAnalysis` —
/// the type the probe actually returns.
///
/// Import used to build its rule document straight from that type
/// (`build_file_doc`), which copies the probe's raw codec string: `"h264"`.
/// Re-deriving an incumbent's bar goes through `MediaFileAnalysis`, whose
/// `video_codec` is a parsed `VideoCodec` rendered as `"H.264"`. Any user rule
/// reading `input.file.video_codec` therefore scored one way at import and
/// another way forever after — and the existing round-trip test could not see
/// it, because both of its documents came from `file_doc_from_analysis`.
///
/// One constructor now: import builds
/// `file_doc_from_analysis(&build_media_file_analysis(&mediainfo))`.
#[cfg(feature = "runtime-media-analysis")]
#[test]
fn a_file_doc_survives_mediainfo_to_row_to_re_derivation() {
    // Constructed rather than probed: the point is the conversion chain, and a
    // real probe would only exercise whatever the fixture file happens to hold.
    let mediainfo = scryer_mediainfo::MediaAnalysis {
        video_codec: Some("h264".to_string()),
        video_width: Some(1920),
        video_height: Some(1080),
        video_bitrate_kbps: Some(8_500),
        video_bit_depth: Some(8),
        video_hdr_format: None,
        dovi_profile: None,
        dovi_bl_compat_id: None,
        video_frame_rate: Some("23.976".to_string()),
        video_profile: Some("High".to_string()),
        audio_codec: Some("eac3".to_string()),
        audio_profile: Some("Atmos".to_string()),
        audio_channels: Some(6),
        audio_bitrate_kbps: Some(640),
        audio_languages: vec!["eng".to_string(), "jpn".to_string()],
        audio_streams: vec![
            scryer_mediainfo::AudioStreamDetail {
                codec: Some("eac3".to_string()),
                profile: Some("Atmos".to_string()),
                channels: Some(6),
                language: Some("eng".to_string()),
                name: Some("English".to_string()),
                bitrate_kbps: Some(640),
            },
            scryer_mediainfo::AudioStreamDetail {
                codec: Some("aac".to_string()),
                profile: Some("LC".to_string()),
                channels: Some(2),
                language: Some("jpn".to_string()),
                name: None,
                bitrate_kbps: Some(192),
            },
        ],
        subtitle_languages: vec!["eng".to_string()],
        subtitle_codecs: vec!["subrip".to_string()],
        subtitle_streams: vec![scryer_mediainfo::SubtitleStreamDetail {
            codec: Some("subrip".to_string()),
            language: Some("eng".to_string()),
            name: Some("English (SDH)".to_string()),
            forced: false,
            default: true,
        }],
        has_multiaudio: true,
        duration_seconds: Some(7_020),
        num_chapters: Some(12),
        container_format: Some("matroska".to_string()),
    };

    // What import computes and persists.
    let stored_analysis = crate::post_download_gate::build_media_file_analysis(&mediainfo);
    let at_import = crate::user_rule_input::file_doc_from_analysis(&stored_analysis);

    // The row import writes, then the document re-derived from it.
    let evidence = announced(8.0).with_analysis(AnalyzedFacts {
        analysis: stored_analysis,
        actual_size_bytes: 8 * GIB,
        rule_file_doc: Some(at_import.clone()),
    });
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let scored = score_release(&evidence, &ctx(&profile, &weights, &tags));
    let row = media_row_as_import_would_write(&evidence, &scored);
    let re_derived = analyzed_facts_from_media_file(&row)
        .rule_file_doc
        .expect("re-derivation always builds a file document");

    assert_eq!(
        serde_json::to_value(&at_import).expect("import doc serializes"),
        serde_json::to_value(&re_derived).expect("re-derived doc serializes"),
        "the rule document a rule sees at import differs from the one it sees \
         when the bar is re-derived"
    );
    // Named explicitly, because this is the field that actually differed.
    assert_eq!(
        at_import.video_codec.as_deref(),
        Some("H.264"),
        "the canonical codec string, not the probe's raw `h264`"
    );
    assert_eq!(at_import.video_codec, re_derived.video_codec);
}

// ── MA1: which vetoes count as the release lying ─────────────────────────────
//
// `Blocked` costs the release a blocklist entry and reopens the search, so it
// has to mean the announcement *asserted* the field the veto keys on and the
// file contradicted it. A veto the probe raises for a fact the name never
// stated is `Vetoed` — held for the operator — because the next silent release
// would burn identically, which is the loop the whole verdict exists to avoid.
// Design §9, "Truth verdicts".

/// A release whose name says nothing about codec, against a codec blocklist.
/// The old "introduced" rule read that silence as a lie.
#[test]
fn a_veto_on_a_fact_the_name_never_stated_is_vetoed_not_blocked() {
    let blocking = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"video_codec_blocklist":["H.264"]}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&blocking, &weights, &tags);

    // No codec token anywhere in the name.
    let silent = ReleaseEvidence::announced(
        parse_release_metadata("Movie.2024.1080p.WEB-DL-GRP"),
        Some(8 * GIB),
    );
    assert!(
        silent.parsed.video_codec.is_none(),
        "fixture precondition: the name must be codec-silent"
    );
    let scored = score_release(&silent.with_analysis(analyzed(8.0, Some("h264"))), &context);

    let TruthVerdict::Vetoed { codes } = &scored.truth_verdict else {
        panic!(
            "silence is not a claim; expected Vetoed, got {:?}",
            scored.truth_verdict
        );
    };
    assert_eq!(codes, &["video_codec_in_profile_blocklist".to_string()]);
    // And the bar stays honest: a veto is never a number (I5).
    assert!(scored.total > BLOCK_SCORE / 2);
}

/// The same veto, once the name made the claim. Now it is a lie.
#[test]
fn a_veto_that_contradicts_a_stated_codec_is_blocked() {
    let blocking = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"video_codec_blocklist":["H.264"]}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&blocking, &weights, &tags);

    let stated = ReleaseEvidence::announced(
        parse_release_metadata("Movie.2024.1080p.WEB-DL.H.265-GRP"),
        Some(8 * GIB),
    );
    assert!(
        stated.parsed.video_codec.is_some(),
        "fixture precondition: the name must state a codec"
    );
    let scored = score_release(&stated.with_analysis(analyzed(8.0, Some("h264"))), &context);

    let TruthVerdict::Blocked { codes } = &scored.truth_verdict else {
        panic!(
            "a stated codec the file contradicts is a lie; got {:?}",
            scored.truth_verdict
        );
    };
    assert!(codes.contains(&"video_codec_in_profile_blocklist".to_string()));
}

/// HDR is read out of `video_hdr_format`; no release name is obliged to carry
/// it. A profile that forbids HDR would otherwise blocklist every untagged HDR
/// release in the catalogue, one release at a time.
#[test]
fn an_hdr_veto_the_name_never_disclosed_is_vetoed() {
    let no_hdr = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"detected_hdr_allowed":false}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&no_hdr, &weights, &tags);

    let mut facts = analyzed(8.0, Some("h264"));
    facts.analysis.video_hdr_format = Some("HDR10".to_string());
    facts.rule_file_doc = Some(crate::user_rule_input::file_doc_from_analysis(
        &facts.analysis,
    ));
    let scored = score_release(&announced(8.0).with_analysis(facts), &context);

    let TruthVerdict::Vetoed { codes } = &scored.truth_verdict else {
        panic!(
            "an HDR flag the name never carried is not a lie; got {:?}",
            scored.truth_verdict
        );
    };
    assert!(codes.contains(&"hdr_not_allowed".to_string()));
}

/// Resolution is the one claim every release name makes, and it is the claim
/// the grab decision was taken on. A file measuring outside the profile's tiers
/// is not what was fetched.
#[test]
fn a_landed_resolution_outside_the_profile_tiers_is_blocked() {
    let profile_1080_only = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile_1080_only, &weights, &tags);

    let mut facts = analyzed(8.0, Some("h264"));
    facts.analysis.video_width = Some(1280);
    facts.analysis.video_height = Some(720);
    let scored = score_release(&announced(8.0).with_analysis(facts), &context);

    let TruthVerdict::Blocked { codes } = &scored.truth_verdict else {
        panic!(
            "a 720p file sold as 1080p in a 1080p/2160p profile is a lie; got {:?}",
            scored.truth_verdict
        );
    };
    assert!(codes.contains(&"quality_not_in_profile_tiers".to_string()));
    assert!(
        codes
            .iter()
            .any(|code| code.starts_with("quality_contradicted:")),
        "the contradiction rides along for the operator: {codes:?}"
    );
}

/// A block that fires on both passes stays `Consistent`, and the file imports
/// with its honest bar. Sonarr has no import-time allow-list gate; the grab
/// side is the gate.
#[test]
fn a_block_on_both_passes_imports_with_an_honest_bar() {
    let blocked_codec = profile(
        r#"{"id":"t","name":"T","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"video_codec_blocklist":["H.264"]}}"#,
    );
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&blocked_codec, &weights, &tags);

    // The name says H.264 and so does the file.
    let scored = score_release(
        &announced(8.0).with_analysis(analyzed(8.0, Some("h264"))),
        &context,
    );

    assert_eq!(
        scored.truth_verdict,
        TruthVerdict::Consistent,
        "the same veto on both passes is the profile refusing the name"
    );
    assert!(
        !scored
            .analyzed_decision
            .as_ref()
            .expect("the analyzed pass ran")
            .allowed,
        "the veto is still reported on the decision"
    );
    assert!(
        scored.total > BLOCK_SCORE / 2,
        "the bar must be the honest preference score, got {}",
        scored.total
    );
}

/// A user rule can only see `input.file.*` on the analyzed pass, so *every*
/// rule keyed on probe facts is structurally analyzed-only. That is operator
/// policy, never a misrepresentation — and it holds however the rule spells its
/// code, which is why the partition reads the entry's source rather than
/// matching the string.
#[test]
fn a_file_rule_block_is_vetoed_however_it_is_named() {
    let policy = scryer_rules::UserPolicy {
        id: "no_eight_bit".to_string(),
        name: "No 8-bit".to_string(),
        rego_source: scryer_rules::rewrite_package_declaration(
            r#"
score_entry["quality_not_in_profile_tiers"] := -10000 if {
    input.file != null
    input.file.video_codec == "H.264"
}
"#,
            "no_eight_bit",
        ),
        origin: scryer_rules::PolicyOrigin::User,
        applied_facets: vec!["movie".to_string()],
    };
    let engine =
        scryer_rules::UserRulesEngine::build(&[policy]).expect("rule fixture should compile");
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let mut context = ctx(&profile, &weights, &tags);
    context.rules = Some(&engine);

    let scored = score_release(
        &announced(8.0).with_analysis(analyzed(8.0, Some("h264"))),
        &context,
    );

    let TruthVerdict::Vetoed { codes } = &scored.truth_verdict else {
        panic!(
            "an operator rule reading probe facts is policy, not a lie; got {:?}",
            scored.truth_verdict
        );
    };
    // Deliberately named like a builtin quality veto: the source is what
    // decides, not the string.
    assert_eq!(codes, &["quality_not_in_profile_tiers".to_string()]);
}

// ── Size basis: grab and import agree inside the overhead band (option c) ───

#[test]
fn a_landed_file_inside_the_overhead_band_is_scored_on_its_announced_size() {
    let announced = 10 * GIB;
    let at_the_edge = (announced as f64 * SIZE_OVERHEAD_TOLERANCE) as i64;
    assert_eq!(size_basis_bytes(at_the_edge, Some(announced)), announced);
    assert_eq!(size_basis_bytes(announced - 1, Some(announced)), announced);
    assert_eq!(size_basis_bytes(announced, Some(announced)), announced);
    // A modest excess is still packaging/measurement drift, so the grab's
    // number stands inside the reciprocal band too.
    assert_eq!(
        size_basis_bytes(announced + GIB, Some(announced)),
        announced
    );
}

#[test]
fn a_real_shortfall_is_scored_on_what_landed() {
    let announced = 10 * GIB;
    let at_the_edge = (announced as f64 * SIZE_OVERHEAD_TOLERANCE) as i64;
    assert_eq!(
        size_basis_bytes(at_the_edge - 1, Some(announced)),
        at_the_edge - 1
    );
    let short = (announced as f64 * 0.80) as i64;
    assert_eq!(size_basis_bytes(short, Some(announced)), short);
}

#[test]
fn a_material_excess_is_scored_on_what_landed() {
    let announced = 10 * GIB;
    let materially_larger = 12 * GIB;
    assert_eq!(
        size_basis_bytes(materially_larger, Some(announced)),
        materially_larger
    );
    assert_eq!(
        persisted_announced_size_bytes(materially_larger, Some(announced)),
        None,
        "a discarded announcement must not become the incumbent's size basis"
    );
}

#[test]
fn a_landed_aggregate_does_not_keep_a_member_sized_announcement() {
    let profile = series_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let basis = CoverageSizeBasis::aggregate(Some(12 * 45), Some(45), 12);
    let parsed = parse_release_metadata("Quiet.Meridian.S01.1080p.WEB-DL.H.264-GroupTag");
    let announced = (2.5 * GIB as f64) as i64;
    let landed = 30 * GIB;

    let selected = size_basis_bytes(landed, Some(announced));
    assert_eq!(selected, landed);
    assert_eq!(
        persisted_announced_size_bytes(landed, Some(announced)),
        None
    );

    let landed_score = score_release(
        &ReleaseEvidence::announced(parsed.clone(), Some(selected)),
        &episode_ctx(&profile, &weights, &tags, basis),
    );
    assert!(
        !landed_score
            .announced_decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == SIZE_PACK_MEMBER_BASIS_CODE),
        "known aggregate bytes were still read as one member: {:?}",
        landed_score.announced_decision.scoring_log
    );

    let member_score = score_release(
        &ReleaseEvidence::announced(parsed, Some(announced)),
        &episode_ctx(&profile, &weights, &tags, basis),
    );
    assert!(
        member_score
            .announced_decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == SIZE_PACK_MEMBER_BASIS_CODE),
        "fixture precondition: the announcement must look member-sized"
    );
    assert!(landed_score.total > member_score.total);
}

#[test]
fn without_an_announced_size_the_landed_size_is_the_basis() {
    assert_eq!(size_basis_bytes(7 * GIB, None), 7 * GIB);
    assert_eq!(size_basis_bytes(7 * GIB, Some(0)), 7 * GIB);
    assert_eq!(size_basis_bytes(7 * GIB, Some(-1)), 7 * GIB);
}

/// The point of the rule: across the whole overhead band the size term the
/// import scores is the size term the grab scored, and a real shortfall is not
/// papered over.
#[test]
fn inside_the_overhead_band_the_size_term_matches_the_grab() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);
    let parsed = parse_release_metadata("Glass.Harbor.2021.1080p.WEB-DL.H.264-GRP");
    let announced = 8 * GIB;

    let grab = score_release(
        &ReleaseEvidence::announced(parsed.clone(), Some(announced)),
        &context,
    )
    .release_score;
    for permille in (850..=1000).step_by(5) {
        // Round up so the 850‰ edge lands on the band, not one byte below it.
        let landed = ((announced as f64) * (permille as f64 / 1000.0)).ceil() as i64;
        let import = score_release(
            &ReleaseEvidence::announced(
                parsed.clone(),
                Some(size_basis_bytes(landed, Some(announced))),
            ),
            &context,
        )
        .release_score;
        assert_eq!(
            grab, import,
            "a file that landed at {permille}\u{2030} of its announced size must score like the grab"
        );
    }

    // Load-bearing: below the band the landed bytes are the basis, and that
    // moves the number.
    let short = ((announced as f64) * 0.8) as i64;
    let landed_basis = score_release(
        &ReleaseEvidence::announced(
            parsed.clone(),
            Some(size_basis_bytes(short, Some(announced))),
        ),
        &context,
    )
    .release_score;
    assert_ne!(
        grab, landed_basis,
        "fixture precondition: a 20% shortfall must actually move the size term"
    );
}

// ── Option c, round trip: the row remembers the size it was scored on ───────

#[test]
fn only_an_engaged_announced_size_is_persisted_on_the_row() {
    let announced = 8 * GIB;
    assert_eq!(
        persisted_announced_size_bytes((7.2 * GIB as f64) as i64, Some(announced)),
        Some(announced),
        "inside the band the import scored on the announced size, so the row keeps it"
    );
    assert_eq!(
        persisted_announced_size_bytes(6 * GIB, Some(announced)),
        None,
        "a real shortfall was scored on the landed size; nothing to remember"
    );
    assert_eq!(persisted_announced_size_bytes(6 * GIB, None), None);
}

/// I7 for option c: a file that landed inside the overhead band was scored on
/// its announced size; the row remembers that size, so re-deriving its bar
/// reproduces the import score — and forgetting it would not.
#[test]
fn a_row_that_remembers_its_announced_size_reproduces_the_import_score() {
    let profile = movie_profile();
    let weights = balanced_weights();
    let tags: Vec<String> = Vec::new();
    let context = ctx(&profile, &weights, &tags);

    // Announced 8 GiB, landed 7.2 GiB (90 %): inside the band, so both of the
    // import's passes scored the size term on 8 GiB.
    let announced_bytes = 8 * GIB;
    let landed_bytes = (7.2 * GIB as f64) as i64;
    assert_eq!(
        size_basis_bytes(landed_bytes, Some(announced_bytes)),
        announced_bytes
    );
    let evidence = announced(8.0).with_analysis(analyzed(8.0, None));
    let at_import = score_release(&evidence, &context);

    let mut row = media_row_as_import_would_write(&evidence, &at_import);
    row.size_bytes = landed_bytes;
    row.announced_size_bytes = persisted_announced_size_bytes(landed_bytes, Some(announced_bytes));
    let re_derived = score_media_file(&row, &context);
    assert_eq!(
        re_derived.total, at_import.total,
        "the bar must reproduce the import score: import wrote {}, row yields {}",
        at_import.total, re_derived.total
    );

    // Load-bearing: a row that forgot its announcement drifts to the landed size.
    row.announced_size_bytes = None;
    let landed_only = score_media_file(&row, &context);
    assert_ne!(
        landed_only.total, at_import.total,
        "fixture precondition: a 10 % shortfall must move the size term"
    );
}
