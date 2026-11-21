//! Request rules end to end: authoring, arbitration, and the request flow.
//!
//! The arbitration half is pure and table-driven. The flow half runs real
//! submissions through `bootstrap_media_request_app` with the in-memory rule,
//! decision, and claim doubles, so what is asserted is the behaviour a requester
//! would actually get — a title created, a request rejected with no resolver, a
//! dormant lease claim of the length they asked for.

use super::*;

#[path = "request_rules_failure_tests.rs"]
mod failure_tests;
use scryer_domain::{
    LifecycleClaimKind, LifecycleClaimProducer, LifecycleClaimState, MediaRequestStatus,
    RequestDecisionOutcome,
};
use scryer_rules::request::{RequestRuleDecision, RequestVote};
use scryer_rules::request_actions::{RequestActionHead, RequestHeadValue};

use crate::request_rules::arbitration::{
    ALL_FALLBACKS, FALLBACK_ERROR, FALLBACK_HELD, FALLBACK_LIBRARY_PERMISSION,
    FALLBACK_NO_RULE_MATCHED, FALLBACK_RULE_MANUAL, FALLBACK_SHAPING_CONFLICT,
    FALLBACK_SHAPING_INVALID, LIBRARY_PERMISSION_DECIDER, ScopedError, ScopedVote, arbitrate,
};
use crate::request_rules::{
    RequestRuleDraft, RequestRulePreviewRequest, RequestRulePreviewSource, RequestRuleSample,
};

// ── Matchers used throughout ────────────────────────────────────────────────

/// Approves anything. The simplest possible enforce rule.
const APPROVE_EVERYTHING: &str = r#"package rules
import rego.v1

approve if {
	input.request.origin == "manual"
}

add_tags := ["auto-approved"] if {
	input.request.origin == "manual"
}
"#;

/// Sends everything to a human, and asks for two tags — one the registry
/// defines and one it does not.
const MANUAL_WITH_TWO_TAGS: &str = r#"package rules
import rego.v1

manual if {
	input.request.origin == "manual"
}

add_tags := ["auto-approved", "nobody-defined-this"] if {
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
        decision: RequestRuleDecision {
            vote,
            reason_codes: vec![format!("{id}_reason")],
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            shaping: Default::default(),
            notify: None,
            held,
        },
    }
}

fn shaping_vote(
    id: &str,
    vote_value: RequestVote,
    head: RequestActionHead,
    value: RequestHeadValue,
) -> ScopedVote {
    let mut scoped = vote(id, vote_value, false, &[]);
    scoped.decision.shaping.set(head, Some(value));
    scoped
}

fn error(id: &str) -> ScopedError {
    ScopedError {
        rule_set_id: id.to_string(),
        rule_set_name: format!("rule {id}"),
        revision_number: 1,
        content_hash: format!("hash-{id}"),
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
            fallback: Some(FALLBACK_LIBRARY_PERMISSION),
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
fn fallback_codes_are_distinct() {
    let unique: std::collections::BTreeSet<&str> = ALL_FALLBACKS.into_iter().collect();
    assert_eq!(unique.len(), ALL_FALLBACKS.len());
}

#[test]
fn rules_that_agree_on_a_shaping_value_still_approve() {
    let votes = vec![
        shaping_vote(
            "a",
            RequestVote::Approve,
            RequestActionHead::SetLeaseDays,
            RequestHeadValue::Number(30),
        ),
        shaping_vote(
            "b",
            RequestVote::Abstain,
            RequestActionHead::SetLeaseDays,
            RequestHeadValue::Number(30),
        ),
    ];
    let arbitration = arbitrate(&votes, &[], false);
    assert_eq!(
        arbitration.policy_outcome,
        RequestDecisionOutcome::AutoApprove
    );
    assert_eq!(arbitration.fallback_reason, None);
    let shaped = arbitration
        .shaping
        .value(RequestActionHead::SetLeaseDays)
        .expect("agreed value kept");
    assert_eq!(shaped.value, RequestHeadValue::Number(30));
    // The first rule to set the value is the one the trace names.
    assert_eq!(shaped.rule_set_id, "a");
    assert!(arbitration.shaping.conflicts.is_empty());
}

#[test]
fn rules_that_disagree_on_a_shaping_value_need_a_person() {
    let votes = vec![
        shaping_vote(
            "a",
            RequestVote::Approve,
            RequestActionHead::SetQualityProfile,
            RequestHeadValue::Text("HD".to_string()),
        ),
        shaping_vote(
            "b",
            RequestVote::Approve,
            RequestActionHead::SetQualityProfile,
            RequestHeadValue::Text("4K".to_string()),
        ),
        shaping_vote(
            "c",
            RequestVote::Abstain,
            RequestActionHead::SetLeaseDays,
            RequestHeadValue::Number(7),
        ),
    ];
    let arbitration = arbitrate(&votes, &[], true);
    assert_eq!(
        arbitration.policy_outcome,
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(arbitration.fallback_reason, Some(FALLBACK_SHAPING_CONFLICT));
    assert_eq!(arbitration.deciding_rule_set_ids, vec!["a", "b"]);
    assert_eq!(arbitration.shaping.conflicts.len(), 1);
    assert_eq!(
        arbitration.shaping.conflicts[0].head,
        RequestActionHead::SetQualityProfile
    );
    assert!(
        arbitration
            .shaping
            .value(RequestActionHead::SetQualityProfile)
            .is_none()
    );
    // The head nobody disagreed on keeps its value for the approver form.
    assert!(
        arbitration
            .shaping
            .value(RequestActionHead::SetLeaseDays)
            .is_some()
    );
}

#[test]
fn a_shaping_conflict_does_not_override_a_denial() {
    let votes = vec![
        shaping_vote(
            "a",
            RequestVote::Deny,
            RequestActionHead::SetLeaseDays,
            RequestHeadValue::Number(30),
        ),
        shaping_vote(
            "b",
            RequestVote::Abstain,
            RequestActionHead::SetLeaseDays,
            RequestHeadValue::Number(60),
        ),
    ];
    let arbitration = arbitrate(&votes, &[], false);
    assert_eq!(arbitration.policy_outcome, RequestDecisionOutcome::Deny);
    assert_eq!(arbitration.fallback_reason, None);
    assert_eq!(arbitration.deciding_rule_set_ids, vec!["a"]);
}

#[test]
fn a_held_rule_contributes_no_shaping() {
    let mut held = shaping_vote(
        "h",
        RequestVote::Manual,
        RequestActionHead::SetLeaseDays,
        RequestHeadValue::Number(60),
    );
    held.decision.held = true;
    let votes = vec![
        shaping_vote(
            "a",
            RequestVote::Approve,
            RequestActionHead::SetLeaseDays,
            RequestHeadValue::Number(30),
        ),
        held,
    ];
    let arbitration = arbitrate(&votes, &[], false);
    assert_eq!(arbitration.fallback_reason, Some(FALLBACK_HELD));
    assert!(arbitration.shaping.conflicts.is_empty());
    assert_eq!(
        arbitration
            .shaping
            .value(RequestActionHead::SetLeaseDays)
            .map(|shaped| shaped.value.clone()),
        Some(RequestHeadValue::Number(30))
    );
}

#[test]
fn an_invalid_shaping_value_turns_an_approval_into_manual_review() {
    let votes = vec![
        shaping_vote(
            "a",
            RequestVote::Approve,
            RequestActionHead::SetMonitor,
            RequestHeadValue::Text("future_episodes".to_string()),
        ),
        shaping_vote(
            "b",
            RequestVote::Approve,
            RequestActionHead::SetLeaseDays,
            RequestHeadValue::Number(30),
        ),
    ];
    let mut arbitration = arbitrate(&votes, &[], false);
    arbitration.refuse_invalid_shaping(&[RequestActionHead::SetMonitor]);
    assert_eq!(
        arbitration.policy_outcome,
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(arbitration.fallback_reason, Some(FALLBACK_SHAPING_INVALID));
    assert_eq!(arbitration.deciding_rule_set_ids, vec!["a"]);
    assert_eq!(arbitration.shaping.invalid.len(), 1);
    assert!(
        arbitration
            .shaping
            .value(RequestActionHead::SetMonitor)
            .is_none()
    );
}

#[test]
fn notifications_are_collected_once_per_rule_that_ran() {
    let mut first = vote("a", RequestVote::Approve, false, &[]);
    first.decision.notify = Some("Approved for the kids' library".to_string());
    let mut held = vote("h", RequestVote::Manual, true, &[]);
    held.decision.notify = Some("never sent".to_string());
    let arbitration = arbitrate(&[first, held], &[], false);
    let notifications = arbitration.shaping.rule_notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].rule_set_name, "rule a");
    assert_eq!(notifications[0].text, "Approved for the kids' library");
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
                builder_model: None,
            },
        )
        .await
        .expect("rule set should be created");

    assert!(!detail.rule_set.enabled);
    assert_eq!(detail.revision.revision_number, 1);
    assert!(!detail.revision.source_content_hash.is_empty());
    // A disabled rule is not compiled at all, so the engine is still empty.
    assert!(harness.app.request_rules_engine_snapshot().is_empty());
}

#[tokio::test]
async fn enabling_a_rule_set_rebuilds_the_engine() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;

    let enabled = harness
        .app
        .set_request_rule_enabled(&harness.manager, &detail.rule_set.id, true)
        .await
        .expect("enabling should succeed");
    assert!(enabled.enabled);

    let snapshot = harness.app.request_rules_engine_snapshot();
    assert!(!snapshot.is_empty());
    let scope = snapshot
        .scopes
        .get(&detail.rule_set.id)
        .expect("the enabled rule is loaded");
    assert_eq!(scope.revision_number, 1);

    // …and disabling it takes it back out.
    let disabled = harness
        .app
        .set_request_rule_enabled(&harness.manager, &detail.rule_set.id, false)
        .await
        .expect("disabling should succeed");
    assert!(!disabled.enabled);
    assert!(harness.app.request_rules_engine_snapshot().is_empty());
}

#[tokio::test]
async fn the_engine_reads_watchlists_only_while_a_loaded_rule_may_use_them() {
    const READS_WATCHLIST: &str = r#"package rules
import rego.v1

approve if {
	input.facts.on_requester_watchlist == true
}
"#;
    let harness = bootstrap_media_request_app();
    let plain = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    harness
        .app
        .set_request_rule_enabled(&harness.manager, &plain.rule_set.id, true)
        .await
        .expect("enabling should succeed");
    assert!(
        !harness
            .app
            .request_rules_engine_snapshot()
            .fact_reads
            .watchlist
    );

    let watching = create_rule(&harness, "Watchlisted titles", READS_WATCHLIST).await;
    // A disabled rule is not loaded, so it cannot need the read.
    assert!(
        !harness
            .app
            .request_rules_engine_snapshot()
            .fact_reads
            .watchlist
    );
    harness
        .app
        .set_request_rule_enabled(&harness.manager, &watching.rule_set.id, true)
        .await
        .expect("enabling should succeed");
    assert!(
        harness
            .app
            .request_rules_engine_snapshot()
            .fact_reads
            .watchlist
    );
}

#[tokio::test]
async fn the_engine_reads_only_the_fact_families_a_loaded_rule_may_use() {
    use crate::request_rules::facts::RequestFactReads;
    const READS_DEMAND: &str = r#"package rules
import rego.v1

approve if {
	input.facts.other_requester_count > 1
}
"#;
    const READS_LIBRARY_AND_QUALITY: &str = r#"package rules
import rego.v1

approve if {
	input.facts.library_title_count < 100
	input.observations.quality_profile_tiers.status == "known"
}
"#;
    let harness = bootstrap_media_request_app();
    let plain = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    harness
        .app
        .set_request_rule_enabled(&harness.manager, &plain.rule_set.id, true)
        .await
        .expect("enabling should succeed");
    assert_eq!(
        harness.app.request_rules_engine_snapshot().fact_reads,
        RequestFactReads::NONE
    );

    let demand = create_rule(&harness, "Popular titles", READS_DEMAND).await;
    harness
        .app
        .set_request_rule_enabled(&harness.manager, &demand.rule_set.id, true)
        .await
        .expect("enabling should succeed");
    assert_eq!(
        harness.app.request_rules_engine_snapshot().fact_reads,
        RequestFactReads {
            catalog: true,
            ..RequestFactReads::NONE
        }
    );

    // A fact named under observations counts as read too.
    let anything = create_rule(&harness, "Small libraries", READS_LIBRARY_AND_QUALITY).await;
    harness
        .app
        .set_request_rule_enabled(&harness.manager, &anything.rule_set.id, true)
        .await
        .expect("enabling should succeed");
    assert_eq!(
        harness.app.request_rules_engine_snapshot().fact_reads,
        RequestFactReads {
            catalog: true,
            library_title_count: true,
            quality: true,
            ..RequestFactReads::NONE
        }
    );
}

#[tokio::test]
async fn enabling_a_rule_set_needs_catalog_settings_authority() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    let mut viewer = test_admin_user();
    viewer.id = "viewer".to_string();
    viewer.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::NONE,
        loaded: true,
        ..Default::default()
    };

    let refusal = harness
        .app
        .set_request_rule_enabled(&viewer, &detail.rule_set.id, true)
        .await
        .expect_err("enabling a rule is catalog administration");
    assert!(matches!(
        refusal,
        AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
    ));
    assert!(harness.app.request_rules_engine_snapshot().is_empty());
}

#[tokio::test]
async fn a_person_targeting_source_needs_permission_authority() {
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
                builder_model: None,
            },
        )
        .await
        .expect_err("a person-targeting rule needs permission authority");
    assert!(
        matches!(refusal, AppError::Unauthorized(ref message) | AppError::UnauthorizedReason { ref message, .. }
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
                builder_model: None,
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
                builder_model: None,
            },
        )
        .await
        .expect("a content-only rule needs only catalog authority");
}

#[tokio::test]
async fn an_invalid_source_is_refused_with_the_reference_wording() {
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
                builder_model: None,
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
async fn editing_a_source_appends_a_revision_and_leaves_its_predecessor_alone() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;

    let updated = harness
        .app
        .update_request_rule_source(&harness.manager, &detail.rule_set.id, DENY_EVERYTHING, None)
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
        first.source_content_hash,
        detail.revision.source_content_hash
    );
}

#[tokio::test]
async fn deleting_a_rule_set_keeps_the_decisions_it_made() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9040, None).await;
    assert_eq!(harness.request_rule_decisions.recorded().await.len(), 1);

    harness
        .app
        .delete_request_rule_set(&harness.manager, &detail.rule_set.id)
        .await
        .expect("delete should succeed");

    // The trace outlives the rule and the engine no longer holds it.
    assert_eq!(harness.request_rule_decisions.recorded().await.len(), 1);
    assert!(harness.app.request_rules_engine_snapshot().is_empty());
}

/// Every application permission the account holds reaches the rule, not a
/// hand-picked subset of them.
#[tokio::test]
async fn previewing_lists_every_app_permission_the_requester_holds() {
    let harness = bootstrap_media_request_app();
    harness
        .app
        .services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(
            &harness.user.id,
            scryer_domain::AppPermissionMask::MANAGE_LISTS,
        )
        .await
        .expect("grant the requester one app permission");
    harness.users.store.lock().await.push(harness.user.clone());
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let preview = harness
        .app
        .preview_request_rule(
            &harness.manager,
            RequestRulePreviewRequest {
                source: RequestRulePreviewSource::Inline {
                    rego_source: APPROVE_EVERYTHING.to_string(),
                },
                sample: sample(&harness, &library_id, 9043),
            },
        )
        .await
        .expect("inline preview should evaluate");

    let input: serde_json::Value =
        serde_json::from_str(&preview.input_json).expect("the rendered input is JSON");
    assert_eq!(
        input["requester"]["app_permissions"],
        serde_json::json!(["manage_lists"])
    );
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
                source: RequestRulePreviewSource::Stored {
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
                source: RequestRulePreviewSource::Inline {
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
        .validate_request_rule_source(&harness.manager, APPROVE_EVERYTHING, &[])
        .await
        .expect("validation should run")
        .result;
    assert!(result.valid, "unexpected errors: {:?}", result.errors);

    let broken = harness
        .app
        .validate_request_rule_source(
            &harness.manager,
            "package rules\nimport rego.v1\n\nreasons contains \"x\" if { true }\n",
            &[],
        )
        .await
        .expect("validation should run")
        .result;
    assert!(!broken.valid);
    assert!(
        broken
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == scryer_rules::diagnostic_codes::RULE_VOTE_REQUIRED),
        "unexpected diagnostics: {:?}",
        broken.diagnostics
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

// ── The flow ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_unauthorized_requester_is_refused_before_any_rule_runs() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

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
    assert!(matches!(
        error,
        AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
    ));
    assert!(
        harness.request_rule_decisions.recorded().await.is_empty(),
        "no rule may run for a request that was never allowed"
    );
}

#[tokio::test]
async fn an_enabled_rule_decides_with_no_instance_switch_and_stops_once_disabled() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let first = submit(&harness, &library_id, 9043, None).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == first)
        .expect("the first request row exists");
    assert_eq!(request.status, MediaRequestStatus::Rejected);
    assert_eq!(
        request.decided_by_rule_set_ids,
        vec![detail.rule_set.id.clone()]
    );
    drop(requests);

    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].effective_outcome, RequestDecisionOutcome::Deny);

    harness
        .app
        .set_request_rule_enabled(&harness.manager, &detail.rule_set.id, false)
        .await
        .expect("disabling should succeed");
    let preflight = harness
        .app
        .preview_my_request_decision(&harness.user, media_request_input(library_id.clone(), 9045))
        .await
        .expect("pre-flight should answer");
    assert!(
        !preflight.rules_consulted,
        "a disabled rule casts no vote, so no rule is consulted"
    );
    assert_eq!(preflight.outcome, RequestDecisionOutcome::ManualReview);
    let second = submit(&harness, &library_id, 9045, None).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == second)
        .expect("the second request row exists");
    assert_eq!(
        request.status,
        MediaRequestStatus::Pending,
        "a disabled rule decides nothing, so the request waits for review"
    );
    drop(requests);
    assert_eq!(
        harness.request_rule_decisions.recorded().await.len(),
        1,
        "no enabled rule covers the library, so nothing new is traced"
    );
}

#[tokio::test]
async fn an_enforced_approval_creates_the_title_with_the_policy_tags_and_a_dormant_lease() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

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
async fn a_forever_request_writes_no_claim() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9046, None).await;

    assert!(
        harness.lifecycle_claims.all().await.is_empty(),
        "a request without a lease has no time to delete"
    );
}

#[tokio::test]
async fn an_enforced_denial_rejects_the_request_with_no_human_resolver() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

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
async fn trace_write_failure_does_not_disarm_an_enforced_denial() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;
    harness
        .request_rule_decisions
        .fail_writes
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9047, Some(7)).await;
    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Rejected
    );
    assert!(harness.titles.store.lock().await.is_empty());
    assert!(harness.lifecycle_claims.all().await.is_empty());
    assert!(harness.request_rule_decisions.recorded().await.is_empty());
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
    enable(&harness, &detail.rule_set.id).await;
    harness.lifecycle_claims.set_unreadable(true);

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9048, None).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(
        requests[0].status,
        MediaRequestStatus::Pending,
        "an unobservable fact must never read as an approval"
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
    assert!(
        votes[0].get("mode").is_none(),
        "a recorded vote carries no evaluation mode"
    );
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
    enable(&harness, &detail.rule_set.id).await;

    // A subject SMG cannot identify: the snapshot is explicitly partial, so
    // every metadata fact is unknown rather than empty.
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 8_901);
    input.external_ids = vec![ExternalId::new("tvdb".to_string(), "8901".to_string())];

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
                builder_model: None,
            },
        )
        .await
        .expect("rule set should be created");
    enable(&harness, &detail.rule_set.id).await;

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
    // answer without a trace.
    assert!(harness.request_rule_decisions.recorded().await.is_empty());
}

#[tokio::test]
async fn editing_a_pending_request_re_evaluates_it() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Short leases only", APPROVE_SHORT_LEASE).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    // A forever request carries no lease_days, so the rule abstains and the
    // request waits for a human.
    let request_id = submit(&harness, &library_id, 9051, None).await;
    assert_eq!(
        harness.media_requests.requests.lock().await[0].status,
        MediaRequestStatus::Pending
    );

    let updated = harness
        .app
        .update_my_media_request(
            &harness.user,
            UpdateMediaRequestInput {
                request_id: request_id.clone(),
                requested_quality_profile_id: "1080p".to_string(),
                requested_monitor_type: None,
                requested_monitor_selection: None,
                requested_lease_days: Some(Some(7)),
                requested_required_audio_languages: None,
                requested_preferred_audio_languages: None,
            },
        )
        .await
        .expect("the requester may shorten their lease");
    assert_eq!(updated.status, MediaRequestStatus::Approved);
    assert_eq!(updated.approved_lease_days, Some(7));

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
    enable(&harness, &detail.rule_set.id).await;

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
    assert!(preflight.rules_consulted);

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("submit should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Approved);
    assert_eq!(requests[0].policy_tags, preflight.tags);
    drop(requests);

    // Only the submit keeps a trace; the pre-flight persisted nothing.
    let traces = harness.request_rule_decisions.recorded().await;
    assert_eq!(traces.len(), 1);
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    assert_eq!(traces[0].request_id, request_id);
}

#[tokio::test]
async fn preflight_writes_no_decision_trace() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9054);
    for lease in [Some(7), Some(14), None] {
        input.requested_lease_days = lease;
        let preflight = harness
            .app
            .preview_my_request_decision(&harness.user, input.clone())
            .await
            .expect("pre-flight should answer");
        assert_eq!(preflight.outcome, RequestDecisionOutcome::AutoApprove);
        assert!(preflight.rules_consulted);
    }
    assert!(
        harness.request_rule_decisions.recorded().await.is_empty(),
        "a preview keeps no trace and writes no rule set link"
    );
    assert_eq!(
        harness
            .app
            .request_rule_decision_counts(
                &harness.manager,
                std::slice::from_ref(&detail.rule_set.id)
            )
            .await
            .expect("counts")
            .get(&detail.rule_set.id)
            .copied()
            .unwrap_or(0),
        0
    );
}

#[tokio::test]
async fn rule_decision_counts_are_one_batched_read_with_zero_for_unused_rules() {
    let harness = bootstrap_media_request_app();
    let trace = |id: &str, votes: &str| scryer_domain::RequestRuleDecisionRecord {
        id: id.to_string(),
        request_id: format!("request-{id}"),
        evaluated_at: chrono::Utc::now(),
        effective_outcome: RequestDecisionOutcome::AutoApprove,
        policy_outcome: RequestDecisionOutcome::AutoApprove,
        fallback_reason: None,
        votes_json: votes.to_string(),
        tags: Vec::new(),
        shaping_json: None,
        input_hash: String::new(),
        input_schema_version: 1,
        created_at: chrono::Utc::now(),
    };
    for (id, votes) in [
        ("d1", r#"[{"rule_set_id":"rule-a"}]"#),
        (
            "d2",
            r#"[{"rule_set_id":"rule-a"},{"rule_set_id":"rule-b"}]"#,
        ),
        ("d3", r#"[{"rule_set_id":"rule-a"}]"#),
    ] {
        harness.request_rule_decisions.seed(trace(id, votes)).await;
    }
    let ids = ["rule-a", "rule-b", "rule-unused"].map(str::to_string);

    let counts = harness
        .app
        .request_rule_decision_counts(&harness.manager, &ids)
        .await
        .expect("counts");

    assert_eq!(
        counts,
        std::collections::HashMap::from([
            ("rule-a".to_string(), 3),
            ("rule-b".to_string(), 1),
            ("rule-unused".to_string(), 0),
        ])
    );
    assert_eq!(
        harness
            .request_rule_decisions
            .count_batches
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    // The permission check still comes first.
    let mut viewer = test_admin_user();
    viewer.id = "viewer".to_string();
    viewer.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::NONE,
        loaded: true,
        ..Default::default()
    };
    assert!(
        harness
            .app
            .request_rule_decision_counts(&viewer, &ids)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn housekeeping_purges_only_preflight_traces() {
    let harness = bootstrap_media_request_app();
    let trace = |id: String, request_id: String| scryer_domain::RequestRuleDecisionRecord {
        id,
        request_id,
        evaluated_at: chrono::Utc::now(),
        effective_outcome: RequestDecisionOutcome::AutoApprove,
        policy_outcome: RequestDecisionOutcome::AutoApprove,
        fallback_reason: None,
        votes_json: "[]".to_string(),
        tags: Vec::new(),
        shaping_json: None,
        input_hash: String::new(),
        input_schema_version: 1,
        created_at: chrono::Utc::now(),
    };
    // More than two batches, so the pass loops and stops on a short batch.
    for index in 0..1_203 {
        harness
            .request_rule_decisions
            .seed(trace(
                format!("decision-preflight-{index}"),
                format!(
                    "{}{index}",
                    crate::request_rules::PREFLIGHT_REQUEST_ID_PREFIX
                ),
            ))
            .await;
    }
    for index in 0..4 {
        harness
            .request_rule_decisions
            .seed(trace(
                format!("decision-real-{index}"),
                format!("request-real-{index}"),
            ))
            .await;
    }

    assert_eq!(
        harness.app.purge_preflight_request_rule_traces().await,
        1_203
    );
    assert_eq!(harness.app.purge_preflight_request_rule_traces().await, 0);
    let kept: Vec<String> = harness
        .request_rule_decisions
        .recorded()
        .await
        .into_iter()
        .map(|trace| trace.id)
        .collect();
    assert_eq!(
        kept,
        vec![
            "decision-real-0",
            "decision-real-1",
            "decision-real-2",
            "decision-real-3",
        ]
    );
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
    assert!(matches!(
        error,
        AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
    ));
}

// ── The Auto-Approve Requests permission ────────────────────────────────────

fn auto_approve_requester(library_id: &str) -> User {
    library_permission_user(
        "auto-approve-requester",
        library_id,
        &[scryer_domain::LibraryPermission::AutoApproveRequests],
    )
}

async fn submit_as(
    harness: &MediaRequestTestHarness,
    requester: &User,
    library_id: &str,
    tvdb_id: i64,
) -> String {
    harness
        .app
        .submit_media_request(
            requester,
            media_request_input(library_id.to_string(), tvdb_id),
        )
        .await
        .expect("submit should succeed")
        .request_id
}

fn approved_event_data(
    events: &[scryer_domain::DomainEvent],
) -> scryer_domain::MediaRequestResolvedEventData {
    events
        .iter()
        .find_map(|event| match &event.payload {
            scryer_domain::DomainEventPayload::MediaRequestApproved(data) => Some(data.clone()),
            _ => None,
        })
        .expect("an approval event is published")
}

#[tokio::test]
async fn the_auto_approve_permission_approves_a_real_request_and_says_so() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let requester = auto_approve_requester(&library_id);

    let preflight = harness
        .app
        .preview_my_request_decision(&requester, media_request_input(library_id.clone(), 9960))
        .await
        .expect("pre-flight should answer");
    assert_eq!(preflight.outcome, RequestDecisionOutcome::AutoApprove);
    assert_eq!(
        preflight.fallback_reason.as_deref(),
        Some(FALLBACK_LIBRARY_PERMISSION)
    );

    let request_id = submit_as(&harness, &requester, &library_id, 9960).await;

    let titles = harness.titles.store.lock().await;
    assert_eq!(titles.len(), 1, "the approval adds the title");
    let title_id = titles[0].id.clone();
    drop(titles);

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == request_id)
        .expect("the request row exists");
    assert_eq!(request.status, MediaRequestStatus::Approved);
    assert_eq!(request.created_by_user_id, requester.id);
    assert_eq!(request.created_title_id.as_deref(), Some(title_id.as_str()));
    assert_eq!(
        request.decided_by_rule_set_ids,
        vec![LIBRARY_PERMISSION_DECIDER.to_string()]
    );
    drop(requests);

    let events = harness.domain_events.events.lock().await;
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        scryer_domain::DomainEventPayload::MediaRequestSubmitted(data) if data.request_id == request_id
    )));
    let approved = approved_event_data(&events);
    assert_eq!(approved.request_id, request_id);
    assert_eq!(
        approved.decided_by_rule_set_ids,
        vec![LIBRARY_PERMISSION_DECIDER.to_string()]
    );
    drop(events);

    // No rule took part, so nothing is traced.
    assert!(harness.request_rule_decisions.recorded().await.is_empty());
}

#[tokio::test]
async fn an_enforced_denial_beats_the_auto_approve_permission() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let requester = auto_approve_requester(&library_id);

    let preflight = harness
        .app
        .preview_my_request_decision(&requester, media_request_input(library_id.clone(), 9961))
        .await
        .expect("pre-flight should answer");
    assert_eq!(preflight.outcome, RequestDecisionOutcome::Deny);

    submit_as(&harness, &requester, &library_id, 9961).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Rejected);
    assert_eq!(
        requests[0].decided_by_rule_set_ids,
        vec![detail.rule_set.id.clone()]
    );
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());
}

#[tokio::test]
async fn a_disabled_denial_leaves_the_permission_approval_credited_to_the_permission() {
    let harness = bootstrap_media_request_app();
    create_rule(&harness, "Deny everything", DENY_EVERYTHING).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let requester = auto_approve_requester(&library_id);

    let request_id = submit_as(&harness, &requester, &library_id, 9962).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].id, request_id);
    assert_eq!(requests[0].status, MediaRequestStatus::Approved);
    assert_eq!(
        requests[0].decided_by_rule_set_ids,
        vec![LIBRARY_PERMISSION_DECIDER.to_string()],
        "a disabled rule acted on nothing, so it is not the decider"
    );
    drop(requests);

    let events = harness.domain_events.events.lock().await;
    let approved = approved_event_data(&events);
    assert_eq!(
        approved.decided_by_rule_set_ids,
        vec![LIBRARY_PERMISSION_DECIDER.to_string()]
    );
    assert!(
        approved.decision_reason_codes.is_empty(),
        "the disabled denial's reason must not ride on the approval"
    );
    drop(events);

    assert!(
        harness.request_rule_decisions.recorded().await.is_empty(),
        "a disabled rule is never evaluated"
    );
}

#[tokio::test]
async fn a_held_admission_waits_for_review_despite_the_auto_approve_permission() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let requester = auto_approve_requester(&library_id);
    let mut input = media_request_input(library_id, 9963);
    input.admission = crate::MediaRequestAdmission::HoldForReview;

    harness
        .app
        .submit_media_request(&requester, input)
        .await
        .expect("a held request is admitted");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());
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
        matches!(error, AppError::Validation(ref message) | AppError::ValidationRefused { ref message, .. } | AppError::ValidationRefusedWithCount { ref message, .. } if message.contains("scryer:")),
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
    enable(&harness, &detail.rule_set.id).await;

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

/// The "would be tagged" chip on a pending row promises only what an approval
/// can actually deliver, while the trace keeps the label the operator still has
/// to define.
#[tokio::test]
async fn a_pending_request_records_only_the_tags_that_can_land() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Send to a human", MANUAL_WITH_TWO_TAGS).await;
    enable(&harness, &detail.rule_set.id).await;
    harness
        .app
        .create_title_tag_definition(&harness.manager, "auto-approved", None)
        .await
        .expect("one of the two labels is defined");

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let preflight = harness
        .app
        .preview_my_request_decision(&harness.user, media_request_input(library_id.clone(), 9061))
        .await
        .expect("pre-flight should answer");
    assert_eq!(preflight.outcome, RequestDecisionOutcome::ManualReview);
    assert_eq!(preflight.tags, vec!["auto-approved"]);

    let request_id = submit(&harness, &library_id, 9061, None).await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    assert_eq!(
        requests[0].policy_tags,
        vec!["auto-approved".to_string()],
        "the row never offers the approver a label the approval would drop"
    );
    drop(requests);

    let traces = harness.request_rule_decisions.recorded().await;
    let trace = traces
        .iter()
        .find(|trace| trace.request_id == request_id)
        .expect("the decision was traced");
    let mut traced = trace.tags.clone();
    traced.sort();
    assert_eq!(
        traced,
        vec![
            "auto-approved".to_string(),
            "nobody-defined-this".to_string()
        ],
        "the trace still names the label nobody has defined yet"
    );
}

/// A registry rename is a data migration, and a pending request holds the list
/// an approver is about to apply — so it has to move with the label. The
/// request rule that emits it cannot be rewritten, so it is counted instead.
#[tokio::test]
async fn renaming_a_tag_follows_a_pending_request_and_counts_the_request_rule() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Send to a human", MANUAL_WITH_TWO_TAGS).await;
    enable(&harness, &detail.rule_set.id).await;
    let definition = harness
        .app
        .create_title_tag_definition(&harness.manager, "auto-approved", None)
        .await
        .expect("the label is defined");

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9062, None).await;

    let renamed = harness
        .app
        .update_title_tag_definition(
            &harness.manager,
            &definition.id,
            Some("policy approved".to_string()),
            None,
        )
        .await
        .expect("the rename should succeed");
    assert_eq!(
        renamed.counts.request_rule_sets, 1,
        "the rule that emits the old label is named, because Rego is never rewritten"
    );

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    assert_eq!(
        requests[0].policy_tags,
        vec!["policy approved".to_string()],
        "the pending row followed the rename"
    );
}

/// Deleting the label takes it off the pending row too: leaving it there would
/// offer an approver a tag the approval then drops in silence.
#[tokio::test]
async fn deleting_a_tag_strips_it_from_a_pending_request() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Send to a human", MANUAL_WITH_TWO_TAGS).await;
    enable(&harness, &detail.rule_set.id).await;
    let definition = harness
        .app
        .create_title_tag_definition(&harness.manager, "auto-approved", None)
        .await
        .expect("the label is defined");

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    submit(&harness, &library_id, 9063, None).await;

    let counts = harness
        .app
        .delete_title_tag_definition(&harness.manager, &definition.id)
        .await
        .expect("the delete should succeed");
    assert_eq!(counts.request_rule_sets, 1);

    let requests = harness.media_requests.requests.lock().await;
    assert!(
        requests[0].policy_tags.is_empty(),
        "the pending row no longer offers a label nothing defines: {:?}",
        requests[0].policy_tags
    );
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
        matches!(error, AppError::Validation(ref message) | AppError::ValidationRefused { ref message, .. } | AppError::ValidationRefusedWithCount { ref message, .. } if message.contains("not a defined tag")),
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
        matches!(error, AppError::Validation(ref message) | AppError::ValidationRefused { ref message, .. } | AppError::ValidationRefusedWithCount { ref message, .. } if message.contains("at most")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn every_overlapping_pending_request_gets_its_own_claim() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let first = submit(&harness, &library_id, 9057, Some(30)).await;
    let second = submit(&harness, &library_id, 9057, Some(5)).await;
    {
        let mut requests = harness.media_requests.requests.lock().await;
        let sibling = requests
            .iter_mut()
            .find(|request| request.id == second)
            .unwrap();
        sibling.decision_id = Some("sibling-trace".into());
        sibling.decided_by_rule_set_ids = vec!["sibling-policy".into()];
        sibling.policy_tags = vec!["sibling-tag".into()];
    }

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
    let requests = harness.media_requests.requests.lock().await;
    let sibling = requests
        .iter()
        .find(|request| request.id == second)
        .unwrap();
    assert_eq!(sibling.approved_lease_days, Some(5));
    assert_eq!(sibling.decision_id.as_deref(), Some("sibling-trace"));
    assert_eq!(sibling.decided_by_rule_set_ids, ["sibling-policy"]);
    assert_eq!(sibling.policy_tags, ["sibling-tag"]);
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
            matches!(
                error,
                Some(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. })
            ),
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
    harness
        .app
        .activate_dormant_claims_for_title(&extended.title_id, chrono::Utc::now())
        .await
        .expect("activate the extended dormant claim");
    assert_eq!(
        harness.lifecycle_claims.all().await[0].expires_at,
        Some(expires_at)
    );

    // Keeping it permanently ends the lease and adds nothing in its place:
    // the title is left without an expiry, not with a new hold.
    let converted = harness
        .app
        .convert_title_claim_to_permanent(&harness.manager, &claim_id)
        .await
        .expect("convert should succeed");
    assert_eq!(converted.id, claim_id);
    assert_eq!(converted.state, LifecycleClaimState::Converted);
    assert_eq!(
        converted.released_reason.as_deref(),
        Some("converted_to_permanent")
    );
    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims.len(), 1, "converting must not write a replacement");
    assert!(!claims[0].is_live());
    let again = harness
        .app
        .convert_title_claim_to_permanent(&harness.manager, &claim_id)
        .await
        .expect_err("a converted lease cannot convert again");
    assert!(matches!(
        again,
        AppError::Validation(_)
            | AppError::ValidationRefused { .. }
            | AppError::ValidationRefusedWithCount { .. }
    ));

    // Releasing withdraws a lease outright.
    let harness = bootstrap_media_request_app();
    let second = submit(&harness, &library_id, 9063, Some(30)).await;
    harness
        .app
        .approve_media_request(&harness.manager, &second, "1080p", None, None, None, None)
        .await
        .expect("approval should succeed");
    let lease_id = harness.lifecycle_claims.all().await[0].id.clone();
    let released = harness
        .app
        .release_title_claim(&harness.manager, &lease_id, "operator changed their mind")
        .await
        .expect("release should succeed");
    assert_eq!(released.state, LifecycleClaimState::Released);
    assert_eq!(
        released.released_reason.as_deref(),
        Some("operator changed their mind")
    );

    let blank = harness
        .app
        .release_title_claim(&harness.manager, &lease_id, "   ")
        .await
        .expect_err("a release reason is required");
    assert!(matches!(
        blank,
        AppError::Validation(_)
            | AppError::ValidationRefused { .. }
            | AppError::ValidationRefusedWithCount { .. }
    ));
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
                builder_model: None,
            },
        )
        .await
        .expect("rule set should be created")
}

async fn enable(harness: &MediaRequestTestHarness, rule_set_id: &str) {
    harness
        .app
        .set_request_rule_enabled(&harness.manager, rule_set_id, true)
        .await
        .expect("enabling should succeed");
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
        external_ids: vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
        quality_profile_id: Some("1080p".to_string()),
        monitor_type: None,
        lease_days: Some(Some(14)),
        required_audio_languages: None,
        preferred_audio_languages: None,
    }
}

const DENY_REQUIRING_NO_AUDIO: &str = r#"package rules
import rego.v1

deny if {
	input.request.required_audio_languages == []
}
"#;

const MANUAL_WHEN_AUDIO_LEFT_TO_LIBRARY: &str = r#"package rules
import rego.v1

manual if {
	input.request.required_audio_languages == null
}
"#;

fn audio_request_input(
    library_id: &str,
    tvdb_id: i64,
    required: Option<Vec<String>>,
    preferred: Option<Vec<String>>,
) -> crate::SubmitMediaRequestInput {
    let mut input = media_request_input(library_id.to_string(), tvdb_id);
    input.requested_required_audio_languages = required;
    input.requested_preferred_audio_languages = preferred;
    input
}

#[tokio::test]
async fn a_rule_refuses_a_request_that_requires_no_audio_language() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "No silent requests", DENY_REQUIRING_NO_AUDIO).await;
    enable(&harness, &detail.rule_set.id).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    harness
        .app
        .submit_media_request(
            &harness.user,
            audio_request_input(&library_id, 9401, Some(Vec::new()), None),
        )
        .await
        .expect("submit");
    harness
        .app
        .submit_media_request(
            &harness.user,
            audio_request_input(&library_id, 9402, None, None),
        )
        .await
        .expect("submit");

    let requests = harness.media_requests.requests.lock().await;
    let status_of = |tvdb: &str| {
        requests
            .iter()
            .find(|request| {
                request
                    .external_ids
                    .iter()
                    .any(|id| id.source == "tvdb" && id.value == tvdb)
            })
            .map(|request| request.status)
            .expect("request")
    };
    assert_eq!(status_of("9401"), MediaRequestStatus::Rejected);
    assert_eq!(
        status_of("9402"),
        MediaRequestStatus::Pending,
        "leaving it to the library is not requiring none"
    );
}

#[tokio::test]
async fn a_rule_can_approve_on_a_specific_requested_audio_language() {
    let harness = bootstrap_media_request_app();
    let code = crate::normalize_required_audio_requirements(["ja".to_string()])
        .pop()
        .expect("a normalized code");
    let source = format!(
        "package rules\nimport rego.v1\n\napprove if {{\n\t\"{code}\" in input.request.required_audio_languages\n}}\n"
    );
    let detail = create_rule(&harness, "Japanese audio", &source).await;
    enable(&harness, &detail.rule_set.id).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let asked = audio_request_input(&library_id, 9403, Some(vec!["ja".to_string()]), None);
    let preflight = harness
        .app
        .preview_my_request_decision(&harness.user, asked.clone())
        .await
        .expect("preflight");
    assert_eq!(preflight.outcome, RequestDecisionOutcome::AutoApprove);
    let other = audio_request_input(&library_id, 9404, Some(vec!["en".to_string()]), None);
    assert_ne!(
        harness
            .app
            .preview_my_request_decision(&harness.user, other)
            .await
            .expect("preflight")
            .outcome,
        RequestDecisionOutcome::AutoApprove
    );

    harness
        .app
        .submit_media_request(&harness.user, asked)
        .await
        .expect("submit");
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Approved);
}

#[tokio::test]
async fn preflight_and_submit_see_the_same_audio_facts() {
    let harness = bootstrap_media_request_app();
    let deny = create_rule(&harness, "No silent requests", DENY_REQUIRING_NO_AUDIO).await;
    enable(&harness, &deny.rule_set.id).await;
    let manual = create_rule(
        &harness,
        "Review inherited audio",
        MANUAL_WHEN_AUDIO_LEFT_TO_LIBRARY,
    )
    .await;
    enable(&harness, &manual.rule_set.id).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let none = audio_request_input(
        &library_id,
        9405,
        Some(Vec::new()),
        Some(vec!["fr".to_string()]),
    );
    let inherit = audio_request_input(&library_id, 9406, None, None);
    let preview = |input: crate::SubmitMediaRequestInput| {
        let harness = &harness;
        async move {
            harness
                .app
                .preview_my_request_decision(&harness.user, input)
                .await
                .expect("preflight")
        }
    };
    let none_preview = preview(none.clone()).await;
    assert_eq!(none_preview.outcome, RequestDecisionOutcome::Deny);
    let inherit_preview = preview(inherit.clone()).await;
    assert_eq!(
        inherit_preview.outcome,
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(
        inherit_preview.fallback_reason.as_deref(),
        Some(FALLBACK_RULE_MANUAL)
    );

    harness
        .app
        .submit_media_request(&harness.user, none)
        .await
        .expect("submit");
    harness
        .app
        .submit_media_request(&harness.user, inherit)
        .await
        .expect("submit");
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests[0].status, MediaRequestStatus::Rejected);
    assert_eq!(requests[1].status, MediaRequestStatus::Pending);
}

// ── Shaping heads ───────────────────────────────────────────────────────────

/// Approves every manual request into a seven-day lease on the 4k profile,
/// tags it, and adds a line to the submission notification.
const APPROVE_SHAPED: &str = r#"package rules
import rego.v1

approve if {
	input.request.origin == "manual"
}

set_lease_days := 7 if {
	input.request.origin == "manual"
}

set_quality_profile := "4K" if {
	input.request.origin == "manual"
}

notify := "Short lease applied" if {
	input.request.origin == "manual"
}
"#;

/// The same shaping, but leaves the request for a person.
const MANUAL_SHAPED: &str = r#"package rules
import rego.v1

manual if {
	input.request.origin == "manual"
}

set_lease_days := 7 if {
	input.request.origin == "manual"
}
"#;

fn submitted_rule_notifications(
    events: &[scryer_domain::DomainEvent],
    request_id: &str,
) -> Vec<scryer_domain::RequestRuleNotification> {
    events
        .iter()
        .find_map(|event| match &event.payload {
            scryer_domain::DomainEventPayload::MediaRequestSubmitted(data)
                if data.request_id == request_id =>
            {
                Some(data.rule_notifications.clone())
            }
            _ => None,
        })
        .expect("a submitted event for the request")
}

#[tokio::test]
async fn an_automatic_approval_applies_the_shaping_and_traces_who_set_it() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Short leases", APPROVE_SHAPED).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9301, Some(30)).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == request_id)
        .expect("request stored");
    assert_eq!(request.status, MediaRequestStatus::Approved);
    // The rule's lease replaces the 30 days asked for.
    assert_eq!(request.requested_lease_days, Some(30));
    assert_eq!(request.approved_lease_days, Some(7));
    assert_eq!(request.approved_quality_profile_id.as_deref(), Some("4k"));
    assert_eq!(request.approved_quality_profile_name.as_deref(), Some("4K"));
    drop(requests);

    let claims = harness.lifecycle_claims.all().await;
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].duration_days, Some(7));

    let decisions = harness.request_rule_decisions.recorded().await;
    let shaping: crate::request_rules::ArbitratedShaping = serde_json::from_str(
        decisions[0]
            .shaping_json
            .as_deref()
            .expect("the trace records the shaping"),
    )
    .expect("the shaping reads back");
    let lease = shaping
        .value(RequestActionHead::SetLeaseDays)
        .expect("the lease is traced");
    assert_eq!(lease.rule_set_id, detail.rule_set.id);
    assert_eq!(lease.rule_set_name, "Short leases");

    // The notification text rides on the submitted event, attributed.
    let events = harness.domain_events.events.lock().await;
    let notifications = submitted_rule_notifications(&events, &request_id);
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].rule_set_name, "Short leases");
    assert_eq!(notifications[0].text, "Short lease applied");
}

#[tokio::test]
async fn a_lease_of_zero_days_approves_forever_and_writes_no_claim() {
    let harness = bootstrap_media_request_app();
    let source = APPROVE_SHAPED.replace("set_lease_days := 7", "set_lease_days := 0");
    let detail = create_rule(&harness, "Keep forever", &source).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9302, Some(30)).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == request_id)
        .expect("request stored");
    assert_eq!(request.status, MediaRequestStatus::Approved);
    assert_eq!(request.approved_lease_days, None);
    drop(requests);
    assert!(harness.lifecycle_claims.all().await.is_empty());
}

#[tokio::test]
async fn manual_review_keeps_the_shaping_for_the_approver_and_applies_nothing() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Suggest a lease", MANUAL_SHAPED).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9303, Some(30)).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == request_id)
        .expect("request stored");
    assert_eq!(request.status, MediaRequestStatus::Pending);
    assert_eq!(request.approved_lease_days, None);
    assert!(request.created_title_id.is_none());
    drop(requests);
    assert!(harness.lifecycle_claims.all().await.is_empty());

    let decisions = harness.request_rule_decisions.recorded().await;
    let shaping: crate::request_rules::ArbitratedShaping =
        serde_json::from_str(decisions[0].shaping_json.as_deref().expect("stored"))
            .expect("the shaping reads back");
    assert_eq!(
        shaping
            .value(RequestActionHead::SetLeaseDays)
            .map(|shaped| shaped.value.clone()),
        Some(RequestHeadValue::Number(7))
    );
}

#[tokio::test]
async fn a_preflight_shows_the_shaping_but_sends_no_notification() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Short leases", APPROVE_SHAPED).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9304);
    input.requested_lease_days = Some(30);
    let preflight = harness
        .app
        .preview_my_request_decision(&harness.user, input)
        .await
        .expect("preflight runs");
    assert_eq!(preflight.outcome, RequestDecisionOutcome::AutoApprove);
    assert_eq!(
        preflight
            .shaping
            .value(RequestActionHead::SetLeaseDays)
            .map(|shaped| shaped.value.clone()),
        Some(RequestHeadValue::Number(7))
    );
    // Nothing was submitted, so nothing was announced.
    assert!(
        !harness
            .domain_events
            .events
            .lock()
            .await
            .iter()
            .any(|event| matches!(
                event.payload,
                scryer_domain::DomainEventPayload::MediaRequestSubmitted(_)
            ))
    );
}

#[tokio::test]
async fn a_shaping_value_this_instance_cannot_apply_is_refused_at_save_with_its_location() {
    let harness = bootstrap_media_request_app();
    let source = "package rules\nimport rego.v1\n\napprove if { input.request.origin == \"manual\" }\n\nset_quality_profile := \"No Such Profile\" if { input.request.origin == \"manual\" }\n";
    let refusal = harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: "Unknown profile".to_string(),
                description: String::new(),
                rego_source: source.to_string(),
                library_ids: Vec::new(),
                builder_model: None,
            },
        )
        .await
        .expect_err("an unknown profile must not be stored");
    let AppError::ValidationRefused { diagnostics, .. } = &refusal else {
        panic!("unexpected refusal: {refusal:?}");
    };
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.code == scryer_rules::diagnostic_codes::RULE_QUALITY_PROFILE_UNKNOWN
        })
        .unwrap_or_else(|| panic!("unexpected diagnostics: {diagnostics:?}"));
    assert!(diagnostic.line.is_some() && diagnostic.column.is_some());
    assert_eq!(diagnostic.path.as_deref(), Some("set_quality_profile"));
    assert!(
        harness
            .app
            .list_request_rule_sets(&harness.manager)
            .await
            .expect("list")
            .is_empty()
    );

    // The draft validator reports the same condition without storing anything.
    let validation = harness
        .app
        .validate_request_rule_source(&harness.manager, source, &[])
        .await
        .expect("validation runs");
    assert!(!validation.result.valid);
    assert!(
        validation
            .result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == scryer_rules::diagnostic_codes::RULE_QUALITY_PROFILE_UNKNOWN)
    );
    let declared = validation.declared_actions.expect("the source reads");
    assert_eq!(declared.quality_profiles, vec!["No Such Profile"]);
}

#[tokio::test]
async fn a_monitor_no_scoped_library_accepts_is_refused_at_save() {
    // `advanced` needs a season selection a rule cannot make, so no facet
    // accepts it from a rule.
    let harness = bootstrap_media_request_app();
    let movie_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let source = "package rules\nimport rego.v1\n\napprove if { input.request.origin == \"manual\" }\n\nset_monitor := \"advanced\" if { input.request.origin == \"manual\" }\n";
    let refusal = harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: "Advanced monitor".to_string(),
                description: String::new(),
                rego_source: source.to_string(),
                library_ids: vec![movie_library],
                builder_model: None,
            },
        )
        .await
        .expect_err("no library accepts advanced monitoring from a rule");
    let AppError::ValidationRefused { diagnostics, .. } = &refusal else {
        panic!("unexpected refusal: {refusal:?}");
    };
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.code
            == scryer_rules::diagnostic_codes::RULE_MONITOR_UNSUPPORTED),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

#[tokio::test]
async fn the_builder_model_round_trips_and_is_bounded() {
    let harness = bootstrap_media_request_app();
    let model =
        serde_json::json!({ "family": "request", "version": 1, "rows": [{ "fact": "origin" }] });
    let created = harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: "Built".to_string(),
                description: String::new(),
                rego_source: APPROVE_EVERYTHING.to_string(),
                library_ids: Vec::new(),
                builder_model: Some(model.clone()),
            },
        )
        .await
        .expect("a builder model is stored");
    let id = created.rule_set.id.clone();
    let read = |with: bool| {
        let app = &harness.app;
        let manager = &harness.manager;
        let id = id.clone();
        async move {
            app.get_request_rule_set(manager, &id, with)
                .await
                .expect("read")
                .expect("present")
                .builder_model
        }
    };
    assert_eq!(read(true).await, Some(model.clone()));
    // Only the view that asks for it reads it.
    assert_eq!(read(false).await, None);

    // Omitted keeps it while the source is unchanged.
    harness
        .app
        .update_request_rule_source(&harness.manager, &id, APPROVE_EVERYTHING, None)
        .await
        .expect("edit");
    assert_eq!(read(true).await, Some(model.clone()));

    // Omitted with a changed source clears it, so a later builder save cannot
    // revert an edit made by hand.
    harness
        .app
        .update_request_rule_source(&harness.manager, &id, DENY_EVERYTHING, None)
        .await
        .expect("edit");
    assert_eq!(read(true).await, None);
    harness
        .app
        .update_request_rule_source(
            &harness.manager,
            &id,
            DENY_EVERYTHING,
            Some(Some(model.clone())),
        )
        .await
        .expect("store it again");
    assert_eq!(read(true).await, Some(model));

    // Not an object, another family's document, and too large, are refused
    // with the codes every rule family uses.
    for (bad, code) in [
        (
            serde_json::json!(["not", "an", "object"]),
            crate::error_reasons::rules::BUILDER_MODEL_INVALID,
        ),
        (
            serde_json::json!({ "family": "lifecycle", "version": 1 }),
            crate::error_reasons::rules::BUILDER_MODEL_INVALID,
        ),
        (
            serde_json::json!({
                "family": "request",
                "version": 1,
                "blob": "x".repeat(crate::rules::builder_model::MAX_BUILDER_MODEL_BYTES),
            }),
            crate::error_reasons::rules::BUILDER_MODEL_TOO_LARGE,
        ),
    ] {
        let refusal = harness
            .app
            .update_request_rule_source(&harness.manager, &id, DENY_EVERYTHING, Some(Some(bad)))
            .await
            .expect_err("refused");
        assert_eq!(refusal.validation_reason(), Some(code));
    }

    // Null clears it.
    harness
        .app
        .update_request_rule_source(&harness.manager, &id, DENY_EVERYTHING, Some(None))
        .await
        .expect("clear");
    assert_eq!(read(true).await, None);
}

async fn create_scoped_rule(
    harness: &MediaRequestTestHarness,
    name: &str,
    source: &str,
    library_ids: Vec<String>,
) -> AppResult<crate::request_rules::RequestRuleSetDetail> {
    harness
        .app
        .create_request_rule_set(
            &harness.manager,
            RequestRuleDraft {
                name: name.to_string(),
                description: String::new(),
                rego_source: source.to_string(),
                library_ids,
                builder_model: None,
            },
        )
        .await
}

fn diagnostic_codes(refusal: &AppError) -> Vec<&'static str> {
    let AppError::ValidationRefused { diagnostics, .. } = refusal else {
        panic!("unexpected refusal: {refusal:?}");
    };
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn approve_setting_monitor(monitor: &str) -> String {
    format!(
        "package rules\nimport rego.v1\n\napprove if {{ input.request.origin == \"manual\" }}\n\nset_monitor := \"{monitor}\" if {{ input.request.origin == \"manual\" }}\n"
    )
}

#[tokio::test]
async fn a_movie_rule_with_an_episode_monitor_type_is_refused_at_save() {
    let harness = bootstrap_media_request_app();
    let movie_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    for episode_monitor in ["allEpisodes", "futureEpisodes", "missingAndFutureEpisodes"] {
        let refusal = create_scoped_rule(
            &harness,
            "Episode monitor on movies",
            &approve_setting_monitor(episode_monitor),
            vec![movie_library.clone()],
        )
        .await
        .expect_err("a movie library has no episode monitoring");
        assert!(
            diagnostic_codes(&refusal)
                .contains(&scryer_rules::diagnostic_codes::RULE_MONITOR_UNSUPPORTED),
            "{episode_monitor}: {refusal:?}"
        );
    }
    create_scoped_rule(
        &harness,
        "Unmonitored movies",
        &approve_setting_monitor("unmonitored"),
        vec![movie_library],
    )
    .await
    .expect("a movie library accepts unmonitored");
}

#[tokio::test]
async fn an_episode_monitor_type_on_a_movie_request_goes_to_manual_review() {
    // Unscoped, so the series libraries let it save; the movie request it
    // meets cannot take it.
    let harness = bootstrap_media_request_app();
    let detail = create_rule(
        &harness,
        "All episodes",
        &approve_setting_monitor("allEpisodes"),
    )
    .await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9311, None).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == request_id)
        .expect("request stored");
    assert_eq!(request.status, MediaRequestStatus::Pending);
    assert!(request.created_title_id.is_none());
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());

    let decisions = harness.request_rule_decisions.recorded().await;
    assert_eq!(
        decisions[0].fallback_reason.as_deref(),
        Some(FALLBACK_SHAPING_INVALID)
    );
    let shaping: crate::request_rules::ArbitratedShaping =
        serde_json::from_str(decisions[0].shaping_json.as_deref().expect("stored"))
            .expect("the shaping reads back");
    assert!(shaping.value(RequestActionHead::SetMonitor).is_none());
    assert_eq!(shaping.invalid.len(), 1);
    assert_eq!(shaping.invalid[0].head, RequestActionHead::SetMonitor);
}

#[tokio::test]
async fn re_scoping_to_a_library_that_cannot_take_the_shaped_monitor_is_refused() {
    let harness = bootstrap_media_request_app();
    let series_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let movie_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let detail = create_scoped_rule(
        &harness,
        "Future episodes",
        &approve_setting_monitor("futureEpisodes"),
        vec![series_library.clone()],
    )
    .await
    .expect("a series library takes future episodes");

    let refusal = harness
        .app
        .update_request_rule_metadata(
            &harness.manager,
            &detail.rule_set.id,
            "Future episodes".to_string(),
            String::new(),
            vec![movie_library],
        )
        .await
        .expect_err("a movie library has no episode monitoring");
    assert!(
        diagnostic_codes(&refusal)
            .contains(&scryer_rules::diagnostic_codes::RULE_MONITOR_UNSUPPORTED),
        "{refusal:?}"
    );
    let stored = harness
        .app
        .get_request_rule_set(&harness.manager, &detail.rule_set.id, false)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(stored.rule_set.library_ids, vec![series_library.clone()]);

    // A rename that keeps the scope is not re-checked and still saves.
    harness
        .app
        .update_request_rule_metadata(
            &harness.manager,
            &detail.rule_set.id,
            "Renamed".to_string(),
            String::new(),
            vec![series_library],
        )
        .await
        .expect("rename");
}

#[tokio::test]
async fn an_ambiguous_quality_profile_name_is_refused_at_save_and_held_at_run_time() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Short leases", APPROVE_SHAPED).await;
    enable(&harness, &detail.rule_set.id).await;

    // A second profile now carries the name the rule sets.
    let mut twin = crate::builtin_4k_profile();
    twin.id = "4k-twin".to_string();
    harness
        .quality_profiles
        .set_profiles(vec![
            crate::builtin_4k_profile(),
            twin,
            crate::builtin_1080p_profile(),
        ])
        .await;

    let refusal = create_rule_result(&harness, "Twin", APPROVE_SHAPED)
        .await
        .expect_err("the name does not say which profile");
    assert!(
        diagnostic_codes(&refusal)
            .contains(&scryer_rules::diagnostic_codes::RULE_QUALITY_PROFILE_AMBIGUOUS),
        "{refusal:?}"
    );

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9312, Some(30)).await;
    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == request_id)
        .expect("request stored");
    assert_eq!(request.status, MediaRequestStatus::Pending);
    assert_eq!(request.approved_quality_profile_id, None);
    drop(requests);
    let decisions = harness.request_rule_decisions.recorded().await;
    assert_eq!(
        decisions[0].fallback_reason.as_deref(),
        Some(FALLBACK_SHAPING_INVALID)
    );
}

async fn create_rule_result(
    harness: &MediaRequestTestHarness,
    name: &str,
    source: &str,
) -> AppResult<crate::request_rules::RequestRuleSetDetail> {
    create_scoped_rule(harness, name, source, Vec::new()).await
}

#[tokio::test]
async fn a_denial_traces_its_shaping_applies_none_of_it_and_still_notifies() {
    let harness = bootstrap_media_request_app();
    let source = APPROVE_SHAPED.replacen("approve if", "deny if", 1);
    let detail = create_rule(&harness, "Shaped denial", &source).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9313, Some(30)).await;

    let requests = harness.media_requests.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.id == request_id)
        .expect("request stored");
    assert_eq!(request.status, MediaRequestStatus::Rejected);
    assert_eq!(request.approved_lease_days, None);
    assert_eq!(request.approved_quality_profile_id, None);
    assert!(request.created_title_id.is_none());
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());
    assert!(harness.lifecycle_claims.all().await.is_empty());

    let decisions = harness.request_rule_decisions.recorded().await;
    assert_eq!(decisions[0].effective_outcome, RequestDecisionOutcome::Deny);
    let shaping: crate::request_rules::ArbitratedShaping =
        serde_json::from_str(decisions[0].shaping_json.as_deref().expect("stored"))
            .expect("the shaping reads back");
    assert_eq!(
        shaping
            .value(RequestActionHead::SetLeaseDays)
            .map(|shaped| shaped.value.clone()),
        Some(RequestHeadValue::Number(7))
    );

    // The request was really submitted, so the rule's line rides on the
    // submitted event like any other.
    let events = harness.domain_events.events.lock().await;
    let notifications = submitted_rule_notifications(&events, &request_id);
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].rule_set_name, "Shaped denial");
    assert_eq!(notifications[0].text, "Short lease applied");
}

#[tokio::test]
async fn a_manual_review_request_carries_the_rule_notification() {
    let harness = bootstrap_media_request_app();
    let source = format!(
        "{MANUAL_SHAPED}\nnotify := \"Needs a look\" if {{\n\tinput.request.origin == \"manual\"\n}}\n"
    );
    let detail = create_rule(&harness, "Review", &source).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_id = submit(&harness, &library_id, 9314, None).await;
    assert_eq!(
        harness
            .media_requests
            .requests
            .lock()
            .await
            .iter()
            .find(|request| request.id == request_id)
            .expect("request stored")
            .status,
        MediaRequestStatus::Pending
    );
    let events = harness.domain_events.events.lock().await;
    let notifications = submitted_rule_notifications(&events, &request_id);
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].rule_set_name, "Review");
    assert_eq!(notifications[0].text, "Needs a look");
}

#[tokio::test]
async fn an_automatic_approval_applies_the_shaped_monitor_and_audio_languages() {
    let harness = bootstrap_media_request_app();
    let source = "package rules\nimport rego.v1\n\napprove if { input.request.origin == \"manual\" }\n\nset_monitor := \"unmonitored\" if { input.request.origin == \"manual\" }\n\nset_required_audio_languages := [\"en\"] if { input.request.origin == \"manual\" }\n\nset_preferred_audio_languages := [\"fr\"] if { input.request.origin == \"manual\" }\n";
    let detail = create_rule(&harness, "Quiet English", source).await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = audio_request_input(
        &library_id,
        9315,
        Some(vec!["ja".to_string()]),
        Some(vec!["de".to_string()]),
    );
    input.requested_monitor_type = Some("monitored".to_string());
    let request_id = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("submit")
        .request_id;

    let request = harness
        .media_requests
        .requests
        .lock()
        .await
        .iter()
        .find(|request| request.id == request_id)
        .cloned()
        .expect("request stored");
    assert_eq!(request.status, MediaRequestStatus::Approved);
    // The requester's own picks stay on the request; the rule's go to the title.
    assert_eq!(request.requested_monitor_type.as_deref(), Some("monitored"));
    let title_id = request.created_title_id.expect("a title was created");

    let title = harness
        .titles
        .store
        .lock()
        .await
        .iter()
        .find(|title| title.id == title_id)
        .cloned()
        .expect("title stored");
    assert!(!title.monitored);
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:monitor-type:unmonitored"),
        "{:?}",
        title.tags
    );

    let expected = |codes: &[&str]| {
        crate::settings::runtime::validated_preference_languages(
            "audio language",
            codes.iter().map(|code| code.to_string()).collect(),
        )
        .expect("known languages")
    };
    assert_eq!(
        harness
            .app
            .load_title_required_audio_override(&title_id)
            .await
            .unwrap(),
        Some(expected(&["en"]))
    );
    assert_eq!(
        harness
            .app
            .load_title_preferred_audio_override(&title_id)
            .await
            .unwrap(),
        Some(expected(&["fr"]))
    );

    let decisions = harness.request_rule_decisions.recorded().await;
    let shaping: crate::request_rules::ArbitratedShaping =
        serde_json::from_str(decisions[0].shaping_json.as_deref().expect("stored"))
            .expect("the shaping reads back");
    for head in [
        RequestActionHead::SetMonitor,
        RequestActionHead::SetRequiredAudioLanguages,
        RequestActionHead::SetPreferredAudioLanguages,
    ] {
        assert_eq!(
            shaping
                .value(head)
                .map(|shaped| shaped.rule_set_name.as_str()),
            Some("Quiet English"),
            "{head:?}"
        );
    }
}

/// An approver's tag list is held to the same shape a rule's tags are: each
/// condition refuses with its own reason, and the sentence names the tag.
#[test]
fn an_approver_tag_list_is_checked_like_a_rule_tag() {
    use crate::error_reasons::request_rules::{
        TAG_CONTAINS_UNSUPPORTED_CHARACTER, TAG_LIMIT_EXCEEDED, TAG_REQUIRED, TAG_TOO_LONG,
        TAG_USES_RESERVED_PREFIX,
    };
    use crate::request_rules::validate_tag_list;
    use scryer_rules::policy::{MAX_TAG_LEN, MAX_TAGS, RESERVED_TAG_PREFIX};

    fn refusal(tags: &[&str]) -> (&'static str, String) {
        let error = validate_tag_list(tags.iter().map(|tag| tag.to_string()).collect())
            .expect_err("refused");
        match error {
            AppError::ValidationRefused {
                reason,
                message,
                facts,
                diagnostics,
            } => {
                assert!(facts.is_empty(), "{tags:?}");
                assert!(diagnostics.is_empty(), "{tags:?}");
                (reason, message)
            }
            other => panic!("{tags:?}: {other:?}"),
        }
    }
    fn accepted(tags: &[&str]) -> Vec<String> {
        validate_tag_list(tags.iter().map(|tag| tag.to_string()).collect()).expect("accepted")
    }

    let longest = "a".repeat(MAX_TAG_LEN);
    let too_long = "a".repeat(MAX_TAG_LEN + 1);
    // Two bytes each: within the length in characters, past it in bytes.
    let wide = "é".repeat(MAX_TAG_LEN / 2 + 1);
    let reserved = format!("{RESERVED_TAG_PREFIX}family");

    assert_eq!(accepted(&[]), Vec::<String>::new());
    assert_eq!(
        accepted(&[" family ", "a.b_c d-E9", "family", longest.as_str()]),
        vec![
            "family".to_string(),
            "a.b_c d-E9".to_string(),
            longest.clone()
        ]
    );
    let most: Vec<String> = (0..MAX_TAGS).map(|i| format!("t{i}")).collect();
    let most: Vec<&str> = most.iter().map(String::as_str).collect();
    assert_eq!(accepted(&most).len(), MAX_TAGS);

    let too_many: Vec<String> = (0..=MAX_TAGS).map(|i| format!("t{i}")).collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert_eq!(
        refusal(&too_many),
        (
            TAG_LIMIT_EXCEEDED,
            format!(
                "a request may carry at most {MAX_TAGS} tags, {} were supplied",
                MAX_TAGS + 1
            )
        )
    );
    for empty in ["", "   ", "\t"] {
        assert_eq!(
            refusal(&["ok", empty]),
            (TAG_REQUIRED, "tag must not be empty".to_string()),
            "{empty:?}"
        );
    }
    assert_eq!(
        refusal(&[too_long.as_str()]),
        (
            TAG_TOO_LONG,
            format!(
                "tag is {} characters, at most {MAX_TAG_LEN} are allowed",
                MAX_TAG_LEN + 1
            )
        )
    );
    assert_eq!(
        refusal(&[wide.as_str()]),
        (
            TAG_TOO_LONG,
            format!(
                "tag is {} characters, at most {MAX_TAG_LEN} are allowed",
                wide.len()
            )
        )
    );
    assert_eq!(
        refusal(&[reserved.as_str()]),
        (
            TAG_USES_RESERVED_PREFIX,
            format!("tag '{reserved}' uses the reserved '{RESERVED_TAG_PREFIX}' prefix")
        )
    );
    for (tag, bad) in [
        ("fam/ily", '/'),
        ("café", 'é'),
        ("a:b", ':'),
        ("a\tb", '\t'),
    ] {
        assert_eq!(
            refusal(&[tag]),
            (
                TAG_CONTAINS_UNSUPPORTED_CHARACTER,
                format!(
                    "tag '{tag}' contains unsupported character '{bad}'; use letters, digits, \
                     '.', '_', '-', or spaces"
                )
            ),
            "{tag:?}"
        );
    }
}

/// A list request is submitted as the list's owner, whose authorization the
/// owner row does not carry. It is read once for the whole submission, the
/// rule check and the requester document included, and every permission the
/// owner holds still reaches the rules.
#[tokio::test]
async fn a_list_request_reads_its_owners_authorization_once() {
    use crate::lists::AppListActions;
    use crate::lists::act::ListActions;

    const APPROVE_FOR_GRANTED_REQUESTERS: &str = r#"package rules
import rego.v1

approve if {
	"request" in input.requester.library_permissions
	"view" in input.requester.library_permissions
	not "manage_library" in input.requester.library_permissions
	"manage_lists" in input.requester.app_permissions
}
"#;
    let harness = bootstrap_media_request_app();
    let detail = create_rule(
        &harness,
        "Approve granted requesters",
        APPROVE_FOR_GRANTED_REQUESTERS,
    )
    .await;
    enable(&harness, &detail.rule_set.id).await;

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut owner = harness.user.clone();
    owner.id = "list-owner-fixture".to_string();
    owner.username = "list_owner_fixture".to_string();
    owner.authorization = scryer_domain::UserAuthorization {
        loaded: false,
        ..Default::default()
    };
    harness.users.store.lock().await.push(owner.clone());
    harness
        .libraries
        .set_app_permission_mask_for_user(&owner.id, AppPermissionMask::MANAGE_LISTS)
        .await
        .unwrap();
    harness
        .libraries
        .set_grants_for_user(
            &owner.id,
            vec![scryer_domain::LibraryGrant {
                user_id: owner.id.clone(),
                library_id: library_id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::from_permissions([
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::Request,
                ]),
            }],
        )
        .await
        .unwrap();

    let mut list = crate::lists::test_support::subscription("public-list-one");
    list.owner_user_id = owner.id.clone();
    let route = crate::lists::test_support::route(MediaFacet::Movie, &library_id);
    let mut item = crate::lists::test_support::resolved_item("alpha");
    item.external_ids = vec![ExternalId::new("tvdb", "9056")];

    AppListActions::new(&harness.app)
        .submit_request(&list, &route, &item, false)
        .await
        .expect("list request admitted");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].status,
        MediaRequestStatus::Approved,
        "the rule saw every permission the owner holds"
    );
    drop(requests);
    assert_eq!(
        harness
            .libraries
            .app_permission_reads
            .lock()
            .unwrap()
            .get(&owner.id)
            .copied(),
        Some(1)
    );
}
