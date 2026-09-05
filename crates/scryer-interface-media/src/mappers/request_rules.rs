//! Projections for the request-rule surface (spec 0003 §7).
//!
//! Three of these are load-bearing rather than mechanical:
//!
//! - [`from_request_rule_decision`] takes a `redacted` flag and, when it is set,
//!   returns the trace **without its votes**. That is the same rule the
//!   pre-flight enforces by having no vote field at all (FR-020), applied to a
//!   stored decision read back by the person it was about. `reasons` survives
//!   the redaction, because a requester who is told "denied" and nothing else
//!   has been told nothing.
//! - [`stored_votes`] parses `votes_json`. The recorded shape is written by the
//!   application layer with `&'static str` discriminants, so it cannot derive
//!   `Deserialize`; the mirror here is the one place that knows the wire names,
//!   and a `mode` or `vote` string this build does not recognize degrades to the
//!   safe reading rather than dropping the row.
//! - [`from_media_request_metadata`] reads the submit-time snapshot back through
//!   the *same* helpers the fact builder uses, so what an approver sees and what
//!   the rule saw cannot drift.

use super::*;

use scryer_application::request_rules::{
    MediaRequestPolicyFacts, RequestRuleDecisionView, RequestRulePreviewResult as AppPreviewResult,
    RequestRuleSetDetail as AppRuleSetDetail, snapshot_age_rating, us_certification_label,
};
use scryer_application::{MediaRequestMetadataSnapshot, RequestPreflight};
use scryer_domain::{
    LifecycleClaim, LifecycleClaimKind, LifecycleClaimProducer, LifecycleClaimState,
    RequestDecisionOutcome, RequestRuleDecisionRecord, RequestRuleEvaluationMode,
};
use scryer_rules::request::{
    REQUEST_INPUT_SCHEMA_VERSION, RequestVote, certification_rank_for_label,
};

/// Revision numbers, lease windows, and counts are stored as `i64` but are small
/// by construction; saturating keeps a corrupt row from panicking a read.
fn to_graphql_int(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

// ── Enum projections ────────────────────────────────────────────────────────

pub fn request_rule_evaluation_mode_value(
    mode: RequestRuleEvaluationMode,
) -> RequestRuleEvaluationModeValue {
    match mode {
        RequestRuleEvaluationMode::Disabled => RequestRuleEvaluationModeValue::Disabled,
        RequestRuleEvaluationMode::Shadow => RequestRuleEvaluationModeValue::Shadow,
        RequestRuleEvaluationMode::Enforce => RequestRuleEvaluationModeValue::Enforce,
    }
}

pub fn request_rule_evaluation_mode_into_application(
    mode: RequestRuleEvaluationModeValue,
) -> RequestRuleEvaluationMode {
    match mode {
        RequestRuleEvaluationModeValue::Disabled => RequestRuleEvaluationMode::Disabled,
        RequestRuleEvaluationModeValue::Shadow => RequestRuleEvaluationMode::Shadow,
        RequestRuleEvaluationModeValue::Enforce => RequestRuleEvaluationMode::Enforce,
    }
}

pub fn request_decision_outcome_value(
    outcome: RequestDecisionOutcome,
) -> RequestDecisionOutcomeValue {
    match outcome {
        RequestDecisionOutcome::AutoApprove => RequestDecisionOutcomeValue::AutoApprove,
        RequestDecisionOutcome::ManualReview => RequestDecisionOutcomeValue::ManualReview,
        RequestDecisionOutcome::Deny => RequestDecisionOutcomeValue::Deny,
    }
}

pub fn request_decision_outcome_into_application(
    outcome: RequestDecisionOutcomeValue,
) -> RequestDecisionOutcome {
    match outcome {
        RequestDecisionOutcomeValue::AutoApprove => RequestDecisionOutcome::AutoApprove,
        RequestDecisionOutcomeValue::ManualReview => RequestDecisionOutcome::ManualReview,
        RequestDecisionOutcomeValue::Deny => RequestDecisionOutcome::Deny,
    }
}

pub fn request_vote_value(vote: RequestVote) -> RequestVoteValue {
    match vote {
        RequestVote::Approve => RequestVoteValue::Approve,
        RequestVote::Deny => RequestVoteValue::Deny,
        RequestVote::Manual => RequestVoteValue::Manual,
        RequestVote::Abstain => RequestVoteValue::Abstain,
    }
}

fn lifecycle_claim_producer_value(producer: LifecycleClaimProducer) -> LifecycleClaimProducerValue {
    match producer {
        LifecycleClaimProducer::RequestLease => LifecycleClaimProducerValue::RequestLease,
        LifecycleClaimProducer::RequestPermanent => LifecycleClaimProducerValue::RequestPermanent,
        LifecycleClaimProducer::OperatorKeep => LifecycleClaimProducerValue::OperatorKeep,
    }
}

fn lifecycle_claim_kind_value(kind: LifecycleClaimKind) -> LifecycleClaimKindValue {
    match kind {
        LifecycleClaimKind::RetainUntil => LifecycleClaimKindValue::RetainUntil,
        LifecycleClaimKind::Keep => LifecycleClaimKindValue::Keep,
    }
}

fn lifecycle_claim_state_value(state: LifecycleClaimState) -> LifecycleClaimStateValue {
    match state {
        LifecycleClaimState::Dormant => LifecycleClaimStateValue::Dormant,
        LifecycleClaimState::Active => LifecycleClaimStateValue::Active,
        LifecycleClaimState::Expired => LifecycleClaimStateValue::Expired,
        LifecycleClaimState::Released => LifecycleClaimStateValue::Released,
        LifecycleClaimState::Converted => LifecycleClaimStateValue::Converted,
    }
}

// ── Rule sets and revisions ─────────────────────────────────────────────────

/// `decision_count` is supplied by the caller rather than read here: the list
/// view wants one count per rule, and a mapper has no repository.
pub fn from_request_rule_set(
    rule_set: &scryer_domain::RequestRuleSet,
    decision_count: u64,
) -> RequestRuleSet {
    RequestRuleSet {
        id: ID::from(rule_set.id.clone()),
        name: rule_set.name.clone(),
        description: rule_set.description.clone(),
        enabled: rule_set.enabled,
        evaluation_mode: request_rule_evaluation_mode_value(rule_set.evaluation_mode),
        library_ids: rule_set.library_ids.clone(),
        current_revision_number: to_graphql_int(rule_set.current_revision_number),
        decision_count: to_graphql_int(i64::try_from(decision_count).unwrap_or(i64::MAX)),
        created_at: rule_set.created_at,
        updated_at: rule_set.updated_at,
    }
}

/// The stored source always carries the system-assigned package declaration and
/// the rego.v1 import; both are stripped here so what the editor shows is what
/// the author wrote.
pub fn from_request_rule_revision(
    revision: scryer_domain::RequestRuleRevision,
) -> RequestRuleRevision {
    RequestRuleRevision {
        id: ID::from(revision.id),
        rule_set_id: ID::from(revision.rule_set_id),
        revision_number: to_graphql_int(revision.revision_number),
        rego_source: scryer_rules::strip_editor_source(&revision.rego_source),
        matcher_content_hash: revision.matcher_content_hash,
        created_by: revision.created_by.map(ID::from),
        created_at: revision.created_at,
    }
}

pub fn from_request_rule_set_detail(
    detail: AppRuleSetDetail,
    decision_count: u64,
) -> RequestRuleSetDetail {
    RequestRuleSetDetail {
        rule_set: from_request_rule_set(&detail.rule_set, decision_count),
        revision: from_request_rule_revision(detail.revision),
    }
}

// ── Decision traces ─────────────────────────────────────────────────────────

/// The `votes_json` row shape, mirrored for reading.
///
/// Every field is defaulted: a trace written by a newer build with extra fields
/// still reads, and one written by an older build without `tags` reads as
/// untagged rather than failing the whole decision.
#[derive(serde::Deserialize)]
struct StoredVote {
    #[serde(default)]
    rule_set_id: String,
    #[serde(default)]
    rule_set_name: String,
    #[serde(default)]
    revision_number: i64,
    #[serde(default)]
    vote: Option<String>,
    #[serde(default)]
    held: bool,
    #[serde(default)]
    reason_codes: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    error: Option<String>,
}

fn request_vote_from_storage(value: &str) -> Option<RequestVoteValue> {
    match value {
        "approve" => Some(RequestVoteValue::Approve),
        "deny" => Some(RequestVoteValue::Deny),
        "manual" => Some(RequestVoteValue::Manual),
        "abstain" => Some(RequestVoteValue::Abstain),
        _ => None,
    }
}

/// Parse the stored vote table. An unparseable document reads as *no votes*,
/// never as an error: the outcome and the fallback reason are stored in their
/// own columns and stay true even when the trace's detail cannot be read.
fn stored_votes(votes_json: &str) -> Vec<RequestRuleVote> {
    serde_json::from_str::<Vec<StoredVote>>(votes_json)
        .unwrap_or_default()
        .into_iter()
        .map(|vote| RequestRuleVote {
            rule_set_id: ID::from(vote.rule_set_id),
            rule_set_name: vote.rule_set_name,
            revision_number: to_graphql_int(vote.revision_number),
            vote: vote.vote.as_deref().and_then(request_vote_from_storage),
            held: vote.held,
            reason_codes: vote.reason_codes,
            tags: vote.tags,
            error: vote.error,
        })
        .collect()
}

fn reasons_from_votes(votes: &[RequestRuleVote]) -> Vec<RequestPreflightReason> {
    votes
        .iter()
        .flat_map(|vote| {
            vote.reason_codes.iter().map(|code| RequestPreflightReason {
                code: code.clone(),
                rule_name: vote.rule_set_name.clone(),
            })
        })
        .collect()
}

/// Project one stored decision.
///
/// `redacted` is the requester view: the reasons and the verdict survive, the
/// vote table does not. It is a parameter rather than a second function so no
/// caller can reach the unredacted projection by forgetting to ask.
pub fn from_request_rule_decision(
    record: RequestRuleDecisionRecord,
    redacted: bool,
) -> RequestRuleDecision {
    let votes = stored_votes(&record.votes_json);
    let reasons = reasons_from_votes(&votes);
    RequestRuleDecision {
        id: Some(ID::from(record.id)),
        request_id: Some(ID::from(record.request_id)),
        evaluated_at: record.evaluated_at,
        mode: request_rule_evaluation_mode_value(record.mode),
        effective_outcome: request_decision_outcome_value(record.effective_outcome),
        policy_outcome: request_decision_outcome_value(record.policy_outcome),
        fallback_reason: record.fallback_reason,
        votes: if redacted { Vec::new() } else { votes },
        reasons,
        tags: record.tags,
        input_schema_version: to_graphql_int(record.input_schema_version),
    }
}

pub fn from_request_rule_decision_view(view: RequestRuleDecisionView) -> RequestRuleDecision {
    from_request_rule_decision(view.record, view.redacted)
}

// ── Author-side preview ─────────────────────────────────────────────────────

/// The outcome one rule's vote contributes, on its own.
///
/// A preview runs a single matcher, so there is no arbitration across rules and
/// no instance gate: an approve previews as an auto-approval, a deny as a
/// denial, and everything uncertain — a manual, an abstain, a hold, a failure —
/// as manual review, which is the direction the real arbitration also takes.
fn preview_outcome(vote: Option<RequestVote>) -> RequestDecisionOutcome {
    match vote {
        Some(RequestVote::Approve) => RequestDecisionOutcome::AutoApprove,
        Some(RequestVote::Deny) => RequestDecisionOutcome::Deny,
        _ => RequestDecisionOutcome::ManualReview,
    }
}

/// Why a preview did not follow a vote, in the same vocabulary the stored
/// traces use.
fn preview_fallback_reason(result: &AppPreviewResult) -> Option<String> {
    if result.error.is_some() {
        return Some(scryer_application::FALLBACK_ERROR.to_string());
    }
    match result.vote {
        Some(RequestVote::Manual) if result.held => {
            Some(scryer_application::FALLBACK_HELD.to_string())
        }
        Some(RequestVote::Manual) => Some(scryer_application::FALLBACK_RULE_MANUAL.to_string()),
        Some(RequestVote::Abstain) => {
            Some(scryer_application::FALLBACK_NO_RULE_MATCHED.to_string())
        }
        _ => None,
    }
}

/// `mode` is the stored rule set's mode, or `DISABLED` for an unsaved draft —
/// an unsaved matcher is not armed, because it is not stored.
pub fn from_request_rule_preview_result(
    result: AppPreviewResult,
    rule_set_name: String,
    mode: RequestRuleEvaluationMode,
) -> RequestRulePreviewPayload {
    let outcome = request_decision_outcome_value(preview_outcome(result.vote));
    let fallback_reason = preview_fallback_reason(&result);
    let reasons: Vec<RequestPreflightReason> = result
        .reasons
        .iter()
        .map(|reason| RequestPreflightReason {
            code: reason.code.clone(),
            rule_name: reason.rule_name.clone(),
        })
        .collect();
    let vote = RequestRuleVote {
        rule_set_id: ID::from(result.rule_set_id.clone()),
        rule_set_name,
        // A preview always runs the revision currently in force; the trace's
        // revision number is not part of the preview result, and reporting a
        // guess would be worse than reporting nothing.
        revision_number: 0,
        vote: result.vote.map(request_vote_value),
        held: result.held,
        reason_codes: result
            .reasons
            .iter()
            .map(|reason| reason.code.clone())
            .collect(),
        tags: result.tags.clone(),
        error: result.error.clone(),
    };

    RequestRulePreviewPayload {
        rule_set_id: result.rule_set_id,
        matcher_content_hash: result.matcher_content_hash,
        decision: RequestRuleDecision {
            id: None,
            request_id: None,
            evaluated_at: result.evaluated_at,
            mode: request_rule_evaluation_mode_value(mode),
            effective_outcome: outcome,
            policy_outcome: outcome,
            fallback_reason,
            votes: vec![vote],
            reasons,
            tags: result.tags,
            input_schema_version: to_graphql_int(i64::from(REQUEST_INPUT_SCHEMA_VERSION)),
        },
        metadata_partial: result.metadata_partial,
        input_document: async_graphql::Json(
            serde_json::from_str(&result.input_json).unwrap_or(serde_json::Value::Null),
        ),
    }
}

// ── Requester-side pre-flight ───────────────────────────────────────────────

pub fn from_request_preflight(preflight: RequestPreflight) -> RequestPreflightPayload {
    RequestPreflightPayload {
        outcome: request_decision_outcome_value(preflight.outcome),
        reasons: preflight
            .reasons
            .into_iter()
            .map(|reason| RequestPreflightReason {
                code: reason.code,
                rule_name: reason.rule_name,
            })
            .collect(),
        tags: preflight.tags,
        metadata_partial: preflight.metadata_partial,
        evaluation_mode: request_rule_evaluation_mode_value(preflight.evaluation_mode),
        fallback_reason: preflight.fallback_reason,
    }
}

// ── Gates and claims ────────────────────────────────────────────────────────

pub fn from_request_rule_instance_gates(
    gates: scryer_application::RequestRuleGates,
) -> RequestRuleInstanceGates {
    RequestRuleInstanceGates {
        evaluation_enabled: gates.evaluation_enabled,
    }
}

pub fn from_title_claim(claim: LifecycleClaim) -> TitleClaim {
    TitleClaim {
        id: ID::from(claim.id),
        title_id: ID::from(claim.title_id),
        library_id: ID::from(claim.library_id),
        producer: lifecycle_claim_producer_value(claim.producer),
        producer_ref: claim.producer_ref,
        kind: lifecycle_claim_kind_value(claim.kind),
        state: lifecycle_claim_state_value(claim.state),
        duration_days: claim.duration_days.map(to_graphql_int),
        starts_at: claim.starts_at,
        expires_at: claim.expires_at,
        created_by: claim.created_by.map(ID::from),
        created_at: claim.created_at,
        updated_at: claim.updated_at,
        released_reason: claim.released_reason,
    }
}

/// The lease view of a request: the days from the request row, the window from
/// the claim that actually holds the title.
pub fn from_media_request_lease(
    claim: &LifecycleClaim,
    requested_lease_days: Option<i64>,
    approved_lease_days: Option<i64>,
) -> MediaRequestLease {
    MediaRequestLease {
        requested_days: requested_lease_days.map(to_graphql_int),
        approved_days: approved_lease_days.map(to_graphql_int),
        state: lifecycle_claim_state_value(claim.state),
        starts_at: claim.starts_at,
        expires_at: claim.expires_at,
    }
}

// ── Submit-time metadata snapshot ───────────────────────────────────────────

pub fn from_media_request_metadata(
    snapshot: &MediaRequestMetadataSnapshot,
) -> MediaRequestMetadataPayload {
    let certification_label = us_certification_label(snapshot);
    MediaRequestMetadataPayload {
        partial: snapshot.partial,
        missing: snapshot.missing.clone(),
        genres: snapshot.genres.clone(),
        content_ratings: snapshot
            .content_ratings
            .iter()
            .map(|rating| DiscoveryContentRatingPayload {
                country: rating.country.clone(),
                certifications: rating
                    .certifications
                    .iter()
                    .map(|certification| DiscoveryContentCertificationPayload {
                        value: certification.value.clone(),
                        source: certification.source.clone(),
                        release_type: certification.release_type,
                    })
                    .collect(),
                age_rating: rating.age_rating,
                age_rating_source: rating.age_rating_source.clone(),
            })
            .collect(),
        age_rating: snapshot_age_rating(snapshot).map(to_graphql_int),
        certification_rank: certification_label
            .as_deref()
            .and_then(certification_rank_for_label)
            .map(to_graphql_int),
        certification_label,
        is_adult: snapshot.is_adult,
        tmdb_vote_average: snapshot.tmdb_vote_average,
        tmdb_vote_count: snapshot.tmdb_vote_count.map(to_graphql_int),
        popularity: snapshot.popularity,
        award_count: to_graphql_int(snapshot.awards.len() as i64),
    }
}

/// The policy half of a media-request payload.
///
/// Split out so `from_media_request` keeps one shape whether or not the caller
/// pre-loaded the facts: a caller that did not ask for them projects a request
/// with no lease and no decision, which is exactly what a request that was never
/// evaluated looks like.
pub(crate) struct MediaRequestPolicyProjection {
    pub lease: Option<MediaRequestLease>,
    pub decision: Option<RequestRuleDecision>,
}

pub(crate) fn project_media_request_policy(
    request: &MediaRequest,
    facts: Option<&MediaRequestPolicyFacts>,
) -> MediaRequestPolicyProjection {
    let Some(facts) = facts else {
        return MediaRequestPolicyProjection {
            lease: None,
            decision: None,
        };
    };
    MediaRequestPolicyProjection {
        lease: facts.lease_claim.as_ref().map(|claim| {
            from_media_request_lease(
                claim,
                request.requested_lease_days,
                request.approved_lease_days,
            )
        }),
        decision: facts
            .decision
            .clone()
            .map(|record| from_request_rule_decision(record, facts.decision_redacted)),
    }
}

/// The one conversion from the submit input to the application's draft.
///
/// The pre-flight query and the submit mutation both go through this, which is
/// what makes FR-021 — identical drafts yield identical decisions — a property
/// of the code: the two cannot disagree about what a draft *is*.
pub fn submit_media_request_input_into_application(
    input: SubmitMediaRequestInput,
) -> scryer_application::SubmitMediaRequestInput {
    scryer_application::SubmitMediaRequestInput {
        library_id: String::from(input.library_id),
        facet: input.facet.into_domain(),
        title: input.title,
        sort_title: input.sort_title,
        slug: input.slug,
        year: input.year,
        overview: input.overview,
        runtime_minutes: input.runtime_minutes,
        language: input.language,
        content_status: input.content_status,
        rating_summary: TitleRatingSummary {
            rating: input.rating,
            rating_sources: input.rating_sources.unwrap_or_default(),
            external_ratings: input
                .external_ratings
                .unwrap_or_default()
                .into_iter()
                .map(|rating| scryer_domain::TitleExternalRating {
                    source: rating.source,
                    value: rating.value,
                    score: rating.score,
                    normalized: rating.normalized,
                    votes: rating.votes,
                    url: rating.url,
                })
                .collect(),
        },
        requested_quality_profile_id: input.requested_quality_profile_id.map(String::from),
        requested_monitor_type: input
            .requested_monitor_type
            .map(|value| value.as_tag_value().to_string()),
        requested_monitor_selection: input
            .requested_monitor_selection
            .map(super::monitor_selection_from_input),
        // Omitted means forever, which is what Scryer granted before leases
        // existed. The service validates the range.
        requested_lease_days: input.requested_lease_days.map(i64::from),
        external_ids: input
            .external_ids
            .into_iter()
            .map(|external_id| scryer_domain::ExternalId {
                source: external_id.source,
                value: external_id.value,
            })
            .collect(),
    }
}

/// The sample an author-side preview is evaluated against.
pub fn request_rule_sample_into_application(
    sample: RequestRuleSampleInput,
) -> Result<scryer_application::RequestRuleSample, String> {
    let lease_days = match (sample.lease_days, sample.lease_forever.unwrap_or(false)) {
        (Some(_), true) => {
            return Err(
                "a preview sample accepts either 'leaseDays' or 'leaseForever', not both"
                    .to_string(),
            );
        }
        (Some(days), false) => Some(Some(i64::from(days))),
        (None, true) => Some(None),
        (None, false) => None,
    };
    Ok(scryer_application::RequestRuleSample {
        user_id: String::from(sample.user_id),
        library_id: String::from(sample.library_id),
        external_ids: sample
            .external_ids
            .into_iter()
            .map(|external_id| scryer_domain::ExternalId {
                source: external_id.source,
                value: external_id.value,
            })
            .collect(),
        quality_profile_id: sample.quality_profile_id.map(String::from),
        monitor_type: sample
            .monitor_type
            .map(|value| value.as_tag_value().to_string()),
        lease_days,
    })
}

/// The Rules Context Reference document, as the `JSON` scalar.
///
/// Aliased so resolver crates can name the return type without taking a
/// `serde_json` dependency of their own.
pub type RequestRuleInputReference = async_graphql::Json<serde_json::Value>;

/// The request family's Rules Context Reference, served from the crate's own
/// copy of the contract.
pub fn request_rule_input_reference() -> RequestRuleInputReference {
    async_graphql::Json(
        serde_json::from_str(scryer_rules::validation::request_input_contract_json())
            .unwrap_or(serde_json::Value::Null),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(votes_json: &str) -> RequestRuleDecisionRecord {
        RequestRuleDecisionRecord {
            id: "decision-1".to_string(),
            request_id: "request-1".to_string(),
            evaluated_at: Utc::now(),
            mode: RequestRuleEvaluationMode::Enforce,
            effective_outcome: RequestDecisionOutcome::Deny,
            policy_outcome: RequestDecisionOutcome::Deny,
            fallback_reason: None,
            votes_json: votes_json.to_string(),
            tags: vec!["kids".to_string()],
            input_hash: "hash".to_string(),
            input_schema_version: 1,
            created_at: Utc::now(),
        }
    }

    const VOTES: &str = r#"[
        {"rule_set_id":"a","rule_set_name":"Family ratings only","revision_number":3,
         "content_hash":"h","mode":"enforce","vote":"deny","held":false,
         "reason_codes":["policy_denied"],"tags":["kids"]},
        {"rule_set_id":"b","rule_set_name":"Quota","revision_number":1,
         "content_hash":"h","mode":"shadow","held":false,
         "reason_codes":[],"tags":[],"error":"policy evaluation exceeded its time budget"}
    ]"#;

    #[test]
    fn a_manager_sees_the_votes_and_a_requester_sees_only_the_reasons() {
        let full = from_request_rule_decision(record(VOTES), false);
        assert_eq!(full.votes.len(), 2);
        assert_eq!(full.votes[0].vote, Some(RequestVoteValue::Deny));
        // A rule that failed produced no vote; rendering that as an abstain
        // would say it looked and had no opinion.
        assert_eq!(full.votes[1].vote, None);
        assert_eq!(
            full.votes[1].error.as_deref(),
            Some("policy evaluation exceeded its time budget")
        );

        let redacted = from_request_rule_decision(record(VOTES), true);
        assert!(redacted.votes.is_empty(), "a requester sees no vote table");
        assert_eq!(redacted.reasons.len(), 1);
        assert_eq!(redacted.reasons[0].code, "policy_denied");
        assert_eq!(redacted.reasons[0].rule_name, "Family ratings only");
        assert_eq!(redacted.tags, vec!["kids".to_string()]);
        assert_eq!(redacted.policy_outcome, RequestDecisionOutcomeValue::Deny);
    }

    #[test]
    fn an_unreadable_vote_table_leaves_the_verdict_intact() {
        let decision = from_request_rule_decision(record("not json"), false);
        assert!(decision.votes.is_empty());
        assert!(decision.reasons.is_empty());
        assert_eq!(
            decision.effective_outcome,
            RequestDecisionOutcomeValue::Deny
        );
    }

    #[test]
    fn a_vote_this_build_does_not_know_reads_as_no_vote() {
        let decision = from_request_rule_decision(
            record(r#"[{"rule_set_id":"a","rule_set_name":"n","vote":"veto"}]"#),
            false,
        );
        assert_eq!(decision.votes.len(), 1);
        assert_eq!(decision.votes[0].vote, None);
    }

    #[test]
    fn preview_outcomes_never_approve_on_uncertainty() {
        assert_eq!(
            preview_outcome(Some(RequestVote::Approve)),
            RequestDecisionOutcome::AutoApprove
        );
        assert_eq!(
            preview_outcome(Some(RequestVote::Deny)),
            RequestDecisionOutcome::Deny
        );
        for vote in [Some(RequestVote::Manual), Some(RequestVote::Abstain), None] {
            assert_eq!(preview_outcome(vote), RequestDecisionOutcome::ManualReview);
        }
    }

    #[test]
    fn the_contract_reference_is_a_json_document() {
        let reference = request_rule_input_reference();
        assert!(
            reference.0.get("sections").is_some(),
            "the request contract should expose its sections"
        );
    }
}
