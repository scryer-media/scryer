//! Arbitration across request-rule votes.
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
//! out-vote one rule that could not be evaluated. Only when nothing
//! denied and nothing was uncertain does an approve vote, or the library's
//! existing Auto-Approve permission, decide.
//!
//! Tags are collected from every rule that actually ran, whatever it voted: "this
//! is a kids' film" is true whether or not the rule had an opinion about
//! approving it. Whether they are *applied* is the caller's decision — a denied
//! request keeps its tags in the trace only.

use std::collections::BTreeMap;

use scryer_domain::RequestDecisionOutcome;
use scryer_rules::request::{RequestReportVote, RequestRuleDecision, RequestVote};
use scryer_rules::request_actions::{RequestActionHead, RequestHeadValue, RequestShaping};
use serde::{Deserialize, Serialize};

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
/// No rule decided, and the requester's Auto-Approve permission approved. A
/// verdict no rule reached is a fallback like any other, and naming it is what
/// lets the trace, the pre-flight and the requests view say the permission
/// approved rather than showing an approval with nothing behind it.
pub const FALLBACK_LIBRARY_PERMISSION: &str = LIBRARY_PERMISSION_DECIDER;
/// The rules would have approved, but two of them set one shaping head to
/// different values, so a person picks.
pub const FALLBACK_SHAPING_CONFLICT: &str = "shaping_conflict";
/// The rules would have approved, but a shaping value cannot apply to this
/// request: a quality profile that no longer exists, a monitor value the
/// facet does not have, or an audio language Scryer does not know.
pub const FALLBACK_SHAPING_INVALID: &str = "shaping_invalid";
/// Report rules disagreed: one marked the file bad and another rejected the
/// report, or one rule did both, so a person decides.
pub const FALLBACK_REPORT_CONFLICT: &str = "report_conflict";

/// Every fallback code a decision can carry. Released codes are API: never
/// renamed, reused or repurposed.
pub const ALL_FALLBACKS: [&str; 8] = [
    FALLBACK_RULE_MANUAL,
    FALLBACK_HELD,
    FALLBACK_ERROR,
    FALLBACK_NO_RULE_MATCHED,
    FALLBACK_LIBRARY_PERMISSION,
    FALLBACK_SHAPING_CONFLICT,
    FALLBACK_SHAPING_INVALID,
    FALLBACK_REPORT_CONFLICT,
];

/// One rule's vote, already narrowed to the rules whose library scope covers
/// this request (see [`super::engine`]). Carries the rule's identity so
/// the trace can be read without a second lookup.
#[derive(Clone, Debug)]
pub struct ScopedVote {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub revision_number: i64,
    pub content_hash: String,
    pub decision: RequestRuleDecision,
}

/// One rule that could not be evaluated at all.
#[derive(Clone, Debug)]
pub struct ScopedError {
    pub rule_set_id: String,
    pub rule_set_name: String,
    pub revision_number: i64,
    pub content_hash: String,
    pub message: String,
}

/// What the policy concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arbitration {
    /// The verdict the rules (plus the permission vote) reached.
    pub policy_outcome: RequestDecisionOutcome,
    /// Why the verdict is a fallback rather than a rule's own decision.
    /// `None` exactly when a rule voted `deny` or `approve` and won; an
    /// approval by the Auto-Approve permission is
    /// [`FALLBACK_LIBRARY_PERMISSION`].
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
    /// What the shaping heads of every rule that ran set, each value with the
    /// rule set that set it.
    pub shaping: ArbitratedShaping,
}

/// One shaping value or notification, attributed to the rule set that set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapedValue {
    pub value: RequestHeadValue,
    pub rule_set_id: String,
    pub rule_set_name: String,
}

/// A shaping head that could not be applied, with the rule sets that set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapingRefusal {
    pub head: RequestActionHead,
    pub rule_set_ids: Vec<String>,
}

/// The shaping every rule that ran agreed on, and what it disagreed on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitratedShaping {
    /// One agreed value per shaping head that carries one.
    #[serde(default)]
    pub values: BTreeMap<RequestActionHead, ShapedValue>,
    /// Heads two rules set to different values. Such a head has no value.
    #[serde(default)]
    pub conflicts: Vec<ShapingRefusal>,
    /// Heads whose value cannot apply to this request. Such a head has no
    /// value.
    #[serde(default)]
    pub invalid: Vec<ShapingRefusal>,
    /// The notifications rules produced, one per rule, in evaluation order.
    #[serde(default)]
    pub notifications: Vec<ShapedValue>,
}

impl ArbitratedShaping {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn value(&self, head: RequestActionHead) -> Option<&ShapedValue> {
        self.values.get(&head)
    }

    /// The agreed values, as one rule's shaping.
    pub fn to_shaping(&self) -> RequestShaping {
        let mut shaping = RequestShaping::default();
        for (head, shaped) in &self.values {
            shaping.set(*head, Some(shaped.value.clone()));
        }
        shaping
    }

    /// The notifications, in evaluation order, as the submitted event carries
    /// them.
    pub fn rule_notifications(&self) -> Vec<scryer_domain::RequestRuleNotification> {
        self.notifications
            .iter()
            .filter_map(|shaped| match &shaped.value {
                RequestHeadValue::Text(text) => Some(scryer_domain::RequestRuleNotification {
                    rule_set_id: shaped.rule_set_id.clone(),
                    rule_set_name: shaped.rule_set_name.clone(),
                    text: text.clone(),
                }),
                _ => None,
            })
            .collect()
    }
}

impl Arbitration {
    /// Drop shaping values that cannot apply to this request. An approval
    /// they were part of becomes manual review, decided by the rule sets that
    /// set them.
    pub fn refuse_invalid_shaping(&mut self, heads: &[RequestActionHead]) {
        let mut refused_ids: Vec<String> = Vec::new();
        for head in heads {
            let Some(shaped) = self.shaping.values.remove(head) else {
                continue;
            };
            push_unique(&mut refused_ids, &shaped.rule_set_id);
            self.shaping.invalid.push(ShapingRefusal {
                head: *head,
                rule_set_ids: vec![shaped.rule_set_id],
            });
        }
        if !refused_ids.is_empty() && self.policy_outcome == RequestDecisionOutcome::AutoApprove {
            self.policy_outcome = RequestDecisionOutcome::ManualReview;
            self.fallback_reason = Some(FALLBACK_SHAPING_INVALID);
            self.deciding_rule_set_ids = refused_ids;
        }
    }
}

fn push_unique(ids: &mut Vec<String>, id: &str) {
    if !ids.iter().any(|seen| seen == id) {
        ids.push(id.to_string());
    }
}

/// Collect the shaping of every rule that ran. A held rule did not run, and a
/// rule that errored has no vote at all, so neither contributes.
fn collect_shaping(votes: &[ScopedVote]) -> ArbitratedShaping {
    let mut shaping = ArbitratedShaping::default();
    for vote in votes.iter().filter(|vote| !vote.decision.held) {
        for head in RequestActionHead::shaping() {
            let Some(value) = vote.decision.shaping.get(head) else {
                continue;
            };
            if let Some(conflict) = shaping.conflicts.iter_mut().find(|c| c.head == head) {
                push_unique(&mut conflict.rule_set_ids, &vote.rule_set_id);
                continue;
            }
            match shaping.values.get(&head) {
                Some(agreed) if agreed.value == value => {}
                Some(_) => {
                    let agreed = shaping.values.remove(&head).expect("value present");
                    let mut rule_set_ids = vec![agreed.rule_set_id];
                    push_unique(&mut rule_set_ids, &vote.rule_set_id);
                    shaping
                        .conflicts
                        .push(ShapingRefusal { head, rule_set_ids });
                }
                None => {
                    shaping.values.insert(
                        head,
                        ShapedValue {
                            value,
                            rule_set_id: vote.rule_set_id.clone(),
                            rule_set_name: vote.rule_set_name.clone(),
                        },
                    );
                }
            }
        }
        if let Some(text) = &vote.decision.notify {
            shaping.notifications.push(ShapedValue {
                value: RequestHeadValue::Text(text.clone()),
                rule_set_id: vote.rule_set_id.clone(),
                rule_set_name: vote.rule_set_name.clone(),
            });
        }
    }
    shaping
}

/// One reason code, attributed to the rule that emitted it. The rule *name* is
/// what a requester or approver can act on; the id is for the trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbitrationReason {
    pub code: String,
    pub rule_set_id: String,
    pub rule_set_name: String,
}

/// Arbitrate one request's votes.
///
/// | condition | outcome | deciders | fallback |
/// |---|---|---|---|
/// | any `deny` | `Deny` | the denying rules | — |
/// | else any `manual`, any held rule, any error | `ManualReview` | those rules | `rule_manual` \| `held` \| `error` |
/// | else any `approve` | `AutoApprove` | the approving rules | — |
/// | else Auto-Approve permission | `AutoApprove` | `library_permission` | `library_permission` |
/// | else | `ManualReview` | — | `no_rule_matched` |
///
/// Shaping values come from every rule that ran. When two of them set one
/// head to different values, an `AutoApprove` becomes `ManualReview`, decided
/// by the disagreeing rule sets, with fallback `shaping_conflict`.
pub fn arbitrate(
    votes: &[ScopedVote],
    errors: &[ScopedError],
    permission_grants_auto_approve: bool,
) -> Arbitration {
    let verdict = arbitrate_votes(votes, errors, permission_grants_auto_approve);
    let mut arbitration = Arbitration {
        policy_outcome: verdict.policy_outcome,
        fallback_reason: verdict.fallback_reason,
        deciding_rule_set_ids: verdict.deciding_rule_set_ids,
        tags: collect_tags(votes),
        reasons: verdict.reasons,
        shaping: collect_shaping(votes),
    };
    if arbitration.policy_outcome == RequestDecisionOutcome::AutoApprove
        && !arbitration.shaping.conflicts.is_empty()
    {
        let mut deciding: Vec<String> = Vec::new();
        for conflict in &arbitration.shaping.conflicts {
            for id in &conflict.rule_set_ids {
                push_unique(&mut deciding, id);
            }
        }
        arbitration.policy_outcome = RequestDecisionOutcome::ManualReview;
        arbitration.fallback_reason = Some(FALLBACK_SHAPING_CONFLICT);
        arbitration.deciding_rule_set_ids = deciding;
    }
    arbitration
}

/// The verdict the votes alone reach, before tags and shaping are collected.
struct Verdict {
    policy_outcome: RequestDecisionOutcome,
    fallback_reason: Option<&'static str>,
    deciding_rule_set_ids: Vec<String>,
    reasons: Vec<ArbitrationReason>,
}

fn arbitrate_votes(
    votes: &[ScopedVote],
    errors: &[ScopedError],
    permission_grants_auto_approve: bool,
) -> Verdict {
    let denials: Vec<&ScopedVote> = votes
        .iter()
        .filter(|vote| vote.decision.vote == RequestVote::Deny)
        .collect();
    if !denials.is_empty() {
        return Verdict {
            policy_outcome: RequestDecisionOutcome::Deny,
            fallback_reason: None,
            deciding_rule_set_ids: rule_set_ids(&denials),
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
        return Verdict {
            policy_outcome: RequestDecisionOutcome::ManualReview,
            fallback_reason: Some(fallback_reason),
            deciding_rule_set_ids: deciding,
            reasons: reasons_from(&manuals),
        };
    }

    let approvals: Vec<&ScopedVote> = votes
        .iter()
        .filter(|vote| vote.decision.vote == RequestVote::Approve)
        .collect();
    if !approvals.is_empty() {
        return Verdict {
            policy_outcome: RequestDecisionOutcome::AutoApprove,
            fallback_reason: None,
            deciding_rule_set_ids: rule_set_ids(&approvals),
            reasons: reasons_from(&approvals),
        };
    }

    if permission_grants_auto_approve {
        return Verdict {
            policy_outcome: RequestDecisionOutcome::AutoApprove,
            fallback_reason: Some(FALLBACK_LIBRARY_PERMISSION),
            deciding_rule_set_ids: vec![LIBRARY_PERMISSION_DECIDER.to_string()],
            reasons: Vec::new(),
        };
    }

    Verdict {
        policy_outcome: RequestDecisionOutcome::ManualReview,
        fallback_reason: Some(FALLBACK_NO_RULE_MATCHED),
        deciding_rule_set_ids: Vec::new(),
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

/// What the report rules concluded about one bad-file report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportVerdict {
    /// A person decides.
    Review,
    /// The rules reject the report.
    Reject,
    /// The rules mark the file bad. Nothing more: a recycle and search always
    /// waits for an operator.
    MarkBad,
}

/// What the report rules concluded, with the fallback and deciders the trace
/// records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportArbitration {
    pub verdict: ReportVerdict,
    pub fallback_reason: Option<&'static str>,
    pub deciding_rule_set_ids: Vec<String>,
    pub reasons: Vec<ArbitrationReason>,
}

impl ReportArbitration {
    /// The verdict in the trace's outcome column: marking bad records as an
    /// automatic approval, a rejection as a denial.
    pub fn outcome(&self) -> RequestDecisionOutcome {
        match self.verdict {
            ReportVerdict::Review => RequestDecisionOutcome::ManualReview,
            ReportVerdict::Reject => RequestDecisionOutcome::Deny,
            ReportVerdict::MarkBad => RequestDecisionOutcome::AutoApprove,
        }
    }
}

/// Arbitrate one bad-file report's votes.
///
/// | condition | verdict | deciders | fallback |
/// |---|---|---|---|
/// | any `manual`, any held rule, any error | `Review` | those rules | `rule_manual` \| `held` \| `error` |
/// | else `mark_bad` and `reject_report` both fired | `Review` | those rules | `report_conflict` |
/// | else any `mark_bad` | `MarkBad` | the marking rules | — |
/// | else any `reject_report` | `Reject` | the rejecting rules | — |
/// | else | `Review` | — | `no_rule_matched` |
///
/// There is no permission fallback: without a rule's verdict, a person
/// decides.
pub fn arbitrate_report(votes: &[ScopedVote], errors: &[ScopedError]) -> ReportArbitration {
    let manuals: Vec<&ScopedVote> = votes
        .iter()
        .filter(|vote| vote.decision.held || vote.decision.vote == RequestVote::Manual)
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
            push_unique(&mut deciding, &error.rule_set_id);
        }
        return ReportArbitration {
            verdict: ReportVerdict::Review,
            fallback_reason: Some(fallback_reason),
            deciding_rule_set_ids: deciding,
            reasons: reasons_from(&manuals),
        };
    }

    let with_vote = |wanted: &[RequestReportVote]| -> Vec<&ScopedVote> {
        votes
            .iter()
            .filter(|vote| wanted.contains(&vote.decision.report_vote))
            .collect()
    };
    let marks = with_vote(&[RequestReportVote::MarkBad, RequestReportVote::Conflict]);
    let rejects = with_vote(&[RequestReportVote::Reject, RequestReportVote::Conflict]);
    match (marks.is_empty(), rejects.is_empty()) {
        (false, false) => {
            let mut disagreeing: Vec<&ScopedVote> = marks.clone();
            for vote in rejects {
                if !disagreeing
                    .iter()
                    .any(|seen| seen.rule_set_id == vote.rule_set_id)
                {
                    disagreeing.push(vote);
                }
            }
            ReportArbitration {
                verdict: ReportVerdict::Review,
                fallback_reason: Some(FALLBACK_REPORT_CONFLICT),
                deciding_rule_set_ids: rule_set_ids(&disagreeing),
                reasons: reasons_from(&disagreeing),
            }
        }
        (false, true) => ReportArbitration {
            verdict: ReportVerdict::MarkBad,
            fallback_reason: None,
            deciding_rule_set_ids: rule_set_ids(&marks),
            reasons: reasons_from(&marks),
        },
        (true, false) => ReportArbitration {
            verdict: ReportVerdict::Reject,
            fallback_reason: None,
            deciding_rule_set_ids: rule_set_ids(&rejects),
            reasons: reasons_from(&rejects),
        },
        (true, true) => ReportArbitration {
            verdict: ReportVerdict::Review,
            fallback_reason: Some(FALLBACK_NO_RULE_MATCHED),
            deciding_rule_set_ids: Vec::new(),
            reasons: Vec::new(),
        },
    }
}
