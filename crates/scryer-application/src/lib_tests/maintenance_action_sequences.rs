//! Service-boundary regression tests for the actions a rule declares.
//!
//! The executor has its own sequence tests. These cover the authoring boundary:
//! a source's heads become an immutable revision, an edit that can delete
//! invalidates the acknowledgement while any other edit keeps the rule's
//! state, storage rules use only the filesystem-safe heads, and a revision
//! stored before sources declared their actions is refused rather than read.

use super::*;

use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::maintenance_rules::{
    MaintenanceActionSequence, MaintenanceActionStep, MaintenanceActionStepKind,
    MaintenanceActionStepParameters, MaintenanceDeclaredActions, MaintenanceMatcherDraft,
    MaintenanceRuleDraft, MaintenanceSearchCondition,
};
use scryer_domain::{
    MaintenanceEffectArming, MaintenanceRuleState, MaintenanceRuleSubjectKind, User,
};
use scryer_rules::maintenance_actions::MaintenanceActionHead;
use std::sync::Arc;

const UNMONITOR_AND_SEARCH: &str = "package whatever\n\
    import rego.v1\n\n\
    unmonitor := true\n\n\
    search := true\n";

const SEARCH_ONLY: &str = "package whatever\n\
    import rego.v1\n\n\
    search := true\n";

const DELETES_FILES: &str = "package whatever\n\
    import rego.v1\n\n\
    delete_files := true\n";

const STORAGE_DELETE: &str = "package whatever\n\
    import rego.v1\n\n\
    delete_files if {\n\
    \tinput.facts.storage_available_percent < 5\n\
    }\n";

const STORAGE_SEARCH: &str = "package whatever\n\
    import rego.v1\n\n\
    search if {\n\
    \tinput.facts.storage_available_percent < 5\n\
    }\n";

fn authoring_app() -> (AppUseCase, User) {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let app =
        app.with_test_overrides(|services| services.with_maintenance_rule_set_store(rules.clone()));
    (app, user)
}

fn draft(
    rego_source: &str,
    grace_days: i64,
    storage_root_id: Option<String>,
) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        subject_kind: MaintenanceRuleSubjectKind::Title,
        name: "Declared rule".to_string(),
        description: "A declared-actions fixture".to_string(),
        rego_source: rego_source.to_string(),
        grace_days,
        storage_root_id,
        library_ids: Vec::new(),
    }
}

fn matcher(rego_source: &str, grace_days: i64) -> MaintenanceMatcherDraft {
    MaintenanceMatcherDraft {
        rego_source: rego_source.to_string(),
        grace_days,
        storage_root_id: None,
    }
}

fn persisted_heads(action_spec_json: &str) -> Vec<MaintenanceActionHead> {
    MaintenanceDeclaredActions::from_persisted_json(action_spec_json)
        .expect("persisted actions decode")
        .expect("persisted actions are declared")
        .heads
}

#[tokio::test]
async fn declared_heads_are_saved_as_an_immutable_revision() {
    let (app, user) = authoring_app();
    let created = app
        .create_maintenance_rule_set(&user, draft(UNMONITOR_AND_SEARCH, 19, None))
        .await
        .expect("create declared rule");

    assert!(!created.rule_set.enabled, "new rules ship disabled");
    assert_eq!(
        created.rule_set.effect_arming,
        MaintenanceEffectArming::None
    );
    assert_eq!(created.revision.grace_days, 19);
    assert_eq!(created.load_refusal, None);
    assert_eq!(
        persisted_heads(&created.revision.action_spec_json),
        vec![
            MaintenanceActionHead::Unmonitor,
            MaintenanceActionHead::Search
        ],
        "heads are recorded in source order"
    );
    assert_eq!(
        created.declared_actions.heads,
        persisted_heads(&created.revision.action_spec_json)
    );

    let updated = app
        .update_maintenance_rule_matcher(&user, &created.rule_set.id, matcher(SEARCH_ONLY, 41))
        .await
        .expect("save a new revision");
    assert_eq!(updated.rule_set.current_revision_number, 2);
    assert_eq!(
        updated.declared_actions.heads.clone(),
        vec![MaintenanceActionHead::Search]
    );

    let revisions = app
        .list_maintenance_rule_revisions(&user, &created.rule_set.id)
        .await
        .expect("list immutable revisions");
    let first = revisions
        .iter()
        .find(|revision| revision.revision_number == 1)
        .expect("first revision remains available");
    assert_eq!(first.grace_days, 19);
    assert_eq!(
        persisted_heads(&first.action_spec_json),
        vec![
            MaintenanceActionHead::Unmonitor,
            MaintenanceActionHead::Search
        ]
    );
}

/// The acknowledgement an Enabled rule carries is bound to its delete heads:
/// an edit to a source that can delete drops the rule to Dry run, and an
/// edit to one that cannot keeps the rule as it was.
#[tokio::test]
async fn only_an_edit_that_can_delete_drops_an_enabled_rule_to_dry_run() {
    let (app, user) = authoring_app();
    let created = app
        .create_maintenance_rule_set(&user, draft(UNMONITOR_AND_SEARCH, 0, None))
        .await
        .expect("create declared rule");
    super::maintenance_rules::arm_rule(
        &app,
        &user,
        &created.rule_set.id,
        MaintenanceEffectArming::Reversible,
        None,
    )
    .await;

    let kept = app
        .update_maintenance_rule_matcher(&user, &created.rule_set.id, matcher(SEARCH_ONLY, 3))
        .await
        .expect("edit without a delete head");
    assert_eq!(
        crate::maintenance_rules::maintenance_rule_state(&kept),
        MaintenanceRuleState::Enabled,
        "an edit that cannot delete keeps the rule's state"
    );
    assert_eq!(
        kept.rule_set.effect_arming,
        MaintenanceEffectArming::Reversible
    );
    assert!(!kept.rule_set.destructive_rearm_required);

    let dropped = app
        .update_maintenance_rule_matcher(&user, &created.rule_set.id, matcher(DELETES_FILES, 3))
        .await
        .expect("edit to a delete head");
    assert_eq!(
        crate::maintenance_rules::maintenance_rule_state(&dropped),
        MaintenanceRuleState::DryRun,
        "a source that can delete needs a fresh confirmation"
    );
    assert_eq!(
        dropped.rule_set.effect_arming,
        MaintenanceEffectArming::None
    );
    assert!(dropped.rule_set.destructive_rearm_required);
}

/// An action document written before sources declared their actions does not
/// decode as declared actions, so the revision it belongs to can never be read
/// as a set of heads that run.
#[test]
fn a_stored_step_sequence_is_not_read_as_declared_actions() {
    let sequence = MaintenanceActionSequence::new(vec![
        MaintenanceActionStep {
            id: "unmonitor".to_string(),
            kind: MaintenanceActionStepKind::Unmonitor,
            parameters: MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        },
        MaintenanceActionStep {
            id: "search".to_string(),
            kind: MaintenanceActionStepKind::Search,
            parameters: MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::PreviousProfileChanged,
            },
        },
    ]);
    let stored = serde_json::to_string(&sequence).expect("encode");
    assert_eq!(
        MaintenanceDeclaredActions::from_persisted_json(&stored).expect("decode"),
        None
    );
    assert_eq!(
        MaintenanceDeclaredActions::from_persisted_json(r#"{"kind":"delete_title_and_files"}"#)
            .expect("decode"),
        None,
        "a v1 action document has no schema version and is not declared"
    );
    assert!(MaintenanceDeclaredActions::from_persisted_json("not json").is_err());

    let declared = MaintenanceDeclaredActions::none();
    assert_eq!(
        MaintenanceDeclaredActions::from_persisted_json(&declared.to_persisted_json())
            .expect("decode"),
        Some(declared)
    );
}

#[tokio::test]
async fn storage_scoped_rules_require_a_root_and_the_storage_safe_heads() {
    let (app, user) = authoring_app();
    let missing_root = app
        .create_maintenance_rule_set(&user, draft(STORAGE_DELETE, 3, None))
        .await
        .expect_err("storage facts require a selected root");
    assert!(
        missing_root.validation_reason().is_some(),
        "{missing_root:?}"
    );

    let root_id = scryer_domain::root_folder_id_for_path("/data/movies");
    let valid = app
        .create_maintenance_rule_set(&user, draft(STORAGE_DELETE, 3, Some(root_id.clone())))
        .await
        .expect("a filesystem-safe head can use the configured root");
    assert_eq!(
        valid.revision.storage_root_id.as_deref(),
        Some(root_id.as_str())
    );

    let unsupported = app
        .create_maintenance_rule_set(&user, draft(STORAGE_SEARCH, 3, Some(root_id)))
        .await
        .expect_err("acquisition dispatch cannot be attached to a storage root");
    assert_eq!(
        unsupported.validation_reason(),
        Some(crate::error_reasons::maintenance::ACTION_HEAD_UNSUPPORTED_FOR_SCOPE),
        "{unsupported:?}"
    );
}

/// An edit that removes every delete head leaves nothing armed for deletes:
/// the arming drops to what the new heads can arm, and a stale save is
/// refused with a stable code.
#[tokio::test]
async fn an_edit_without_delete_heads_drops_destructive_arming() {
    let (app, user) = authoring_app();
    let created = app
        .create_maintenance_rule_set(&user, draft(DELETES_FILES, 0, None))
        .await
        .expect("create delete rule");
    super::maintenance_rules::arm_rule(
        &app,
        &user,
        &created.rule_set.id,
        MaintenanceEffectArming::Destructive,
        Some(0),
    )
    .await;

    let edited = app
        .update_maintenance_rule_matcher(&user, &created.rule_set.id, matcher(SEARCH_ONLY, 0))
        .await
        .expect("edit away the delete head");
    assert_eq!(
        edited.rule_set.effect_arming,
        MaintenanceEffectArming::Reversible
    );
    let stored = app
        .get_maintenance_rule_set(&user, &created.rule_set.id)
        .await
        .expect("read rule")
        .expect("rule exists");
    assert_eq!(
        stored.rule_set.effect_arming,
        MaintenanceEffectArming::Reversible
    );

    // The revision already in force, saved again as if prepared before it.
    let stale = stored.revision.clone();
    let refused = app
        .services
        .customization
        .maintenance_rule_sets
        .add_revision_keeping_arming(
            &stale,
            MaintenanceEffectArming::Reversible,
            chrono::Utc::now(),
        )
        .await
        .expect_err("a save that does not follow the current revision is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::maintenance::RULE_REVISION_CONFLICT),
        "{refused:?}"
    );
}
