//! Service-boundary regression tests for the actions a rule declares.
//!
//! The executor has its own sequence tests. These cover the authoring boundary:
//! a source's heads become an immutable revision, an edit that can delete
//! invalidates the acknowledgement while any other edit keeps the rule's
//! state, storage rules use only the filesystem-safe heads, and revisions
//! stored with a step sequence still read.

use super::*;

use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::maintenance_rules::{
    ACTION_SEQUENCE_KIND, MaintenanceActionDefinition, MaintenanceActionSequence,
    MaintenanceActionStep, MaintenanceActionStepKind, MaintenanceActionStepParameters,
    MaintenanceMatcherDraft, MaintenanceRuleDraft, MaintenanceSearchCondition,
    action_definition_from_persisted_json,
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

fn declared_heads(definition: &MaintenanceActionDefinition) -> Vec<MaintenanceActionHead> {
    match definition {
        MaintenanceActionDefinition::Declared(declared) => declared.heads.clone(),
        other => panic!("expected declared actions, got {other:?}"),
    }
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
    let persisted = action_definition_from_persisted_json(&created.revision.action_spec_json)
        .expect("persisted action definition");
    assert_eq!(
        declared_heads(&persisted),
        vec![
            MaintenanceActionHead::Unmonitor,
            MaintenanceActionHead::Search
        ],
        "heads are recorded in source order"
    );
    assert_eq!(persisted, created.action_definition);

    let updated = app
        .update_maintenance_rule_matcher(&user, &created.rule_set.id, matcher(SEARCH_ONLY, 41))
        .await
        .expect("save a new revision");
    assert_eq!(updated.rule_set.current_revision_number, 2);
    assert_eq!(
        declared_heads(&updated.action_definition),
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
        declared_heads(
            &action_definition_from_persisted_json(&first.action_spec_json)
                .expect("decode original definition")
        ),
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

/// History written before sources declared their actions still decodes.
#[test]
fn a_stored_step_sequence_still_reads() {
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
    let stored = MaintenanceActionDefinition::Sequence(sequence.clone())
        .to_persisted_json()
        .expect("encode");
    assert_eq!(
        action_definition_from_persisted_json(&stored).expect("decode"),
        MaintenanceActionDefinition::Sequence(sequence)
    );

    let empty = MaintenanceActionDefinition::Sequence(MaintenanceActionSequence::new(Vec::new()));
    assert_eq!(empty.action_kind(), ACTION_SEQUENCE_KIND);
    assert_eq!(
        action_definition_from_persisted_json(&empty.to_persisted_json().expect("encode"))
            .expect("decode empty envelope"),
        empty
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
