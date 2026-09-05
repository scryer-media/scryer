//! Arbitration across request-rule votes (spec 0003 FR-011, plan §3.3).
//!
//! Pure: votes and errors in, one verdict out. No repository, no clock, no
//! permission lookup beyond the single boolean the caller resolved — which is
//! what makes the whole table testable as data.
//!
//! The order is not a preference, it is a safety property. `deny` is the only
//! vote that can refuse a requester, so it wins outright. Everything *uncertain*
//! — an author-declared `manual`, a rule the host held because a fact was
//! unobservable, a rule that errored or timed out — collapses to manual review,
//! and it collapses **above** approve, so no amount of approving rules can
//! out-vote one rule that could not be evaluated (FR-012). Only when nothing
//! denied and nothing was uncertain does an approve vote, or the library's
//! existing Auto-Approve permission, decide.
//!
//! Tags are collected from every rule that actually ran, whatever it voted: "this
//! is a kids' film" is true whether or not the rule had an opinion about
//! approving it. Whether they are *applied* is the caller's decision — a denied
//! request keeps its tags in the trace only (FR-050).

use scryer_domain::{RequestDecisionOutcome, RequestRuleEvaluationMode};
use scryer_rules::request::{RequestRuleDecision, RequestVote};

/// Pseudo rule-set id recorded when the library's Auto-Approve permission — not
/// a rule — is what approved the request. It is deliberately not a real id: the
/// trace has to say *something* decided, and "the permission did" is the honest
/// answer for every instance that has no rules at all.
pub const LIBRARY_PERMISSION_DECIDER: &str = "library_permission";

/// The rule voted `manual` in its own source.
pub const FALLBACK_RULE_MANUAL: &str = "rule_manual";
/// The host held the rule because a fact it reads was unobservable.
pub const FALLBACK_HELD: &str = "held";
/// A rule failed to evaluate (compile fault, timeout, malformed output).
pub const FALLBACK_ERROR: &str = "error";
/// Nothing voted, and the requester holds no Auto-Approve permission.
pub const FALLBACK_NO_RULE_MATCHED: &str = "no_rule_matched";

/// One rule's vote, already narrowed to the rules whose library scope covers
/// this request (see [`super::engine`]). Carries the rule's identity and mode so
/// the trace can be read without a second lookup, and so the caller can ask
/// whether any *deciding* rule was actually enforcing.
#[derive(Clone, Debug)]
pub struct ScopedVote {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub revision_number: i64,
    pub content_hash: String,
    pub mode: RequestRuleEvaluationMode,
    pub decision: RequestRuleDecision,
}

/// One rule that could not be evaluated at all.
#[derive(Clone, Debug)]
pub struct ScopedError {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub revision_number: i64,
    pub content_hash: String,
    pub mode: RequestRuleEvaluationMode,
    pub message: String,
}

/// What the policy concluded, before the instance gate and the per-rule
/// evaluation modes are applied to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arbitration {
    /// The verdict the rules (plus the permission vote) reached. This is the
    /// *policy's* answer, recorded whether or not it is allowed to act.
    pub policy_outcome: RequestDecisionOutcome,
    /// Why the verdict is a fallback rather than a rule's own decision.
    /// `None` exactly when a rule voted `deny` or `approve` and won.
    pub fallback_reason: Option<&'static str>,
    /// The rule sets that produced `policy_outcome`, or
    /// [`LIBRARY_PERMISSION_DECIDER`] when the Auto-Approve permission did.
    pub deciding_rule_set_ids: Vec<String>,
    /// Union of the tags every rule that ran emitted, in first-appearance
    /// order.
    pub tags: Vec<String>,
    /// Reason codes from the deciding rules, deduplicated, first appearance
    /// preserved.
    pub reasons: Vec<ArbitrationReason>,
}

/// One reason code, attributed to the rule that emitted it. The rule *name* is
/// what a requester or approver can act on; the id is for the trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbitrationReason {
    pub code: String,
    pub rule_set_id: String,
    pub rule_set_name: String,
}

impl Arbitration {
    /// Whether at least one of the rules that produced this verdict is allowed
    /// to act on it.
    ///
    /// A verdict nobody enforces is a *shadow* verdict: it is recorded in full
    /// and changes nothing. The permission decider counts as enforcing because
    /// it is today's behaviour, not a rule an operator has to arm.
    pub fn is_enforceable(&self, votes: &[ScopedVote], errors: &[ScopedError]) -> bool {
        if self
            .deciding_rule_set_ids
            .iter()
            .any(|id| id == LIBRARY_PERMISSION_DECIDER)
        {
            return true;
        }
        self.deciding_rule_set_ids.iter().any(|id| {
            votes.iter().any(|vote| {
                &vote.rule_set_id == id && vote.mode == RequestRuleEvaluationMode::Enforce
            }) || errors.iter().any(|error| {
                &error.rule_set_id == id && error.mode == RequestRuleEvaluationMode::Enforce
            })
        })
    }

    /// The outcome that actually takes effect.
    ///
    /// `gate_enabled` is the instance switch; `enforceable` is
    /// [`Self::is_enforceable`]. When either is false the effective outcome is
    /// exactly today's behaviour — the library's Auto-Approve permission
    /// approves, and everything else waits for a human — while `policy_outcome`
    /// keeps saying what the rules concluded. That difference is the whole of
    /// shadow mode (FR-013).
    pub fn effective_outcome(
        &self,
        gate_enabled: bool,
        enforceable: bool,
        permission_grants_auto_approve: bool,
    ) -> RequestDecisionOutcome {
        if gate_enabled && enforceable {
            self.policy_outcome
        } else {
            legacy_outcome(permission_grants_auto_approve)
        }
    }
}

/// What Scryer did before request rules existed, and what it still does
/// whenever policy is not allowed to act.
pub const fn legacy_outcome(permission_grants_auto_approve: bool) -> RequestDecisionOutcome {
    if permission_grants_auto_approve {
        RequestDecisionOutcome::AutoApprove
    } else {
        RequestDecisionOutcome::ManualReview
    }
}

/// Arbitrate one request's votes (spec 0003 FR-011).
///
/// | condition | outcome | deciders | fallback |
/// |---|---|---|---|
/// | any `deny` | `Deny` | the denying rules | — |
/// | else any `manual`, any held rule, any error | `ManualReview` | those rules | `rule_manual` \| `held` \| `error` |
/// | else any `approve` | `AutoApprove` | the approving rules | — |
/// | else Auto-Approve permission | `AutoApprove` | `library_permission` | — |
/// | else | `ManualReview` | — | `no_rule_matched` |
pub fn arbitrate(
    votes: &[ScopedVote],
    errors: &[ScopedError],
    permission_grants_auto_approve: bool,
) -> Arbitration {
    let tags = collect_tags(votes);

    let denials: Vec<&ScopedVote> = votes
        .iter()
        .filter(|vote| vote.decision.vote == RequestVote::Deny)
        .collect();
    if !denials.is_empty() {
        return Arbitration {
            policy_outcome: RequestDecisionOutcome::Deny,
            fallback_reason: None,
            deciding_rule_set_ids: rule_set_ids(&denials),
            tags,
            reasons: reasons_from(&denials),
        };
    }

    // Held and author-declared `manual` arbitrate identically; only the
    // fallback code, and the `held` flag in the trace, tell them apart. A rule
    // that errored is uncertainty of a third kind and lands in the same bucket:
    // an unevaluable rule must never be readable as "no objection".
    let manuals: Vec<&ScopedVote> = votes
        .iter()
        .filter(|vote| vote.decision.vote == RequestVote::Manual)
        .collect();
    if !manuals.is_empty() || !errors.is_empty() {
        let authored = manuals.iter().any(|vote| !vote.decision.held);
        let held = manuals.iter().any(|vote| vote.decision.held);
        let fallback_reason = if authored {
            FALLBACK_RULE_MANUAL
        } else if held {
            FALLBACK_HELD
        } else {
            FALLBACK_ERROR
        };
        let mut deciding = rule_set_ids(&manuals);
        for error in errors {
            if !deciding.contains(&error.rule_set_id) {
                deciding.push(error.rule_set_id.clone());
            }
        }
        return Arbitration {
            policy_outcome: RequestDecisionOutcome::ManualReview,
            fallback_reason: Some(fallback_reason),
            deciding_rule_set_ids: deciding,
            tags,
            reasons: reasons_from(&manuals),
        };
    }

    let approvals: Vec<&ScopedVote> = votes
        .iter()
        .filter(|vote| vote.decision.vote == RequestVote::Approve)
        .collect();
    if !approvals.is_empty() {
        return Arbitration {
            policy_outcome: RequestDecisionOutcome::AutoApprove,
            fallback_reason: None,
            deciding_rule_set_ids: rule_set_ids(&approvals),
            tags,
            reasons: reasons_from(&approvals),
        };
    }

    if permission_grants_auto_approve {
        return Arbitration {
            policy_outcome: RequestDecisionOutcome::AutoApprove,
            fallback_reason: None,
            deciding_rule_set_ids: vec![LIBRARY_PERMISSION_DECIDER.to_string()],
            tags,
            reasons: Vec::new(),
        };
    }

    Arbitration {
        policy_outcome: RequestDecisionOutcome::ManualReview,
        fallback_reason: Some(FALLBACK_NO_RULE_MATCHED),
        deciding_rule_set_ids: Vec::new(),
        tags,
        reasons: Vec::new(),
    }
}

/// Union of every tag emitted by a rule that ran, in the order the rules were
/// evaluated and, within a rule, the order it produced them. Duplicates keep
/// their first appearance so a tag two rules agree on is applied once and the
/// list stays stable across evaluations.
fn collect_tags(votes: &[ScopedVote]) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for vote in votes {
        for tag in &vote.decision.tags {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
    }
    tags
}

fn rule_set_ids(votes: &[&ScopedVote]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::with_capacity(votes.len());
    for vote in votes {
        if !ids.contains(&vote.rule_set_id) {
            ids.push(vote.rule_set_id.clone());
        }
    }
    ids
}

fn reasons_from(votes: &[&ScopedVote]) -> Vec<ArbitrationReason> {
    let mut reasons: Vec<ArbitrationReason> = Vec::new();
    for vote in votes {
        for code in &vote.decision.reason_codes {
            if reasons
                .iter()
                .any(|reason| &reason.code == code && reason.rule_set_id == vote.rule_set_id)
            {
                continue;
            }
            reasons.push(ArbitrationReason {
                code: code.clone(),
                rule_set_id: vote.rule_set_id.clone(),
                rule_set_name: vote.rule_set_name.clone(),
            });
        }
    }
    reasons
}
