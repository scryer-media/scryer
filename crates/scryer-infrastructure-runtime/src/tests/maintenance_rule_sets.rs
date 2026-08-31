use super::*;
use scryer_application::MaintenanceRuleSetRepository;
use scryer_domain::{
    MaintenanceEvaluationMode, MaintenanceRuleRevision, MaintenanceRuleSet,
    MaintenanceRuleSubjectKind,
};

fn maintenance_rule_set_store(services: &SqliteServices) -> crate::MaintenanceRuleSetStore {
    crate::MaintenanceRuleSetStore::new(services.datastore())
}

fn rule_set(id: &str, library_ids: Vec<String>) -> MaintenanceRuleSet {
    let now = Utc::now();
    MaintenanceRuleSet {
        id: id.to_string(),
        name: "Stale movies".to_string(),
        description: "Unwatched for a long time".to_string(),
        enabled: false,
        evaluation_mode: MaintenanceEvaluationMode::Disabled,
        library_ids,
        subject_kind: MaintenanceRuleSubjectKind::Title,
        current_revision_number: 1,
        created_at: now,
        updated_at: now,
    }
}

fn revision(rule_set_id: &str, number: i64) -> MaintenanceRuleRevision {
    MaintenanceRuleRevision {
        id: format!("{rule_set_id}-rev-{number}"),
        rule_set_id: rule_set_id.to_string(),
        revision_number: number,
        rego_source: format!("package scryer.maintenance.user.{rule_set_id}\nmatch := true\n"),
        action_spec_json: r#"{"kind":"unmonitor_scope_keep_files","schema_version":1}"#.to_string(),
        grace_days: 7 * number,
        matcher_content_hash: format!("hash-{number}"),
        created_by: Some("user-1".to_string()),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn rule_sets_and_revisions_round_trip() {
    let (services, db) = temp_services("scryer_maintenance_rules").await;
    let store = maintenance_rule_set_store(&services);

    let created = rule_set("rule-a", vec!["library-1".to_string()]);
    store
        .create_rule_set(&created, &revision("rule-a", 1))
        .await
        .expect("create rule set");

    let loaded = store
        .get_rule_set("rule-a")
        .await
        .expect("read rule set")
        .expect("rule set exists");
    assert_eq!(loaded.name, created.name);
    assert!(!loaded.enabled);
    assert_eq!(loaded.evaluation_mode, MaintenanceEvaluationMode::Disabled);
    assert_eq!(loaded.subject_kind, MaintenanceRuleSubjectKind::Title);
    assert_eq!(loaded.library_ids, vec!["library-1".to_string()]);
    assert_eq!(loaded.current_revision_number, 1);

    let stored_revision = store
        .get_revision("rule-a", 1)
        .await
        .expect("read revision")
        .expect("revision exists");
    let expected = revision("rule-a", 1);
    assert_eq!(stored_revision.rego_source, expected.rego_source);
    assert_eq!(stored_revision.action_spec_json, expected.action_spec_json);
    assert_eq!(stored_revision.matcher_content_hash, "hash-1");
    assert_eq!(stored_revision.grace_days, 7);
    assert_eq!(stored_revision.created_by.as_deref(), Some("user-1"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn adding_a_revision_repoints_the_rule_set_and_preserves_the_old_one() {
    let (services, db) = temp_services("scryer_maintenance_revisions").await;
    let store = maintenance_rule_set_store(&services);

    store
        .create_rule_set(&rule_set("rule-b", Vec::new()), &revision("rule-b", 1))
        .await
        .expect("create rule set");

    let updated_at = Utc::now();
    store
        .add_revision(&revision("rule-b", 2), updated_at)
        .await
        .expect("add revision");

    let loaded = store
        .get_rule_set("rule-b")
        .await
        .unwrap()
        .expect("rule set exists");
    assert_eq!(loaded.current_revision_number, 2);

    let first = store
        .get_revision("rule-b", 1)
        .await
        .unwrap()
        .expect("revision 1 survives");
    assert_eq!(first.grace_days, 7, "revision 1 must not be rewritten");

    let all = store
        .list_revisions("rule-b")
        .await
        .expect("list revisions");
    assert_eq!(
        all.iter()
            .map(|revision| revision.revision_number)
            .collect::<Vec<_>>(),
        vec![2, 1],
        "newest revision first"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn metadata_updates_leave_the_revision_pointer_alone() {
    let (services, db) = temp_services("scryer_maintenance_metadata").await;
    let store = maintenance_rule_set_store(&services);

    store
        .create_rule_set(&rule_set("rule-c", Vec::new()), &revision("rule-c", 1))
        .await
        .expect("create rule set");

    store
        .update_rule_set_metadata(
            "rule-c",
            "Renamed",
            "New description",
            &["library-9".to_string()],
            Utc::now(),
        )
        .await
        .expect("update metadata");

    let loaded = store
        .get_rule_set("rule-c")
        .await
        .unwrap()
        .expect("rule set exists");
    assert_eq!(loaded.name, "Renamed");
    assert_eq!(loaded.description, "New description");
    assert_eq!(loaded.library_ids, vec!["library-9".to_string()]);
    assert_eq!(loaded.current_revision_number, 1);
    assert_eq!(store.list_revisions("rule-c").await.unwrap().len(), 1);

    let _ = std::fs::remove_file(db);
}

/// The FK cascade is what keeps deleted rules from leaving orphan revisions
/// behind; SQLite only enforces it when foreign keys are on, so this asserts
/// the deployed configuration, not just the DDL.
#[tokio::test]
async fn deleting_a_rule_set_cascades_to_its_revisions() {
    let (services, db) = temp_services("scryer_maintenance_delete").await;
    let store = maintenance_rule_set_store(&services);

    store
        .create_rule_set(&rule_set("rule-d", Vec::new()), &revision("rule-d", 1))
        .await
        .expect("create rule set");
    store
        .add_revision(&revision("rule-d", 2), Utc::now())
        .await
        .expect("add revision");

    store.delete_rule_set("rule-d").await.expect("delete");

    assert!(store.get_rule_set("rule-d").await.unwrap().is_none());
    assert!(store.list_revisions("rule-d").await.unwrap().is_empty());

    let _ = std::fs::remove_file(db);
}

/// A revision number is unique per rule set: replaying the same number must be
/// rejected rather than silently duplicating history.
#[tokio::test]
async fn a_duplicate_revision_number_is_rejected() {
    let (services, db) = temp_services("scryer_maintenance_duplicate").await;
    let store = maintenance_rule_set_store(&services);

    store
        .create_rule_set(&rule_set("rule-e", Vec::new()), &revision("rule-e", 1))
        .await
        .expect("create rule set");

    store
        .add_revision(&revision("rule-e", 1), Utc::now())
        .await
        .expect_err("the unique constraint must reject a replayed revision number");

    assert_eq!(store.list_revisions("rule-e").await.unwrap().len(), 1);

    let _ = std::fs::remove_file(db);
}
