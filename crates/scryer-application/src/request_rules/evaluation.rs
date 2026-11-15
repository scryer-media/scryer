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
//! With no enabled rule covering the library, and whenever shared facts are
//! unavailable, the library's existing Auto-Approve permission decides.
//! Uncertainty in an enabled rule holds the request for a human. Trace persistence is best effort: a failed trace write
//! must not change the verdict already reached by policy.

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
    ArbitratedShaping, Arbitration, FALLBACK_ERROR, ScopedError, ScopedVote, arbitrate,
    legacy_outcome,
};
use scryer_rules::request_actions::{RequestActionHead, RequestHeadValue, RequestShaping};
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
    /// What the rules concluded.
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
    /// `Enforce` when an enabled rule was consulted, `Disabled` when none was.
    pub evaluation_mode: RequestRuleEvaluationMode,
    /// True when the metadata snapshot could not be fully established, so some
    /// facts were unknown.
    pub metadata_partial: bool,
    /// What the rules set for the approval to write, each value attributed to
    /// its rule set. On manual review it is what the approver form prefills.
    pub shaping: ArbitratedShaping,
    /// [`Self::shaping`] checked against this request and resolved to what an
    /// approval writes. Applied only on an automatic approval.
    pub resolved_shaping: ResolvedShaping,
}

/// Shaping values checked against one request, in the form an approval
/// writes them. Each `None` keeps what the requester asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedShaping {
    /// The quality profile id and name the rule named.
    pub quality_profile: Option<(String, String)>,
    /// `Some(None)` keeps the title forever.
    pub lease_days: Option<Option<i64>>,
    /// The normalized monitor type.
    pub monitor_type: Option<String>,
    pub required_audio_languages: Option<Vec<String>>,
    pub preferred_audio_languages: Option<Vec<String>>,
}

impl RequestEvaluation {
    /// The verdict Scryer had before rules existed, for the paths that never
    /// reached a rule.
    fn legacy(
        permission_grants_auto_approve: bool,
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
            shaping: ArbitratedShaping::default(),
            resolved_shaping: ResolvedShaping::default(),
        }
    }
}

// ── The recorded vote shape ─────────────────────────────────────────────────

/// One row of `request_rule_decisions.votes_json`.
///
/// `vote` is `null` exactly when `error` is set: a rule that failed produced no
/// vote, and rendering that as an abstain would tell an approver the rule looked
/// and had no opinion. `mode` rides along so the trace can be
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
    /// What the rule's shaping heads set.
    #[serde(skip_serializing_if = "RequestShaping::is_empty")]
    pub shaping: RequestShaping,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify: Option<String>,
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
            shaping: vote.decision.shaping.clone(),
            notify: vote.decision.notify.clone(),
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
        shaping: RequestShaping::default(),
        notify: None,
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
        let cache = self.request_rules_engine_snapshot();
        if cache.unavailable {
            tracing::warn!(
                library_id = library.id.as_str(),
                "request rules engine generation is unavailable; holding the request"
            );
            return Ok(self
                .record_unavailable_engine_fallback(&purpose, permission, metadata_partial)
                .await);
        }
        let consulted: Vec<(String, crate::request_rules::engine::RequestRuleScope)> = cache
            .scopes_for_library(&library.id)
            .into_iter()
            .map(|(id, scope)| (id.clone(), scope.clone()))
            .collect();

        // No rule can speak about this library. Arbitration would reach the
        // same verdict the legacy check does, and building the fact document
        // would cost a dozen repository reads to prove it — so short-circuit
        // without a trace: no rule took part.
        if consulted.is_empty() {
            let arbitration = arbitrate(&[], &[], permission);
            return Ok(RequestEvaluation {
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
                shaping: ArbitratedShaping::default(),
                resolved_shaping: ResolvedShaping::default(),
            });
        }

        let evaluation_time = Utc::now();
        let context = match self
            .assemble_request_input_context(
                actor,
                library,
                draft,
                snapshot.clone(),
                evaluation_time,
                match &purpose {
                    RequestEvaluationPurpose::Resubmit { request_id } => Some(request_id.as_str()),
                    _ => None,
                },
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
                    .record_error_fallback(&purpose, permission, metadata_partial)
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
                    .record_error_fallback(&purpose, permission, metadata_partial)
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
        for (rule_set_id, scope) in &consulted {
            if let Some(message) = cache.load_errors.get(rule_set_id) {
                errors.push(ScopedError {
                    rule_set_id: rule_set_id.clone(),
                    rule_set_name: scope.name.clone(),
                    revision_number: scope.revision_number,
                    content_hash: scope.content_hash.clone(),
                    mode: scope.mode,
                    message: message.clone(),
                });
            }
        }
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

        let mut arbitration = arbitrate(&votes, &errors, permission);
        let resolved_shaping = self.resolve_request_shaping(draft, &mut arbitration).await;
        let mut effective_outcome = arbitration.policy_outcome;
        let mut fallback_reason = arbitration.fallback_reason.map(str::to_string);
        let applicable_tags = match self
            .applicable_policy_tags(&purpose, &arbitration.tags)
            .await
        {
            Ok(tags) => tags,
            Err(error) => {
                tracing::warn!(%error, "request rule tag registry unavailable; holding request");
                effective_outcome = RequestDecisionOutcome::ManualReview;
                fallback_reason = Some(FALLBACK_ERROR.to_string());
                Vec::new()
            }
        };

        let mut evaluation = RequestEvaluation {
            decision_id: None,
            policy_outcome: arbitration.policy_outcome,
            effective_outcome,
            fallback_reason,
            deciding_rule_set_ids: arbitration.deciding_rule_set_ids.clone(),
            tags: applicable_tags,
            emitted_tags: arbitration.tags.clone(),
            reasons: reason_views(&arbitration),
            evaluation_mode: RequestRuleEvaluationMode::Enforce,
            metadata_partial,
            shaping: arbitration.shaping.clone(),
            resolved_shaping,
        };
        evaluation.decision_id = self
            .record_request_decision(&purpose, &evaluation, &votes, &errors, &input_hash)
            .await;
        Ok(evaluation)
    }

    /// Check every agreed shaping value against this request, through the
    /// same validators an approver's override goes through, and resolve it to
    /// what an approval writes. A value that cannot apply is refused on the
    /// arbitration, which turns an approval into manual review.
    pub(crate) async fn resolve_request_shaping(
        &self,
        draft: &RequestDraft,
        arbitration: &mut Arbitration,
    ) -> ResolvedShaping {
        let mut resolved = ResolvedShaping::default();
        let mut invalid: Vec<RequestActionHead> = Vec::new();
        let values: Vec<(RequestActionHead, RequestHeadValue)> = arbitration
            .shaping
            .values
            .iter()
            .map(|(head, shaped)| (*head, shaped.value.clone()))
            .collect();
        for (head, value) in values {
            let applied = match (head, value) {
                (RequestActionHead::SetQualityProfile, RequestHeadValue::Text(name)) => {
                    match self.quality_profile_by_name(&name).await {
                        Some(profile) => {
                            resolved.quality_profile = Some(profile);
                            true
                        }
                        None => false,
                    }
                }
                (RequestActionHead::SetLeaseDays, RequestHeadValue::Number(days)) => {
                    let days = (days != 0).then_some(days);
                    match crate::request_rules::validate_lease_days(days) {
                        Ok(days) => {
                            resolved.lease_days = Some(days);
                            true
                        }
                        Err(_) => false,
                    }
                }
                (RequestActionHead::SetMonitor, RequestHeadValue::Text(monitor)) => {
                    match shaped_monitor_type(draft, &monitor) {
                        Some(monitor) => {
                            resolved.monitor_type = Some(monitor);
                            true
                        }
                        None => false,
                    }
                }
                (RequestActionHead::SetRequiredAudioLanguages, RequestHeadValue::List(list)) => {
                    match crate::settings::runtime::validated_preference_languages(
                        "required audio language",
                        list,
                    ) {
                        Ok(list) => {
                            resolved.required_audio_languages = Some(list);
                            true
                        }
                        Err(_) => false,
                    }
                }
                (RequestActionHead::SetPreferredAudioLanguages, RequestHeadValue::List(list)) => {
                    match crate::settings::runtime::validated_preference_languages(
                        "preferred audio language",
                        list,
                    ) {
                        Ok(list) => {
                            resolved.preferred_audio_languages = Some(list);
                            true
                        }
                        Err(_) => false,
                    }
                }
                _ => false,
            };
            if !applied {
                invalid.push(head);
            }
        }
        arbitration.refuse_invalid_shaping(&invalid);
        resolved
    }

    /// The id and name of the one quality profile called `name`, or `None`
    /// when none is, more than one is, or the profiles cannot be read.
    async fn quality_profile_by_name(&self, name: &str) -> Option<(String, String)> {
        let settings = match self.load_quality_profile_settings().await {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(%error, "quality profiles unavailable; holding a shaped request");
                return None;
            }
        };
        let mut matches =
            crate::request_rules::service::quality_profiles_named(&settings.profiles, name);
        let profile = matches.next()?;
        if matches.next().is_some() {
            tracing::warn!(
                profile = name,
                "more than one quality profile has the name a rule set; holding a shaped request"
            );
            return None;
        }
        Some((profile.id.clone(), profile.name.clone()))
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
            shaping_json: (!evaluation.shaping.is_empty())
                .then(|| serde_json::to_string(&evaluation.shaping).ok())
                .flatten(),
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
        metadata_partial: bool,
    ) -> RequestEvaluation {
        let mut evaluation =
            RequestEvaluation::legacy(permission, metadata_partial, Some(FALLBACK_ERROR));
        // The policy reached no verdict at all, and an unevaluable policy is
        // never an approval (FR-012).
        evaluation.policy_outcome = RequestDecisionOutcome::ManualReview;
        evaluation.evaluation_mode = RequestRuleEvaluationMode::Enforce;
        evaluation.decision_id = self
            .record_request_decision(purpose, &evaluation, &[], &[], "")
            .await;
        evaluation
    }

    /// A rule mutation committed but its replacement generation could not be
    /// built. The prior cache might still contain a rule that was just
    /// disabled, so this is stricter than an ordinary fact-read failure: never
    /// fall back to Auto-Approve.
    async fn record_unavailable_engine_fallback(
        &self,
        purpose: &RequestEvaluationPurpose,
        permission: bool,
        metadata_partial: bool,
    ) -> RequestEvaluation {
        let mut evaluation =
            RequestEvaluation::legacy(permission, metadata_partial, Some(FALLBACK_ERROR));
        evaluation.policy_outcome = RequestDecisionOutcome::ManualReview;
        evaluation.effective_outcome = RequestDecisionOutcome::ManualReview;
        evaluation.evaluation_mode = RequestRuleEvaluationMode::Enforce;
        evaluation.decision_id = self
            .record_request_decision(purpose, &evaluation, &[], &[], "")
            .await;
        evaluation
    }
}

/// The normalized monitor type a rule's `set_monitor` value gives this draft,
/// or `None` when it cannot apply: a value the facet does not have,
/// `advanced`, which needs a selection a rule cannot write, or any value on a
/// list movie, whose approval must keep the movie it selected.
pub(crate) fn shaped_monitor_type(draft: &RequestDraft, monitor: &str) -> Option<String> {
    if draft.origin.subscription_id().is_some()
        && draft.identity_fingerprint.starts_with("list-movie:")
    {
        return None;
    }
    crate::request_rules::service::shaping_monitor_for_facet(&draft.facet, monitor)
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
