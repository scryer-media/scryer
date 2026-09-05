//! Round-trips for the request rule set, revision, and decision stores
//! (spec 0003 section 6).
//!
//! Every assertion body is shared between the SQLite and PostgreSQL tests: the
//! two dialects have different timestamp and boolean storage, and a round-trip
//! that only ever ran on SQLite would let a Postgres-only mapping bug ship.

use super::*;
use scryer_application::{RequestRuleDecisionRepository, RequestRuleSetRepository};
use scryer_domain::{
    RequestDecisionOutcome, RequestRuleDecisionRecord, RequestRuleEvaluationMode,
    RequestRuleRevision, RequestRuleSet,
};

fn rule_set(id: &str, library_ids: Vec<String>) -> RequestRuleSet {
    let now = Utc::now();
    RequestRuleSet {
        id: id.to_string(),
        name: "Family friendly".to_string(),
        description: "Auto-approve low-certification titles".to_string(),
        enabled: false,
        evaluation_mode: RequestRuleEvaluationMode::Disabled,
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
        matcher_content_hash: format!("hash-{number}"),
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
        mode: RequestRuleEvaluationMode::Enforce,
        effective_outcome: outcome,
        policy_outcome: outcome,
        fallback_reason: None,
        votes_json: r#"[{"ruleSetId":"rule-a","vote":"approve"}]"#.to_string(),
        tags: vec!["family".to_string()],
        input_hash: "input-hash-1".to_string(),
        input_schema_version: 1,
        created_at: now,
    }
}

async fn assert_rule_set_round_trip(store: &dyn RequestRuleSetRepository) -> AppResult<()> {
    let created = rule_set("rule-a", vec!["library-1".to_string()]);
    store
        .create_rule_set(&created, &revision("rule-a", 1))
        .await?;

    let loaded = store
        .get_rule_set("rule-a")
        .await?
        .expect("rule set should exist");
    assert_eq!(loaded.name, created.name);
    assert_eq!(loaded.description, created.description);
    assert!(!loaded.enabled, "a new rule set is created disabled");
    assert_eq!(loaded.evaluation_mode, RequestRuleEvaluationMode::Disabled);
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
    assert_eq!(stored_revision.matcher_content_hash, "hash-1");
    assert_eq!(stored_revision.created_by.as_deref(), Some("user-1"));

    // Appending a revision repoints the rule set and rewrites nothing.
    let updated_at = Utc::now();
    store
        .add_revision(&revision("rule-a", 2), updated_at)
        .await?;
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
            .matcher_content_hash,
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
        .add_revision(&revision("rule-a", 2), Utc::now())
        .await
        .expect_err("the unique constraint must reject a replayed revision number");
    assert_eq!(store.list_revisions("rule-a").await?.len(), 2);

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
        .update_rule_set_evaluation_mode(
            "rule-a",
            RequestRuleEvaluationMode::Shadow,
            true,
            Utc::now(),
        )
        .await?;
    let loaded = store
        .get_rule_set("rule-a")
        .await?
        .expect("rule set should exist");
    assert_eq!(loaded.name, "Renamed");
    assert_eq!(loaded.library_ids, vec!["library-9".to_string()]);
    assert_eq!(loaded.evaluation_mode, RequestRuleEvaluationMode::Shadow);
    assert!(loaded.enabled);
    assert_eq!(
        loaded.current_revision_number, 2,
        "metadata and mode edits leave the revision pointer alone"
    );

    // The FK cascade is what keeps a deleted rule from leaving orphan revisions.
    store.delete_rule_set("rule-a").await?;
    assert!(store.get_rule_set("rule-a").await?.is_none());
    assert!(store.list_revisions("rule-a").await?.is_empty());
    Ok(())
}

async fn assert_decision_round_trip(store: &dyn RequestRuleDecisionRepository) -> AppResult<()> {
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
    // Shadow and enforce disagree on purpose, so the two verdicts are stored
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

    // The rule ids live inside the serialized votes, so the count is a
    // substring match over that column.
    assert_eq!(store.count_for_rule_set("rule-a").await?, 3);
    assert_eq!(store.count_for_rule_set("rule-zzz").await?, 0);
    assert_eq!(store.count_for_rule_set("  ").await?, 0);
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
        .create_rule_set(&rule_set("rule-b", Vec::new()), &revision("rule-b", 1))
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
    assert_rule_set_round_trip(&store)
        .await
        .expect("rule sets should round-trip");
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
            assert_rule_set_round_trip(&rules).await?;
            assert_decision_round_trip(&decisions).await?;
            assert_decisions_outlive_their_rule_set(&rules, &decisions).await
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
