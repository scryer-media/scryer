use super::*;
use scryer_application::LifecycleRuleSetRepository;
use scryer_domain::{LifecycleRuleRevision, LifecycleRuleSet, LifecycleRuleState};

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

async fn lifecycle_rule_set_store(services: &SqliteServices) -> crate::LifecycleRuleSetStore {
    seed_rule_libraries(&services.datastore()).await;
    crate::LifecycleRuleSetStore::new(services.datastore())
}

#[tokio::test]
async fn destructive_arming_atomically_rechecks_the_reviewed_configuration() {
    use scryer_domain::LifecycleRuleArmingConfirmation;
    let temp = tempfile::tempdir().unwrap();
    let services =
        SqliteServices::new(temp.path().join("arming.db").to_string_lossy().into_owned())
            .await
            .unwrap();
    let store = lifecycle_rule_set_store(&services).await;
    let original = rule_set("reviewed", vec!["library-1".into()]);
    store
        .create_rule_set(&original, &revision("reviewed", 1))
        .await
        .unwrap();
    let reviewed = LifecycleRuleArmingConfirmation::from(&original);
    // A state write without a confirmation stores none: the row is enabled
    // but holds nothing that lets a deleting step run.
    assert!(
        store
            .update_rule_set_state("reviewed", LifecycleRuleState::Enabled, None, Utc::now())
            .await
            .unwrap()
    );
    let unconfirmed = store.get_rule_set("reviewed").await.unwrap().unwrap();
    assert_eq!(unconfirmed.destructive_confirmed_revision, None);
    assert!(!unconfirmed.destructive_confirmed());
    assert!(
        store
            .update_rule_set_state(
                "reviewed",
                LifecycleRuleState::Enabled,
                Some(&reviewed),
                Utc::now()
            )
            .await
            .unwrap()
    );

    // The application may have validated the old snapshot before this edit.
    // Hold the writer transaction across the arming attempt to exercise the
    // serialization boundary, then commit the new scope without a revision bump.
    let mut edit = services.pool().begin().await.unwrap();
    sqlx::query("UPDATE lifecycle_rule_sets SET state = 'dry_run', destructive_confirmed_revision = NULL WHERE id = 'reviewed'")
        .execute(&mut *edit)
        .await
        .unwrap();
    sqlx::query("UPDATE lifecycle_rule_set_libraries SET library_id = 'library-2' WHERE rule_set_id = 'reviewed'")
        .execute(&mut *edit).await.unwrap();
    let arming = store.update_rule_set_state(
        "reviewed",
        LifecycleRuleState::Enabled,
        Some(&reviewed),
        Utc::now(),
    );
    tokio::pin!(arming);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut arming)
            .await
            .is_err()
    );
    edit.commit().await.unwrap();
    assert!(
        !arming.await.unwrap(),
        "a stale acknowledgement must lose to the committed scope edit"
    );
    let current = store.get_rule_set("reviewed").await.unwrap().unwrap();
    assert_eq!(current.state, LifecycleRuleState::DryRun);
    assert_eq!(current.destructive_confirmed_revision, None);
    assert_eq!(current.library_ids, vec!["library-2"]);
    let reviewed = LifecycleRuleArmingConfirmation::from(&current);
    assert!(
        store
            .update_rule_set_state(
                "reviewed",
                LifecycleRuleState::Enabled,
                Some(&reviewed),
                Utc::now()
            )
            .await
            .unwrap()
    );

    store
        .add_revision(&revision("reviewed", 2), None, Utc::now())
        .await
        .unwrap();
    assert!(
        !store
            .update_rule_set_state(
                "reviewed",
                LifecycleRuleState::Enabled,
                Some(&reviewed),
                Utc::now()
            )
            .await
            .unwrap()
    );
    let after_revision = store.get_rule_set("reviewed").await.unwrap().unwrap();
    assert_eq!(after_revision.state, LifecycleRuleState::DryRun);
    assert_eq!(after_revision.destructive_confirmed_revision, None);
    assert!(
        store
            .update_rule_set_state("reviewed", LifecycleRuleState::DryRun, None, Utc::now())
            .await
            .unwrap()
    );
}

#[tokio::test]
#[ignore = "requires SCRYER_TEST_POSTGRES_URL pointing to an isolated PostgreSQL fixture"]
async fn postgres_destructive_arming_waits_for_scope_edits_before_checking_confirmation() {
    use scryer_domain::LifecycleRuleArmingConfirmation;
    let raw_url =
        std::env::var("SCRYER_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture URL");
    let admin = sqlx::PgPool::connect(&raw_url).await.unwrap();
    let schema = format!("lifecycle_arming_{}", Utc::now().timestamp_micros());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let mut url = url::Url::parse(&raw_url).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let services = scryer_infrastructure_datastore::postgres::PostgresServices::new_with_mode(
        url.as_str(),
        scryer_infrastructure_datastore::MigrationMode::Apply,
    )
    .await
    .unwrap();
    seed_rule_libraries(&services.datastore()).await;
    let store = crate::LifecycleRuleSetStore::new(services.datastore());
    let original = rule_set("reviewed", vec!["library-1".into()]);
    store
        .create_rule_set(&original, &revision("reviewed", 1))
        .await
        .unwrap();
    let reviewed = LifecycleRuleArmingConfirmation::from(&original);
    // A state write without a confirmation stores none: the row is enabled
    // but holds nothing that lets a deleting step run.
    assert!(
        store
            .update_rule_set_state("reviewed", LifecycleRuleState::Enabled, None, Utc::now())
            .await
            .unwrap()
    );
    let unconfirmed = store.get_rule_set("reviewed").await.unwrap().unwrap();
    assert_eq!(unconfirmed.destructive_confirmed_revision, None);
    assert!(!unconfirmed.destructive_confirmed());
    assert!(
        store
            .update_rule_set_state(
                "reviewed",
                LifecycleRuleState::Enabled,
                Some(&reviewed),
                Utc::now()
            )
            .await
            .unwrap()
    );

    let mut edit = services.pool().begin().await.unwrap();
    sqlx::query("UPDATE lifecycle_rule_sets SET state = 'dry_run', destructive_confirmed_revision = NULL WHERE id = 'reviewed'")
        .execute(&mut *edit)
        .await
        .unwrap();
    sqlx::query("UPDATE lifecycle_rule_set_libraries SET library_id = 'library-2' WHERE rule_set_id = 'reviewed'")
        .execute(&mut *edit).await.unwrap();
    let arming = store.update_rule_set_state(
        "reviewed",
        LifecycleRuleState::Enabled,
        Some(&reviewed),
        Utc::now(),
    );
    tokio::pin!(arming);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut arming)
            .await
            .is_err()
    );
    edit.commit().await.unwrap();
    assert!(
        !arming.await.unwrap(),
        "the post-lock read must see the new PostgreSQL scope"
    );
    let current = store.get_rule_set("reviewed").await.unwrap().unwrap();
    assert_eq!(current.state, LifecycleRuleState::DryRun);
    assert_eq!(current.destructive_confirmed_revision, None);
    let reviewed = LifecycleRuleArmingConfirmation::from(&current);
    assert!(
        store
            .update_rule_set_state(
                "reviewed",
                LifecycleRuleState::Enabled,
                Some(&reviewed),
                Utc::now()
            )
            .await
            .unwrap()
    );
    store
        .add_revision(&revision("reviewed", 2), None, Utc::now())
        .await
        .unwrap();
    assert!(
        !store
            .update_rule_set_state(
                "reviewed",
                LifecycleRuleState::Enabled,
                Some(&reviewed),
                Utc::now()
            )
            .await
            .unwrap()
    );
    assert!(
        store
            .update_rule_set_state("reviewed", LifecycleRuleState::DryRun, None, Utc::now())
            .await
            .unwrap()
    );
    services.pool().close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
async fn normalized_library_scopes_preserve_intent_and_enforce_integrity() {
    let (services, db) = temp_services("scryer_lifecycle_scope_integrity").await;
    let store = lifecycle_rule_set_store(&services).await;
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

    let columns = sqlx::query("PRAGMA table_info(lifecycle_rule_sets)")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(
        !columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "library_ids")
    );
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT library_id FROM lifecycle_rule_set_libraries WHERE rule_set_id = 'scoped' ORDER BY position"
    ).fetch_all(pool).await.unwrap();
    assert_eq!(stored, expected);
    assert!(sqlx::query(
        "INSERT INTO lifecycle_rule_set_libraries (rule_set_id, library_id, position) VALUES ('scoped', 'library-1', 9)"
    ).execute(pool).await.is_err(), "duplicate associations must be rejected");
    assert!(sqlx::query(
        "INSERT INTO lifecycle_rule_set_libraries (rule_set_id, library_id, position) VALUES ('scoped', 'library-9', 0)"
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
        .update_rule_set_state(
            "scoped",
            LifecycleRuleState::Enabled,
            Some(&scryer_domain::LifecycleRuleArmingConfirmation {
                revision_number: 1,
                library_ids: expected.clone(),
            }),
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
                None,
                None,
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
    assert_eq!(unchanged.state, LifecycleRuleState::Enabled);
    assert_eq!(unchanged.destructive_confirmed_revision, Some(1));

    store
        .update_rule_set_metadata(
            "scoped",
            "Scoped",
            "",
            &["library-1".into()],
            None,
            None,
            true,
            Utc::now(),
        )
        .await
        .unwrap();
    let rescoped = store.get_rule_set("scoped").await.unwrap().unwrap();
    assert_eq!(rescoped.state, LifecycleRuleState::DryRun);
    assert_eq!(rescoped.destructive_confirmed_revision, None);
    assert!(rescoped.edited_since_enabled);
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
        .update_rule_set_metadata("scoped", "Global", "", &[], None, None, true, Utc::now())
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
            None,
            None,
            true,
            Utc::now(),
        )
        .await
        .unwrap();
    store.delete_rule_set("scoped").await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lifecycle_rule_set_libraries")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        remaining, 0,
        "deleting a rule must cascade its associations"
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_rule_maximum_round_trips_and_clears_without_touching_state() {
    let (services, db) = temp_services("scryer_lifecycle_rule_maximum").await;
    let store = lifecycle_rule_set_store(&services).await;
    let mut seeded = rule_set("bounded", vec![]);
    seeded.max_titles_per_run = Some(7);
    seeded.state = LifecycleRuleState::Enabled;
    store
        .create_rule_set(&seeded, &revision("bounded", 1))
        .await
        .unwrap();
    let read = store.get_rule_set("bounded").await.unwrap().unwrap();
    assert_eq!(read.max_titles_per_run, Some(7));

    store
        .set_rule_set_max_titles_per_run("bounded", Some(100_000), Utc::now())
        .await
        .unwrap();
    let raised = store.get_rule_set("bounded").await.unwrap().unwrap();
    assert_eq!(raised.max_titles_per_run, Some(100_000));
    assert_eq!(raised.state, LifecycleRuleState::Enabled);

    store
        .set_rule_set_max_titles_per_run("bounded", None, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        store
            .get_rule_set("bounded")
            .await
            .unwrap()
            .unwrap()
            .max_titles_per_run,
        None
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_rule_maximum_rides_in_the_revision_and_metadata_writes() {
    let (services, db) = temp_services("scryer_lifecycle_rule_maximum_writes").await;
    let store = lifecycle_rule_set_store(&services).await;
    store
        .create_rule_set(&rule_set("bounded", vec![]), &revision("bounded", 1))
        .await
        .unwrap();
    store
        .add_revision(&revision("bounded", 2), Some(Some(3)), Utc::now())
        .await
        .unwrap();
    assert_eq!(stored_maximum(&store).await, Some(3));
    store
        .add_revision_keeping_state(&revision("bounded", 3), false, false, None, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        stored_maximum(&store).await,
        Some(3),
        "omitted keeps the maximum"
    );
    store
        .add_revision_keeping_state(
            &revision("bounded", 4),
            false,
            false,
            Some(Some(9)),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(stored_maximum(&store).await, Some(9));

    store
        .update_rule_set_metadata(
            "bounded",
            "Bounded",
            "",
            &[],
            None,
            Some(Some(11)),
            false,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(stored_maximum(&store).await, Some(11));
    store
        .update_rule_set_metadata(
            "bounded",
            "Bounded",
            "",
            &[],
            None,
            Some(None),
            true,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(stored_maximum(&store).await, None);
    let _ = std::fs::remove_file(db);
}

/// The check that a deleting rule carries a maximum runs inside each write's
/// own transaction, against the row as it is then, not as the caller read it:
/// a save that keeps "the stored maximum" after another save cleared it, and a
/// clear that lands after another save made the rule delete, are both refused
/// with the existing reason code and leave nothing written.
#[tokio::test]
async fn a_deleting_rule_is_never_stored_without_a_maximum() {
    let (services, db) = temp_services("scryer_lifecycle_rule_maximum_atomic").await;
    let store = lifecycle_rule_set_store(&services).await;
    store
        .create_rule_set(&rule_set("bounded", vec![]), &revision("bounded", 1))
        .await
        .unwrap();
    let deleting = |number| {
        let mut revision = revision("bounded", number);
        revision.action_spec_json = scryer_application::lifecycle_rules::LifecycleDeclaredActions {
            destructive: true,
            ..scryer_application::lifecycle_rules::LifecycleDeclaredActions::none()
        }
        .to_persisted_json();
        revision
    };
    let required = scryer_application::error_reasons::lifecycle::RULE_MAX_TITLES_PER_RUN_REQUIRED;

    // The caller thought a maximum was stored; none is, so "keep it" is refused.
    let refused = store
        .add_revision(&deleting(2), None, Utc::now())
        .await
        .expect_err("a deleting revision without a stored maximum is refused");
    assert_eq!(refused.validation_reason(), Some(required));
    let refused = store
        .add_revision_keeping_state(&deleting(2), false, false, None, Utc::now())
        .await
        .expect_err("the state-keeping write refuses it too");
    assert_eq!(refused.validation_reason(), Some(required));
    let unchanged = store.get_rule_set("bounded").await.unwrap().unwrap();
    assert_eq!(unchanged.current_revision_number, 1, "nothing was written");
    assert!(store.get_revision("bounded", 2).await.unwrap().is_none());

    store
        .add_revision(&deleting(2), Some(Some(4)), Utc::now())
        .await
        .unwrap();
    assert_eq!(stored_maximum(&store).await, Some(4));

    // A clear checked against an older, non-deleting revision is refused now
    // that the revision in force deletes.
    let refused = store
        .update_rule_set_metadata(
            "bounded",
            "Renamed",
            "",
            &[],
            None,
            Some(None),
            false,
            Utc::now(),
        )
        .await
        .expect_err("clearing the maximum of a deleting rule is refused");
    assert_eq!(refused.validation_reason(), Some(required));
    let kept = store.get_rule_set("bounded").await.unwrap().unwrap();
    assert_eq!(kept.max_titles_per_run, Some(4));
    assert_ne!(
        kept.name, "Renamed",
        "the refused write left nothing behind"
    );
    let _ = std::fs::remove_file(db);
}

async fn stored_maximum(store: &crate::LifecycleRuleSetStore) -> Option<u32> {
    store
        .get_rule_set("bounded")
        .await
        .unwrap()
        .unwrap()
        .max_titles_per_run
}

fn rule_set(id: &str, library_ids: Vec<String>) -> LifecycleRuleSet {
    let now = Utc::now();
    LifecycleRuleSet {
        id: id.to_string(),
        name: "Stale movies".to_string(),
        description: "Unwatched for a long time".to_string(),
        state: LifecycleRuleState::Disabled,
        edited_since_enabled: false,
        destructive_confirmed_revision: None,
        library_ids,
        system_owned: false,
        current_revision_number: 1,
        schedule: scryer_domain::default_lifecycle_rule_schedule(),
        max_titles_per_run: None,
        created_at: now,
        updated_at: now,
    }
}

fn revision(rule_set_id: &str, number: i64) -> LifecycleRuleRevision {
    LifecycleRuleRevision {
        id: format!("{rule_set_id}-rev-{number}"),
        rule_set_id: rule_set_id.to_string(),
        revision_number: number,
        rego_source: format!("package scryer.maintenance.user.{rule_set_id}\nmatch := true\n"),
        action_spec_json: r#"{"kind":"unmonitor_scope_keep_files","schema_version":1}"#.to_string(),
        grace_days: 7 * number,
        source_content_hash: format!("hash-{number}"),
        created_by: Some("user-1".to_string()),
        created_at: Utc::now(),
        builder_model_json: None,
    }
}

fn title_and_season_revision(rule_set_id: &str, number: i64) -> LifecycleRuleRevision {
    LifecycleRuleRevision {
        action_spec_json: format!(
            r#"{{"schema_version":{},"heads":["unmonitor","unmonitor_season"],"destructive":false,"quality_profiles":[],"add_tags":[],"remove_tags":[]}}"#,
            scryer_application::lifecycle_rules::LIFECYCLE_DECLARED_ACTIONS_SCHEMA_VERSION
        ),
        ..revision(rule_set_id, number)
    }
}

#[tokio::test]
async fn rule_sets_and_revisions_round_trip() {
    let (services, db) = temp_services("scryer_lifecycle_rules").await;
    let store = lifecycle_rule_set_store(&services).await;

    // A rule no longer stores the level it acts at: its revision's declared
    // heads carry it, so the round trip keeps a title head and a season head.
    let created = rule_set("rule-a", vec!["library-1".to_string()]);
    let first = title_and_season_revision("rule-a", 1);
    store
        .create_rule_set(&created, &first)
        .await
        .expect("create rule set");

    let loaded = store
        .get_rule_set("rule-a")
        .await
        .expect("read rule set")
        .expect("rule set exists");
    assert_eq!(loaded.name, created.name);
    assert_eq!(loaded.state, LifecycleRuleState::Disabled);
    assert!(!loaded.edited_since_enabled);
    assert_eq!(loaded.destructive_confirmed_revision, None);
    assert_eq!(loaded.library_ids, vec!["library-1".to_string()]);
    assert_eq!(loaded.current_revision_number, 1);
    assert!(!loaded.system_owned);

    let mut shipped = rule_set("rule-shipped", Vec::new());
    shipped.system_owned = true;
    store
        .create_rule_set(&shipped, &revision("rule-shipped", 1))
        .await
        .expect("create system-owned rule set");
    assert!(
        store
            .get_rule_set("rule-shipped")
            .await
            .expect("read rule set")
            .expect("rule set exists")
            .system_owned
    );

    let stored_revision = store
        .get_revision("rule-a", 1)
        .await
        .expect("read revision")
        .expect("revision exists");
    let expected = first;
    assert_eq!(stored_revision.rego_source, expected.rego_source);
    assert_eq!(stored_revision.action_spec_json, expected.action_spec_json);
    let declared = |json: &str| {
        scryer_application::lifecycle_rules::LifecycleDeclaredActions::from_persisted_json(json)
            .expect("declared actions parse")
            .expect("declared actions are current")
    };
    let stored_levels = declared(&stored_revision.action_spec_json).levels();
    assert_eq!(stored_levels.len(), 2, "title and season levels survive");
    assert_eq!(stored_levels, declared(&expected.action_spec_json).levels());
    assert_eq!(stored_revision.source_content_hash, "hash-1");
    assert_eq!(stored_revision.grace_days, 7);
    assert_eq!(stored_revision.created_by.as_deref(), Some("user-1"));
    assert_eq!(stored_revision.builder_model_json, None);

    let mut built = revision("rule-a", 2);
    built.builder_model_json = Some(r#"{"family":"lifecycle","version":2}"#.to_string());
    store
        .add_revision(&built, None, Utc::now())
        .await
        .expect("append revision with builder model");
    assert_eq!(
        store
            .get_revision("rule-a", 2)
            .await
            .expect("read revision 2")
            .expect("revision 2 exists")
            .builder_model_json,
        built.builder_model_json
    );
    assert_eq!(
        store
            .get_revision("rule-a", 1)
            .await
            .expect("reread revision 1")
            .expect("revision 1 exists")
            .builder_model_json,
        None
    );

    let _ = std::fs::remove_file(db);
}

/// The batched revision read answers every key exactly as one `get_revision`
/// call does, in key order, across chunk boundaries, with repeats and misses.
#[tokio::test]
async fn batched_revision_reads_match_the_per_key_reads() {
    let (services, db) = temp_services("scryer_lifecycle_revision_batch").await;
    let store = lifecycle_rule_set_store(&services).await;
    for id in ["emberfall", "cinderwake"] {
        store
            .create_rule_set(&rule_set(id, Vec::new()), &revision(id, 1))
            .await
            .expect("create rule set");
        store
            .add_revision(&revision(id, 2), None, Utc::now())
            .await
            .expect("add revision");
    }

    let mut keys = vec![
        ("cinderwake".to_string(), 2),
        ("emberfall".to_string(), 1),
        ("ashgrove".to_string(), 1),
        ("emberfall".to_string(), 3),
        ("emberfall".to_string(), 1),
    ];
    for index in 0..900 {
        let id = if index % 2 == 0 {
            "emberfall"
        } else {
            "cinderwake"
        };
        keys.push((id.to_string(), (index % 3) as i64 + 1));
    }
    let batched = store.get_revisions(&keys).await.expect("batched revisions");
    assert_eq!(batched.len(), keys.len());
    for ((rule_set_id, revision_number), found) in keys.iter().zip(&batched) {
        let expected = store
            .get_revision(rule_set_id, *revision_number)
            .await
            .expect("per-key revision");
        assert_eq!(found, &expected, "{rule_set_id} revision {revision_number}");
    }
    assert!(batched[2].is_none() && batched[3].is_none());
    assert!(
        store
            .get_revisions(&[])
            .await
            .expect("empty batch")
            .is_empty()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn adding_a_revision_repoints_the_rule_set_and_preserves_the_old_one() {
    let (services, db) = temp_services("scryer_lifecycle_revisions").await;
    let store = lifecycle_rule_set_store(&services).await;

    store
        .create_rule_set(&rule_set("rule-b", Vec::new()), &revision("rule-b", 1))
        .await
        .expect("create rule set");

    store
        .update_rule_set_state(
            "rule-b",
            LifecycleRuleState::Enabled,
            Some(&scryer_domain::LifecycleRuleArmingConfirmation {
                revision_number: 1,
                library_ids: vec![],
            }),
            Utc::now(),
        )
        .await
        .expect("arm the rule against revision 1");

    let updated_at = Utc::now();
    store
        .add_revision(&revision("rule-b", 2), None, updated_at)
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
        loaded.state,
        LifecycleRuleState::DryRun,
        "appending a revision must disarm the rule"
    );
    assert_eq!(
        loaded.destructive_confirmed_revision, None,
        "appending a revision clears the deletion confirmation"
    );
    assert!(
        loaded.edited_since_enabled,
        "disarming an enabled rule records that it was edited since it was enabled"
    );
    assert!(
        store
            .update_rule_set_state("rule-b", LifecycleRuleState::DryRun, None, Utc::now(),)
            .await
            .unwrap()
    );
    store
        .add_revision(&revision("rule-b", 3), None, Utc::now())
        .await
        .expect("add revision to an unarmed rule");
    let unarmed = store.get_rule_set("rule-b").await.unwrap().unwrap();
    assert!(
        !unarmed.edited_since_enabled,
        "a state write clears the flag and editing a rule that is not enabled never sets it"
    );
    assert_eq!(unarmed.state, LifecycleRuleState::DryRun);

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
        vec![3, 2, 1],
        "newest revision first"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn metadata_updates_leave_the_revision_pointer_alone() {
    let (services, db) = temp_services("scryer_lifecycle_metadata").await;
    let store = lifecycle_rule_set_store(&services).await;

    let mut armed = rule_set("rule-c", Vec::new());
    armed.state = LifecycleRuleState::Enabled;
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
            None,
            None,
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
    assert_eq!(loaded.state, LifecycleRuleState::Enabled);
    assert!(!loaded.edited_since_enabled);
    assert_eq!(store.list_revisions("rule-c").await.unwrap().len(), 1);
    assert_eq!(
        loaded.schedule,
        scryer_domain::default_lifecycle_rule_schedule(),
        "a save without a schedule keeps the stored one"
    );

    // A schedule rides in the same metadata write and reads back as stored.
    let weekly = scryer_domain::ScriptSchedule::Weekly {
        days: vec![scryer_domain::ScheduleWeekday::Monday],
        time_local: "04:15".to_string(),
    };
    store
        .update_rule_set_metadata(
            "rule-c",
            "Renamed",
            "New description",
            &["library-9".to_string()],
            Some(&weekly),
            None,
            false,
            Utc::now(),
        )
        .await
        .expect("reschedule");
    let loaded = store
        .get_rule_set("rule-c")
        .await
        .unwrap()
        .expect("rule set exists");
    assert_eq!(loaded.schedule, weekly);
    assert_eq!(
        loaded.state,
        LifecycleRuleState::Enabled,
        "a schedule change leaves the state alone"
    );
    assert_eq!(
        store.list_rule_sets().await.unwrap()[0].schedule,
        weekly,
        "the list read decodes the schedule too"
    );

    let _ = std::fs::remove_file(db);
}

/// The disarm and the scope it invalidates are one write, exactly as the
/// revision pointer and its disarm are: there must be no instant at which the
/// new scope is in force under the previous scope's arming.
#[tokio::test]
async fn a_scope_change_disarms_in_the_same_write() {
    let (services, db) = temp_services("scryer_lifecycle_rescope").await;
    let store = lifecycle_rule_set_store(&services).await;

    let mut armed = rule_set("rule-f", vec!["library-1".to_string()]);
    armed.state = LifecycleRuleState::Enabled;
    armed.destructive_confirmed_revision = Some(1);
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
            None,
            None,
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
        loaded.state,
        LifecycleRuleState::DryRun,
        "a widened scope must never stay enabled under the narrower acknowledgement"
    );
    assert_eq!(
        loaded.destructive_confirmed_revision, None,
        "the deletion confirmation is cleared in the same write"
    );
    assert!(loaded.edited_since_enabled);
    assert_eq!(loaded.current_revision_number, 1);

    let _ = std::fs::remove_file(db);
}

/// A stored state this build does not know reads as disabled, and a NULL
/// confirmation reads as none, so neither can make a rule act or delete.
#[tokio::test]
async fn an_unknown_stored_state_reads_disabled() {
    let (services, db) = temp_services("scryer_lifecycle_unknown_state").await;
    let store = lifecycle_rule_set_store(&services).await;
    store
        .create_rule_set(&rule_set("unknown", Vec::new()), &revision("unknown", 1))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE lifecycle_rule_sets
            SET state = 'sweeping', destructive_confirmed_revision = NULL
          WHERE id = 'unknown'",
    )
    .execute(services.pool())
    .await
    .unwrap();

    let loaded = store.get_rule_set("unknown").await.unwrap().unwrap();
    assert_eq!(loaded.state, LifecycleRuleState::Disabled);
    assert!(!loaded.evaluates());
    assert_eq!(loaded.destructive_confirmed_revision, None);
    assert!(!loaded.destructive_confirmed());
    assert_eq!(
        store.list_rule_sets().await.unwrap()[0].state,
        LifecycleRuleState::Disabled,
        "the list read decodes the same way"
    );
    let _ = std::fs::remove_file(db);
}

/// A confirmation the store cannot decode is a read error, not a missing
/// confirmation: the read fails instead of hiding it.
#[tokio::test]
async fn an_undecodable_confirmation_fails_the_read() {
    let (services, db) = temp_services("scryer_lifecycle_undecodable_confirmation").await;
    let store = lifecycle_rule_set_store(&services).await;
    store
        .create_rule_set(
            &rule_set("undecodable", Vec::new()),
            &revision("undecodable", 1),
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE lifecycle_rule_sets
            SET state = 'enabled', destructive_confirmed_revision = 'not-a-revision'
          WHERE id = 'undecodable'",
    )
    .execute(services.pool())
    .await
    .unwrap();

    assert!(store.get_rule_set("undecodable").await.is_err());
    assert!(store.list_rule_sets().await.is_err());
    let _ = std::fs::remove_file(db);
}

/// The FK cascade is what keeps deleted rules from leaving orphan revisions
/// behind; SQLite only enforces it when foreign keys are on, so this asserts
/// the deployed configuration, not just the DDL.
#[tokio::test]
async fn deleting_a_rule_set_cascades_to_its_revisions() {
    let (services, db) = temp_services("scryer_lifecycle_delete").await;
    let store = lifecycle_rule_set_store(&services).await;

    store
        .create_rule_set(&rule_set("rule-d", Vec::new()), &revision("rule-d", 1))
        .await
        .expect("create rule set");
    store
        .add_revision(&revision("rule-d", 2), None, Utc::now())
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
    let (services, db) = temp_services("scryer_lifecycle_duplicate").await;
    let store = lifecycle_rule_set_store(&services).await;

    store
        .create_rule_set(&rule_set("rule-e", Vec::new()), &revision("rule-e", 1))
        .await
        .expect("create rule set");

    store
        .add_revision(&revision("rule-e", 1), None, Utc::now())
        .await
        .expect_err("the unique constraint must reject a replayed revision number");

    assert_eq!(store.list_revisions("rule-e").await.unwrap().len(), 1);

    let _ = std::fs::remove_file(db);
}

/// An edit prepared from a read taken before another admin disabled the rule
/// must not re-enable it, and a kept state never carries a deletion
/// confirmation the caller did not ask to carry.
#[tokio::test]
async fn a_revision_keeping_state_never_re_enables_a_concurrently_disabled_rule() {
    let temp = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(
        temp.path()
            .join("lowered.db")
            .to_string_lossy()
            .into_owned(),
    )
    .await
    .unwrap();
    let store = lifecycle_rule_set_store(&services).await;
    let mut original = rule_set("lowered", vec!["library-1".into()]);
    original.state = LifecycleRuleState::Enabled;
    store
        .create_rule_set(&original, &revision("lowered", 1))
        .await
        .unwrap();
    // The editor reads here: Enabled.
    let read = store.get_rule_set("lowered").await.unwrap().unwrap();
    assert_eq!(read.state, LifecycleRuleState::Enabled);

    // Another admin disables it in between.
    assert!(
        store
            .update_rule_set_state("lowered", LifecycleRuleState::Disabled, None, Utc::now())
            .await
            .unwrap()
    );

    // The edit computed from its stale read still asks to stay enabled.
    store
        .add_revision_keeping_state(&revision("lowered", 2), true, false, None, Utc::now())
        .await
        .expect("the edit itself lands");
    let current = store.get_rule_set("lowered").await.unwrap().unwrap();
    assert_eq!(current.current_revision_number, 2);
    assert_eq!(current.state, LifecycleRuleState::Disabled);
    assert_eq!(current.destructive_confirmed_revision, None);

    // An enabled rule confirmed for deletion keeps its state but not the
    // confirmation unless asked to carry it.
    let mut armed = rule_set("armed", vec!["library-1".into()]);
    armed.state = LifecycleRuleState::Enabled;
    armed.destructive_confirmed_revision = Some(1);
    store
        .create_rule_set(&armed, &revision("armed", 1))
        .await
        .unwrap();
    store
        .add_revision_keeping_state(&revision("armed", 2), true, false, None, Utc::now())
        .await
        .unwrap();
    let current = store.get_rule_set("armed").await.unwrap().unwrap();
    assert_eq!(current.state, LifecycleRuleState::Enabled);
    assert_eq!(current.destructive_confirmed_revision, None);

    // Not keeping it enabled drops it to a dry run.
    store
        .add_revision_keeping_state(&revision("armed", 3), false, false, None, Utc::now())
        .await
        .unwrap();
    let current = store.get_rule_set("armed").await.unwrap().unwrap();
    assert_eq!(current.state, LifecycleRuleState::DryRun);
    assert!(!current.edited_since_enabled);
}

/// A carried confirmation moves to the new revision only when it named
/// exactly the revision being replaced; any other stored confirmation is
/// cleared in the same write.
#[tokio::test]
async fn a_carried_confirmation_moves_only_from_the_replaced_revision() {
    let (services, db) = temp_services("scryer_lifecycle_carry").await;
    let store = lifecycle_rule_set_store(&services).await;
    let mut confirmed = rule_set("confirmed", Vec::new());
    confirmed.state = LifecycleRuleState::Enabled;
    confirmed.destructive_confirmed_revision = Some(1);
    store
        .create_rule_set(&confirmed, &revision("confirmed", 1))
        .await
        .unwrap();
    store
        .add_revision_keeping_state(&revision("confirmed", 2), true, true, None, Utc::now())
        .await
        .unwrap();
    let current = store.get_rule_set("confirmed").await.unwrap().unwrap();
    assert_eq!(current.state, LifecycleRuleState::Enabled);
    assert_eq!(current.destructive_confirmed_revision, Some(2));
    assert!(current.destructive_confirmed());

    let mut older = rule_set("older", Vec::new());
    older.state = LifecycleRuleState::Enabled;
    older.current_revision_number = 2;
    older.destructive_confirmed_revision = Some(1);
    store
        .create_rule_set(&older, &revision("older", 2))
        .await
        .unwrap();
    store
        .add_revision_keeping_state(&revision("older", 3), true, true, None, Utc::now())
        .await
        .unwrap();
    let current = store.get_rule_set("older").await.unwrap().unwrap();
    assert_eq!(current.current_revision_number, 3);
    assert_eq!(
        current.destructive_confirmed_revision, None,
        "a confirmation for an older revision is never carried forward"
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_revision_keeping_state_moves_only_from_the_revision_it_follows() {
    let temp = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(
        temp.path()
            .join("keeping.db")
            .to_string_lossy()
            .into_owned(),
    )
    .await
    .unwrap();
    let store = lifecycle_rule_set_store(&services).await;
    let mut original = rule_set("kept", vec!["library-1".into()]);
    original.state = LifecycleRuleState::Enabled;
    original.destructive_confirmed_revision = Some(1);
    store
        .create_rule_set(&original, &revision("kept", 1))
        .await
        .unwrap();

    store
        .add_revision_keeping_state(&revision("kept", 2), true, false, None, Utc::now())
        .await
        .expect("the save follows the current revision");
    let current = store.get_rule_set("kept").await.unwrap().unwrap();
    assert_eq!(current.current_revision_number, 2);
    assert_eq!(current.state, LifecycleRuleState::Enabled);

    // A save prepared against revision 1 lost the race to the one above.
    let mut stale = revision("kept", 2);
    stale.id = "kept-stale-2".into();
    let refused = store
        .add_revision_keeping_state(&stale, true, false, None, Utc::now())
        .await
        .expect_err("a stale save is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(scryer_application::error_reasons::lifecycle::RULE_REVISION_CONFLICT),
        "{refused:?}"
    );
    let skipped = store
        .add_revision_keeping_state(&revision("kept", 4), false, false, None, Utc::now())
        .await
        .expect_err("a save that skips a revision is refused");
    assert_eq!(
        skipped.validation_reason(),
        Some(scryer_application::error_reasons::lifecycle::RULE_REVISION_CONFLICT)
    );
    let current = store.get_rule_set("kept").await.unwrap().unwrap();
    assert_eq!(current.current_revision_number, 2);
    assert_eq!(current.state, LifecycleRuleState::Enabled);
    assert_eq!(store.list_revisions("kept").await.unwrap().len(), 2);

    // The disarming save follows the same rule, and a stale one disarms
    // nothing.
    let mut stale_disarm = revision("kept", 2);
    stale_disarm.id = "kept-stale-disarm-2".into();
    let refused = store
        .add_revision(&stale_disarm, None, Utc::now())
        .await
        .expect_err("a stale disarming save is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(scryer_application::error_reasons::lifecycle::RULE_REVISION_CONFLICT),
        "{refused:?}"
    );
    let current = store.get_rule_set("kept").await.unwrap().unwrap();
    assert_eq!(current.current_revision_number, 2);
    assert_eq!(current.state, LifecycleRuleState::Enabled);

    store
        .add_revision(&revision("kept", 3), None, Utc::now())
        .await
        .expect("a disarming save that follows the current revision");
    let current = store.get_rule_set("kept").await.unwrap().unwrap();
    assert_eq!(current.current_revision_number, 3);
    assert_eq!(current.state, LifecycleRuleState::DryRun);
    assert!(current.edited_since_enabled);
    assert_eq!(store.list_revisions("kept").await.unwrap().len(), 3);
}
