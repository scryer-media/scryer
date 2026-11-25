//! Round-trips for the request rule set, revision, and decision stores
//! (spec 0003 section 6).
//!
//! Every assertion body is shared between the SQLite and PostgreSQL tests: the
//! two dialects have different timestamp and boolean storage, and a round-trip
//! that only ever ran on SQLite would let a Postgres-only mapping bug ship.

use super::*;
use scryer_application::{RequestRuleDecisionRepository, RequestRuleSetRepository};
use scryer_domain::{
    RequestDecisionOutcome, RequestRuleDecisionRecord, RequestRuleRevision, RequestRuleSet,
};

#[tokio::test]
async fn normalized_library_scopes_preserve_intent_and_enforce_integrity() {
    let (services, db) = temp_services("scryer_request_scope_integrity").await;
    super::lifecycle_rule_sets::seed_rule_libraries(&services.datastore()).await;
    let store = crate::RequestRuleSetStore::new(services.datastore());
    let pool = services.pool();
    let scope = vec![
        "library-2".to_string(),
        "library-1".to_string(),
        "library-2".to_string(),
    ];
    store
        .create_rule_set(&rule_set("scoped", scope), &revision("scoped", 1), None)
        .await
        .unwrap();
    store
        .create_rule_set(&rule_set("global", vec![]), &revision("global", 1), None)
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

    let columns = sqlx::query("PRAGMA table_info(request_rule_sets)")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(
        !columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "library_ids")
    );
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT library_id FROM request_rule_set_libraries WHERE rule_set_id = 'scoped' ORDER BY position"
    ).fetch_all(pool).await.unwrap();
    assert_eq!(stored, expected);
    let positions: Vec<i64> = sqlx::query_scalar(
        "SELECT position FROM request_rule_set_libraries WHERE rule_set_id = 'scoped' ORDER BY position"
    ).fetch_all(pool).await.unwrap();
    assert_eq!(positions, vec![0, 1], "a skipped duplicate leaves no gap");
    assert!(sqlx::query(
        "INSERT INTO request_rule_set_libraries (rule_set_id, library_id, position) VALUES ('scoped', 'library-1', 9)"
    ).execute(pool).await.is_err(), "duplicate associations must be rejected");
    assert!(sqlx::query(
        "INSERT INTO request_rule_set_libraries (rule_set_id, library_id, position) VALUES ('scoped', 'library-9', 0)"
    ).execute(pool).await.is_err(), "positions must be unique within a rule");

    let bad = rule_set("bad", vec!["library-1".into(), "missing-library".into()]);
    assert!(
        store
            .create_rule_set(&bad, &revision("bad", 1), None)
            .await
            .is_err()
    );
    assert!(store.get_rule_set("bad").await.unwrap().is_none());
    assert!(store.list_revisions("bad").await.unwrap().is_empty());
    let before = store.get_rule_set("scoped").await.unwrap().unwrap();
    assert!(
        store
            .update_rule_set_metadata(
                "scoped",
                "Must roll back",
                "bad scope",
                &["library-9".into(), "missing-library".into()],
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

    store
        .update_rule_set_metadata("scoped", "Scoped", "", &["library-1".into()], Utc::now())
        .await
        .unwrap();
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
        .update_rule_set_metadata("scoped", "Global", "", &[], Utc::now())
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
            Utc::now(),
        )
        .await
        .unwrap();
    store.delete_rule_set("scoped").await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM request_rule_set_libraries")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        remaining, 0,
        "deleting a rule must cascade its associations"
    );
    let _ = std::fs::remove_file(db);
}

fn rule_set(id: &str, library_ids: Vec<String>) -> RequestRuleSet {
    let now = Utc::now();
    RequestRuleSet {
        id: id.to_string(),
        name: "Family friendly".to_string(),
        description: "Auto-approve low-certification titles".to_string(),
        enabled: false,
        library_ids,
        current_revision_number: 1,
        created_at: now,
        updated_at: now,
    }
}

fn revision(rule_set_id: &str, number: i64) -> RequestRuleRevision {
    RequestRuleRevision {
        id: format!("{rule_set_id}-rev-{number}"),
        rule_set_id: rule_set_id.to_string(),
        revision_number: number,
        rego_source: format!(
            "package scryer.request.user.{rule_set_id}\napprove if {{ true }}\n# rev {number}\n"
        ),
        source_content_hash: format!("hash-{number}"),
        created_by: Some("user-1".to_string()),
        created_at: Utc::now(),
    }
}

fn decision(
    id: &str,
    request_id: &str,
    outcome: RequestDecisionOutcome,
) -> RequestRuleDecisionRecord {
    let now = Utc::now();
    RequestRuleDecisionRecord {
        id: id.to_string(),
        request_id: request_id.to_string(),
        evaluated_at: now,
        effective_outcome: outcome,
        policy_outcome: outcome,
        fallback_reason: None,
        votes_json: r#"[{"rule_set_id":"rule-a","vote":"approve"}]"#.to_string(),
        tags: vec!["family".to_string()],
        shaping_json: Some(r#"{"set_lease_days":{"value":30}}"#.to_string()),
        input_hash: "input-hash-1".to_string(),
        input_schema_version: 1,
        created_at: now,
    }
}

async fn assert_rule_set_round_trip(store: &dyn RequestRuleSetRepository) -> AppResult<()> {
    let created = rule_set("rule-a", vec!["library-1".to_string()]);
    store
        .create_rule_set(&created, &revision("rule-a", 1), Some(r#"{"rows":[1]}"#))
        .await?;
    assert_eq!(
        store.get_builder_model("rule-a").await?.as_deref(),
        Some(r#"{"rows":[1]}"#)
    );

    let loaded = store
        .get_rule_set("rule-a")
        .await?
        .expect("rule set should exist");
    assert_eq!(loaded.name, created.name);
    assert_eq!(loaded.description, created.description);
    assert!(!loaded.enabled, "a new rule set is created disabled");
    assert_eq!(loaded.library_ids, vec!["library-1".to_string()]);
    assert_eq!(loaded.current_revision_number, 1);

    let stored_revision = store
        .get_revision("rule-a", 1)
        .await?
        .expect("revision should exist");
    assert_eq!(
        stored_revision.rego_source,
        revision("rule-a", 1).rego_source
    );
    assert_eq!(stored_revision.source_content_hash, "hash-1");
    assert_eq!(stored_revision.created_by.as_deref(), Some("user-1"));

    // Appending a revision repoints the rule set and rewrites nothing.
    let updated_at = Utc::now();
    store
        .add_revision(&revision("rule-a", 2), updated_at, None)
        .await?;
    // A revision that does not mention the builder document keeps it.
    assert_eq!(
        store.get_builder_model("rule-a").await?.as_deref(),
        Some(r#"{"rows":[1]}"#)
    );
    let loaded = store
        .get_rule_set("rule-a")
        .await?
        .expect("rule set should exist");
    assert_eq!(loaded.current_revision_number, 2);
    assert_eq!(
        store
            .get_revision("rule-a", 1)
            .await?
            .expect("revision 1 survives")
            .source_content_hash,
        "hash-1",
        "an appended revision must not rewrite its predecessor"
    );
    assert_eq!(
        store
            .list_revisions("rule-a")
            .await?
            .iter()
            .map(|revision| revision.revision_number)
            .collect::<Vec<_>>(),
        vec![2, 1],
        "newest revision first"
    );

    // A replayed revision number is refused rather than duplicating history.
    store
        .add_revision(&revision("rule-a", 2), Utc::now(), None)
        .await
        .expect_err("the unique constraint must reject a replayed revision number");
    assert_eq!(store.list_revisions("rule-a").await?.len(), 2);
    // A refused revision leaves the builder document alone.
    assert_eq!(
        store.get_builder_model("rule-a").await?.as_deref(),
        Some(r#"{"rows":[1]}"#)
    );

    store
        .update_rule_set_metadata(
            "rule-a",
            "Renamed",
            "New description",
            &["library-9".to_string()],
            Utc::now(),
        )
        .await?;
    store
        .update_rule_set_enabled("rule-a", true, Utc::now())
        .await?;
    let loaded = store
        .get_rule_set("rule-a")
        .await?
        .expect("rule set should exist");
    assert_eq!(loaded.name, "Renamed");
    assert_eq!(loaded.library_ids, vec!["library-9".to_string()]);
    assert!(loaded.enabled);
    assert_eq!(
        loaded.current_revision_number, 2,
        "metadata and enabled edits leave the revision pointer alone"
    );
    assert_eq!(
        store.get_builder_model("rule-a").await?.as_deref(),
        Some(r#"{"rows":[1]}"#),
        "metadata and enabled edits leave the builder document alone"
    );

    // A revision can replace the builder document, and another can clear it.
    store
        .add_revision(
            &revision("rule-a", 3),
            Utc::now(),
            Some(Some(r#"{"rows":[3]}"#)),
        )
        .await?;
    assert_eq!(
        store.get_builder_model("rule-a").await?.as_deref(),
        Some(r#"{"rows":[3]}"#)
    );
    store
        .add_revision(&revision("rule-a", 4), Utc::now(), Some(None))
        .await?;
    assert_eq!(store.get_builder_model("rule-a").await?, None);

    // The FK cascade is what keeps a deleted rule from leaving orphan revisions.
    store.delete_rule_set("rule-a").await?;
    assert!(store.get_rule_set("rule-a").await?.is_none());
    assert!(store.list_revisions("rule-a").await?.is_empty());
    Ok(())
}

async fn assert_decision_round_trip(store: &dyn RequestRuleDecisionRepository) -> AppResult<()> {
    // No media request row is inserted in this fixture. This is the real-store
    // regression for submit's pre-insert history snapshot: its decision trace
    // must accept the generated request id before the request row exists.
    let mut first = decision(
        "decision-1",
        "request-1",
        RequestDecisionOutcome::AutoApprove,
    );
    first.evaluated_at = Utc::now() - chrono::Duration::minutes(5);
    store.record(&first).await?;

    let mut second = decision(
        "decision-2",
        "request-1",
        RequestDecisionOutcome::ManualReview,
    );
    second.policy_outcome = RequestDecisionOutcome::Deny;
    second.fallback_reason = Some("held".to_string());
    second.tags = Vec::new();
    store.record(&second).await?;

    store
        .record(&decision(
            "decision-3",
            "request-2",
            RequestDecisionOutcome::Deny,
        ))
        .await?;

    let latest = store
        .latest_for_request("request-1")
        .await?
        .expect("a decision was recorded for request-1");
    assert_eq!(latest.id, "decision-2");
    // The policy and effective verdicts can disagree, so the two are stored
    // and read back separately.
    assert_eq!(
        latest.effective_outcome,
        RequestDecisionOutcome::ManualReview
    );
    assert_eq!(latest.policy_outcome, RequestDecisionOutcome::Deny);
    assert_eq!(latest.fallback_reason.as_deref(), Some("held"));
    assert_eq!(latest.input_hash, "input-hash-1");
    assert_eq!(latest.input_schema_version, 1);
    assert!(latest.tags.is_empty());

    let approved = store
        .list_recent(10, Some(RequestDecisionOutcome::AutoApprove))
        .await?;
    assert_eq!(
        approved
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec!["decision-1"]
    );
    assert_eq!(approved[0].tags, vec!["family".to_string()]);

    assert_eq!(store.list_recent(10, None).await?.len(), 3);
    assert_eq!(
        store.list_recent(2, None).await?.len(),
        2,
        "the limit bounds the page"
    );
    assert_eq!(store.list_recent(0, None).await?.len(), 0);

    // The batched read gives each request the decision the single read gives
    // it, and says nothing for a request with none.
    for asked in [
        Vec::new(),
        vec!["request-1".to_string()],
        vec![
            "request-2".to_string(),
            "request-unknown".to_string(),
            "request-1".to_string(),
            "request-1".to_string(),
        ],
    ] {
        let batched = store.latest_for_requests(&asked).await?;
        let mut expected = std::collections::HashMap::new();
        for request_id in &asked {
            if let Some(decision) = store.latest_for_request(request_id).await? {
                expected.insert(request_id.clone(), decision);
            }
        }
        let shape = |decisions: &std::collections::HashMap<String, RequestRuleDecisionRecord>| {
            let mut rows = decisions
                .iter()
                .map(|(request_id, decision)| format!("{request_id}: {decision:?}"))
                .collect::<Vec<_>>();
            rows.sort();
            rows
        };
        assert_eq!(shape(&batched), shape(&expected));
    }
    assert_eq!(
        store
            .latest_for_requests(&["request-1".to_string()])
            .await?["request-1"]
            .id,
        "decision-2"
    );

    // Each decision is linked to the rule sets its votes name when it is
    // recorded, and the count reads those links.
    assert_eq!(store.count_for_rule_set("rule-a").await?, 3);
    assert_eq!(store.count_for_rule_set("rule-zzz").await?, 0);
    assert_eq!(store.count_for_rule_set("  ").await?, 0);
    assert_eq!(
        store
            .count_for_rule_sets(&["rule-a", "rule-zzz", "  ", "rule-a"].map(str::to_string))
            .await?,
        std::collections::HashMap::from([("rule-a".to_string(), 3)])
    );
    Ok(())
}

/// Deleting a rule set takes its revisions and leaves its decisions: a trace
/// explains a decision that was already made, and it outlives the rule that
/// made it (spec 0003 FR-016).
async fn assert_decisions_outlive_their_rule_set(
    rules: &dyn RequestRuleSetRepository,
    decisions: &dyn RequestRuleDecisionRepository,
) -> AppResult<()> {
    rules
        .create_rule_set(
            &rule_set("rule-b", Vec::new()),
            &revision("rule-b", 1),
            None,
        )
        .await?;
    decisions
        .record(&decision(
            "decision-b",
            "request-b",
            RequestDecisionOutcome::AutoApprove,
        ))
        .await?;
    rules.delete_rule_set("rule-b").await?;
    assert!(
        decisions.latest_for_request("request-b").await?.is_some(),
        "a decision trace must survive the deletion of the rule that made it"
    );
    Ok(())
}

#[tokio::test]
async fn request_rule_sets_and_revisions_round_trip() {
    let (services, db) = temp_services("scryer_request_rules").await;
    let store = crate::RequestRuleSetStore::new(services.datastore());
    super::lifecycle_rule_sets::seed_rule_libraries(&services.datastore()).await;
    assert_rule_set_round_trip(&store)
        .await
        .expect("rule sets should round-trip");
    let _ = std::fs::remove_file(db);
}

/// The batched revision read answers every key exactly as one `get_revision`
/// call does, in key order, across chunk boundaries, with repeats and misses.
#[tokio::test]
async fn batched_request_revision_reads_match_the_per_key_reads() {
    let (services, db) = temp_services("scryer_request_revision_batch").await;
    let store = crate::RequestRuleSetStore::new(services.datastore());
    for id in ["emberfall", "cinderwake"] {
        store
            .create_rule_set(&rule_set(id, Vec::new()), &revision(id, 1), None)
            .await
            .expect("create rule set");
        store
            .add_revision(&revision(id, 2), Utc::now(), None)
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
async fn request_rule_decisions_round_trip() {
    let (services, db) = temp_services("scryer_request_rule_decisions").await;
    let store = crate::RequestRuleDecisionStore::new(services.datastore());
    assert_decision_round_trip(&store)
        .await
        .expect("decisions should round-trip");
    let _ = std::fs::remove_file(db);
}

/// Neither a rule set nor a decision stores a mode: `enabled` alone decides
/// whether a rule set takes part, and decision rows recorded before the mode
/// was dropped, with votes that still carry a per-rule mode, still list.
#[tokio::test]
async fn request_rule_modes_are_not_stored_and_older_decisions_still_list() {
    let (services, db) = temp_services("scryer_request_rule_retired_modes").await;
    let rules = crate::RequestRuleSetStore::new(services.datastore());
    let decisions = crate::RequestRuleDecisionStore::new(services.datastore());
    let pool = services.pool();

    for (table, column) in [
        ("request_rule_sets", "evaluation_mode"),
        ("request_rule_decisions", "mode"),
    ] {
        let present: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
                .bind(table)
                .bind(column)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(present, 0, "{table}.{column} is not stored");
    }

    rules
        .create_rule_set(
            &rule_set("rule-quiet", Vec::new()),
            &revision("rule-quiet", 1),
            None,
        )
        .await
        .unwrap();
    rules
        .update_rule_set_enabled("rule-quiet", true, Utc::now())
        .await
        .unwrap();
    assert!(
        rules
            .get_rule_set("rule-quiet")
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    rules
        .update_rule_set_enabled("rule-quiet", false, Utc::now())
        .await
        .unwrap();
    assert!(
        !rules
            .get_rule_set("rule-quiet")
            .await
            .unwrap()
            .unwrap()
            .enabled
    );

    decisions
        .record(&decision(
            "decision-new",
            "request-new",
            RequestDecisionOutcome::AutoApprove,
        ))
        .await
        .unwrap();

    // Rows recorded before the mode was dropped, with votes that still carry
    // a per-rule mode, including a mode value this build never knew.
    for (id, mode) in [
        ("decision-old-enforce", "enforce"),
        ("decision-old-disabled", "disabled"),
        ("decision-old-shadow", "shadow"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO request_rule_decisions
                (id, request_id, effective_outcome, policy_outcome, fallback_reason,
                 votes_json, tags_json, input_hash, input_schema_version)
             VALUES ('{id}', 'request-old', 'deny', 'deny', NULL,
                 '[{{\"rule_set_id\":\"rule-quiet\",\"mode\":\"{mode}\",\"vote\":\"deny\"}}]',
                 '[]', 'old-hash', 1)"
        )))
        .execute(pool)
        .await
        .unwrap();
    }
    let listed = decisions.list_recent(10, None).await.unwrap();
    let mut ids: Vec<&str> = listed.iter().map(|row| row.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![
            "decision-new",
            "decision-old-disabled",
            "decision-old-enforce",
            "decision-old-shadow",
        ]
    );
    let shadow = listed
        .iter()
        .find(|row| row.id == "decision-old-shadow")
        .unwrap();
    assert_eq!(shadow.effective_outcome, RequestDecisionOutcome::Deny);
    assert!(shadow.votes_json.contains(r#""mode":"shadow""#));
    assert_eq!(
        decisions
            .latest_for_request("request-old")
            .await
            .unwrap()
            .map(|row| row.request_id),
        Some("request-old".to_string())
    );
    let _ = std::fs::remove_file(db);
}

/// The rule set count reads the links written with each decision, and finds
/// the decisions the scan of every stored vote found before it: the same ids,
/// for every rule set, across decisions that name a rule set twice, name
/// none, or name more rule sets than one link statement binds.
#[tokio::test]
async fn the_linked_rule_set_count_finds_what_the_votes_scan_found() {
    let (services, db) = temp_services("scryer_request_rule_decision_links").await;
    let decisions = crate::RequestRuleDecisionStore::new(services.datastore());
    let pool = services.pool();

    let rule_set_ids: Vec<String> = (0..450)
        .map(|index| format!("0b6f3a2e-77c4-4d1a-9e0b-{index:012}"))
        .collect();
    let votes = |ids: &[&String]| {
        serde_json::to_string(
            &ids.iter()
                .map(|id| {
                    serde_json::json!({
                        "rule_set_id": id,
                        "rule_set_name": "Fixture rule",
                        "revision_number": 1,
                        "vote": "approve",
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap()
    };
    let seeded: Vec<(String, String)> = vec![
        ("decision-one".into(), votes(&[&rule_set_ids[0]])),
        (
            "decision-two".into(),
            votes(&[&rule_set_ids[0], &rule_set_ids[1]]),
        ),
        (
            "decision-repeat".into(),
            votes(&[&rule_set_ids[1], &rule_set_ids[1], &rule_set_ids[2]]),
        ),
        ("decision-none".into(), "[]".into()),
        (
            "decision-wide".into(),
            votes(&rule_set_ids.iter().collect::<Vec<_>>()),
        ),
    ];
    for (id, votes_json) in &seeded {
        let mut record = decision(
            id,
            &format!("request-{id}"),
            RequestDecisionOutcome::AutoApprove,
        );
        record.votes_json = votes_json.clone();
        decisions.record(&record).await.unwrap();
    }

    let mut checked = rule_set_ids.clone();
    checked.push("rule-unknown".to_string());
    for rule_set_id in &checked {
        let mut scanned: Vec<String> =
            sqlx::query_scalar("SELECT id FROM request_rule_decisions WHERE votes_json LIKE ?")
                .bind(format!("%{rule_set_id}%"))
                .fetch_all(pool)
                .await
                .unwrap();
        scanned.sort();
        let mut linked: Vec<String> = sqlx::query_scalar(
            "SELECT decision_id FROM request_rule_decision_rule_sets WHERE rule_set_id = ?",
        )
        .bind(rule_set_id)
        .fetch_all(pool)
        .await
        .unwrap();
        linked.sort();
        assert_eq!(linked, scanned, "{rule_set_id}");
        assert_eq!(
            decisions.count_for_rule_set(rule_set_id).await.unwrap(),
            scanned.len() as u64,
            "{rule_set_id}"
        );
    }
    assert_eq!(
        decisions
            .count_for_rule_set(&rule_set_ids[1])
            .await
            .unwrap(),
        3,
        "a decision that names a rule set twice counts once"
    );
    assert_eq!(
        decisions
            .count_for_rule_set(&rule_set_ids[449])
            .await
            .unwrap(),
        1,
        "a rule set past the first link statement is linked"
    );

    // The grouped read answers what the per-rule count answers, across more
    // ids than one statement binds.
    let mut many = checked.clone();
    many.extend((0..800).map(|index| format!("rule-absent-{index}")));
    many.push(rule_set_ids[3].clone());
    let grouped = decisions.count_for_rule_sets(&many).await.unwrap();
    for rule_set_id in &many {
        assert_eq!(
            grouped.get(rule_set_id).copied().unwrap_or(0),
            decisions.count_for_rule_set(rule_set_id).await.unwrap(),
            "{rule_set_id}"
        );
    }
    assert!(grouped.values().all(|count| *count > 0));
    let _ = std::fs::remove_file(db);
}

/// Seeds pre-flight traces as older builds wrote them beside real ones, then
/// removes the pre-flight ones in batches: only they and their links go, and
/// a pass with nothing left removes nothing.
async fn assert_preflight_traces_purge(store: &dyn RequestRuleDecisionRepository) -> AppResult<()> {
    let traced = |id: &str, request_id: &str| {
        let mut record = decision(id, request_id, RequestDecisionOutcome::AutoApprove);
        record.votes_json = r#"[{"rule_set_id":"rule-purge","vote":"approve"}]"#.to_string();
        record
    };
    for index in 0..5 {
        store
            .record(&traced(
                &format!("decision-preflight-{index}"),
                &format!(
                    "{}{index:04}",
                    scryer_application::PREFLIGHT_REQUEST_ID_PREFIX
                ),
            ))
            .await?;
    }
    for index in 0..3 {
        store
            .record(&traced(
                &format!("decision-real-{index}"),
                &format!("request-real-{index}"),
            ))
            .await?;
    }
    // A real request id that merely contains the word is not a pre-flight.
    store
        .record(&traced(
            "decision-real-lookalike",
            "request-preflight:lookalike",
        ))
        .await?;
    assert_eq!(store.count_for_rule_set("rule-purge").await?, 9);

    assert_eq!(store.delete_preflight_traces(2).await?, 2);
    assert_eq!(store.count_for_rule_set("rule-purge").await?, 7);
    assert_eq!(store.delete_preflight_traces(10).await?, 3);
    assert_eq!(store.delete_preflight_traces(10).await?, 0);
    assert_eq!(store.count_for_rule_set("rule-purge").await?, 4);

    let mut kept: Vec<String> = store
        .list_recent(1000, None)
        .await?
        .into_iter()
        .map(|row| row.id)
        .filter(|id| id.starts_with("decision-real") || id.starts_with("decision-preflight"))
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        vec![
            "decision-real-0",
            "decision-real-1",
            "decision-real-2",
            "decision-real-lookalike",
        ]
    );
    Ok(())
}

/// Seeds traces for two rule sets, one trace both voted in, and one no rule
/// set voted in, then prunes to the newest three per rule set: a trace goes
/// only when every rule set that voted in it has three newer ones.
async fn assert_superseded_traces_prune(
    store: &dyn RequestRuleDecisionRepository,
) -> AppResult<()> {
    let base = Utc::now() - chrono::Duration::days(1);
    let traced = |id: &str, minute: i64, rule_sets: &[&str]| {
        let mut record = decision(
            id,
            &format!("request-{id}"),
            RequestDecisionOutcome::AutoApprove,
        );
        record.evaluated_at = base + chrono::Duration::minutes(minute);
        record.created_at = record.evaluated_at;
        record.votes_json = serde_json::to_string(
            &rule_sets
                .iter()
                .map(|rule_set| serde_json::json!({ "rule_set_id": rule_set, "vote": "approve" }))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        record
    };
    // Rule set "keep-a": six traces, a0 oldest.
    for index in 0..6 {
        store
            .record(&traced(&format!("keep-a{index}"), index, &["keep-a"]))
            .await?;
    }
    // Rule set "keep-b": two of its own, plus one shared with "keep-a" that is
    // older than everything "keep-a" has, so only "keep-b" still counts it.
    store
        .record(&traced("keep-shared", -10, &["keep-a", "keep-b"]))
        .await?;
    store.record(&traced("keep-b0", 1, &["keep-b"])).await?;
    store.record(&traced("keep-b1", 2, &["keep-b"])).await?;
    // No rule set voted: fallback decisions, a group of their own. All are
    // older than everything a rule set voted in.
    for index in 0..5 {
        store
            .record(&traced(&format!("keep-u{index}"), -100 + index, &[]))
            .await?;
    }
    // A pre-flight trace has its own purge: it neither goes here nor takes
    // one of the kept places, though it is the oldest of all.
    let mut preflight = traced("keep-preflight", -200, &[]);
    preflight.request_id = "preflight:keep".to_string();
    store.record(&preflight).await?;

    assert_eq!(
        store
            .delete_traces_beyond_latest_per_rule_set(0, 100)
            .await?,
        0
    );
    assert_eq!(
        store.delete_traces_beyond_latest_per_rule_set(3, 0).await?,
        0
    );
    // The limit bounds one call; the rest goes on the next.
    assert_eq!(
        store.delete_traces_beyond_latest_per_rule_set(3, 2).await?,
        2
    );
    assert_eq!(
        store
            .delete_traces_beyond_latest_per_rule_set(3, 100)
            .await?,
        3
    );
    assert_eq!(
        store
            .delete_traces_beyond_latest_per_rule_set(3, 100)
            .await?,
        0
    );

    let mut kept: Vec<String> = store
        .list_recent(1000, None)
        .await?
        .into_iter()
        .map(|row| row.id)
        .filter(|id| id.starts_with("keep-"))
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        vec![
            "keep-a3",
            "keep-a4",
            "keep-a5",
            "keep-b0",
            "keep-b1",
            "keep-preflight",
            "keep-shared",
            "keep-u2",
            "keep-u3",
            "keep-u4",
        ]
    );
    // The removed traces took their links with them; the shared trace keeps
    // its link to both rule sets.
    assert_eq!(store.count_for_rule_set("keep-a").await?, 4);
    assert_eq!(store.count_for_rule_set("keep-b").await?, 3);
    Ok(())
}

#[tokio::test]
async fn superseded_traces_prune_to_the_newest_per_rule_set() {
    let (services, db) = temp_services("scryer_request_rule_trace_retention").await;
    let decisions = crate::RequestRuleDecisionStore::new(services.datastore());
    assert_superseded_traces_prune(&decisions)
        .await
        .expect("superseded traces prune");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn preflight_traces_purge_leaves_real_traces() {
    let (services, db) = temp_services("scryer_request_rule_preflight_purge").await;
    let decisions = crate::RequestRuleDecisionStore::new(services.datastore());
    assert_preflight_traces_purge(&decisions)
        .await
        .expect("pre-flight traces purge");

    // The batch select is served by the partial index, so a pass with nothing
    // left reads an empty index rather than the whole trace table.
    let detail: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT id FROM request_rule_decisions
          WHERE request_id LIKE 'preflight:%' LIMIT 500",
    )
    .fetch_all(services.pool())
    .await
    .unwrap();
    assert!(
        detail
            .iter()
            .any(|(_, _, _, step)| step.contains("idx_request_rule_decisions_preflight")),
        "{detail:?}"
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn request_rule_decisions_survive_rule_set_deletion() {
    let (services, db) = temp_services("scryer_request_rule_trace_survival").await;
    let rules = crate::RequestRuleSetStore::new(services.datastore());
    let decisions = crate::RequestRuleDecisionStore::new(services.datastore());
    assert_decisions_outlive_their_rule_set(&rules, &decisions)
        .await
        .expect("traces should outlive their rule set");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn request_rule_stores_round_trip_postgres() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping PostgreSQL request rule test; SCRYER_TEST_POSTGRES_URL is not set");
        return Ok(());
    };

    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_test_{}_{}",
        std::process::id(),
        Id::new().0.replace('-', "_")
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to create schema: {error}")))?;

    let result = async {
        let mut url = url::Url::parse(&raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let services =
            crate::PostgresServices::new_with_mode(url.to_string(), crate::MigrationMode::Apply)
                .await?;
        let rules = crate::RequestRuleSetStore::new(services.datastore());
        let decisions = crate::RequestRuleDecisionStore::new(services.datastore());
        let result = async {
            super::lifecycle_rule_sets::seed_rule_libraries(&services.datastore()).await;
            assert_rule_set_round_trip(&rules).await?;
            assert_decision_round_trip(&decisions).await?;
            assert_decisions_outlive_their_rule_set(&rules, &decisions).await?;
            assert_preflight_traces_purge(&decisions).await
        }
        .await;
        services.pool().close().await;
        result
    }
    .await;

    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    admin_pool.close().await;
    cleanup.map_err(|error| AppError::Repository(format!("failed to drop schema: {error}")))?;
    result
}
