//! The single import decision.
//!
//! One function, [`decide_import`], answers the only question the import gate
//! actually asks: *may these bytes take this scope, and if not, what happens to
//! the release?* Every completed-download path — episode, title, series-movie
//! link, and each member of a pack — goes through it.
//!
//! ## Where this sits in the loop
//!
//! ```text
//! Grabbed ──download completes──▶ ImportGate ──admit──▶ Occupied | Satisfied
//!                                            ──reject──▶ Rejected  (blocklist + reopen)
//!                                            ──hold────▶ Held      (operator decides)
//! ```
//!
//! `decide_import` *is* the ImportGate transition. Everything before
//! it is path-specific plumbing (which file, which destination, which rename
//! tokens); everything after it is carrying out the plan. The three paths used
//! to hand-assemble this sequence, which is how they drifted: the title path
//! had no rejected-admission branch at all and fell through to a plain insert,
//! writing a *second primary file* for a movie it had just refused.
//!
//! ## The sequence, in order
//!
//! 1. Build the [`crate::admission::AdmissionSubject`] for the scope — the same
//!    builder the grab lanes use, so both sides measure the incumbent with the
//!    same canonical bar.
//! 2. Score the landed evidence **once**, before the occupancy branch: the truth
//!    verdict is a fact about the release, not about the scope.
//! 3. Apply the verdict under the submission origin's guard policy, unless an
//!    explicit manual import asked for the bypass.
//! 4. Run [`crate::admission::evaluate_admission`] — the same comparator the
//!    grab used, under the import policy (ties accepted, no floor, no churn
//!    threshold).
//! 5. Resolve the displaced incumbent *rows* by id from the caller's list. A
//!    subject that says "occupied" against a list that has no matching row is a
//!    rejection, never a panic.
//!
//! ## What is deliberately not a rejection
//!
//! **`!analyzed.allowed` on its own.** A profile block that fires on the file's
//! evidence *and* on the release name is the profile refusing the release, which
//! is a grab-side decision; Sonarr has no import-time allow-list gate at all and
//! neither does this. Only [`crate::canonical_scoring::TruthVerdict`] speaks
//! here. Automatic lanes burn guard failures so convergence can find another
//! release; operator-queued lanes hold them for manual import. A file whose
//! vetoes fired on both passes imports with its honest bar.
//!
//! **A tie.** Import is never stricter than grab on the same facts: the
//! bytes are already on disk, and discarding a file that merely matches the
//! incumbent wastes the download.

use crate::AppUseCase;
use crate::post_download_gate::{
    ImportedFileAcceptance, ImportedFileRejection, PostDownloadAcquisitionDecision,
    compute_post_download_acquisition_decision, resolve_truth_verdict_action_for_origin,
};
use crate::quality::canonical_context::ResolvedScoringContext;
use scryer_domain::{ImportSkipReason, Title};

/// The origin determines what a failed import guard costs the release.
/// Explicit manual import keeps its separate `operator_intent` bypass; this
/// value describes normal post-download guard handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportOrigin {
    Automatic,
    OperatorQueued,
}

impl ImportOrigin {
    pub(crate) fn from_submission_purpose(purpose: crate::DownloadSubmissionPurpose) -> Self {
        if purpose.is_operator_queued() || purpose.is_manual_replacement() {
            Self::OperatorQueued
        } else {
            Self::Automatic
        }
    }

    fn rejection_disposition(self) -> RejectionDisposition {
        match self {
            Self::Automatic => RejectionDisposition::Blocklist,
            Self::OperatorQueued => RejectionDisposition::Hold,
        }
    }

    pub(crate) fn held_rejection(
        self,
        mut rejection: ImportedFileRejection,
    ) -> ImportedFileRejection {
        if self == Self::OperatorQueued {
            rejection.message = format!(
                "held for manual import because the file failed {}: {}",
                rejection.recycle_reason, rejection.message
            );
        }
        rejection
    }
}

/// Everything the decision needs, and nothing about where the bytes go.
///
/// Held by reference so the caller can re-use it after the decision — see
/// [`rescore_landed_size`], which re-runs the same scoring pass over the size
/// that actually landed.
pub(crate) struct ImportDecisionInput<'a> {
    pub title: &'a Title,
    /// Resolved once by the caller and shared with the post-transfer re-score,
    /// so a first import resolves the profile, weights, rules and language
    /// requirements a single time instead of twice.
    pub scoring_context: &'a ResolvedScoringContext,
    pub scope: &'a crate::SubmissionScope,
    /// The size basis for this scope: the episode span's runtime, the title's,
    /// or the linked movie's, together with the per-member runtime and member
    /// count a pack needs. Size scoring is runtime-derived, so this is what
    /// keeps the import's number comparable with the grab's.
    pub scope_size_basis: crate::quality_profile::CoverageSizeBasis,
    /// The **announced** evidence: the canonical import parse as it came off the
    /// release name, before the probe merged its findings in.
    ///
    /// This must not be `PreparedImportCandidate::parsed`, which every path used
    /// to hand over. That one is already
    /// [`crate::post_download_gate::rescore_from_mediainfo`]'d, so the announced
    /// and analyzed passes come out identical by construction: no release can be
    /// caught contradicting itself, `classify_truth` always reads `Consistent`,
    /// and the whole verdict machinery — the blocklist for a quality lie, the
    /// hold for an undisclosed veto — is unreachable. The prepared parse stays
    /// the right input for *paths and rename tokens*; scoring needs both halves
    /// of the evidence, and only the raw parse still has the announced half.
    ///
    /// Pinned end to end by
    /// `a_release_that_lied_about_its_quality_is_blocklisted_and_the_scope_reopened`.
    pub parsed: &'a crate::ParsedReleaseMetadata,
    pub accepted: &'a ImportedFileAcceptance,
    pub prior_rescore_changes: &'a [String],
    pub landed_size_bytes: i64,
    /// The size the release announced (`DownloadSubmission.release_size_bytes`),
    /// when the grab recorded one. Inside the overhead band the landed pass
    /// scores the size term on it, so grab and import agree
    /// (`canonical_scoring::size_basis_bytes`); a real shortfall scores on what
    /// landed.
    pub announced_size_bytes: Option<i64>,
    pub is_filler: bool,
    /// Guard-failure policy for the source that queued this download.
    pub origin: ImportOrigin,
    /// `operator_initiated_import(..)` — an operator picked this file by hand.
    /// Bypasses the verdict gate (blocklisting the release they chose would
    /// fight them) and selects [`crate::admission::AdmissionPolicy::manual`].
    pub operator_intent: bool,
    /// The superset the displaced rows are resolved from: every primary file of
    /// the title on the movie and link paths, the episode-scoped list on the
    /// episode path. Never a path-filtered list — a renamed incumbent lives at a
    /// path this import would never guess.
    pub incumbent_rows: IncumbentRows<'a>,
    /// How to name this scope in an operator-facing message ("this title",
    /// "this series-movie link", "this episode").
    pub scope_label: &'a str,
}

/// The caller's row list, in whichever shape its repository returns.
pub(crate) enum IncumbentRows<'a> {
    Title(&'a [crate::TitleMediaFile]),
    Episodes(&'a [crate::EpisodeScopedMediaFile]),
}

/// The rows this import displaces, best-first, in the caller's own row type.
#[derive(Debug, Clone)]
pub(crate) enum SupersededIncumbents {
    Title(Vec<crate::TitleMediaFile>),
    Episodes(Vec<crate::EpisodeScopedMediaFile>),
}

impl SupersededIncumbents {
    /// Empty means first import: nothing to recycle, nothing to supersede.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::Title(rows) => rows.is_empty(),
            Self::Episodes(rows) => rows.is_empty(),
        }
    }
}

/// An import that must go ahead *and* burn its own release.
///
/// The quality-lie case for an unoccupied scope: an honest 720p beats no episode
/// at all, but the release must never be offered back as an "upgrade" to the
/// tier it claimed.
#[derive(Debug, Clone)]
pub(crate) struct BlocklistDirective {
    pub code: &'static str,
    pub reason: String,
}

/// What happens to the release and the scope when the import is refused.
///
/// There is no "discard" outcome to choose from: `result_state.rs` maps every
/// non-`Imported` decision to `TrackedDownloadState::ImportBlocked`, so the
/// download sits waiting for an operator either way. What these three differ in
/// is the *side effects*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectionDisposition {
    /// Blocklist the release for this title and reopen the scope's search.
    ///
    /// Only for a release that provably lied — a truth verdict of
    /// [`crate::canonical_scoring::TruthVerdict::Blocked`], or a landed tier
    /// below the announced one. Reopening is safe precisely *because* the
    /// release is burned: the retry cannot fetch the same lie again.
    Blocklist,
    /// Record the decision and stop. No blocklist, no reopen.
    ///
    /// The release is fine, it simply is not better than what is already there.
    /// Blocklisting it would burn a good release; reopening the scope would ask
    /// convergence to re-fetch something it has already decided against.
    Skip,
    /// Record the decision and stop, and the reason is one an operator has to
    /// resolve: a runtime band miss, a replace guard, a broader incumbent span,
    /// a profile that forbids upgrades, an incumbent row that cannot be found.
    ///
    /// Same side effects as [`RejectionDisposition::Skip`]; kept distinct
    /// because the two answer different questions ("this release lost" versus
    /// "nobody can decide this automatically"), and because the operator surface
    /// should not have to infer which it is from a reason code.
    Hold,
}

/// The admitted plan: the number to persist, the parse to write, and the rows to
/// displace.
pub(crate) struct ImportAdmitPlan {
    pub score: i32,
    pub scoring_log: Option<String>,
    /// The parse after the analyzed pass merged its findings in — the row's
    /// quality, codecs and channels come from here, not from the announcement.
    pub parsed: crate::ParsedReleaseMetadata,
    /// Empty ⇒ first import. Non-empty ⇒ upgrade; the first entry leads.
    pub superseded: SupersededIncumbents,
    /// The bar this candidate cleared. `0` when the scope was empty.
    pub previous_best_score: i32,
    pub blocklist_after_import: Option<BlocklistDirective>,
}

pub(crate) enum ImportDecisionOutcome {
    Admit(Box<ImportAdmitPlan>),
    Reject {
        rejection: ImportedFileRejection,
        disposition: RejectionDisposition,
    },
}

/// Decide whether this file may take this scope. **The one import decision.**
pub(crate) async fn decide_import(
    app: &AppUseCase,
    input: &ImportDecisionInput<'_>,
) -> ImportDecisionOutcome {
    let subject = app
        .admission_subject_for_scope(
            input.title,
            input.scope,
            input.scoring_context,
            input.scope_size_basis.total_runtime_minutes,
            crate::quality::canonical_context::SubjectIntent::Import,
        )
        .await;

    let scored = score_landed(input, input.landed_size_bytes);

    // The verdict is about the release, so it is resolved before admission.
    // Automatic failures burn; operator-queued failures are held for review.
    let mut blocklist_after_import = None;
    if !input.operator_intent {
        match resolve_truth_verdict_action_for_origin(
            &scored.truth_verdict,
            &input.scoring_context.profile().criteria,
            !subject.is_unoccupied(),
            input.origin,
        ) {
            crate::post_download_gate::TruthVerdictAction::Import => {}
            crate::post_download_gate::TruthVerdictAction::ImportAndBlocklist { code, reason } => {
                blocklist_after_import = Some(BlocklistDirective { code, reason });
            }
            crate::post_download_gate::TruthVerdictAction::Reject(rejection) => {
                return ImportDecisionOutcome::Reject {
                    rejection: input.origin.held_rejection(rejection),
                    disposition: input.origin.rejection_disposition(),
                };
            }
        }
    }

    let candidate =
        crate::admission::CandidateFacts::new(scored.tier_index, scored.revision, scored.score);

    match evaluate_import_admission(
        &subject,
        candidate,
        input.operator_intent,
        &input.incumbent_rows,
        input.scope_label,
    ) {
        Err((rejection, disposition)) => ImportDecisionOutcome::Reject {
            rejection,
            disposition,
        },
        Ok(admitted) => ImportDecisionOutcome::Admit(Box::new(ImportAdmitPlan {
            score: scored.score,
            scoring_log: scored.scoring_log,
            parsed: scored.parsed,
            superseded: admitted.superseded,
            previous_best_score: admitted.previous_best_score,
            blocklist_after_import,
        })),
    }
}

/// What an admitted candidate displaces, and the bar it cleared.
pub(crate) struct AdmittedIncumbents {
    pub superseded: SupersededIncumbents,
    pub previous_best_score: i32,
}

/// The comparison half of the decision: the shared gate, plus resolving the rows
/// it names.
///
/// Split out from [`decide_import`] because it is pure and synchronous — no
/// clock, no database, no file system — so the dispositions table can be tested
/// exhaustively without building an app.
pub(crate) fn evaluate_import_admission(
    subject: &crate::admission::AdmissionSubject,
    candidate: crate::admission::CandidateFacts,
    operator_intent: bool,
    rows: &IncumbentRows<'_>,
    scope_label: &str,
) -> Result<AdmittedIncumbents, (ImportedFileRejection, RejectionDisposition)> {
    let policy = if operator_intent {
        crate::admission::AdmissionPolicy::manual()
    } else {
        crate::admission::AdmissionPolicy::not_a_downgrade()
    };

    let (ranked_superseded, previous_best_score) =
        match crate::admission::evaluate_admission(subject, candidate, &policy) {
            crate::admission::AdmissionVerdict::Reject(rejection) => {
                return Err((
                    admission_rejection_to_import(scope_label, &rejection),
                    disposition_for(&rejection.reason),
                ));
            }
            crate::admission::AdmissionVerdict::Admit {
                ranked_superseded,
                previous_best_score,
            } => (ranked_superseded, previous_best_score),
        };

    let superseded = resolve_superseded_rows(rows, &ranked_superseded);

    // Admission says the scope is occupied and the row list disagrees. One
    // condition, one arm: it is the episode path's "no primary episode file to
    // replace" and the movie paths' `incumbent_record_for_verdict` returning
    // `None`, which used to be spelled two different ways — and on the episode
    // path used to *blocklist* the release for a bookkeeping mismatch that says
    // nothing about it.
    if !subject.is_unoccupied() && superseded.is_empty() {
        return Err((
            missing_incumbent_rejection(scope_label),
            RejectionDisposition::Hold,
        ));
    }

    Ok(AdmittedIncumbents {
        superseded,
        previous_best_score,
    })
}

/// Re-run the scoring pass over the size that actually landed.
///
/// The bytes written can differ from the source's size (a repaired par2 set, a
/// container remux at transfer), and the persisted bar has to be the score of
/// what is on disk or a later re-derivation will not reproduce it. This is
/// the same pipeline [`decide_import`] ran, over the same resolved context — no
/// profile lookup, no rules load, no database round trip; only the term
/// sequence, with one number changed.
pub(crate) fn rescore_landed_size(
    input: &ImportDecisionInput<'_>,
    landed_size_bytes: i64,
) -> PostDownloadAcquisitionDecision {
    score_landed(input, landed_size_bytes)
}

fn score_landed(
    input: &ImportDecisionInput<'_>,
    size_bytes: i64,
) -> PostDownloadAcquisitionDecision {
    compute_post_download_acquisition_decision(
        input.scoring_context,
        input.title,
        input.parsed,
        input.accepted,
        input.scope_size_basis,
        crate::canonical_scoring::size_basis_bytes(size_bytes, input.announced_size_bytes),
        input.prior_rescore_changes,
        input.is_filler,
    )
}

/// What an admission refusal costs the release.
///
/// Exhaustive on purpose: the enum is shared with the grab lanes, and a
/// wildcard arm here would silently give a future refusal the wrong disposition
/// — most likely `Skip`, which is the one that quietly does nothing.
///
/// The split is "did this release lose a comparison" (`Skip`) versus "can nobody
/// decide this without an operator" (`Hold`). **No admission refusal ever
/// blocklists**: a release that is merely not an upgrade is a perfectly good
/// release for some other scope or some later day, and burning it is how a
/// library ends up unable to re-grab anything. Sonarr's equivalents
/// (`DiskCustomFormatScore`, `DiskHigherPreference`, `DiskHigherRevision`,
/// `SameEpisodesImportSpecification`, `DiskUpgradesNotAllowed`) are all plain
/// rejections there too; only dangerous files get blocklisted.
pub(crate) fn disposition_for(
    reason: &crate::admission::AdmissionRejectionReason,
) -> RejectionDisposition {
    use crate::admission::AdmissionRejectionReason as Reason;

    match reason {
        // The candidate lost a fair comparison. Nothing to escalate.
        Reason::NotAnUpgrade { .. }
        | Reason::LowerQualityTier
        | Reason::LowerRevision { .. }
        | Reason::QueuedEqualOrBetter { .. }
        | Reason::QueuedSameRelease { .. } => RejectionDisposition::Skip,
        // Reopening a scope whose profile forbids upgrades is pure churn: the
        // search would find candidates the same guard refuses.
        Reason::UpgradesDisabled
        // The release is fine — it just cannot be placed here without dropping
        // coverage. A double-episode file can be admitted per-member at grab but
        // refused as a span at import. Queue-aware admission keeps it from being
        // re-grabbed while it sits held.
        | Reason::BroaderIncumbentSpan
        // Grab-only reasons, mapped honestly rather than left to a wildcard: an
        // import policy never sets `cutoff_score`, and a pack verdict cannot
        // reach a per-file import at all. If one ever does, holding is the
        // outcome that asks a human rather than the one that shrugs.
        | Reason::FormatCutoffReached { .. }
        | Reason::SeasonIncomplete => RejectionDisposition::Hold,
    }
}

/// What a refusal raised *before* the decision — by `prepare_import_candidate`'s
/// probe gate — costs the release.
///
/// The probe gate predates the verdict model and refuses for three reasons.
/// Bytes that are not what was claimed (a corrupt or unreadable container, a
/// source that changed under the import) mean the release lied: `Blocklist`.
/// The runtime band is held by the callers before they reach here. A
/// user/system **rule** BLOCK at the gate is the same event `classify_truth`
/// would classify as [`crate::canonical_scoring::TruthVerdict::Vetoed`] had
/// the gate not fired first — operator policy on the file, not a
/// misrepresentation — it is still an import failure. The release is
/// blocklisted and the scope reopens so the next search cannot retry it.
pub(crate) fn prepare_rejection_disposition_for_origin(
    rejection: &ImportedFileRejection,
    origin: ImportOrigin,
) -> RejectionDisposition {
    let _ = rejection;
    origin.rejection_disposition()
}

/// Translate a shared admission refusal into the import layer's rejection shape.
///
/// `scope_label` is what the operator sees ("existing episode file …" versus
/// "existing movie file …"); the judgement itself is the admission module's and
/// is not restated here.
pub(crate) fn admission_rejection_to_import(
    scope_label: &str,
    rejection: &crate::admission::AdmissionRejection,
) -> ImportedFileRejection {
    use crate::admission::AdmissionRejectionReason as Reason;

    let path = rejection.incumbent_file_path.as_str();
    let (recycle_reason, skip_reason, message) = match rejection.reason {
        Reason::NotAnUpgrade {
            incumbent_score,
            candidate_score,
            ..
        } => (
            "already_imported",
            ImportSkipReason::AlreadyImported,
            format!(
                "existing file {path} for {scope_label} is equal or better \
                 (score {incumbent_score} >= {candidate_score})"
            ),
        ),
        Reason::LowerQualityTier => (
            "already_imported",
            ImportSkipReason::AlreadyImported,
            format!("existing file {path} for {scope_label} is a better quality than this import"),
        ),
        // Same quality, but what is on disk is the PROPER/REPACK and this is
        // the original. "Already imported" is the honest reading: the fix is
        // already there, so this file has nothing to add.
        Reason::LowerRevision {
            incumbent_revision,
            candidate_revision,
        } => (
            "already_imported",
            ImportSkipReason::AlreadyImported,
            format!(
                "existing file {path} for {scope_label} is a later revision \
                 (PROPER/REPACK {incumbent_revision} > {candidate_revision})"
            ),
        ),
        Reason::BroaderIncumbentSpan => (
            "policy_mismatch",
            ImportSkipReason::PolicyMismatch,
            format!(
                "existing file {path} spans a broader episode set and cannot be replaced by this import"
            ),
        ),
        Reason::UpgradesDisabled => (
            "policy_mismatch",
            ImportSkipReason::PolicyMismatch,
            format!(
                "existing file {path} for {scope_label} cannot be replaced because the quality \
                 profile disallows upgrades"
            ),
        ),
        Reason::QueuedEqualOrBetter { .. } | Reason::QueuedSameRelease { .. } => (
            "already_imported",
            ImportSkipReason::AlreadyImported,
            rejection.message.clone(),
        ),
        Reason::FormatCutoffReached { .. } => (
            "already_imported",
            ImportSkipReason::AlreadyImported,
            rejection.message.clone(),
        ),
        Reason::SeasonIncomplete => (
            "policy_mismatch",
            ImportSkipReason::PolicyMismatch,
            rejection.message.clone(),
        ),
    };

    ImportedFileRejection {
        message,
        recycle_reason,
        skip_reason: Some(skip_reason),
        blocking_rule_codes: Vec::new(),
    }
}

/// An occupied scope whose incumbent row cannot be found. Structural, not a
/// judgement about the release, so nothing is burned: the bytes stay where they
/// are and the operator can retry.
pub(crate) fn missing_incumbent_rejection(scope_label: &str) -> ImportedFileRejection {
    ImportedFileRejection {
        message: format!(
            "{scope_label} is occupied but its media file row could not be resolved; refusing to import over it"
        ),
        recycle_reason: "policy_mismatch",
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: Vec::new(),
    }
}

/// Re-attach the full records, in the order admission ranked them.
///
/// By id, against the caller's *unfiltered* list. A path-scoped lookup here is
/// what used to panic the link path: rename the incumbent and the subject still
/// says "occupied" while the filtered list is empty.
fn resolve_superseded_rows(rows: &IncumbentRows<'_>, ranked: &[String]) -> SupersededIncumbents {
    match rows {
        IncumbentRows::Title(files) => SupersededIncumbents::Title(
            ranked
                .iter()
                .filter_map(|file_id| files.iter().find(|file| &file.id == file_id).cloned())
                .collect(),
        ),
        IncumbentRows::Episodes(files) => SupersededIncumbents::Episodes(
            ranked
                .iter()
                .filter_map(|file_id| {
                    files
                        .iter()
                        .find(|file| &file.media_file.id == file_id)
                        .cloned()
                })
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejection() -> ImportedFileRejection {
        ImportedFileRejection {
            message: "required audio language is missing".to_string(),
            recycle_reason: "language_mismatch",
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            blocking_rule_codes: Vec::new(),
        }
    }

    #[test]
    fn operator_queued_guard_rejections_are_held_with_an_actionable_message() {
        let held = ImportOrigin::OperatorQueued.held_rejection(rejection());
        assert_eq!(
            prepare_rejection_disposition_for_origin(&held, ImportOrigin::OperatorQueued),
            RejectionDisposition::Hold
        );
        assert!(held.message.contains("held for manual import"));
        assert!(held.message.contains("language_mismatch"));
    }

    #[test]
    fn automatic_guard_rejections_still_burn_the_release() {
        assert_eq!(
            prepare_rejection_disposition_for_origin(&rejection(), ImportOrigin::Automatic),
            RejectionDisposition::Blocklist
        );
    }

    /// A perfectly good download is refused because something *still sitting in
    /// the queue* is equal or better, and that refusal is reported as
    /// `AlreadyImported`. Downstream that reads as a successful import, so the
    /// client entry is cleaned up and no import/failure history is written.
    #[test]
    fn a_queued_equal_or_better_release_is_reported_as_already_imported() {
        let rejection = crate::admission::AdmissionRejection {
            reason: crate::admission::AdmissionRejectionReason::QueuedEqualOrBetter {
                queued_title: "Show.S01E01.1080p.WEB-DL".to_string(),
                queued_score: 100,
                candidate_score: 100,
            },
            message: "already downloading for this scope and is equal or better".to_string(),
            incumbent_file_id: String::new(),
            incumbent_file_path: String::new(),
        };

        let imported = admission_rejection_to_import("S01E01", &rejection);

        assert_eq!(
            imported.skip_reason,
            Some(ImportSkipReason::AlreadyImported),
            "a queued-equal-or-better refusal must not be laundered into AlreadyImported"
        );
        assert_eq!(imported.recycle_reason, "already_imported");
    }
}
