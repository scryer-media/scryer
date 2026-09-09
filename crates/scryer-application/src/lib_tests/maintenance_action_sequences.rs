//! Service-boundary regression tests for v2 maintenance action definitions.
//!
//! The executor has its own sequence tests. These cover the authoring boundary:
//! an ordered definition becomes an immutable revision, changes invalidate an
//! acknowledgement, and storage rules use only the filesystem-safe subset.

use super::*;

use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::maintenance_rules::{
    ACTION_SEQUENCE_KIND, MaintenanceActionDefinition, MaintenanceActionSequence,
    MaintenanceActionStep, MaintenanceActionStepKind, MaintenanceActionStepParameters,
    MaintenanceMatcherDraft, MaintenanceRuleDraft, MaintenanceSearchCondition,
    action_definition_from_persisted_json,
};
use scryer_domain::{MaintenanceEffectArming, MaintenanceRuleSubjectKind, User};
use std::sync::Arc;

const MATCHER: &str = "package whatever\n\
    import rego.v1\n\n\
    match := true\n";

const STORAGE_MATCHER: &str = "package whatever\n\
    import rego.v1\n\n\
    match if {\n\
    \tinput.facts.storage_available_percent < 5\n\
    }\n";

fn authoring_app() -> (AppUseCase, User) {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let app =
        app.with_test_overrides(|services| services.with_maintenance_rule_set_store(rules.clone()));
    (app, user)
}

fn step(
    id: &str,
    kind: MaintenanceActionStepKind,
    parameters: MaintenanceActionStepParameters,
) -> MaintenanceActionStep {
    MaintenanceActionStep {
        id: id.to_string(),
        kind,
        parameters,
    }
}

fn profile_then_search_sequence() -> MaintenanceActionSequence {
    MaintenanceActionSequence::new(vec![
        step(
            "unmonitor",
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        ),
        step(
            "profile",
            MaintenanceActionStepKind::ChangeQualityProfile,
            MaintenanceActionStepParameters::ChangeQualityProfile {
                target_quality_profile_id: "archive".to_string(),
            },
        ),
        step(
            "search",
            MaintenanceActionStepKind::Search,
            MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::PreviousProfileChanged,
            },
        ),
    ])
}

fn reordered_profile_then_search_sequence() -> MaintenanceActionSequence {
    MaintenanceActionSequence::new(vec![
        step(
            "profile",
            MaintenanceActionStepKind::ChangeQualityProfile,
            MaintenanceActionStepParameters::ChangeQualityProfile {
                target_quality_profile_id: "archive".to_string(),
            },
        ),
        step(
            "search",
            MaintenanceActionStepKind::Search,
            MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::PreviousProfileChanged,
            },
        ),
        step(
            "unmonitor",
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        ),
    ])
}

fn storage_sequence() -> MaintenanceActionSequence {
    MaintenanceActionSequence::new(vec![
        step(
            "unmonitor",
            MaintenanceActionStepKind::Unmonitor,
            MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        ),
        step(
            "delete-files",
            MaintenanceActionStepKind::DeleteFiles,
            MaintenanceActionStepParameters::None,
        ),
    ])
}

fn draft(
    rego_source: &str,
    sequence: MaintenanceActionSequence,
    grace_days: i64,
    storage_root_id: Option<String>,
) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        subject_kind: MaintenanceRuleSubjectKind::Title,
        name: "Sequence rule".to_string(),
        description: "A v2 sequence fixture".to_string(),
        rego_source: rego_source.to_string(),
        action_definition: MaintenanceActionDefinition::Sequence(sequence),
        grace_days,
        storage_root_id,
        library_ids: Vec::new(),
        evaluation_mode: None,
    }
}

#[tokio::test]
async fn an_ordered_sequence_is_saved_as_an_immutable_revision_and_disarms_on_change() {
    let (app, user) = authoring_app();
    let first_sequence = profile_then_search_sequence();
    let created = app
        .create_maintenance_rule_set(&user, draft(MATCHER, first_sequence.clone(), 19, None))
        .await
        .expect("create sequence rule");

    assert!(!created.rule_set.enabled, "sequence rules ship disabled");
    assert_eq!(
        created.rule_set.effect_arming,
        MaintenanceEffectArming::None,
        "a saved sequence is initially unarmed"
    );
    assert_eq!(created.revision.grace_days, 19);
    assert_eq!(
        action_definition_from_persisted_json(&created.revision.action_spec_json)
            .expect("persisted action definition"),
        MaintenanceActionDefinition::Sequence(first_sequence.clone())
    );

    app.set_maintenance_rule_arming(
        &user,
        &created.rule_set.id,
        MaintenanceEffectArming::Reversible,
        None,
    )
    .await
    .expect("arm the reversible sequence");

    let reordered = reordered_profile_then_search_sequence();
    assert_ne!(
        first_sequence.content_hash().expect("first hash"),
        reordered.content_hash().expect("reordered hash"),
        "ordered step content is a revision-bound identity"
    );
    let updated = app
        .update_maintenance_rule_matcher(
            &user,
            &created.rule_set.id,
            MaintenanceMatcherDraft {
                rego_source: MATCHER.to_string(),
                action_definition: MaintenanceActionDefinition::Sequence(reordered.clone()),
                grace_days: 41,
                storage_root_id: None,
            },
        )
        .await
        .expect("save reordered sequence as a new revision");

    assert_eq!(updated.rule_set.current_revision_number, 2);
    assert!(
        !updated.rule_set.enabled,
        "a revision copy remains disabled"
    );
    assert_eq!(
        updated.rule_set.effect_arming,
        MaintenanceEffectArming::None
    );
    assert_eq!(updated.revision.grace_days, 41);
    assert_eq!(
        updated.revision.matcher_content_hash, created.revision.matcher_content_hash,
        "the matcher did not change; only the ordered action definition did"
    );
    assert_eq!(
        updated.action_definition,
        MaintenanceActionDefinition::Sequence(reordered.clone())
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
        action_definition_from_persisted_json(&first.action_spec_json)
            .expect("decode original definition"),
        MaintenanceActionDefinition::Sequence(first_sequence)
    );
}

#[tokio::test]
async fn an_empty_sequence_is_persisted_as_explicit_observe_only_configuration() {
    let (app, user) = authoring_app();
    let sequence = MaintenanceActionSequence::new(Vec::new());
    let created = app
        .create_maintenance_rule_set(&user, draft(MATCHER, sequence.clone(), 0, None))
        .await
        .expect("save explicit empty v2 sequence");

    assert_eq!(
        created.action_definition.action_kind(),
        ACTION_SEQUENCE_KIND
    );
    assert!(!created.rule_set.enabled);
    assert_eq!(
        created.rule_set.effect_arming,
        MaintenanceEffectArming::None
    );
    assert_eq!(
        action_definition_from_persisted_json(&created.revision.action_spec_json)
            .expect("decode empty envelope"),
        MaintenanceActionDefinition::Sequence(sequence)
    );
}

#[tokio::test]
async fn storage_scoped_sequences_require_a_root_and_the_storage_safe_step_subset() {
    let (app, user) = authoring_app();
    let missing_root = app
        .create_maintenance_rule_set(&user, draft(STORAGE_MATCHER, storage_sequence(), 3, None))
        .await
        .expect_err("storage facts require a selected root");
    assert!(
        missing_root
            .to_string()
            .contains("must select a configured library root"),
        "{missing_root}"
    );

    let root_id = scryer_domain::root_folder_id_for_path("/data/movies");
    let valid = app
        .create_maintenance_rule_set(
            &user,
            draft(
                STORAGE_MATCHER,
                storage_sequence(),
                3,
                Some(root_id.clone()),
            ),
        )
        .await
        .expect("filesystem-safe sequence can use the configured root");
    assert_eq!(
        valid.revision.storage_root_id.as_deref(),
        Some(root_id.as_str())
    );

    let search = MaintenanceActionSequence::new(vec![step(
        "search",
        MaintenanceActionStepKind::Search,
        MaintenanceActionStepParameters::Search {
            condition: MaintenanceSearchCondition::Unconditional,
        },
    )]);
    let unsupported = app
        .create_maintenance_rule_set(&user, draft(MATCHER, search, 3, Some(root_id)))
        .await
        .expect_err("acquisition dispatch cannot be attached to a storage root");
    assert!(
        unsupported
            .to_string()
            .contains("not allowed on a storage-root rule"),
        "{unsupported}"
    );
}
