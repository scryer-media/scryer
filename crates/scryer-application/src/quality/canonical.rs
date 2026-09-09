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
//! total          = analyzed pass, all numeric contributions
//!                                          → the import score; PERSISTED; the upgrade bar
//! ```
//!
//! With no analysis there is only one pass, and `total` is its full numeric sum. `truth_variance` is the difference between the two passes,
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
//! - **Mandatory failures as numbers.** Explicit requirements carry zero-point
//!   rejection entries. Numeric penalties remain recoverable. See [`TruthVerdict`].
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
    QualityProfileDecision, ScoringEntry, ScoringSource, apply_min_score_gate,
    evaluate_profile_requirements, normalize_quality_tier,
};
use crate::{MediaFileAnalysis, ParsedReleaseMetadata, QualityProfile};

/// Clamp on the informational difference between announced and analyzed scores.
/// Customizable arithmetic cannot establish factual contradictions.
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

impl<'a> ScoringContext<'a> {
    /// Remove the rule-engine reference after an evaluator snapshot has been
    /// created for a synchronous batch. This makes a batched score use exactly
    /// one engine: the evaluator it was handed cannot be combined with another
    /// context's engine by accident.
    pub(crate) fn without_rules(mut self) -> Self {
        self.rules = None;
        self
    }
}

/// Mutable rule runtime for one synchronous canonical-scoring batch.
///
/// The evaluator owns a clone of the resolved engine snapshot. It is not
/// shared between tasks: Regorus mutates its input on every evaluation.
pub(crate) struct RuleEvaluationBatch {
    evaluator: Option<scryer_rules::UserRulesEvaluator>,
    collect_diagnostics: bool,
    pub(crate) errors: Vec<scryer_rules::RuleEvalError>,
    pub(crate) engine_error: Option<String>,
}

impl RuleEvaluationBatch {
    /// Capture the rule snapshot that is already present in this scoring
    /// context. Empty contexts retain no evaluator and skip rule evaluation.
    pub(crate) fn from_context(ctx: &ScoringContext<'_>) -> Self {
        Self {
            evaluator: ctx
                .rules
                .filter(|engine| !engine.is_empty())
                .map(scryer_rules::UserRulesEngine::evaluator),
            errors: Vec::new(),
            engine_error: None,
            collect_diagnostics: false,
        }
    }
}

pub(crate) struct ScoredReleasePreview {
    pub scored: ScoredRelease,
    pub rule_errors: Vec<scryer_rules::RuleEvalError>,
    pub engine_error: Option<String>,
}

/// Whether the file backed up what the release claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TruthVerdict {
    /// No analysis yet, or the file matched its announcement within bounds.
    Consistent,
    /// Playback facts cannot be assigned to the requested scope. Preserve the
    /// source for review; this is not evidence against the release.
    ReviewRequired { codes: Vec<String> },
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
            Self::Contradicted { codes }
            | Self::Blocked { codes }
            | Self::Vetoed { codes }
            | Self::ReviewRequired { codes } => codes,
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
    /// The analyzed pass's complete numeric score (the announced pass's
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
    let mut rules = RuleEvaluationBatch::from_context(ctx);
    score_release_with_rules(evidence, ctx, &mut rules)
}

pub(crate) fn score_release_preview(
    evidence: &ReleaseEvidence,
    ctx: &ScoringContext<'_>,
) -> ScoredReleasePreview {
    let mut rules = RuleEvaluationBatch::from_context(ctx);
    rules.collect_diagnostics = true;
    let scored = score_release_with_rules(evidence, ctx, &mut rules);
    ScoredReleasePreview {
        scored,
        rule_errors: rules.errors,
        engine_error: rules.engine_error,
    }
}

/// Score a candidate in a batch that already owns the sole rule evaluator.
///
/// Callers must pass a context produced with [`ScoringContext::without_rules`]
/// after constructing the batch. Keeping the context engine-free prevents a
/// batch evaluator from ever being silently paired with a second snapshot.
pub(crate) fn score_release_in_batch(
    evidence: &ReleaseEvidence,
    ctx: &ScoringContext<'_>,
    rules: &mut RuleEvaluationBatch,
) -> ScoredRelease {
    assert!(
        ctx.rules.is_none(),
        "batched canonical scoring requires a context without a rules engine"
    );
    score_release_with_rules(evidence, ctx, rules)
}

fn score_release_with_rules(
    evidence: &ReleaseEvidence,
    ctx: &ScoringContext<'_>,
    rules: &mut RuleEvaluationBatch,
) -> ScoredRelease {
    score_disc_scope_with_rules(evidence, ctx, rules, None)
}

fn score_disc_scope_with_rules(
    evidence: &ReleaseEvidence,
    ctx: &ScoringContext<'_>,
    rules: &mut RuleEvaluationBatch,
    episode_ids: Option<&[String]>,
) -> ScoredRelease {
    let _timer = crate::rules::metrics::StageTimer::new(
        "scoring_release",
        if rules.collect_diagnostics {
            crate::rules::metrics::Purpose::Preview
        } else {
            crate::rules::metrics::Purpose::Live
        },
    );
    let Some(analyzed) = &evidence.analyzed else {
        return score_single_release_with_rules(evidence, ctx, rules);
    };
    let Some(disc) = &analyzed.analysis.details.disc else {
        return score_single_release_with_rules(evidence, ctx, rules);
    };
    if disc.selection.title_id.as_ref().is_some_and(|id| {
        !disc
            .titles
            .iter()
            .any(|title| title.id == *id || title.aliases.contains(id))
    }) {
        return score_unresolved_disc(evidence, ctx, rules, "disc_title_override_invalid");
    }
    if episode_ids.is_some_and(|ids| {
        ids.is_empty()
            || ids.iter().any(|id| {
                disc.selection
                    .episode_mappings
                    .iter()
                    .filter(|mapping| mapping.episode_ids.contains(id))
                    .count()
                    != 1
            })
    }) {
        return score_unresolved_disc(evidence, ctx, rules, "disc_episode_mapping_unresolved");
    }
    let mut scores = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for mapping in &disc.selection.episode_mappings {
        if episode_ids.is_some_and(|ids| !mapping.episode_ids.iter().any(|id| ids.contains(id))) {
            continue;
        }
        let Some(title) = disc.titles.iter().find(|title| {
            title.id == mapping.disc_title_id || title.aliases.contains(&mapping.disc_title_id)
        }) else {
            return score_unresolved_disc(evidence, ctx, rules, "disc_episode_mapping_unresolved");
        };
        if mapping.episode_ids.len() != 1 || !seen.insert(&title.id) {
            return score_unresolved_disc(evidence, ctx, rules, "disc_episode_mapping_ambiguous");
        }
        if title.report.status != scryer_media_types::ProbeStatus::Complete
            || title
                .duration_seconds
                .is_none_or(|seconds| !seconds.is_finite() || seconds <= 0.0)
        {
            return score_unresolved_disc(evidence, ctx, rules, "disc_mapped_title_inconclusive");
        }
        let analysis = crate::media::disc_analysis::for_title(&analyzed.analysis, title);
        let rule_file_doc = Some(crate::user_rule_input::file_doc_from_analysis(&analysis));
        let scoped = ReleaseEvidence {
            parsed: evidence.parsed.clone(),
            announced_size_bytes: evidence.announced_size_bytes,
            analyzed: Some(AnalyzedFacts {
                analysis,
                actual_size_bytes: analyzed.actual_size_bytes,
                rule_file_doc,
            }),
        };
        scores.push(score_single_release_with_rules(&scoped, ctx, rules));
    }
    if scores.is_empty() {
        return score_single_release_with_rules(evidence, ctx, rules);
    }
    // Every mapped playback sequence must clear the gate. A superior soundtrack
    // or cut elsewhere on the image cannot raise another episode's bar.
    let severity = scores
        .iter()
        .map(|score| match score.truth_verdict {
            TruthVerdict::ReviewRequired { .. } => 4,
            TruthVerdict::Blocked { .. } => 3,
            TruthVerdict::Vetoed { .. } => 2,
            TruthVerdict::Contradicted { .. } => 1,
            TruthVerdict::Consistent => 0,
        })
        .max()
        .unwrap_or(0);
    let mut codes = scores
        .iter()
        .flat_map(|score| score.truth_verdict.codes().iter().cloned())
        .collect::<Vec<_>>();
    codes.sort();
    codes.dedup();
    let mut weakest = scores
        .into_iter()
        .max_by_key(|score| {
            (
                crate::quality_profile::quality_tier_index(
                    &ctx.profile.criteria,
                    score.parsed_quality.as_deref(),
                )
                .unwrap_or(usize::MAX),
                std::cmp::Reverse(score.total),
            )
        })
        .expect("nonempty mapped titles");
    weakest.truth_verdict = match severity {
        4 => TruthVerdict::ReviewRequired { codes },
        3 => TruthVerdict::Blocked { codes },
        2 => TruthVerdict::Vetoed { codes },
        1 => TruthVerdict::Contradicted { codes },
        _ => TruthVerdict::Consistent,
    };
    weakest
}

fn score_unresolved_disc(
    evidence: &ReleaseEvidence,
    ctx: &ScoringContext<'_>,
    rules: &mut RuleEvaluationBatch,
    code: &str,
) -> ScoredRelease {
    // Retain the announcement, but never substitute another playback title's
    // measurements for missing or invalid episode evidence.
    let announced = ReleaseEvidence {
        parsed: evidence.parsed.clone(),
        announced_size_bytes: evidence.announced_size_bytes,
        analyzed: None,
    };
    let mut score = score_single_release_with_rules(&announced, ctx, rules);
    score.truth_verdict = TruthVerdict::ReviewRequired {
        codes: vec![code.into()],
    };
    score
}

fn score_single_release_with_rules(
    evidence: &ReleaseEvidence,
    ctx: &ScoringContext<'_>,
    rules: &mut RuleEvaluationBatch,
) -> ScoredRelease {
    let is_disc = evidence
        .analyzed
        .as_ref()
        .is_some_and(|facts| facts.analysis.details.disc.is_some());
    let announced_decision = run_term_pipeline(
        &evidence.parsed,
        evidence.announced_size_bytes,
        None,
        ctx,
        rules,
    );
    let release_score = announced_decision.preference_score;
    // Preserve the acquisition score while comparing only facts measurable for
    // this playback title. Filesystem overhead and other cuts have no title size.
    let comparable_announcement =
        is_disc.then(|| run_term_pipeline(&evidence.parsed, None, None, ctx, rules));

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
                (!is_disc).then_some(analyzed.actual_size_bytes),
                analyzed.rule_file_doc.clone(),
                ctx,
                rules,
            );
            let mut classified = classify_truth(
                &evidence.parsed,
                comparable_announcement
                    .as_ref()
                    .unwrap_or(&announced_decision),
                &analyzed_pass,
            );
            if matches!(classified.1, TruthVerdict::Consistent)
                && !ctx.size_basis.covers_multiple_members()
                && size_claim_mismatch(evidence.announced_size_bytes, analyzed.actual_size_bytes)
            {
                classified.1 = TruthVerdict::Contradicted {
                    codes: vec!["size_claim_mismatch".into()],
                };
            }
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
                    TruthVerdict::ReviewRequired { codes } => {
                        TruthVerdict::ReviewRequired { codes }
                    }
                    TruthVerdict::Consistent => TruthVerdict::Contradicted { codes: vec![code] },
                };
            }
            analyzed_decision = Some(analyzed_pass);
            classified
        }
    };

    // The bar is what the file *is*. Once the bytes have been measured, the
    // analyzed pass supplies every numeric contribution in `total`; explicit
    // rejections contribute zero points (see [`numeric_score`]).
    //
    // It is deliberately not `release_score + truth_variance`: the variance is
    // clamped, so on a contradiction that sum drifts above the analyzed score —
    // and a row re-derived later (whose stored parse already reflects the
    // analysis, leaving nothing to contradict) would yield the analyzed score
    // instead. The persisted bar would then be unreproducible, which is the
    // defect this whole change set exists to remove. `truth_variance` and
    // `truth_verdict` stay as the *report* of the contradiction.
    let total = numeric_score(analyzed_decision.as_ref().unwrap_or(&announced_decision));
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

/// Sum every numeric contribution, including strong penalties. Mandatory
/// requirements and final score gates carry zero points.
fn numeric_score(decision: &QualityProfileDecision) -> i32 {
    crate::quality_profile::sum_score_deltas(
        decision
            .scoring_log
            .iter()
            .filter(|entry| {
                entry.kind == crate::quality_profile::ScoringEntryKind::ScoreContribution
            })
            .map(|entry| entry.delta),
    )
}

/// The one term sequence. Both evidence levels walk exactly this path; the only
/// thing that varies is the facts handed in.
fn run_term_pipeline(
    parsed: &ParsedReleaseMetadata,
    size_bytes: Option<i64>,
    file_doc: Option<scryer_rules::FileDoc>,
    ctx: &ScoringContext<'_>,
    rules: &mut RuleEvaluationBatch,
) -> QualityProfileDecision {
    let mut resolved_profile = ctx.profile.clone();
    resolved_profile.criteria.required_audio_languages = ctx.required_audio_languages.to_vec();

    // `has_existing_file` is hardcoded false: the profile's upgrade guard is an
    // admission concern and is applied there, against the real incumbent set.
    let mut decision =
        evaluate_profile_requirements(&resolved_profile, parsed, false, Some(ctx.category));
    crate::quality_profile::apply_size_requirement(
        &mut decision,
        &resolved_profile,
        parsed,
        size_bytes,
        Some(ctx.category),
        ctx.size_basis,
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
        rules,
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
    rules: &mut RuleEvaluationBatch,
) {
    let Some(evaluator) = rules.evaluator.as_mut() else {
        return;
    };
    let mut timer = crate::rules::metrics::StageTimer::new(
        "rules_apply",
        if rules.collect_diagnostics {
            crate::rules::metrics::Purpose::Preview
        } else {
            crate::rules::metrics::Purpose::Live
        },
    );

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
            coverage_total_runtime_minutes: ctx.size_basis.total_runtime_minutes,
            coverage_member_runtime_minutes: ctx.size_basis.member_runtime_minutes,
            coverage_member_count: Some(ctx.size_basis.member_count),
            is_filler: ctx.is_filler,
        },
        file_doc,
    );

    match evaluator.evaluate(&input, ctx.category) {
        Ok(result) => {
            if !result.errors.is_empty() {
                timer.outcome("error");
            }
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
                if rules.collect_diagnostics {
                    rules.errors.push(err.clone());
                }
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
            timer.outcome("error");
            if rules.collect_diagnostics {
                rules.engine_error = Some(error.to_string());
            }
            tracing::warn!(
                error = %error,
                title_id = ?ctx.title_id,
                "canonical scoring: rule evaluation failed; built-in terms only"
            );
        }
    }
}

/// The upgrade guard is an admission concern, never evidence against the file.
/// Canonical scoring uses `has_existing_file = false`; exclude it defensively.
/// Final score gates are classified separately by their explicit entry kind.
const POLICY_ONLY_BLOCK_CODES: &[&str] = &["upgrade_blocked_by_profile"];

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

/// Distinguish newly discovered mandatory failures from numeric score changes.
/// Only a requirement that contradicts an advertised field proves a lie;
/// undisclosed requirements and an unrecovered final score are policy refusals.
/// Rules and packs contribute numbers, never mandatory failures, even when
/// their codes resemble built-in requirements.
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
        .filter(|entry| entry.kind == crate::quality_profile::ScoringEntryKind::MandatoryRejection)
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

    // A finalized numeric refusal is policy, never proof of misrepresentation.
    let score_rejections = analyzed
        .scoring_log
        .iter()
        .filter(|entry| entry.kind == crate::quality_profile::ScoringEntryKind::FinalScoreRejection)
        .map(|entry| entry.code.clone())
        .collect::<Vec<_>>();
    if !score_rejections.is_empty() && announced.allowed {
        return (
            0,
            TruthVerdict::Vetoed {
                codes: score_rejections,
            },
        );
    }

    // Rule/pack changes report variance but cannot on their own prove a lie.
    let raw = numeric_score(analyzed).saturating_sub(numeric_score(announced));
    (
        raw.clamp(-TRUTH_VARIANCE_BOUND, TRUTH_VARIANCE_BOUND),
        TruthVerdict::Consistent,
    )
}

/// Compare only known positive byte counts. A fourfold discrepancy is well
/// outside normal packaging overhead; score weights cannot affect this fact.
/// The caller must establish that both byte counts cover the same single item.
fn size_claim_mismatch(announced: Option<i64>, actual: i64) -> bool {
    let Some(announced) = announced.filter(|bytes| *bytes > 0) else {
        return false;
    };
    actual > 0
        && (i128::from(actual) * 4 < i128::from(announced)
            || i128::from(announced) * 4 < i128::from(actual))
}

/// Rebuild the analyzed half of a stored file's evidence.
///
/// The file-rule document is rebuilt too, not left empty: `file.*` rules score
/// at import, so an incumbent bar derived without them would sit below the score
/// its own file was written with, and every candidate would look like an upgrade.
pub(crate) fn analyzed_facts_from_media_file(file: &crate::TitleMediaFile) -> AnalyzedFacts {
    let analysis = MediaFileAnalysis {
        details: file.analysis_details.clone(),
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

pub(crate) fn score_media_file_for_episodes(
    file: &crate::TitleMediaFile,
    episode_ids: &[String],
    ctx: &ScoringContext<'_>,
) -> ScoredRelease {
    let mut rules = RuleEvaluationBatch::from_context(ctx);
    score_disc_scope_with_rules(
        &evidence_from_media_file(file),
        ctx,
        &mut rules,
        Some(episode_ids),
    )
}
