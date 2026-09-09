use super::*;
use scryer_application::MaintenanceRuleSetRepository;
use scryer_domain::{
    MaintenanceEvaluationMode, MaintenanceRuleRevision, MaintenanceRuleSet,
    MaintenanceRuleSubjectKind,
};

pub(super) async fn seed_rule_libraries(
    datastore: &scryer_infrastructure_sql::runtime::StoreDatastore,
) {
    use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime};
    for id in ["library-1", "library-2", "library-9"] {
        SqlRuntime::execute_write(
            datastore,
            "seed_rule_library",
            "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
             VALUES ({}, 'movie', {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(id.into()),
                SqlArg::Text(id.into()),
                SqlArg::Text(id.into()),
                SqlArg::Bool(false),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
        .unwrap();
    }
}

async fn maintenance_rule_set_store(services: &SqliteServices) -> crate::MaintenanceRuleSetStore {
    seed_rule_libraries(&services.datastore()).await;
    crate::MaintenanceRuleSetStore::new(services.datastore())
}

#[tokio::test]
async fn normalized_library_scopes_preserve_intent_and_enforce_integrity() {
    let (services, db) = temp_services("scryer_maintenance_scope_integrity").await;
    let store = maintenance_rule_set_store(&services).await;
    let pool = services.pool();
    let scope = vec![
        "library-2".to_string(),
        "library-1".to_string(),
        "library-2".to_string(),
    ];
    store
        .create_rule_set(&rule_set("scoped", scope), &revision("scoped", 1))
        .await
        .unwrap();
    store
        .create_rule_set(&rule_set("global", vec![]), &revision("global", 1))
        .await
        .unwrap();
    let expected = vec!["library-2".to_string(), "library-1".to_string()];
    assert_eq!(
        store
            .get_rule_set("scoped")
            .await
            .unwrap()
            .unwrap()
            .library_ids,
        expected
    );
    let listed = store.list_rule_sets().await.unwrap();
    assert_eq!(listed.len(), 2, "joins must not duplicate rules");
    assert!(
        listed
            .iter()
            .find(|rule| rule.id == "global")
            .unwrap()
            .library_ids
            .is_empty()
    );
    assert_eq!(
        listed
            .iter()
            .find(|rule| rule.id == "scoped")
            .unwrap()
            .library_ids,
        expected
    );
    assert!(store.get_rule_set("absent").await.unwrap().is_none());

    let columns = sqlx::query("PRAGMA table_info(maintenance_rule_sets)")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(
        !columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "library_ids")
    );
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT library_id FROM maintenance_rule_set_libraries WHERE rule_set_id = 'scoped' ORDER BY position"
    ).fetch_all(pool).await.unwrap();
    assert_eq!(stored, expected);
    assert!(sqlx::query(
        "INSERT INTO maintenance_rule_set_libraries (rule_set_id, library_id, position) VALUES ('scoped', 'library-1', 9)"
    ).execute(pool).await.is_err(), "duplicate associations must be rejected");
    assert!(sqlx::query(
        "INSERT INTO maintenance_rule_set_libraries (rule_set_id, library_id, position) VALUES ('scoped', 'library-9', 0)"
    ).execute(pool).await.is_err(), "positions must be unique within a rule");

    let bad = rule_set("bad", vec!["library-1".into(), "missing-library".into()]);
    assert!(
        store
            .create_rule_set(&bad, &revision("bad", 1))
            .await
            .is_err()
    );
    assert!(store.get_rule_set("bad").await.unwrap().is_none());
    assert!(store.list_revisions("bad").await.unwrap().is_empty());
    store
        .update_rule_set_arming(
            "scoped",
            scryer_domain::MaintenanceEffectArming::Destructive,
            Utc::now(),
        )
        .await
        .unwrap();
    let before = store.get_rule_set("scoped").await.unwrap().unwrap();
    assert!(
        store
            .update_rule_set_metadata(
                "scoped",
                "Must roll back",
                "bad scope",
                &["library-9".into(), "missing-library".into()],
                true,
                Utc::now()
            )
            .await
            .is_err()
    );
    let unchanged = store.get_rule_set("scoped").await.unwrap().unwrap();
    assert_eq!(
        unchanged.library_ids, expected,
        "failed replacement must preserve every scope row"
    );
    assert_eq!(unchanged.name, before.name);
    assert_eq!(unchanged.updated_at, before.updated_at);
    assert_eq!(
        unchanged.effect_arming,
        scryer_domain::MaintenanceEffectArming::Destructive
    );

    store
        .update_rule_set_metadata(
            "scoped",
            "Scoped",
            "",
            &["library-1".into()],
            true,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_rule_set("scoped")
            .await
            .unwrap()
            .unwrap()
            .effect_arming,
        scryer_domain::MaintenanceEffectArming::None
    );
    assert!(
        sqlx::query("DELETE FROM libraries WHERE id = 'library-1'")
            .execute(pool)
            .await
            .is_err(),
        "deleting the final scoped library must not silently make the rule global"
    );
    assert_eq!(
        store
            .get_rule_set("scoped")
            .await
            .unwrap()
            .unwrap()
            .library_ids,
        vec!["library-1"]
    );

    store
        .update_rule_set_metadata("scoped", "Global", "", &[], true, Utc::now())
        .await
        .unwrap();
    assert!(
        store
            .get_rule_set("scoped")
            .await
            .unwrap()
            .unwrap()
            .library_ids
            .is_empty()
    );
    sqlx::query("DELETE FROM libraries WHERE id = 'library-1'")
        .execute(pool)
        .await
        .unwrap();
    store
        .update_rule_set_metadata(
            "scoped",
            "Scoped again",
            "",
            &["library-2".into()],
            true,
            Utc::now(),
        )
        .await
        .unwrap();
    store.delete_rule_set("scoped").await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM maintenance_rule_set_libraries")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        remaining, 0,
        "deleting a rule must cascade its associations"
    );
    let _ = std::fs::remove_file(db);
}

fn rule_set(id: &str, library_ids: Vec<String>) -> MaintenanceRuleSet {
    let now = Utc::now();
    MaintenanceRuleSet {
        id: id.to_string(),
        name: "Stale movies".to_string(),
        description: "Unwatched for a long time".to_string(),
        enabled: false,
        evaluation_mode: MaintenanceEvaluationMode::Disabled,
        effect_arming: scryer_domain::MaintenanceEffectArming::None,
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
    let store = maintenance_rule_set_store(&services).await;

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
    let store = maintenance_rule_set_store(&services).await;

    store
        .create_rule_set(&rule_set("rule-b", Vec::new()), &revision("rule-b", 1))
        .await
        .expect("create rule set");

    store
        .update_rule_set_arming(
            "rule-b",
            scryer_domain::MaintenanceEffectArming::Destructive,
            Utc::now(),
        )
        .await
        .expect("arm the rule against revision 1");

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
    // The pointer move and the disarm are one write: arming acknowledges one
    // matcher's blast radius, so it must not carry over to the next revision.
    assert_eq!(
        loaded.effect_arming,
        scryer_domain::MaintenanceEffectArming::None,
        "appending a revision must disarm the rule"
    );

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
    let store = maintenance_rule_set_store(&services).await;

    let mut armed = rule_set("rule-c", Vec::new());
    armed.effect_arming = scryer_domain::MaintenanceEffectArming::Reversible;
    store
        .create_rule_set(&armed, &revision("rule-c", 1))
        .await
        .expect("create rule set");

    store
        .update_rule_set_metadata(
            "rule-c",
            "Renamed",
            "New description",
            &["library-9".to_string()],
            false,
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
    // The caller decides whether the edit invalidated the arming; the store must
    // not clear it on its own.
    assert_eq!(
        loaded.effect_arming,
        scryer_domain::MaintenanceEffectArming::Reversible
    );
    assert_eq!(store.list_revisions("rule-c").await.unwrap().len(), 1);

    let _ = std::fs::remove_file(db);
}

/// The disarm and the scope it invalidates are one write, exactly as the
/// revision pointer and its disarm are: there must be no instant at which the
/// new scope is in force under the previous scope's arming.
#[tokio::test]
async fn a_scope_change_disarms_in_the_same_write() {
    let (services, db) = temp_services("scryer_maintenance_rescope").await;
    let store = maintenance_rule_set_store(&services).await;

    let mut armed = rule_set("rule-f", vec!["library-1".to_string()]);
    armed.effect_arming = scryer_domain::MaintenanceEffectArming::Destructive;
    store
        .create_rule_set(&armed, &revision("rule-f", 1))
        .await
        .expect("create rule set");

    store
        .update_rule_set_metadata(
            "rule-f",
            "Stale movies",
            "Unwatched for a long time",
            &["library-1".to_string(), "library-2".to_string()],
            true,
            Utc::now(),
        )
        .await
        .expect("re-scope");

    let loaded = store
        .get_rule_set("rule-f")
        .await
        .unwrap()
        .expect("rule set exists");
    assert_eq!(
        loaded.library_ids,
        vec!["library-1".to_string(), "library-2".to_string()]
    );
    assert_eq!(
        loaded.effect_arming,
        scryer_domain::MaintenanceEffectArming::None,
        "a widened scope must never stay armed under the narrower acknowledgement"
    );
    assert_eq!(loaded.current_revision_number, 1);

    let _ = std::fs::remove_file(db);
}

/// The FK cascade is what keeps deleted rules from leaving orphan revisions
/// behind; SQLite only enforces it when foreign keys are on, so this asserts
/// the deployed configuration, not just the DDL.
#[tokio::test]
async fn deleting_a_rule_set_cascades_to_its_revisions() {
    let (services, db) = temp_services("scryer_maintenance_delete").await;
    let store = maintenance_rule_set_store(&services).await;

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
    let store = maintenance_rule_set_store(&services).await;

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
