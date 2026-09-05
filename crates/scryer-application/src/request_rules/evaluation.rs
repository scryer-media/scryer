//! Evaluating one request draft, and recording why (spec 0003 FR-011…FR-016).
//!
//! This is the only path that turns rules into a verdict. Submit, edit, and
//! pre-flight all come through here with different [`RequestEvaluationPurpose`]s
//! and identical everything else, which is what makes FR-021 — "identical drafts
//! yield identical decisions" — a property of the code rather than a promise.
//!
//! # Nothing here may fail the caller
//!
//! A request rule is a convenience layered over a flow that worked without it.
//! So every failure — an unreadable gate, a fact source that is down, a rule
//! that will not compile, an engine that times out, a trace store that refuses
//! the write — degrades to *today's behaviour*: the library's Auto-Approve
//! permission approves, and everything else waits for a human. The policy
//! verdict is still recorded when it can be, with `fallback_reason = "error"`,
//! so an operator can see that policy did not get to speak.

use chrono::Utc;
use scryer_domain::{
    Id, Library, LibraryPermission, RequestDecisionOutcome, RequestRuleDecisionRecord,
    RequestRuleEvaluationMode, User,
};
use scryer_rules::request::{REQUEST_INPUT_SCHEMA_VERSION, RequestVote};
use serde::Serialize;

use crate::helpers::{HashDomain, blake3_identity_hex};
use crate::media_requests::snapshot::MediaRequestMetadataSnapshot;
use crate::request_rules::arbitration::{
    Arbitration, FALLBACK_ERROR, ScopedError, ScopedVote, arbitrate, legacy_outcome,
};
use crate::request_rules::facts::{RequestDraft, build_request_input};
use crate::{AppResult, AppUseCase};

/// Prefix on the `request_id` of a trace produced by a pre-flight preview.
///
/// Pre-flight evaluates a draft that has no request row and may never get one,
/// so its trace cannot carry a real request id — but it must still be
/// inspectable (FR-016, FR-020). The prefix keeps it in the same append-only
/// table without any reader mistaking it for a decision about a submitted
/// request.
pub const PREFLIGHT_REQUEST_ID_PREFIX: &str = "preflight:";

/// Which call is asking.
#[derive(Clone, Debug)]
pub enum RequestEvaluationPurpose {
    /// A preview: nothing is persisted except the trace.
    Preflight,
    /// The request has just been submitted.
    Submit { request_id: String },
    /// A pending request was edited and is being re-judged.
    Resubmit { request_id: String },
}

impl RequestEvaluationPurpose {
    fn trace_request_id(&self) -> String {
        match self {
            Self::Preflight => format!("{PREFLIGHT_REQUEST_ID_PREFIX}{}", Id::new().0),
            Self::Submit { request_id } | Self::Resubmit { request_id } => request_id.clone(),
        }
    }
}

/// One reason, as it is safe to show a requester: a stable code and the name of
/// the rule that produced it. Rule bodies never leave this layer (FR-020).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestDecisionReason {
    pub code: String,
    pub rule_name: String,
}

/// The answer, plus everything the caller needs to act on it and everything the
/// trace already recorded.
#[derive(Clone, Debug)]
pub struct RequestEvaluation {
    /// The recorded trace's id, when one was written.
    pub decision_id: Option<String>,
    /// What the rules concluded, whether or not they were allowed to act.
    pub policy_outcome: RequestDecisionOutcome,
    /// What actually happens.
    pub effective_outcome: RequestDecisionOutcome,
    pub fallback_reason: Option<String>,
    pub deciding_rule_set_ids: Vec<String>,
    /// Tags that can actually land: the subset of [`Self::emitted_tags`] the
    /// title tag registry defines. Filtering happens once, here, so the pending
    /// row, the pre-flight banner, the resolution event and the approval all
    /// read the same list and none of them can promise a label that will be
    /// dropped later (FR-050).
    pub tags: Vec<String>,
    /// Every tag the rules emitted, defined or not.
    ///
    /// The decision trace keeps this list rather than [`Self::tags`] so an
    /// operator reading the trace can see the label they have not defined yet;
    /// that is the only way to find out why a rule's tag never appeared.
    pub emitted_tags: Vec<String>,
    pub reasons: Vec<RequestDecisionReason>,
    /// Strictest mode among the rules consulted, or `Disabled` when none were.
    pub evaluation_mode: RequestRuleEvaluationMode,
    /// True when the metadata snapshot could not be fully established, so some
    /// facts were unknown.
    pub metadata_partial: bool,
    pub gate_enabled: bool,
}

impl RequestEvaluation {
    /// The verdict Scryer had before rules existed, for the paths that never
    /// reached a rule.
    fn legacy(
        permission_grants_auto_approve: bool,
        gate_enabled: bool,
        metadata_partial: bool,
        fallback_reason: Option<&str>,
    ) -> Self {
        let outcome = legacy_outcome(permission_grants_auto_approve);
        Self {
            decision_id: None,
            policy_outcome: outcome,
            effective_outcome: outcome,
            fallback_reason: fallback_reason.map(str::to_string),
            deciding_rule_set_ids: Vec::new(),
            tags: Vec::new(),
            emitted_tags: Vec::new(),
            reasons: Vec::new(),
            evaluation_mode: RequestRuleEvaluationMode::Disabled,
            metadata_partial,
            gate_enabled,
        }
    }
}

// ── The recorded vote shape ─────────────────────────────────────────────────

/// One row of `request_rule_decisions.votes_json`.
///
/// `vote` is `null` exactly when `error` is set: a rule that failed produced no
/// vote, and rendering that as an abstain would tell an approver the rule looked
/// and had no opinion. `mode` rides along so a mixed shadow/enforce trace can be
/// read without joining back to the rule sets, which may have been edited or
/// deleted since (FR-016 — the trace outlives the rule).
#[derive(Clone, Debug, Serialize)]
pub struct RecordedVote {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub revision_number: i64,
    pub content_hash: String,
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vote: Option<&'static str>,
    pub held: bool,
    pub reason_codes: Vec<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

const fn vote_name(vote: RequestVote) -> &'static str {
    match vote {
        RequestVote::Approve => "approve",
        RequestVote::Deny => "deny",
        RequestVote::Manual => "manual",
        RequestVote::Abstain => "abstain",
    }
}

fn recorded_votes(votes: &[ScopedVote], errors: &[ScopedError]) -> Vec<RecordedVote> {
    let mut recorded: Vec<RecordedVote> = votes
        .iter()
        .map(|vote| RecordedVote {
            rule_set_id: vote.rule_set_id.clone(),
            rule_set_name: vote.rule_set_name.clone(),
            revision_number: vote.revision_number,
            content_hash: vote.content_hash.clone(),
            mode: vote.mode.as_storage_str(),
            vote: Some(vote_name(vote.decision.vote)),
            held: vote.decision.held,
            reason_codes: vote.decision.reason_codes.clone(),
            tags: vote.decision.tags.clone(),
            error: None,
        })
        .collect();
    recorded.extend(errors.iter().map(|error| RecordedVote {
        rule_set_id: error.rule_set_id.clone(),
        rule_set_name: error.rule_set_name.clone(),
        revision_number: error.revision_number,
        content_hash: error.content_hash.clone(),
        mode: error.mode.as_storage_str(),
        vote: None,
        held: false,
        reason_codes: Vec::new(),
        tags: Vec::new(),
        error: Some(error.message.clone()),
    }));
    recorded
}

// ── Evaluation ──────────────────────────────────────────────────────────────

impl AppUseCase {
    /// Evaluate a draft and record the trace.
    ///
    /// The caller passes the *enrichment's* snapshot rather than a request row,
    /// so pre-flight and submit share one metadata read (FR-021) — the caller
    /// obtained it through `enrich_request_draft`, which is cached.
    pub(crate) async fn evaluate_request_draft(
        &self,
        actor: &User,
        library: &Library,
        draft: &RequestDraft,
        snapshot: &MediaRequestMetadataSnapshot,
        purpose: RequestEvaluationPurpose,
    ) -> AppResult<RequestEvaluation> {
        let metadata_partial = snapshot.partial;
        let permission = self
            .has_library_permission(actor, &library.id, LibraryPermission::AutoApproveRequests)
            .await
            .unwrap_or(false);
        // An unreadable gate is a closed gate: losing the settings table must
        // disarm policy, never arm it.
        let gate_enabled = self
            .load_request_rule_gates()
            .await
            .map(|gates| gates.evaluation_enabled)
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "could not read the request rules gate; treating it as off");
                false
            });

        let cache = self.request_rules_engine_snapshot();
        let consulted: Vec<(String, crate::request_rules::engine::RequestRuleScope)> = cache
            .scopes_for_library(&library.id)
            .into_iter()
            .map(|(id, scope)| (id.clone(), scope.clone()))
            .collect();

        // No rule can speak about this library. Arbitration would reach the
        // same verdict the legacy check does, and building the fact document
        // would cost a dozen repository reads to prove it — so short-circuit,
        // and record a trace only on an instance that has actually armed the
        // gate and would therefore expect one.
        if consulted.is_empty() {
            let arbitration = arbitrate(&[], &[], permission);
            let mut evaluation = RequestEvaluation {
                decision_id: None,
                policy_outcome: arbitration.policy_outcome,
                effective_outcome: legacy_outcome(permission),
                fallback_reason: arbitration.fallback_reason.map(str::to_string),
                deciding_rule_set_ids: arbitration.deciding_rule_set_ids.clone(),
                tags: Vec::new(),
                emitted_tags: Vec::new(),
                reasons: Vec::new(),
                evaluation_mode: RequestRuleEvaluationMode::Disabled,
                metadata_partial,
                gate_enabled,
            };
            if gate_enabled {
                evaluation.decision_id = self
                    .record_request_decision(&purpose, &evaluation, &[], &[], "")
                    .await;
            }
            return Ok(evaluation);
        }

        let evaluation_time = Utc::now();
        let context = match self
            .assemble_request_input_context(
                actor,
                library,
                draft,
                snapshot.clone(),
                evaluation_time,
            )
            .await
        {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(
                    library_id = library.id.as_str(),
                    error = %error,
                    "could not assemble request rule facts; falling back to the permission check"
                );
                return Ok(self
                    .record_error_fallback(
                        &purpose,
                        permission,
                        gate_enabled,
                        metadata_partial,
                        cache.strictest_mode(&library.id),
                    )
                    .await);
            }
        };
        let input = build_request_input(context);
        let input_hash = blake3_identity_hex(
            HashDomain::RequestRuleInput,
            serde_json::to_string(&input).unwrap_or_default(),
        );

        let mut evaluator = cache.engine.evaluator();
        let outcome = match evaluator.evaluate(&input) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(
                    library_id = library.id.as_str(),
                    error = %error,
                    "request rule evaluation failed; falling back to the permission check"
                );
                return Ok(self
                    .record_error_fallback(
                        &purpose,
                        permission,
                        gate_enabled,
                        metadata_partial,
                        cache.strictest_mode(&library.id),
                    )
                    .await);
            }
        };

        // Library scoping is applied here, on the votes, not on the engine:
        // see `request_rules::engine`.
        let mut votes: Vec<ScopedVote> = Vec::new();
        for record in &outcome.records {
            let Some(scope) = cache.scopes.get(&record.rule_set_id) else {
                continue;
            };
            if !scope.covers(&library.id) {
                continue;
            }
            votes.push(ScopedVote {
                rule_set_id: record.rule_set_id.clone(),
                rule_set_name: record.rule_set_name.clone(),
                revision_number: scope.revision_number,
                content_hash: scope.content_hash.clone(),
                mode: scope.mode,
                decision: record.decision.clone(),
            });
        }
        let mut errors: Vec<ScopedError> = Vec::new();
        for failure in &outcome.errors {
            let Some(scope) = cache.scopes.get(&failure.rule_set_id) else {
                continue;
            };
            if !scope.covers(&library.id) {
                continue;
            }
            errors.push(ScopedError {
                rule_set_id: failure.rule_set_id.clone(),
                rule_set_name: failure.rule_set_name.clone(),
                revision_number: scope.revision_number,
                content_hash: scope.content_hash.clone(),
                mode: scope.mode,
                message: failure.message.clone(),
            });
        }

        let arbitration = arbitrate(&votes, &errors, permission);
        let enforceable = arbitration.is_enforceable(&votes, &errors);
        let effective_outcome =
            arbitration.effective_outcome(gate_enabled, enforceable, permission);

        let emitted_tags = arbitration.tags.clone();
        let applicable_tags = self.applicable_policy_tags(&purpose, &emitted_tags).await?;

        let mut evaluation = RequestEvaluation {
            decision_id: None,
            policy_outcome: arbitration.policy_outcome,
            effective_outcome,
            fallback_reason: arbitration.fallback_reason.map(str::to_string),
            deciding_rule_set_ids: arbitration.deciding_rule_set_ids.clone(),
            tags: applicable_tags,
            emitted_tags,
            reasons: reason_views(&arbitration),
            evaluation_mode: cache.strictest_mode(&library.id),
            metadata_partial,
            gate_enabled,
        };
        evaluation.decision_id = self
            .record_request_decision(&purpose, &evaluation, &votes, &errors, &input_hash)
            .await;
        Ok(evaluation)
    }

    /// The subset of `emitted` the title tag registry defines.
    ///
    /// Rules may name any label within the family's bounds, but a title only
    /// ever carries labels an administrator defined (the same gate
    /// `updateTitleTags` enforces). Dropping the rest here — once, at the
    /// source — is what keeps the pending row's "would be tagged" chip, the
    /// pre-flight banner, the resolution event and the approve dialog's prefill
    /// from promising a label that the approval would silently discard.
    ///
    /// The full list survives in the decision trace, which is where an operator
    /// goes to find out that the label needs defining.
    async fn applicable_policy_tags(
        &self,
        purpose: &RequestEvaluationPurpose,
        emitted: &[String],
    ) -> AppResult<Vec<String>> {
        if emitted.is_empty() {
            return Ok(Vec::new());
        }
        let undefined = self.undefined_title_tag_labels(emitted).await?;
        if undefined.is_empty() {
            return Ok(emitted.to_vec());
        }
        // Pre-flight runs on every debounced change in the request dialog, so
        // its copy of the same news is a debug line; a submit or an edit
        // happens once and deserves the operator's attention. Pre-flight has no
        // request id, and its trace id is minted later at the write, so its line
        // carries no id rather than one nothing can be joined on.
        match purpose {
            RequestEvaluationPurpose::Preflight => tracing::debug!(
                dropped = ?undefined,
                "request rule tags are not defined in the tag registry; the preview shows the rest"
            ),
            RequestEvaluationPurpose::Submit { request_id }
            | RequestEvaluationPurpose::Resubmit { request_id } => {
                tracing::warn!(
                    request_id = request_id.as_str(),
                    dropped = ?undefined,
                    "request rule tags are not defined in the tag registry; applying the rest"
                )
            }
        }
        Ok(emitted
            .iter()
            .filter(|tag| !undefined.contains(tag))
            .cloned()
            .collect())
    }

    /// The trace write, best effort.
    ///
    /// A store that refuses the write must not fail a submission that has
    /// already happened; the warning is the operator's signal that traces are
    /// being lost. Returns the id of the record when it landed.
    async fn record_request_decision(
        &self,
        purpose: &RequestEvaluationPurpose,
        evaluation: &RequestEvaluation,
        votes: &[ScopedVote],
        errors: &[ScopedError],
        input_hash: &str,
    ) -> Option<String> {
        let now = Utc::now();
        let record = RequestRuleDecisionRecord {
            id: Id::new().0,
            request_id: purpose.trace_request_id(),
            evaluated_at: now,
            mode: evaluation.evaluation_mode,
            effective_outcome: evaluation.effective_outcome,
            policy_outcome: evaluation.policy_outcome,
            fallback_reason: evaluation.fallback_reason.clone(),
            votes_json: serde_json::to_string(&recorded_votes(votes, errors))
                .unwrap_or_else(|_| "[]".to_string()),
            // The trace keeps everything the rules asked for, including labels
            // the registry does not define: it is the audit record, and an
            // operator hunting a tag that never appeared has nowhere else to
            // look. What actually lands is `evaluation.tags`.
            tags: evaluation.emitted_tags.clone(),
            input_hash: input_hash.to_string(),
            input_schema_version: i64::from(REQUEST_INPUT_SCHEMA_VERSION),
            created_at: now,
        };
        match self
            .services
            .customization
            .request_rule_decisions
            .record(&record)
            .await
        {
            Ok(()) => Some(record.id),
            Err(error) => {
                tracing::warn!(
                    request_id = record.request_id.as_str(),
                    error = %error,
                    "could not record a request rule decision trace"
                );
                None
            }
        }
    }

    /// The path taken when the facts or the engine failed: today's behaviour,
    /// with the trace saying policy never got to speak.
    async fn record_error_fallback(
        &self,
        purpose: &RequestEvaluationPurpose,
        permission: bool,
        gate_enabled: bool,
        metadata_partial: bool,
        mode: RequestRuleEvaluationMode,
    ) -> RequestEvaluation {
        let mut evaluation = RequestEvaluation::legacy(
            permission,
            gate_enabled,
            metadata_partial,
            Some(FALLBACK_ERROR),
        );
        // The policy reached no verdict at all, and an unevaluable policy is
        // never an approval (FR-012).
        evaluation.policy_outcome = RequestDecisionOutcome::ManualReview;
        evaluation.evaluation_mode = mode;
        evaluation.decision_id = self
            .record_request_decision(purpose, &evaluation, &[], &[], "")
            .await;
        evaluation
    }
}

/// Reasons, narrowed to what a requester may see. Pre-flight returns exactly
/// this; the trace keeps the rule ids.
fn reason_views(arbitration: &Arbitration) -> Vec<RequestDecisionReason> {
    arbitration
        .reasons
        .iter()
        .map(|reason| RequestDecisionReason {
            code: reason.code.clone(),
            rule_name: reason.rule_set_name.clone(),
        })
        .collect()
}
