//! Canonical release scoring.
//!
//! One formula, one term set, two evidence levels. Every score Scryer compares
//! — at grab, at import, and when re-deriving an incumbent's bar — comes from
//! this module, so a release cannot score differently depending on which stage
//! is asking.
//!
//! ## The two computations
//!
//! ```text
//! release_score  = announced pass          → grab decisions (no file, and upgrades)
//! total          = analyzed pass, vetoes excluded
//!                                          → the import score; PERSISTED; the upgrade bar
//! ```
//!
//! With no analysis there is only one pass, and `total` is that pass with its
//! vetoes excluded. `truth_variance` is the difference between the two passes,
//! reported rather than folded in: it says *how far* the file drifted from its
//! announcement, while `truth_verdict` says whether that drift was a lie. Because
//! both passes go through one function with one term set, formula drift between
//! stages is structurally impossible rather than merely tested for.
//!
//! ## What is deliberately not here
//!
//! - **Incumbent state.** `has_existing_file`, `existing_score`, `allow_upgrades`,
//!   upgrade deltas and cooldowns belong to admission, not to a release's
//!   intrinsic worth. A file's persisted score must not depend on what happened
//!   to be on disk the day it landed, or it is useless as the next bar.
//! - **Listing metadata.** Release age, indexer votes, password hints, indexer
//!   priority and pack-coverage preferences describe a *listing*, not a
//!   release. They cannot be reconstructed from a media row, so admitting them
//!   here would make a stored score unreproducible. They belong to search rank.
//! - **Hard blocks as numbers.** A `BLOCK_SCORE` discovered by the analyzed pass
//!   is a verdict, not a bounded numeric adjustment. See [`TruthVerdict`].
//!
//! ## Where this sits in the loop
//!
//! ```text
//! Found → Parsed → Scored(announced) → Ranked → Decided → Grabbed
//!       → Completed → Probed → Scored(landed) + Verdict → Admitted | Vetoed
//!       → Persisted(bar)
//! ```
//!
//! Both `Scored` steps are this module, and so is `Verdict`. Nothing else here:
//! ranking belongs to [`crate::acquisition::scoring`], the comparison to
//! [`crate::admission`], and what a verdict *costs* to
//! [`crate::import::decide`]. The bar a later comparison uses is re-derived from
//! the media row through this same function, which is why a stored score is
//! display-only and cannot become the source of truth for later comparisons.

use crate::quality_profile::{
    BLOCK_SCORE, QualityProfileDecision, ScoringEntry, ScoringSource, apply_min_score_gate,
    apply_size_scoring_for_category_with_remux_preference, evaluate_against_profile_for_category,
    normalize_quality_tier,
};
use crate::scoring_weights::ScoringWeights;
use crate::{MediaFileAnalysis, ParsedReleaseMetadata, QualityProfile};

/// Cap on how far analyzed evidence may move a score before the difference stops
/// being a *variance* and becomes a *contradiction*.
///
/// One size-bucket step on the Balanced curve. A release that probes one bucket
/// away from its announcement is the ordinary case — packaging overhead, a
/// slightly short episode. A release that moves further than this was not the
/// release it claimed to be, and that is a truth verdict for admission to act
/// on, not a number to fold into the bar.
pub(crate) const TRUTH_VARIANCE_BOUND: i32 = 700;

/// The single `search_mode` value handed to score-bearing rules.
///
/// Rules must not be able to score a release differently at grab than at
/// import; the stage is not a property of the release. Retained as a field only
/// because the rule input contract still carries it.
pub(crate) const CANONICAL_RULE_SEARCH_MODE: &str = "canonical";

/// What the file turned out to be, once it existed on disk and was probed.
///
/// Buildable from an import-time acceptance *and* from a stored media row, and
/// those two must agree: every comparison re-derives the incumbent's bar, so
/// anything one path carries and the other drops becomes a permanent skew.
#[derive(Debug, Clone)]
pub(crate) struct AnalyzedFacts {
    pub analysis: MediaFileAnalysis,
    pub actual_size_bytes: i64,
    pub rule_file_doc: Option<scryer_rules::FileDoc>,
}

/// Everything intrinsic to a release, at whatever evidence level is available.
///
/// `analyzed` is `None` before the bytes exist. It is the only field that
/// differs between a grab-time and an import-time view of the same release.
#[derive(Debug, Clone)]
pub(crate) struct ReleaseEvidence {
    pub parsed: ParsedReleaseMetadata,
    pub announced_size_bytes: Option<i64>,
    pub analyzed: Option<AnalyzedFacts>,
}

impl ReleaseEvidence {
    /// Evidence as advertised, before any bytes have been probed.
    pub(crate) fn announced(parsed: ParsedReleaseMetadata, size_bytes: Option<i64>) -> Self {
        Self {
            parsed,
            announced_size_bytes: size_bytes,
            analyzed: None,
        }
    }

    pub(crate) fn with_analysis(mut self, analyzed: AnalyzedFacts) -> Self {
        self.analyzed = Some(analyzed);
        self
    }
}

/// Title- and profile-level facts the scorer needs. Resolved by the caller so
/// that scoring itself stays pure and synchronous — the canonicality invariants
/// are property tests, and they cannot be if scoring reaches for a database.
///
/// Note the absence of any incumbent field. That absence is the point.
pub(crate) struct ScoringContext<'a> {
    pub profile: &'a QualityProfile,
    pub weights: &'a ScoringWeights,
    pub required_audio_languages: &'a [String],
    pub category: &'a str,
    /// What size scoring compares the reported bytes against: the coverage's
    /// total runtime, one member's, and the member count. Resolved once per
    /// scope by the caller so every lane reads the same basis for the same
    /// evidence.
    pub size_basis: crate::quality_profile::CoverageSizeBasis,
    pub rules: Option<&'a scryer_rules::UserRulesEngine>,
    pub title_id: Option<&'a str>,
    pub library_name: Option<&'a str>,
    pub original_language: Option<&'a str>,
    pub original_country: Option<&'a str>,
    pub title_tags: &'a [String],
    pub is_filler: bool,
}

/// Whether the file backed up what the release claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TruthVerdict {
    /// No analysis yet, or the file matched its announcement within bounds.
    Consistent,
    /// The file differs from its announcement by more than [`TRUTH_VARIANCE_BOUND`],
    /// or a scored fact changed materially. The release was mis-advertised.
    Contradicted { codes: Vec<String> },
    /// The announcement **asserted** the field a veto keys on and the file
    /// contradicts it: the name said H.265 and the stream is a blocklisted
    /// H.264, the landed resolution is outside the profile's tiers. The release
    /// lied, and the import gate burns it for that (blocklist + reopen).
    ///
    /// Deliberately *not* "the analyzed pass is blocked", and deliberately not
    /// even "the analyzed pass carries a block the announced pass did not". Two
    /// separate loops hide in the weaker readings:
    ///
    /// - A block that fires on **both** passes is the profile refusing the
    ///   release's *name*, which is a grab-side decision. Acting on it at import
    ///   blocklists a correct file's release and reopens a search that produces
    ///   the same verdict for the next candidate.
    /// - A block the probe introduces for a field the name **never stated** is
    ///   the profile refusing the *file*, not the release lying. A codec-silent
    ///   name against a codec blocklist, HDR or Dolby Vision read out of
    ///   `video_hdr_format`, a file rule keyed on `input.file.*` — every one of
    ///   those fires identically for the next codec-silent release. That is
    ///   [`TruthVerdict::Vetoed`], an import failure that blocklists this
    ///   release and lets convergence test the next candidate.
    ///
    /// Never expressible as a number either way.
    Blocked { codes: Vec<String> },
    /// The file violates a profile veto **the announcement never disclosed**.
    ///
    /// Nothing here says the release misrepresented itself; the name was simply
    /// silent about a fact the probe supplies, and the profile refuses that
    /// fact. The import is rejected-and-reopened: each burned release is
    /// excluded from the next search, so convergence continues until one
    /// imports successfully.
    Vetoed { codes: Vec<String> },
}

#[allow(
    dead_code,
    reason = "read by truth-verdict rejection, which lands in its own change"
)]
impl TruthVerdict {
    pub(crate) fn is_consistent(&self) -> bool {
        matches!(self, Self::Consistent)
    }

    pub(crate) fn codes(&self) -> &[String] {
        match self {
            Self::Consistent => &[],
            Self::Contradicted { codes } | Self::Blocked { codes } | Self::Vetoed { codes } => {
                codes
            }
        }
    }
}

/// The result of one canonical scoring run.
#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "release_score and truth_variance report the decomposition behind \
              `total`; both are read by truth-verdict rejection, which lands in \
              its own change"
)]
pub(crate) struct ScoredRelease {
    /// Score from announced evidence alone. This is what a grab decision uses,
    /// and what an upgrade candidate is measured by.
    pub release_score: i32,
    /// Full term log of the announced pass, including block codes.
    pub announced_decision: QualityProfileDecision,
    /// Term log of the analyzed pass, when there was one. This is the pass that
    /// set `total`, so it is what a scoring log should show.
    pub analyzed_decision: Option<QualityProfileDecision>,
    /// Bounded delta contributed purely by analyzed evidence. `0` when there is
    /// no analysis — never a penalty for a probe that could not run.
    pub truth_variance: i32,
    pub truth_verdict: TruthVerdict,
    /// The quality tier this release parsed as, from whichever evidence level
    /// set the score. Admission compares tier before score, and taking it from
    /// the same pass keeps the two consistent.
    pub parsed_quality: Option<String>,
    /// PROPER/REPACK rank, from the **announced** parse
    /// ([`crate::acquisition::scoring::revision_rank`]). Admission compares it
    /// between tier and score.
    ///
    /// Announced rather than analyzed on purpose: no probe can tell you a file
    /// is a PROPER. Carrying it here is what lets an incumbent's bar report a
    /// revision without a second parse of its row — `score_media_file` runs the
    /// same pipeline over the stored release name, so the number an incumbent
    /// gets is the number that release got when it was a candidate.
    pub revision: i32,
    /// The analyzed pass's score with hard blocks excluded (the announced pass's
    /// when there was no analysis). This is the upgrade bar. It is written to
    /// `media_files.acquisition_score` for display and history, but a comparison
    /// always re-derives it rather than reading it back. A veto never appears
    /// here: it travels as `allowed` / `block_codes` / [`ScoredRelease::truth_verdict`].
    pub total: i32,
}

/// Score a release from whatever evidence exists.
///
/// Runs the term pipeline over announced facts, then — when analyzed facts are
/// present — over those, and reports the difference as a bounded variance plus a
/// verdict. Pure and synchronous by construction.
pub(crate) fn score_release(evidence: &ReleaseEvidence, ctx: &ScoringContext<'_>) -> ScoredRelease {
    let announced_decision =
        run_term_pipeline(&evidence.parsed, evidence.announced_size_bytes, None, ctx);
    let release_score = announced_decision.preference_score;

    let mut analyzed_decision = None;
    let mut analyzed_quality = None;
    let (truth_variance, truth_verdict) = match evidence.analyzed.as_ref() {
        // No probe, or a probe that failed: variance is zero. A file we could
        // not measure must never be scored as though it measured badly.
        None => (0, TruthVerdict::Consistent),
        Some(analyzed) => {
            let analyzed_parsed = crate::post_download_gate::rescore_parsed_from_analysis(
                &evidence.parsed,
                Some(&analyzed.analysis),
            )
            .0;
            let analyzed_pass = run_term_pipeline(
                &analyzed_parsed,
                Some(analyzed.actual_size_bytes),
                analyzed.rule_file_doc.clone(),
                ctx,
            );
            let mut classified =
                classify_truth(&evidence.parsed, &announced_decision, &analyzed_pass);
            analyzed_quality = analyzed_parsed.quality.clone();

            // A quality change is a contradiction on its own, whatever the
            // scores did. Since the tier stopped contributing points, a file
            // that is 720p where the release said 1080p moves the score by
            // nothing at all — the disagreement is only visible by comparing the
            // qualities directly. Admission still refuses it on tier; this is
            // what lets the *reason* be reported rather than inferred.
            let announced_quality = normalize_quality_tier(evidence.parsed.quality.as_deref());
            let landed_quality = normalize_quality_tier(analyzed_quality.as_deref());
            if announced_quality != landed_quality {
                let code = format!(
                    "quality_contradicted:{}->{}",
                    announced_quality.as_deref().unwrap_or("unknown"),
                    landed_quality.as_deref().unwrap_or("unknown"),
                );
                classified.1 = match classified.1 {
                    TruthVerdict::Blocked { mut codes } => {
                        codes.push(code);
                        TruthVerdict::Blocked { codes }
                    }
                    // Carried, not promoted. Whether a mis-stated resolution
                    // outranks an undisclosed veto is an *action* question —
                    // it depends on the profile's tier order — and
                    // [`crate::post_download_gate::resolve_truth_verdict_action`]
                    // is the one place that arbitrates it.
                    TruthVerdict::Vetoed { mut codes } => {
                        codes.push(code);
                        TruthVerdict::Vetoed { codes }
                    }
                    TruthVerdict::Contradicted { mut codes } => {
                        codes.push(code);
                        TruthVerdict::Contradicted { codes }
                    }
                    TruthVerdict::Consistent => TruthVerdict::Contradicted { codes: vec![code] },
                };
            }
            analyzed_decision = Some(analyzed_pass);
            classified
        }
    };

    // The bar is what the file *is*. Once the bytes have been measured, the
    // analyzed pass is the honest number and `total` is exactly that — minus any
    // veto (see [`preference_score_without_blocks`]).
    //
    // It is deliberately not `release_score + truth_variance`: the variance is
    // clamped, so on a contradiction that sum drifts above the analyzed score —
    // and a row re-derived later (whose stored parse already reflects the
    // analysis, leaving nothing to contradict) would yield the analyzed score
    // instead. The persisted bar would then be unreproducible, which is the
    // defect this whole change set exists to remove. `truth_variance` and
    // `truth_verdict` stay as the *report* of the contradiction.
    let total =
        preference_score_without_blocks(analyzed_decision.as_ref().unwrap_or(&announced_decision));
    let parsed_quality = analyzed_quality.or_else(|| evidence.parsed.quality.clone());

    ScoredRelease {
        parsed_quality,
        revision: crate::acquisition::scoring::revision_rank(&evidence.parsed),
        total,
        release_score,
        announced_decision,
        analyzed_decision,
        truth_variance,
        truth_verdict,
    }
}

/// The pass's score with every hard block taken back out.
///
/// `BLOCK_SCORE` is summed into `preference_score` like any other delta, so one
/// veto drags a pass to roughly −10 000. Persisting that as the bar turns a
/// verdict into a number, and the number then misbehaves in both directions: the
/// vetoed file's re-derived bar sits so far below everything that every
/// candidate reads as a large upgrade and is fetched, and when the block is
/// structural (a minimum score no release for this title can reach, a language
/// the files never carry) the *next* file scores −10 000 too, ties are admitted
/// at import, and the scope churns on every RSS cycle.
///
/// So the veto travels as a veto: `allowed`, `block_codes` and
/// [`TruthVerdict`] carry it, admission and the import gate act on it, and the
/// number stays the honest sum of everything that was actually a preference.
fn preference_score_without_blocks(decision: &QualityProfileDecision) -> i32 {
    decision
        .scoring_log
        .iter()
        .filter(|entry| entry.delta != BLOCK_SCORE)
        .map(|entry| entry.delta)
        .sum()
}

/// The one term sequence. Both evidence levels walk exactly this path; the only
/// thing that varies is the facts handed in.
fn run_term_pipeline(
    parsed: &ParsedReleaseMetadata,
    size_bytes: Option<i64>,
    file_doc: Option<scryer_rules::FileDoc>,
    ctx: &ScoringContext<'_>,
) -> QualityProfileDecision {
    let mut resolved_profile = ctx.profile.clone();
    resolved_profile.criteria.required_audio_languages = ctx.required_audio_languages.to_vec();

    // `has_existing_file` is hardcoded false: the profile's upgrade guard is an
    // admission concern and is applied there, against the real incumbent set.
    let mut decision = evaluate_against_profile_for_category(
        &resolved_profile,
        parsed,
        false,
        ctx.weights,
        Some(ctx.category),
    );

    apply_size_scoring_for_category_with_remux_preference(
        &mut decision,
        parsed,
        size_bytes,
        Some(ctx.category),
        ctx.size_basis,
        resolved_profile.criteria.prefer_remux,
        ctx.weights,
    );

    // Deliberately absent: apply_age_scoring. Release age is listing metadata,
    // and a freshness bonus makes a same-size re-grab read as an upgrade —
    // wasted bandwidth cycling equivalent files.

    append_rule_scores(
        parsed,
        &resolved_profile,
        size_bytes,
        file_doc,
        &mut decision,
        ctx,
    );
    apply_min_score_gate(&resolved_profile, &mut decision);
    decision
}

/// Evaluate score-bearing user and system rules with listing metadata stripped
/// and incumbent state absent, so the result is reproducible from a media row.
fn append_rule_scores(
    parsed: &ParsedReleaseMetadata,
    profile: &QualityProfile,
    size_bytes: Option<i64>,
    file_doc: Option<scryer_rules::FileDoc>,
    decision: &mut QualityProfileDecision,
    ctx: &ScoringContext<'_>,
) {
    let Some(engine) = ctx.rules else {
        return;
    };
    if engine.is_empty() {
        return;
    }

    let input = crate::user_rule_input::build_rule_input(
        parsed,
        profile,
        decision,
        crate::user_rule_input::ReleaseRuntimeInfo {
            size_bytes,
            // Listing metadata, all withheld: none of it is a property of the
            // release, and none of it survives on a media row.
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            is_password_protected: None,
            extra: None,
            indexer_languages: None,
        },
        crate::user_rule_input::RuleContextInfo {
            title_id: ctx.title_id,
            library_name: ctx.library_name,
            category: Some(ctx.category),
            original_language: ctx.original_language,
            original_country: ctx.original_country,
            title_tags: ctx.title_tags,
            // Incumbent state withheld: see the module note.
            has_existing_file: false,
            existing_score: None,
            search_mode: CANONICAL_RULE_SEARCH_MODE,
            // Rules see the scope's total runtime, which is what they always
            // saw; the member split is the size term's business alone.
            runtime_minutes: ctx.size_basis.total_runtime_minutes,
            is_filler: ctx.is_filler,
        },
        file_doc,
    );

    let mut evaluator = engine.evaluator();
    match evaluator.evaluate(&input, ctx.category) {
        Ok(result) => {
            for entry in result.entries {
                let source = match entry.origin {
                    scryer_rules::PolicyOrigin::User => ScoringSource::UserRule {
                        id: entry.rule_set_id,
                        name: entry.rule_set_name,
                    },
                    scryer_rules::PolicyOrigin::System => ScoringSource::SystemRule {
                        id: entry.rule_set_id,
                        name: entry.rule_set_name,
                    },
                };
                decision.log_with_source(&entry.code, entry.delta, source);
            }
            for err in result.errors {
                let (code, source) = match err.origin {
                    scryer_rules::PolicyOrigin::User => (
                        "user_rule_error",
                        ScoringSource::UserRule {
                            id: err.rule_set_id,
                            name: err.rule_set_name,
                        },
                    ),
                    scryer_rules::PolicyOrigin::System => (
                        "system_rule_error",
                        ScoringSource::SystemRule {
                            id: err.rule_set_id,
                            name: err.rule_set_name,
                        },
                    ),
                };
                decision.log_with_source(code, 0, source);
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = ?ctx.title_id,
                "canonical scoring: rule evaluation failed; built-in terms only"
            );
        }
    }
}

/// Block codes that describe the *announcement and the profile*, never the file.
///
/// They can only ever fire on both passes at once, so the set difference below
/// would never surface them — except that the announced pass can be blocked by
/// something else first, and `apply_min_score_gate` only fires when the pass is
/// still `allowed`. Listing them explicitly keeps that ordering artefact from
/// reading as evidence against the file.
///
/// - `score_below_minimum` is Sonarr's `MinFormatScore`, a **grab** floor. It is
///   not an import specification there and must not become one here: a file that
///   is on disk and correct cannot be improved by refusing it. A score-only
///   contradiction is not a blocklist reason.
/// - `upgrade_blocked_by_profile` is the profile's upgrade guard, which is an
///   admission concern; canonical scoring hardcodes `has_existing_file = false`,
///   so it should never appear at all — it is listed defensively.
const POLICY_ONLY_BLOCK_CODES: &[&str] = &["score_below_minimum", "upgrade_blocked_by_profile"];

/// Did the announcement *state* the fact this veto keys on?
///
/// The difference between a release that lied and a file the profile refuses.
/// Both are import failures: the latter is [`TruthVerdict::Vetoed`] and is
/// blocklisted one candidate at a time.
///
/// | code | assertable | why |
/// |---|---|---|
/// | `quality_*` | always | the resolution is the one claim every release name makes, and it is the claim the grab decision was taken on. A file whose measured height lands outside the profile's tiers is not what was fetched. |
/// | `size_implausible_for_quality` | always | both passes score the *same* bytes, so this can only be introduced when the landed quality moved — which is the quality claim again, seen through the size band. It is the only size veto left: implausible *smallness* is a penalty on the curve, never a block, so it cannot reach here at all. |
/// | `video_codec_*` | iff the parse carried a codec | `H.265` in the name against an H.264 stream is a lie; a codec-silent name is not a claim. |
/// | `audio_codec_*` | iff the parse carried an audio codec | same rule. Note the gate only fires at all when `normalized_audio_codecs` is non-empty, which for a silent name means the probe populated it. |
/// | `hdr_not_allowed`, `dolby_vision_*` | never | derived from `video_hdr_format`; a profile that forbids them has refused this file, so import burns this release and convergence tries the next candidate. |
/// | user/system rule blocks | never | [`run_term_pipeline`] hands the rules engine a `FileDoc` on the analyzed pass only, so any rule reading `input.file.*` is structurally analyzed-only. Operator policy still makes this an import failure. |
/// | anything else | never | unreachable today (`source_*`, `bd_disk_not_allowed`, `required_audio_language_missing` key on fields the analyzed pass never rewrites). An unrecognised veto is conservatively treated as an import failure. |
fn veto_contradicts_an_assertion(code: &str, announced: &ParsedReleaseMetadata) -> bool {
    if code.starts_with("quality_") || code.starts_with("size_implausib") {
        return true;
    }
    if code.starts_with("video_codec_") {
        return announced.video_codec.is_some();
    }
    if code.starts_with("audio_codec_") {
        return announced.audio.is_some() || !announced.audio_codecs.is_empty();
    }
    false
}

/// Turn the announced/analyzed difference into a bounded variance or a verdict.
///
/// A hard block is never flattened into a number — `allowed` is derived from
/// explicit `BLOCK_SCORE` entries, and collapsing that into arithmetic would
/// silently convert a veto into a survivable penalty. Both sides of the variance
/// are therefore the passes' **non-block** sums, so no `BLOCK_SCORE` ever reaches
/// the subtraction.
///
/// ## `Blocked` means the announcement *asserted* the field and the file
/// contradicts it
///
/// The verdict answers "did this release lie", because that is the question the
/// import gate acts on: `Blocked` costs the release a blocklist entry and
/// reopens the search. Two filters stand between a `BLOCK_SCORE` entry and that
/// answer, and both are load-bearing.
///
/// **First: only blocks the file evidence introduced.** Most `BLOCK_SCORE` terms
/// are decided from the release *name* against the profile — a blocklisted
/// source, a codec outside the allowlist, a missing required audio language — so
/// they fire identically on both passes. Deciding from `!analyzed.allowed` alone
/// (as this briefly did) burns the release, reopens the scope, grabs the next
/// release, blocks it the same way and burns that one too: a loop that ends only
/// when every release for the title is blocklisted, with a provably correct file
/// on disk. If the announcement was already blocked, the release should never
/// have been grabbed; that is a grab-side bug and destroying candidates does not
/// fix it.
///
/// **Second: only fields the announcement actually stated.** "Introduced" alone
/// reads *silence* as a lie, which is the same loop wearing a different hat. A
/// release name that says nothing about codec, against a profile with a codec
/// blocklist, produces `video_codec_in_profile_blocklist` on the analyzed pass
/// only — and so does the next codec-silent release, and the one after that.
/// The same goes for HDR and Dolby Vision (read out of `video_hdr_format`, which
/// no name is obliged to carry) and for user/system file rules, which only see
/// `input.file.*` on the analyzed pass by construction. None of those is the
/// release misrepresenting itself; they are the profile refusing the file, which
/// is [`TruthVerdict::Vetoed`] — a burn followed by another convergence search. See
/// [`veto_contradicts_an_assertion`] for the per-code table.
fn classify_truth(
    announced_parsed: &ParsedReleaseMetadata,
    announced: &QualityProfileDecision,
    analyzed: &QualityProfileDecision,
) -> (i32, TruthVerdict) {
    // Walked over the log rather than `block_codes` so each veto keeps its
    // source: a rule-authored code can be spelled anything, including something
    // that looks builtin, and only the source proves it is operator policy.
    let introduced: Vec<&ScoringEntry> = analyzed
        .scoring_log
        .iter()
        .filter(|entry| entry.delta == BLOCK_SCORE)
        .filter(|entry| !announced.block_codes.contains(&entry.code))
        .filter(|entry| !POLICY_ONLY_BLOCK_CODES.contains(&entry.code.as_str()))
        .collect();
    if !introduced.is_empty() {
        let mut asserted = Vec::new();
        let mut undisclosed = Vec::new();
        for entry in introduced {
            let contradicts_a_claim = matches!(entry.source, ScoringSource::Builtin)
                && veto_contradicts_an_assertion(&entry.code, announced_parsed);
            if contradicts_a_claim {
                asserted.push(entry.code.clone());
            } else {
                undisclosed.push(entry.code.clone());
            }
        }
        // A proven lie outranks an undisclosed veto: the operator gets both code
        // sets, and the release is burned for the half it can be held to.
        if !asserted.is_empty() {
            asserted.extend(undisclosed);
            return (0, TruthVerdict::Blocked { codes: asserted });
        }
        return (0, TruthVerdict::Vetoed { codes: undisclosed });
    }

    // Both sides with their vetoes taken back out, so a block on either pass
    // cannot masquerade as a −10 000 contradiction.
    let raw = preference_score_without_blocks(analyzed)
        .saturating_sub(preference_score_without_blocks(announced));

    if raw.abs() > TRUTH_VARIANCE_BOUND {
        return (
            raw.clamp(-TRUTH_VARIANCE_BOUND, TRUTH_VARIANCE_BOUND),
            TruthVerdict::Contradicted {
                codes: contradiction_codes(announced, analyzed),
            },
        );
    }

    (raw, TruthVerdict::Consistent)
}

/// Name the terms that moved, so a contradiction says what was misadvertised
/// rather than only that something was.
fn contradiction_codes(
    announced: &QualityProfileDecision,
    analyzed: &QualityProfileDecision,
) -> Vec<String> {
    use std::collections::HashMap;

    let announced_by_code: HashMap<&str, i32> = announced
        .scoring_log
        .iter()
        .map(|entry| (entry.code.as_str(), entry.delta))
        .collect();

    let mut codes: Vec<String> = analyzed
        .scoring_log
        .iter()
        .filter(|entry| announced_by_code.get(entry.code.as_str()) != Some(&entry.delta))
        .map(|entry| entry.code.clone())
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

/// Rebuild the analyzed half of a stored file's evidence.
///
/// The file-rule document is rebuilt too, not left empty: `file.*` rules score
/// at import, so an incumbent bar derived without them would sit below the score
/// its own file was written with, and every candidate would look like an upgrade.
pub(crate) fn analyzed_facts_from_media_file(file: &crate::TitleMediaFile) -> AnalyzedFacts {
    let analysis = MediaFileAnalysis {
        video_codec: file.video_codec,
        video_width: file.video_width,
        video_height: file.video_height,
        video_bitrate_kbps: file.video_bitrate_kbps,
        video_bit_depth: file.video_bit_depth,
        video_hdr_format: file.video_hdr_format.clone(),
        dovi_profile: file.dovi_profile,
        dovi_bl_compat_id: file.dovi_bl_compat_id,
        video_frame_rate: file.video_frame_rate.clone(),
        video_profile: file.video_profile.clone(),
        audio_codec: file.audio_codec.clone(),
        audio_profile: file.audio_profile.clone(),
        audio_channels: file.audio_channels,
        audio_bitrate_kbps: file.audio_bitrate_kbps,
        audio_languages: file.audio_languages.clone(),
        audio_streams: file.audio_streams.clone(),
        subtitle_languages: file.subtitle_languages.clone(),
        subtitle_codecs: file.subtitle_codecs.clone(),
        subtitle_streams: file.subtitle_streams.clone(),
        has_multiaudio: file.has_multiaudio,
        duration_seconds: file.duration_seconds,
        num_chapters: file.num_chapters,
        container_format: file.container_format.clone(),
    };
    let rule_file_doc = crate::user_rule_input::file_doc_from_analysis(&analysis);

    AnalyzedFacts {
        analysis,
        // The analyzed pass sets the bar (`total` collapses to it), so it must
        // score the same size the import scored: the announced size inside
        // the overhead band, the real size otherwise.
        actual_size_bytes: size_basis_bytes(file.size_bytes, file.announced_size_bytes),
        rule_file_doc: Some(rule_file_doc),
    }
}

/// Rebuild a release parse from what the row remembers about the release.
///
/// The stored parse columns win over the raw name: they were written from the
/// import's own parse, so honouring them is what keeps a re-derived score equal
/// to the one the import wrote.
pub(crate) fn announced_parse_from_media_file(
    file: &crate::TitleMediaFile,
) -> ParsedReleaseMetadata {
    let path = crate::stored_paths::stored_path_to_path_buf(&file.file_path);
    let fallback = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let raw_title = file
        .grabbed_release_title
        .as_deref()
        .or(file.scene_name.as_deref())
        .unwrap_or(fallback);

    let mut parsed = crate::release_parser::parse_release_metadata(raw_title);

    if let Some(quality) = file
        .quality_label
        .as_ref()
        .or(file.resolution.as_ref())
        .filter(|value| !value.trim().is_empty())
    {
        parsed.quality = Some(quality.clone());
    }
    if let Some(codec) = file.video_codec_parsed {
        parsed.video_codec = Some(codec);
    }
    if let Some(codec) = file
        .audio_codec_parsed
        .as_deref()
        .or(file.audio_codec.as_deref())
        .and_then(crate::release_parser::AudioCodec::parse)
    {
        parsed.audio = Some(codec);
    }
    if let Some(channels) = file
        .audio_channels_parsed
        .clone()
        .or_else(|| file.audio_channels.map(audio_channels_label))
        .filter(|value| !value.trim().is_empty())
    {
        parsed.audio_channels = Some(channels);
    }
    if let Some(group) = file
        .release_group
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        parsed.release_group = Some(group.clone());
    }
    if let Some(edition) = file
        .edition
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        parsed.edition = Some(edition.clone());
    }

    parsed
}

pub(crate) fn audio_channels_label(channels: i32) -> String {
    match channels {
        8 => "7.1".to_string(),
        7 | 6 => "5.1".to_string(),
        3 | 2 => "2.0".to_string(),
        1 => "1.0".to_string(),
        value => value.to_string(),
    }
}

/// The landed-size tolerance that still counts as "the release it announced".
///
/// A usenet payload loses par2/RAR/container overhead between what the indexer
/// advertised and what is written to disk, and a torrent rarely arrives with
/// exactly its announced byte count either. Inside this band the grab and the
/// import are looking at the same release, so the import scores the size term
/// on the **announced** size (option c of the grab-vs-import size decision):
/// the number the grab admitted is the number the import sees. Outside the
/// reciprocal band the difference is material, so the import scores what
/// actually landed. The upper bound matters when an indexer reports one pack
/// member's size but the landed file contains the complete aggregate.
pub(crate) const SIZE_OVERHEAD_TOLERANCE: f64 = 0.85;

/// What the media-file row should remember as its announced size: the
/// announced size when the import scored on it, `None` when the landed size was
/// the basis. Persisting only the engaged case keeps the column honest — a row
/// never carries an "announced" number it was not scored on (a pack's total on
/// an episode row, say).
pub(crate) fn persisted_announced_size_bytes(landed: i64, announced: Option<i64>) -> Option<i64> {
    announced.filter(|announced| size_basis_bytes(landed, Some(*announced)) == *announced)
}

/// The byte count the size term is scored on for a landed file.
///
/// `announced` is the release's advertised size (`download_submissions.release_size_bytes`
/// at import, `media_files.announced_size_bytes` when the bar is re-derived);
/// `landed` is the file on disk. Returns `announced` only when the landed ratio
/// is between [`SIZE_OVERHEAD_TOLERANCE`] and its reciprocal, otherwise
/// `landed`. Both the import decision and the incumbent bar go through here so
/// the re-derived bar reproduces the import score for this term.
pub(crate) fn size_basis_bytes(landed: i64, announced: Option<i64>) -> i64 {
    match announced {
        Some(announced)
            if announced > 0
                && landed > 0
                && (landed as f64) >= SIZE_OVERHEAD_TOLERANCE * (announced as f64)
                && (landed as f64) <= (announced as f64) / SIZE_OVERHEAD_TOLERANCE =>
        {
            announced
        }
        _ => landed,
    }
}

/// Everything a stored row knows about the release it holds.
///
/// The size the row is scored on follows the import's rule ([`size_basis_bytes`]):
/// the announced size the row remembers when the file landed inside the overhead
/// band, otherwise the file's real size. That is what lets a re-derived bar
/// reproduce the import score. A row that remembers no announced size —
/// every row written before the column existed, a scanned file, an adopted
/// download — is scored on its real size, exactly as before.
pub(crate) fn evidence_from_media_file(file: &crate::TitleMediaFile) -> ReleaseEvidence {
    ReleaseEvidence::announced(
        announced_parse_from_media_file(file),
        Some(size_basis_bytes(file.size_bytes, file.announced_size_bytes)),
    )
    .with_analysis(analyzed_facts_from_media_file(file))
}

/// Re-derive a stored file's canonical score.
///
/// This is the incumbent's bar, always — the persisted `acquisition_score` is
/// display and history, never a comparison input. It runs the same pipeline the
/// import ran, which is what makes the two numbers comparable at all.
pub(crate) fn score_media_file(
    file: &crate::TitleMediaFile,
    ctx: &ScoringContext<'_>,
) -> ScoredRelease {
    score_release(&evidence_from_media_file(file), ctx)
}
