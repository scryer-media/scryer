//! Request rules end to end: authoring, arbitration, and the request flow
//! (spec 0003 FR-010…FR-016, FR-020, FR-021, FR-040, FR-041, FR-050).
//!
//! The arbitration half is pure and table-driven. The flow half runs real
//! submissions through `bootstrap_media_request_app` with the in-memory rule,
//! decision, and claim doubles, so what is asserted is the behaviour a requester
//! would actually get — a title created, a request rejected with no resolver, a
//! dormant lease claim of the length they asked for.

use super::*;
use scryer_domain::{
    LifecycleClaimKind, LifecycleClaimProducer, LifecycleClaimState, MediaRequestStatus,
    RequestDecisionOutcome, RequestRuleEvaluationMode,
};
use scryer_rules::request::{RequestRuleDecision, RequestVote};

use crate::request_rules::arbitration::{
    FALLBACK_ERROR, FALLBACK_HELD, FALLBACK_NO_RULE_MATCHED, FALLBACK_RULE_MANUAL,
    LIBRARY_PERMISSION_DECIDER, ScopedError, ScopedVote, arbitrate,
};
use crate::request_rules::{
    RequestRuleDraft, RequestRuleGatesUpdate, RequestRulePreviewMatcher, RequestRulePreviewRequest,
    RequestRuleSample,
};

// ── Matchers used throughout ────────────────────────────────────────────────

/// Approves anything. The simplest possible enforce rule.
const APPROVE_EVERYTHING: &str = r#"package rules
import rego.v1

approve if {
	input.request.origin == "manual"
}

tags contains "auto-approved" if {
	input.request.origin == "manual"
}
"#;

/// Denies anything, with a reason code an approver can read.
const DENY_EVERYTHING: &str = r#"package rules
import rego.v1

deny if {
	input.request.origin == "manual"
}

reasons contains "policy_denied" if {
	input.request.origin == "manual"
}
"#;

/// Approves only a finite lease of at most 14 days, so the same rule can be
/// shown to abstain on a forever request.
const APPROVE_SHORT_LEASE: &str = r#"package rules
import rego.v1

approve if {
	not input.request.lease_forever
	input.request.lease_days <= 14
}
"#;

/// Reads a person fact, so authoring it needs `manage_permissions`.
const PERSON_TARGETED: &str = r#"package rules
import rego.v1

approve if {
	input.requester.username == "operator"
}
"#;

// ── Arbitration (pure) ──────────────────────────────────────────────────────

fn vote(id: &str, vote: RequestVote, held: bool, tags: &[&str]) -> ScopedVote {
    ScopedVote {
        rule_set_id: id.to_string(),
        rule_set_name: format!("rule {id}"),
        revision_number: 1,
        content_hash: format!("hash-{id}"),
        mode: RequestRuleEvaluationMode::Enforce,
        decision: RequestRuleDecision {
            vote,
            reason_codes: vec![format!("{id}_reason")],
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            held,
        },
    }
}

fn error(id: &str) -> ScopedError {
    ScopedError {
        rule_set_id: id.to_string(),
        rule_set_name: format!("rule {id}"),
        revision_number: 1,
        content_hash: format!("hash-{id}"),
        mode: RequestRuleEvaluationMode::Enforce,
        message: "timed out".to_string(),
    }
}

#[test]
fn arbitration_follows_deny_then_manual_then_approve() {
    struct Case {
        name: &'static str,
        votes: Vec<ScopedVote>,
        errors: Vec<ScopedError>,
        permission: bool,
        outcome: RequestDecisionOutcome,
        fallback: Option<&'static str>,
        deciders: Vec<&'static str>,
    }

    let cases = vec![
        Case {
            name: "a deny beats an approve",
            votes: vec![
                vote("a", RequestVote::Approve, false, &[]),
                vote("d", RequestVote::Deny, false, &[]),
            ],
            errors: Vec::new(),
            permission: true,
            outcome: RequestDecisionOutcome::Deny,
            fallback: None,
            deciders: vec!["d"],
        },
        Case {
            name: "a deny beats a manual",
            votes: vec![
                vote("m", RequestVote::Manual, false, &[]),
                vote("d", RequestVote::Deny, false, &[]),
            ],
            errors: Vec::new(),
            permission: false,
            outcome: RequestDecisionOutcome::Deny,
            fallback: None,
            deciders: vec!["d"],
        },
        Case {
            name: "an authored manual beats an approve",
            votes: vec![
                vote("a", RequestVote::Approve, false, &[]),
                vote("m", RequestVote::Manual, false, &[]),
            ],
            errors: Vec::new(),
            permission: true,
            outcome: RequestDecisionOutcome::ManualReview,
            fallback: Some(FALLBACK_RULE_MANUAL),
            deciders: vec!["m"],
        },
        Case {
            name: "a held rule counts as manual",
            votes: vec![
                vote("a", RequestVote::Approve, false, &[]),
                vote("h", RequestVote::Manual, true, &[]),
            ],
            errors: Vec::new(),
            permission: true,
            outcome: RequestDecisionOutcome::ManualReview,
            fallback: Some(FALLBACK_HELD),
            deciders: vec!["h"],
        },
        Case {
            name: "an errored rule counts as manual",
            votes: vec![vote("a", RequestVote::Approve, false, &[])],
            errors: vec![error("e")],
            permission: true,
            outcome: RequestDecisionOutcome::ManualReview,
            fallback: Some(FALLBACK_ERROR),
            deciders: vec!["e"],
        },
        Case {
            name: "an approve wins when nothing objects",
            votes: vec![
                vote("a", RequestVote::Approve, false, &[]),
                vote("x", RequestVote::Abstain, false, &[]),
            ],
            errors: Vec::new(),
            permission: false,
            outcome: RequestDecisionOutcome::AutoApprove,
            fallback: None,
            deciders: vec!["a"],
        },
        Case {
            name: "the permission vote approves only when nothing else votes",
            votes: vec![vote("x", RequestVote::Abstain, false, &[])],
            errors: Vec::new(),
            permission: true,
            outcome: RequestDecisionOutcome::AutoApprove,
            fallback: None,
            deciders: vec![LIBRARY_PERMISSION_DECIDER],
        },
        Case {
            name: "no votes and no permission is no rule matched",
            votes: Vec::new(),
            errors: Vec::new(),
            permission: false,
            outcome: RequestDecisionOutcome::ManualReview,
            fallback: Some(FALLBACK_NO_RULE_MATCHED),
            deciders: Vec::new(),
        },
        Case {
            name: "the permission vote loses to a deny",
            votes: vec![vote("d", RequestVote::Deny, false, &[])],
            errors: Vec::new(),
            permission: true,
            outcome: RequestDecisionOutcome::Deny,
            fallback: None,
            deciders: vec!["d"],
        },
        Case {
            name: "the permission vote loses to a manual",
            votes: vec![vote("m", RequestVote::Manual, false, &[])],
            errors: Vec::new(),
            permission: true,
            outcome: RequestDecisionOutcome::ManualReview,
            fallback: Some(FALLBACK_RULE_MANUAL),
            deciders: vec!["m"],
        },
        Case {
            name: "the permission vote loses to an error",
            votes: Vec::new(),
            errors: vec![error("e")],
            permission: true,
            outcome: RequestDecisionOutcome::ManualReview,
            fallback: Some(FALLBACK_ERROR),
            deciders: vec!["e"],
        },
    ];

    for case in cases {
        let arbitration = arbitrate(&case.votes, &case.errors, case.permission);
        assert_eq!(
            arbitration.policy_outcome, case.outcome,
            "outcome for '{}'",
            case.name
        );
        assert_eq!(
            arbitration.fallback_reason, case.fallback,
            "fallback for '{}'",
            case.name
        );
        assert_eq!(
            arbitration.deciding_rule_set_ids,
            case.deciders
                .iter()
                .map(|id| (*id).to_string())
                .collect::<Vec<_>>(),
            "deciders for '{}'",
            case.name
        );
    }
}

#[test]
fn tags_are_collected_from_every_rule_that_ran_in_first_appearance_order() {
    let votes = vec![
        vote("a", RequestVote::Abstain, false, &["kids", "family"]),
        vote("d", RequestVote::Deny, false, &["family", "flagged"]),
    ];
    let arbitration = arbitrate(&votes, &[], false);

    // Denied, and the tags of the *abstaining* rule are still collected: "this
    // is a kids' film" is true whether or not anyone approved it. Applying them
    // is the caller's decision, and a denial never does.
    assert_eq!(arbitration.policy_outcome, RequestDecisionOutcome::Deny);
    assert_eq!(arbitration.tags, vec!["kids", "family", "flagged"]);
}

#[test]
fn a_shadow_verdict_is_not_enforceable_and_falls_back_to_the_permission() {
    let mut shadow = vote("s", RequestVote::Approve, false, &[]);
    shadow.mode = RequestRuleEvaluationMode::Shadow;
    let votes = vec![shadow];
    let arbitration = arbitrate(&votes, &[], false);

    assert_eq!(
        arbitration.policy_outcome,
        RequestDecisionOutcome::AutoApprove
    );
    assert!(!arbitration.is_enforceable(&votes, &[]));
    assert_eq!(
        arbitration.effective_outcome(true, false, false),
        RequestDecisionOutcome::ManualReview
    );
    // The same shadow verdict on a library whose requester holds Auto-Approve
    // still auto-approves, because that is what happened before rules existed.
    assert_eq!(
        arbitration.effective_outcome(true, false, true),
        RequestDecisionOutcome::AutoApprove
    );
}

#[test]
fn the_gate_being_off_suspends_an_enforceable_verdict() {
    let votes = vec![vote("d", RequestVote::Deny, false, &[])];
    let arbitration = arbitrate(&votes, &[], false);

    assert!(arbitration.is_enforceable(&votes, &[]));
    assert_eq!(
        arbitration.effective_outcome(false, true, false),
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(
        arbitration.effective_outcome(true, true, false),
        RequestDecisionOutcome::Deny
    );
}

// ── Authoring ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_new_rule_set_is_stored_disabled_and_loads_nothing() {
    let harness = bootstrap_media_request_app();
    let detail = harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: "Approve everything".to_string(),
                description: "test".to_string(),
                rego_source: APPROVE_EVERYTHING.to_string(),
                library_ids: Vec::new(),
            },
        )
        .await
        .expect("rule set should be created");

    assert!(!detail.rule_set.enabled);
    assert_eq!(
        detail.rule_set.evaluation_mode,
        RequestRuleEvaluationMode::Disabled
    );
    assert_eq!(detail.revision.revision_number, 1);
    assert!(!detail.revision.matcher_content_hash.is_empty());
    // A disabled rule is not compiled at all, so the engine is still empty.
    assert!(harness.app.request_rules_engine_snapshot().is_empty());
}

#[tokio::test]
async fn arming_a_rule_set_rebuilds_the_engine() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;

    harness
        .app
        .set_request_rule_mode(
            &harness.manager,
            &detail.rule_set.id,
            RequestRuleEvaluationMode::Enforce,
        )
        .await
        .expect("mode change should succeed");

    let snapshot = harness.app.request_rules_engine_snapshot();
    assert!(!snapshot.is_empty());
    let scope = snapshot
        .scopes
        .get(&detail.rule_set.id)
        .expect("the armed rule is loaded");
    assert_eq!(scope.mode, RequestRuleEvaluationMode::Enforce);
    assert_eq!(scope.revision_number, 1);

    // …and disarming it takes it back out.
    harness
        .app
        .set_request_rule_mode(
            &harness.manager,
            &detail.rule_set.id,
            RequestRuleEvaluationMode::Disabled,
        )
        .await
        .expect("mode change should succeed");
    assert!(harness.app.request_rules_engine_snapshot().is_empty());
}

#[tokio::test]
async fn a_person_targeting_matcher_needs_permission_authority() {
    let harness = bootstrap_media_request_app();

    // The requester holds `manage_catalog_settings` but not `manage_permissions`.
    let refusal = harness
        .app
        .create_request_rule_set(
            &harness.user,
            RequestRuleDraft {
                name: "Named requesters".to_string(),
                description: String::new(),
                rego_source: PERSON_TARGETED.to_string(),
                library_ids: Vec::new(),
            },
        )
        .await
        .expect_err("a person-targeting rule needs permission authority");
    assert!(
        matches!(refusal, AppError::Unauthorized(ref message)
            if message.contains("input.requester.username")),
        "unexpected refusal: {refusal:?}"
    );

    // The same matcher is accepted from an actor who can manage permissions…
    harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: "Named requesters".to_string(),
                description: String::new(),
                rego_source: PERSON_TARGETED.to_string(),
                library_ids: Vec::new(),
            },
        )
        .await
        .expect("permission authority may author a person-targeting rule");

    // …and a content-only matcher needs nothing beyond catalog settings.
    harness
        .app
        .create_request_rule_set(
            &harness.user,
            RequestRuleDraft {
                name: "Deny everything".to_string(),
                description: String::new(),
                rego_source: DENY_EVERYTHING.to_string(),
                library_ids: Vec::new(),
            },
        )
        .await
        .expect("a content-only rule needs only catalog authority");
}

#[tokio::test]
async fn an_invalid_matcher_is_refused_with_the_reference_wording() {
    let harness = bootstrap_media_request_app();
    let error = harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: "Broken".to_string(),
                description: String::new(),
                rego_source:
                    "package rules\nimport rego.v1\n\napprove if {\n\tinput.facts.not_a_fact\n}\n"
                        .to_string(),
                library_ids: Vec::new(),
            },
        )
        .await
        .expect_err("an unknown fact path must not be stored");
    let message = error.to_string();
    assert!(
        message.contains("Unknown rule input path") && message.contains("Rules Context Reference"),
        "unexpected message: {message}"
    );
}

#[tokio::test]
async fn editing_a_matcher_appends_a_revision_and_leaves_its_predecessor_alone() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;

    let updated = harness
        .app
        .update_request_rule_matcher(&harness.manager, &detail.rule_set.id, DENY_EVERYTHING)
        .await
        .expect("matcher edit should succeed");
    assert_eq!(updated.revision.revision_number, 2);
    assert_eq!(updated.rule_set.current_revision_number, 2);

    let revisions = harness
        .app
        .list_request_rule_revisions(&harness.manager, &detail.rule_set.id)
        .await
        .expect("revisions should list");
    assert_eq!(revisions.len(), 2);
    let first = revisions
        .iter()
        .find(|revision| revision.revision_number == 1)
        .expect("revision 1 survives");
    assert_eq!(
        first.matcher_content_hash,
        detail.revision.matcher_content_hash
    );
}

#[tokio::test]
async fn deleting_a_rule_set_keeps_the_decisions_it_made() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9040, None).await;
    assert_eq!(harness.request_rule_decisions.recorded().await.len(), 1);

    harness
        .app
        .delete_request_rule_set(&harness.manager, &detail.rule_set.id)
        .await
        .expect("delete should succeed");

    // The trace outlives the rule (FR-016) and the engine no longer holds it.
    assert_eq!(harness.request_rule_decisions.recorded().await.len(), 1);
    assert!(harness.app.request_rules_engine_snapshot().is_empty());
}

#[tokio::test]
async fn previewing_reports_the_vote_the_tags_and_the_rendered_input() {
    let harness = bootstrap_media_request_app();
    harness.users.store.lock().await.push(harness.user.clone());
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let stored = harness
        .app
        .preview_request_rule(
            &harness.manager,
            RequestRulePreviewRequest {
                matcher: RequestRulePreviewMatcher::Stored {
                    rule_set_id: detail.rule_set.id.clone(),
                },
                sample: sample(&harness, &library_id, 9041),
            },
        )
        .await
        .expect("stored preview should evaluate");
    assert_eq!(stored.vote, Some(RequestVote::Approve));
    assert!(!stored.held);
    assert_eq!(stored.tags, vec!["auto-approved"]);
    assert!(stored.error.is_none());
    assert!(stored.input_json.contains("\"observations\""));
    assert_eq!(stored.rule_set_id, detail.rule_set.id);

    // An inline draft is compiled and evaluated without ever being stored.
    let inline = harness
        .app
        .preview_request_rule(
            &harness.manager,
            RequestRulePreviewRequest {
                matcher: RequestRulePreviewMatcher::Inline {
                    rego_source: DENY_EVERYTHING.to_string(),
                },
                sample: sample(&harness, &library_id, 9041),
            },
        )
        .await
        .expect("inline preview should evaluate");
    assert_eq!(inline.vote, Some(RequestVote::Deny));
    assert_eq!(
        inline
            .reasons
            .iter()
            .map(|reason| reason.code.as_str())
            .collect::<Vec<_>>(),
        vec!["policy_denied"]
    );
    assert_eq!(
        harness
            .app
            .list_request_rule_sets(&harness.manager)
            .await
            .expect("list")
            .len(),
        1,
        "an inline preview must not store a rule set"
    );
}

#[tokio::test]
async fn validating_a_source_reports_errors_without_storing_anything() {
    let harness = bootstrap_media_request_app();
    let result = harness
        .app
        .validate_request_rule_source(&harness.manager, APPROVE_EVERYTHING)
        .await
        .expect("validation should run");
    assert!(result.valid, "unexpected errors: {:?}", result.errors);

    let broken = harness
        .app
        .validate_request_rule_source(
            &harness.manager,
            "package rules\nimport rego.v1\n\nreasons contains \"x\" if { true }\n",
        )
        .await
        .expect("validation should run");
    assert!(!broken.valid);
    assert!(
        broken
            .errors
            .iter()
            .any(|error| error.contains("can never vote")),
        "unexpected errors: {:?}",
        broken.errors
    );
    assert!(
        harness
            .app
            .list_request_rule_sets(&harness.manager)
            .await
            .expect("list")
            .is_empty()
    );
}

// ── The gate ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_evaluation_gate_defaults_off_and_needs_system_settings_authority() {
    let harness = bootstrap_media_request_app();
    let gates = harness
        .app
        .request_rule_instance_gates(&harness.manager)
        .await
        .expect("gates should read");
    assert!(!gates.evaluation_enabled);

    let refusal = harness
        .app
        .set_request_rule_instance_gates(
            &harness.user,
            RequestRuleGatesUpdate {
                evaluation_enabled: Some(true),
            },
        )
        .await
        .expect_err("catalog settings authority is not system settings authority");
    assert!(matches!(refusal, AppError::Unauthorized(_)));

    let gates = harness
        .app
        .set_request_rule_instance_gates(
            &harness.manager,
            RequestRuleGatesUpdate {
                evaluation_enabled: Some(true),
            },
        )
        .await
        .expect("an administrator may arm the gate");
    assert!(gates.evaluation_enabled);
}

// ── The flow ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_unauthorized_requester_is_refused_before_any_rule_runs() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let mut stranger = test_admin_user();
    stranger.id = "stranger".to_string();
    stranger.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::NONE,
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let error = harness
        .app
        .submit_media_request(&stranger, media_request_input(library_id, 9042))
        .await
        .expect_err("a requester without Request permission is refused");
    assert!(matches!(error, AppError::Unauthorized(_)));
    assert!(
        harness.request_rule_decisions.recorded().await.is_empty(),
        "no rule may run for a request that was never allowed"
    );
}

#[tokio::test]
async fn with_the_gate_off_the_verdict_is_recorded_but_nothing_acts_on_it() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9043, None).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());

    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(
        traces.len(),
        1,
        "the trace is written even with the gate off"
    );
    assert_eq!(
        traces[0].policy_outcome,
        RequestDecisionOutcome::AutoApprove
    );
    assert_eq!(
        traces[0].effective_outcome,
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(traces[0].tags, vec!["auto-approved"]);
}

#[tokio::test]
async fn a_shadow_rule_records_its_verdict_and_changes_nothing() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Shadow,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9044, None).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    drop(requests);

    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(traces[0].policy_outcome, RequestDecisionOutcome::Deny);
    assert_eq!(
        traces[0].effective_outcome,
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(traces[0].mode, RequestRuleEvaluationMode::Shadow);
}

#[tokio::test]
async fn an_enforced_approval_creates_the_title_with_the_policy_tags_and_a_dormant_lease() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .create_title_tag_definition(&harness.manager, "auto-approved", None)
        .await
        .expect("the policy tag is defined in the registry");
    let request_id = submit(&harness, &library_id, 9045, Some(30)).await;

    let titles = harness.titles.store.lock().await;
    assert_eq!(titles.len(), 1, "the enforced approval created the title");
    let title = &titles[0];
    assert!(
        title.tags.iter().any(|tag| tag == "auto-approved"),
        "policy tags land on the title as plain labels: {:?}",
        title.tags
    );
    assert!(title.tags.iter().any(|tag| tag.starts_with("scryer:")));
    let title_id = title.id.clone();
    drop(titles);

    let requests = harness.media_requests.requests.lock().await;
    let request = &requests[0];
    assert_eq!(request.status, MediaRequestStatus::Approved);
    assert_eq!(request.approved_lease_days, Some(30));
    assert_eq!(
        request.decided_by_rule_set_ids,
        vec![detail.rule_set.id.clone()]
    );
    assert_eq!(request.policy_tags, vec!["auto-approved"]);
    assert!(request.decision_id.is_some());
    drop(requests);

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims.len(), 1);
    let claim = &claims[0];
    assert_eq!(claim.producer, LifecycleClaimProducer::RequestLease);
    assert_eq!(claim.kind, LifecycleClaimKind::RetainUntil);
    // Dormant: the window starts at the title's first import, not the approval.
    assert_eq!(claim.state, LifecycleClaimState::Dormant);
    assert_eq!(claim.duration_days, Some(30));
    assert_eq!(claim.producer_ref.as_deref(), Some(request_id.as_str()));
    assert_eq!(claim.title_id, title_id);
    assert_eq!(claim.library_id, library_id);
}

#[tokio::test]
async fn a_forever_request_becomes_an_active_keep_claim() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9046, None).await;

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].producer, LifecycleClaimProducer::RequestPermanent);
    assert_eq!(claims[0].kind, LifecycleClaimKind::Keep);
    assert_eq!(claims[0].state, LifecycleClaimState::Active);
    assert_eq!(claims[0].duration_days, None);
}

#[tokio::test]
async fn an_enforced_denial_rejects_the_request_with_no_human_resolver() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, Some(7)).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = &requests[0];
    assert_eq!(request.status, MediaRequestStatus::Rejected);
    assert_eq!(
        request.resolved_by_user_id, None,
        "a policy denial has no human resolver"
    );
    assert_eq!(
        request.decided_by_rule_set_ids,
        vec![detail.rule_set.id.clone()]
    );
    drop(requests);

    assert!(
        harness.titles.store.lock().await.is_empty(),
        "a denial creates no title"
    );
    assert!(
        harness.lifecycle_claims.all().await.is_empty(),
        "a denial grants no lease"
    );

    let events = harness.domain_events.events.lock().await;
    let rejected = events
        .iter()
        .find_map(|event| match &event.payload {
            scryer_domain::DomainEventPayload::MediaRequestRejected(data) => Some(data.clone()),
            _ => None,
        })
        .expect("a rejection event is published");
    assert_eq!(rejected.decided_by_rule_set_ids, vec![detail.rule_set.id]);
    assert_eq!(rejected.decision_reason_codes, vec!["policy_denied"]);
}

#[tokio::test]
async fn a_held_rule_never_approves() {
    let harness = bootstrap_media_request_app();
    // Reads a fact the claim store answers. With that store down the fact is
    // *unknown*, and the engine holds any rule that reads an unknown fact
    // before it ever runs.
    let detail = create_rule(
        &harness,
        "Lease quota",
        "package rules\nimport rego.v1\n\napprove if {\n\tinput.facts.active_lease_count < 5\n}\n",
    )
    .await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;
    harness.lifecycle_claims.set_unreadable(true);

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9048, None).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(
        requests[0].status,
        MediaRequestStatus::Pending,
        "an unobservable fact must never read as an approval (FR-012)"
    );
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());

    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(
        traces[0].policy_outcome,
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(traces[0].fallback_reason.as_deref(), Some(FALLBACK_HELD));
    let votes: serde_json::Value =
        serde_json::from_str(&traces[0].votes_json).expect("votes are JSON");
    assert_eq!(votes[0]["vote"], "manual");
    assert_eq!(votes[0]["held"], true);
    assert_eq!(votes[0]["rule_set_id"], detail.rule_set.id);
    assert_eq!(votes[0]["mode"], "enforce");
    assert!(
        votes[0].get("error").is_none(),
        "a held rule is not an errored rule"
    );
}

#[tokio::test]
async fn a_metadata_outage_holds_the_rules_that_read_metadata() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(
        &harness,
        "Family ratings only",
        "package rules\nimport rego.v1\n\napprove if {\n\tinput.facts.certification_rank <= 1\n}\n",
    )
    .await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    // A subject SMG cannot identify: the snapshot is explicitly partial, so
    // every metadata fact is unknown rather than empty.
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 8_901);
    input.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "8901".to_string(),
    }];

    let preflight = harness
        .app
        .preview_my_request_decision(&harness.user, input.clone())
        .await
        .expect("a metadata outage must never fail the preview");
    assert!(preflight.metadata_partial);
    assert_eq!(preflight.outcome, RequestDecisionOutcome::ManualReview);

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("a metadata outage must never fail the submission");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());
}

#[tokio::test]
async fn a_library_scoped_rule_does_not_vote_on_another_librarys_request() {
    let harness = bootstrap_media_request_app();
    let series_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let detail = harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: "Series only".to_string(),
                description: String::new(),
                rego_source: APPROVE_EVERYTHING.to_string(),
                library_ids: vec![series_library],
            },
        )
        .await
        .expect("rule set should be created");
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let movie_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &movie_library, 9050, None).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(
        requests[0].status,
        MediaRequestStatus::Pending,
        "an out-of-scope rule must not approve"
    );
    drop(requests);

    // No rule could speak, so the evaluation short-circuits to the legacy
    // answer — and the trace still says so, because the gate is armed.
    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].votes_json, "[]");
    assert_eq!(traces[0].mode, RequestRuleEvaluationMode::Disabled);
}

#[tokio::test]
async fn editing_a_pending_request_re_evaluates_it() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Short leases only", APPROVE_SHORT_LEASE).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    // A forever request carries no lease_days, so the rule abstains and the
    // request waits for a human.
    let request_id = submit(&harness, &library_id, 9051, None).await;
    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Pending
    );

    harness
        .app
        .update_my_media_request(
            &harness.user,
            UpdateMediaRequestInput {
                request_id: request_id.clone(),
                requested_quality_profile_id: "1080p".to_string(),
                requested_monitor_type: None,
                requested_monitor_selection: None,
                requested_lease_days: Some(7),
            },
        )
        .await
        .expect("the requester may shorten their lease");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(
        requests[0].status,
        MediaRequestStatus::Approved,
        "a request that now matches an approve rule is approved on edit"
    );
    assert_eq!(requests[0].approved_lease_days, Some(7));
    drop(requests);

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].duration_days, Some(7));
    // Two evaluations: the submit and the edit.
    assert_eq!(harness.request_rule_decisions.recorded().await.len(), 2);
}

#[tokio::test]
async fn preflight_and_submit_agree_and_read_the_gateway_once() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    harness
        .app
        .create_title_tag_definition(&harness.manager, "auto-approved", None)
        .await
        .expect("the policy tag is defined in the registry");

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9052);
    input.requested_lease_days = Some(21);

    let preflight = harness
        .app
        .preview_my_request_decision(&harness.user, input.clone())
        .await
        .expect("pre-flight should answer");
    assert_eq!(preflight.outcome, RequestDecisionOutcome::AutoApprove);
    assert_eq!(preflight.tags, vec!["auto-approved"]);
    assert!(!preflight.metadata_partial);
    assert_eq!(
        preflight.evaluation_mode,
        RequestRuleEvaluationMode::Enforce
    );

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("submit should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Approved);
    assert_eq!(requests[0].policy_tags, preflight.tags);
    drop(requests);

    // The pre-flight trace is inspectable but cannot be mistaken for a decision
    // about a submitted request.
    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(traces.len(), 2);
    assert!(traces.iter().any(|trace| {
        trace
            .request_id
            .starts_with(crate::request_rules::PREFLIGHT_REQUEST_ID_PREFIX)
    }));
}

#[tokio::test]
async fn preflight_requires_the_request_permission() {
    let harness = bootstrap_media_request_app();
    let mut stranger = test_admin_user();
    stranger.id = "stranger".to_string();
    stranger.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::NONE,
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let error = harness
        .app
        .preview_my_request_decision(&stranger, media_request_input(library_id, 9053))
        .await
        .expect_err("a preview is not an oracle for a library you may not ask about");
    assert!(matches!(error, AppError::Unauthorized(_)));
}

// ── Human approval, cancellation, and administrator claim operations ─────────

#[tokio::test]
async fn a_human_approval_honours_a_lease_and_tag_override() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9054, Some(90)).await;
    harness
        .app
        .create_title_tag_definition(&harness.manager, "approver tag", None)
        .await
        .expect("the approver's tag is defined in the registry");

    let outcome = harness
        .app
        .approve_media_request(
            &harness.manager,
            &request_id,
            "1080p",
            None,
            None,
            Some(Some(14)),
            Some(vec!["approver tag".to_string()]),
        )
        .await
        .expect("approval should succeed");
    assert!(outcome.claim_error.is_none());

    let titles = harness.titles.store.lock().await;
    assert!(titles[0].tags.iter().any(|tag| tag == "approver tag"));
    drop(titles);

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].approved_lease_days, Some(14));
    assert_eq!(requests[0].policy_tags, vec!["approver tag"]);
    drop(requests);

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].duration_days, Some(14));
    assert_eq!(claims[0].state, LifecycleClaimState::Dormant);
}

#[tokio::test]
async fn an_approver_may_not_mint_a_reserved_tag() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9055, None).await;

    let error = harness
        .app
        .approve_media_request(
            &harness.manager,
            &request_id,
            "1080p",
            None,
            None,
            None,
            Some(vec!["scryer:quality-profile:4k".to_string()]),
        )
        .await
        .expect_err("the reserved prefix is refused");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("scryer:")),
        "unexpected error: {error:?}"
    );
}

/// A rule may name any label within the family's bounds, but titles only
/// carry labels an administrator defined. The undefined one is dropped; the
/// approval it rode on still lands, with the defined labels.
#[tokio::test]
async fn an_undefined_policy_tag_is_dropped_without_failing_the_approval() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    arm(
        &harness,
        &detail.rule_set.id,
        RequestRuleEvaluationMode::Enforce,
    )
    .await;
    enable_gate(&harness).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let preflight = harness
        .app
        .preview_my_request_decision(&harness.user, media_request_input(library_id.clone(), 9057))
        .await
        .expect("pre-flight should answer");
    assert!(
        preflight.tags.is_empty(),
        "the banner shows what would land, not what the rule asked: {:?}",
        preflight.tags
    );
    let request_id = submit(&harness, &library_id, 9057, None).await;

    let titles = harness.titles.store.lock().await;
    assert_eq!(titles.len(), 1, "the approval still created the title");
    assert!(
        !titles[0].tags.iter().any(|tag| tag == "auto-approved"),
        "an undefined label never reaches the title: {:?}",
        titles[0].tags
    );
    drop(titles);

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Approved);
    assert!(
        requests[0].policy_tags.is_empty(),
        "the request records what was applied: {:?}",
        requests[0].policy_tags
    );
    drop(requests);

    // The trace still says what the rule asked for, so an operator can see
    // the label they have not defined yet.
    let traces = harness.request_rule_decisions.recorded().await;
    let trace = traces
        .iter()
        .find(|trace| trace.request_id == request_id)
        .expect("the decision was traced");
    assert_eq!(trace.tags, vec!["auto-approved"]);
}

/// An approver's own list is held to the same bar as the tag editor.
#[tokio::test]
async fn an_approver_may_not_apply_an_undefined_tag() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9058, None).await;

    let error = harness
        .app
        .approve_media_request(
            &harness.manager,
            &request_id,
            "1080p",
            None,
            None,
            None,
            Some(vec!["nobody defined this".to_string()]),
        )
        .await
        .expect_err("an undefined label is refused, not silently dropped");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("not a defined tag")),
        "unexpected error: {error:?}"
    );
    assert!(
        harness.titles.store.lock().await.is_empty(),
        "nothing was created"
    );
}

#[tokio::test]
async fn an_out_of_range_lease_is_refused_at_submit() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9056);
    input.requested_lease_days = Some(4000);

    let error = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect_err("a ten-year cap applies to requested leases");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("at most")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn every_overlapping_pending_request_gets_its_own_claim() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let first = submit(&harness, &library_id, 9057, Some(30)).await;
    let second = submit(&harness, &library_id, 9057, Some(5)).await;

    harness
        .app
        .approve_media_request(&harness.manager, &first, "1080p", None, None, None, None)
        .await
        .expect("approval should succeed");

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims.len(), 2, "each requester keeps their own window");
    let first_claim = claims
        .iter()
        .find(|claim| claim.producer_ref.as_deref() == Some(first.as_str()))
        .expect("the approved request has a claim");
    let second_claim = claims
        .iter()
        .find(|claim| claim.producer_ref.as_deref() == Some(second.as_str()))
        .expect("the overlapping request has a claim");
    assert_eq!(first_claim.duration_days, Some(30));
    assert_eq!(second_claim.duration_days, Some(5));
}

#[tokio::test]
async fn canceling_a_request_releases_the_claim_it_produced() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9058, Some(30)).await;
    // A claim only exists once a request is approved, and an approved request
    // is no longer cancellable — so the claim is seeded here to exercise the
    // release wiring itself, which is the safety net for every path that would
    // otherwise leave a hold behind.
    harness
        .lifecycle_claims
        .seed(seeded_claim(&request_id, &library_id))
        .await;

    harness
        .app
        .cancel_my_media_request(&harness.user, &request_id)
        .await
        .expect("the requester may cancel");

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims[0].state, LifecycleClaimState::Released);
    assert_eq!(
        claims[0].released_reason.as_deref(),
        Some(crate::media_requests::CLAIM_RELEASE_REQUEST_CANCELED)
    );
}

#[tokio::test]
async fn dismissing_a_request_releases_the_claim_it_produced() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9059, Some(30)).await;
    harness
        .lifecycle_claims
        .seed(seeded_claim(&request_id, &library_id))
        .await;

    harness
        .app
        .dismiss_media_request(&harness.manager, &request_id)
        .await
        .expect("a title manager may dismiss");

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims[0].state, LifecycleClaimState::Released);
    assert_eq!(
        claims[0].released_reason.as_deref(),
        Some(crate::media_requests::CLAIM_RELEASE_REQUEST_REJECTED)
    );
}

#[tokio::test]
async fn a_pending_request_that_is_canceled_releases_nothing_and_does_not_fail() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9060, Some(30)).await;

    harness
        .app
        .cancel_my_media_request(&harness.user, &request_id)
        .await
        .expect("the requester may cancel");

    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Canceled
    );
    assert!(harness.lifecycle_claims.all().await.is_empty());
}

#[tokio::test]
async fn administrator_claim_operations_require_manage_titles() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9061, Some(30)).await;
    harness
        .app
        .approve_media_request(
            &harness.manager,
            &request_id,
            "1080p",
            None,
            None,
            None,
            None,
        )
        .await
        .expect("approval should succeed");
    let claim_id = harness.lifecycle_claims.all().await[0].id.clone();
    let title_id = harness.titles.store.lock().await[0].id.clone();

    // The requester holds `request`, never `manage_titles`.
    for error in [
        harness
            .app
            .list_title_claims(&harness.user, &title_id)
            .await
            .err(),
        harness
            .app
            .extend_title_claim(&harness.user, &claim_id, chrono::Utc::now())
            .await
            .err(),
        harness
            .app
            .convert_title_claim_to_permanent(&harness.user, &claim_id)
            .await
            .err(),
        harness
            .app
            .release_title_claim(&harness.user, &claim_id, "no")
            .await
            .err(),
    ] {
        assert!(
            matches!(error, Some(AppError::Unauthorized(_))),
            "unexpected result: {error:?}"
        );
    }

    let claims = harness
        .app
        .list_title_claims(&harness.manager, &title_id)
        .await
        .expect("a title manager may read the claims");
    assert_eq!(claims.len(), 1);
}

#[tokio::test]
async fn an_administrator_can_extend_convert_and_release_a_claim() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let first = submit(&harness, &library_id, 9062, Some(30)).await;
    harness
        .app
        .approve_media_request(&harness.manager, &first, "1080p", None, None, None, None)
        .await
        .expect("approval should succeed");
    let claim_id = harness.lifecycle_claims.all().await[0].id.clone();

    // Extend needs a live claim; the dormant one qualifies.
    let expires_at = chrono::Utc::now() + chrono::Duration::days(60);
    let extended = harness
        .app
        .extend_title_claim(&harness.manager, &claim_id, expires_at)
        .await
        .expect("extend should succeed");
    assert_eq!(extended.expires_at, Some(expires_at));

    // Converting leaves the original as history and produces an operator keep.
    let replacement = harness
        .app
        .convert_title_claim_to_permanent(&harness.manager, &claim_id)
        .await
        .expect("convert should succeed");
    assert_eq!(replacement.producer, LifecycleClaimProducer::OperatorKeep);
    assert_eq!(replacement.kind, LifecycleClaimKind::Keep);
    assert_eq!(replacement.state, LifecycleClaimState::Active);
    assert_eq!(replacement.producer_ref, None);
    let original = harness
        .lifecycle_claims
        .all()
        .await
        .into_iter()
        .find(|claim| claim.id == claim_id)
        .expect("the original claim is kept as history");
    assert_eq!(original.state, LifecycleClaimState::Converted);

    let released = harness
        .app
        .release_title_claim(
            &harness.manager,
            &replacement.id,
            "operator changed their mind",
        )
        .await
        .expect("release should succeed");
    assert_eq!(released.state, LifecycleClaimState::Released);
    assert_eq!(
        released.released_reason.as_deref(),
        Some("operator changed their mind")
    );

    let blank = harness
        .app
        .release_title_claim(&harness.manager, &replacement.id, "   ")
        .await
        .expect_err("a release reason is required");
    assert!(matches!(blank, AppError::Validation(_)));
}

// ── Helpers ─────────────────────────────────────────────────────────────────

async fn create_rule(
    harness: &MediaRequestTestHarness,
    name: &str,
    source: &str,
) -> crate::request_rules::RequestRuleSetDetail {
    harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: name.to_string(),
                description: String::new(),
                rego_source: source.to_string(),
                library_ids: Vec::new(),
            },
        )
        .await
        .expect("rule set should be created")
}

async fn arm(
    harness: &MediaRequestTestHarness,
    rule_set_id: &str,
    mode: RequestRuleEvaluationMode,
) {
    harness
        .app
        .set_request_rule_mode(&harness.manager, rule_set_id, mode)
        .await
        .expect("mode change should succeed");
}

async fn enable_gate(harness: &MediaRequestTestHarness) {
    harness
        .app
        .set_request_rule_instance_gates(
            &harness.manager,
            RequestRuleGatesUpdate {
                evaluation_enabled: Some(true),
            },
        )
        .await
        .expect("the gate should arm");
}

/// Submit as the plain requester — who holds `request` but **not**
/// `auto_approve_requests`, so nothing but a rule can approve them.
async fn submit(
    harness: &MediaRequestTestHarness,
    library_id: &str,
    tvdb_id: i64,
    lease_days: Option<i64>,
) -> String {
    let mut input = media_request_input(library_id.to_string(), tvdb_id);
    input.requested_lease_days = lease_days;
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("submit should succeed")
        .request_id
}

fn seeded_claim(request_id: &str, library_id: &str) -> scryer_domain::LifecycleClaim {
    let now = chrono::Utc::now();
    scryer_domain::LifecycleClaim {
        id: scryer_domain::Id::new().0,
        title_id: "title-1".to_string(),
        library_id: library_id.to_string(),
        producer: LifecycleClaimProducer::RequestLease,
        producer_ref: Some(request_id.to_string()),
        kind: LifecycleClaimKind::RetainUntil,
        state: LifecycleClaimState::Dormant,
        duration_days: Some(30),
        starts_at: None,
        expires_at: None,
        created_by: Some("tester".to_string()),
        created_at: now,
        updated_at: now,
        released_reason: None,
    }
}

fn sample(harness: &MediaRequestTestHarness, library_id: &str, tvdb_id: i64) -> RequestRuleSample {
    RequestRuleSample {
        user_id: harness.user.id.clone(),
        library_id: library_id.to_string(),
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        quality_profile_id: Some("1080p".to_string()),
        monitor_type: None,
        lease_days: Some(Some(14)),
    }
}
