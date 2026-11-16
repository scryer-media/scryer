//! Service-boundary regression tests for the actions a rule declares.
//!
//! The executor has its own sequence tests. These cover the authoring boundary:
//! a source's heads become an immutable revision, an edit that can delete
//! invalidates the acknowledgement while any other edit keeps the rule's
//! state, storage rules use only the filesystem-safe heads, and a revision
//! stored before sources declared their actions is refused rather than read.

use super::*;

use crate::lib_tests::lifecycle_rules::InMemoryLifecycleRuleRepo;
use crate::lifecycle_rules::{
    LifecycleActionSequence, LifecycleActionStep, LifecycleActionStepKind,
    LifecycleActionStepParameters, LifecycleDeclaredActions, LifecycleRuleDraft,
    LifecycleSearchCondition, LifecycleSourceDraft,
};
use scryer_domain::{LifecycleEffectArming, LifecycleRuleState, User};
use scryer_rules::lifecycle_actions::LifecycleActionHead;
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
    \tsome root in input.facts.storage_roots\n\
    \troot.available_percent < 5\n\
    }\n";

const STORAGE_SEARCH: &str = "package whatever\n\
    import rego.v1\n\n\
    search if {\n\
    \tsome root in input.facts.storage_roots\n\
    \troot.available_percent < 5\n\
    }\n";

fn authoring_app() -> (AppUseCase, User) {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryLifecycleRuleRepo::default());
    let app =
        app.with_test_overrides(|services| services.with_lifecycle_rule_set_store(rules.clone()));
    (app, user)
}

fn draft(rego_source: &str, grace_days: i64) -> LifecycleRuleDraft {
    LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        name: "Declared rule".to_string(),
        description: "A declared-actions fixture".to_string(),
        rego_source: rego_source.to_string(),
        grace_days,
        library_ids: Vec::new(),
    }
}

fn source_draft(rego_source: &str, grace_days: i64) -> LifecycleSourceDraft {
    LifecycleSourceDraft {
        rego_source: rego_source.to_string(),
        grace_days,
        max_titles_per_run: None,
    }
}

fn persisted_heads(action_spec_json: &str) -> Vec<LifecycleActionHead> {
    LifecycleDeclaredActions::from_persisted_json(action_spec_json)
        .expect("persisted actions decode")
        .expect("persisted actions are declared")
        .heads
}

#[tokio::test]
async fn declared_heads_are_saved_as_an_immutable_revision() {
    let (app, user) = authoring_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(UNMONITOR_AND_SEARCH, 19))
        .await
        .expect("create declared rule");

    assert!(!created.rule_set.enabled, "new rules ship disabled");
    assert_eq!(created.rule_set.effect_arming, LifecycleEffectArming::None);
    assert_eq!(created.revision.grace_days, 19);
    assert_eq!(created.load_refusal, None);
    assert_eq!(
        persisted_heads(&created.revision.action_spec_json),
        vec![LifecycleActionHead::Unmonitor, LifecycleActionHead::Search],
        "heads are recorded in source order"
    );
    assert_eq!(
        created.declared_actions.heads,
        persisted_heads(&created.revision.action_spec_json)
    );

    let updated = app
        .update_lifecycle_rule_source(&user, &created.rule_set.id, source_draft(SEARCH_ONLY, 41))
        .await
        .expect("save a new revision");
    assert_eq!(updated.rule_set.current_revision_number, 2);
    assert_eq!(
        updated.declared_actions.heads.clone(),
        vec![LifecycleActionHead::Search]
    );

    let revisions = app
        .list_lifecycle_rule_revisions(&user, &created.rule_set.id)
        .await
        .expect("list immutable revisions");
    let first = revisions
        .iter()
        .find(|revision| revision.revision_number == 1)
        .expect("first revision remains available");
    assert_eq!(first.grace_days, 19);
    assert_eq!(
        persisted_heads(&first.action_spec_json),
        vec![LifecycleActionHead::Unmonitor, LifecycleActionHead::Search]
    );
}

/// The acknowledgement an Enabled rule carries is bound to its delete heads:
/// an edit to a source that can delete drops the rule to Dry run, and an
/// edit to one that cannot keeps the rule as it was.
#[tokio::test]
async fn only_an_edit_that_can_delete_drops_an_enabled_rule_to_dry_run() {
    let (app, user) = authoring_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(UNMONITOR_AND_SEARCH, 0))
        .await
        .expect("create declared rule");
    super::lifecycle_rules::arm_rule(
        &app,
        &user,
        &created.rule_set.id,
        LifecycleEffectArming::Reversible,
        None,
    )
    .await;

    let kept = app
        .update_lifecycle_rule_source(&user, &created.rule_set.id, source_draft(SEARCH_ONLY, 3))
        .await
        .expect("edit without a delete head");
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&kept),
        LifecycleRuleState::Enabled,
        "an edit that cannot delete keeps the rule's state"
    );
    assert_eq!(
        kept.rule_set.effect_arming,
        LifecycleEffectArming::Reversible
    );
    assert!(!kept.rule_set.destructive_rearm_required);

    let dropped = app
        .update_lifecycle_rule_source(&user, &created.rule_set.id, source_draft(DELETES_FILES, 3))
        .await
        .expect("edit to a delete head");
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&dropped),
        LifecycleRuleState::DryRun,
        "a source that can delete needs a fresh confirmation"
    );
    assert_eq!(dropped.rule_set.effect_arming, LifecycleEffectArming::None);
    assert!(dropped.rule_set.destructive_rearm_required);
}

/// An action document written before sources declared their actions does not
/// decode as declared actions, so the revision it belongs to can never be read
/// as a set of heads that run.
#[test]
fn a_stored_step_sequence_is_not_read_as_declared_actions() {
    let sequence = LifecycleActionSequence::new(vec![
        LifecycleActionStep {
            id: "unmonitor".to_string(),
            kind: LifecycleActionStepKind::Unmonitor,
            parameters: LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        },
        LifecycleActionStep {
            id: "search".to_string(),
            kind: LifecycleActionStepKind::Search,
            parameters: LifecycleActionStepParameters::Search {
                condition: LifecycleSearchCondition::PreviousProfileChanged,
            },
        },
    ]);
    let stored = serde_json::to_string(&sequence).expect("encode");
    assert_eq!(
        LifecycleDeclaredActions::from_persisted_json(&stored).expect("decode"),
        None
    );
    assert_eq!(
        LifecycleDeclaredActions::from_persisted_json(r#"{"kind":"delete_title_and_files"}"#)
            .expect("decode"),
        None,
        "a v1 action document has no schema version and is not declared"
    );
    assert!(LifecycleDeclaredActions::from_persisted_json("not json").is_err());

    let declared = LifecycleDeclaredActions::none();
    assert_eq!(
        LifecycleDeclaredActions::from_persisted_json(&declared.to_persisted_json())
            .expect("decode"),
        Some(declared)
    );
}

fn storage_rule(head: &str) -> String {
    format!(
        "package whatever\nimport rego.v1\n\n{head} if {{\n\tsome root in input.facts.storage_roots\n\troot.available_percent < 5\n}}\n"
    )
}

/// Storage facts read every root a subject's files are on, so a rule that
/// reads them names no root and may define any head, deletes included.
#[tokio::test]
async fn storage_rules_save_without_a_root_and_with_any_head() {
    let (app, user) = authoring_app();
    app.create_lifecycle_rule_set(&user, draft(STORAGE_DELETE, 3))
        .await
        .expect("a storage rule names no root");
    app.create_lifecycle_rule_set(&user, draft(STORAGE_SEARCH, 3))
        .await
        .expect("a storage rule may search");
    for head in [
        "unmonitor",
        "delete_title",
        "monitor",
        "unmonitor_season",
        "delete_season_files",
        "delete_episode_files",
        "fetch_episode_subtitles",
    ] {
        app.create_lifecycle_rule_set(&user, draft(&storage_rule(head), 3))
            .await
            .unwrap_or_else(|error| panic!("a storage rule may define {head}: {error:?}"));
    }
}

/// An edit that removes every delete head leaves nothing armed for deletes:
/// the arming drops to what the new heads can arm, and a stale save is
/// refused with a stable code.
#[tokio::test]
async fn an_edit_without_delete_heads_drops_destructive_arming() {
    let (app, user) = authoring_app();
    let created = app
        .create_lifecycle_rule_set(&user, draft(DELETES_FILES, 0))
        .await
        .expect("create delete rule");
    super::lifecycle_rules::arm_rule(
        &app,
        &user,
        &created.rule_set.id,
        LifecycleEffectArming::Destructive,
        Some(0),
    )
    .await;

    let edited = app
        .update_lifecycle_rule_source(&user, &created.rule_set.id, source_draft(SEARCH_ONLY, 0))
        .await
        .expect("edit away the delete head");
    assert_eq!(
        edited.rule_set.effect_arming,
        LifecycleEffectArming::Reversible
    );
    let stored = app
        .get_lifecycle_rule_set(&user, &created.rule_set.id)
        .await
        .expect("read rule")
        .expect("rule exists");
    assert_eq!(
        stored.rule_set.effect_arming,
        LifecycleEffectArming::Reversible
    );

    // The revision already in force, saved again as if prepared before it.
    let stale = stored.revision.clone();
    let refused = app
        .services
        .customization
        .lifecycle_rule_sets
        .add_revision_keeping_arming(
            &stale,
            LifecycleEffectArming::Reversible,
            None,
            chrono::Utc::now(),
        )
        .await
        .expect_err("a save that does not follow the current revision is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::lifecycle::RULE_REVISION_CONFLICT),
        "{refused:?}"
    );
}
