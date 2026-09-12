//! Disarm only the destructive title rules whose set of possible series/anime
//! matches expands now that whole-show watch and episode-count facts exist.
//!
//! The migration is deliberately narrow. It preserves rule history and
//! candidates, marks the rule for an explicit review, and journals every
//! decision atomically so a later restart cannot disarm a newly acknowledged
//! rule if recording the application-migration ledger failed.

use chrono::Utc;
use scryer_infrastructure_sql::runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

use super::compatibility_journal as journal;

pub(crate) const ID: &str = "0016_maintenance_show_fact_rearm";

const NEWLY_EXECUTABLE_SHOW_FACTS: [&str; 7] = [
    "episode_count",
    "episode_file_count",
    "monitored_episode_count",
    "watched_by_user_ids",
    "last_watched_at",
    "watched_by_any_requester",
    "watched_by_all_requesters",
];

/// Apply the conservative compatibility review once.
///
/// A malformed source cannot prove that it avoids the newly executable facts,
/// so an otherwise eligible rule is disarmed and flagged for review. The
/// migration records that decision and keeps the original matcher and all
/// lifecycle history intact.
pub(crate) async fn migrate(datastore: &StoreDatastore) -> Result<(), String> {
    journal::ensure(datastore).await?;
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT rules.id AS rule_set_id, rules.current_revision_number, revisions.rego_source
           FROM maintenance_rule_sets rules
           JOIN maintenance_rule_revisions revisions
             ON revisions.rule_set_id = rules.id
            AND revisions.revision_number = rules.current_revision_number
          WHERE rules.subject_kind = 'title'
            AND rules.effect_arming = 'destructive'
            AND rules.destructive_rearm_required = {}
            AND (
                EXISTS (
                    SELECT 1
                      FROM maintenance_rule_set_libraries scope
                      JOIN libraries library ON library.id = scope.library_id
                     WHERE scope.rule_set_id = rules.id
                       AND library.facet IN ('series', 'anime')
                )
                OR NOT EXISTS (
                    SELECT 1 FROM maintenance_rule_set_libraries scope
                     WHERE scope.rule_set_id = rules.id
                )
            )
          ORDER BY rules.id",
        &[SqlArg::Bool(false)],
    )
    .await
    .map_err(|error| error.to_string())?;

    for row in rows {
        let rule_set_id = row.text("rule_set_id").map_err(|error| error.to_string())?;
        let revision_number = row
            .i64("current_revision_number")
            .map_err(|error| error.to_string())?;
        let rego_source = row.text("rego_source").map_err(|error| error.to_string())?;
        let digest = scryer_rules::runtime::content_hash(&rego_source);
        let review_detail = match scryer_rules::validation::maintenance_referenced_facts(
            &rego_source,
            &rule_set_id,
        ) {
            Ok(referenced)
                if NEWLY_EXECUTABLE_SHOW_FACTS
                    .iter()
                    .any(|fact| referenced.contains(*fact)) =>
            {
                Some(
                    "new show watch or episode-count facts can change this rule's matches"
                        .to_string(),
                )
            }
            Ok(_) => None,
            Err(error) => Some(format!(
                "the stored matcher could not be analyzed for newly executable show facts ({error}); review is required"
            )),
        };
        let Some(review_detail) = review_detail else {
            continue;
        };

        let subject = format!("maintenance-rule:{rule_set_id}");
        // The update and immutable journal row share one transaction. If the
        // outer migration ledger write fails after this commits, a restart sees
        // the journal and cannot disarm a rule an operator subsequently armed.
        SqlRuntime::run_in_transaction(
            datastore,
            "require_maintenance_show_fact_rearm",
            move |tx| {
                let rule_set_id = rule_set_id.clone();
                let subject = subject.clone();
                let digest = digest.clone();
                let rego_source = rego_source.clone();
                let review_detail = review_detail.clone();
                Box::pin(async move {
                    if SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT status FROM application_compatibility_journal
                          WHERE migration_id = {} AND subject_id = {} AND source_digest = {}",
                        &[
                            SqlArg::Text(ID.to_string()),
                            SqlArg::Text(subject.clone()),
                            SqlArg::Text(digest.clone()),
                        ],
                    )
                    .await?
                    .is_some()
                    {
                        return Ok(());
                    }
                    // Compare the current revision in the write as well as in
                    // the read: an operator edit already disarms the rule and
                    // must not receive a stale compatibility flag.
                    let disarmed = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE maintenance_rule_sets
                            SET effect_arming = {}, destructive_rearm_required = {}, updated_at = {}
                          WHERE id = {} AND current_revision_number = {}
                            AND effect_arming = {}",
                        &[
                            SqlArg::Text("none".to_string()),
                            SqlArg::Bool(true),
                            SqlArg::Timestamp(Utc::now()),
                            SqlArg::Text(rule_set_id),
                            SqlArg::I64(revision_number),
                            SqlArg::Text("destructive".to_string()),
                        ],
                    )
                    .await?;
                    if disarmed != 1 {
                        return Ok(());
                    }
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO application_compatibility_journal
                         (migration_id, subject_id, source_digest, original_metadata, status, detail)
                         VALUES ({}, {}, {}, {}, {}, {})",
                        &[
                            SqlArg::Text(ID.to_string()),
                            SqlArg::Text(subject),
                            SqlArg::Text(digest),
                            SqlArg::Text(rego_source),
                            SqlArg::Text("validated".to_string()),
                            SqlArg::OptText(Some(review_detail)),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};

    async fn seed_rule(
        datastore: &StoreDatastore,
        id: &str,
        source: &str,
        library_id: Option<&str>,
    ) {
        let now = Utc::now();
        SqlRuntime::execute_write(
            datastore,
            "seed_maintenance_rule_set",
            "INSERT INTO maintenance_rule_sets
             (id, name, effect_arming, subject_kind, current_revision_number, created_at, updated_at)
             VALUES ({}, {}, 'destructive', 'title', 1, {}, {})",
            vec![
                SqlArg::Text(id.to_string()),
                SqlArg::Text(id.to_string()),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await
        .expect("seed rule set");
        SqlRuntime::execute_write(
            datastore,
            "seed_maintenance_rule_revision",
            "INSERT INTO maintenance_rule_revisions
             (id, rule_set_id, revision_number, rego_source, action_spec, grace_days,
              matcher_content_hash, created_at)
             VALUES ({}, {}, 1, {}, {}, 0, {}, {})",
            vec![
                SqlArg::Text(format!("{id}:revision")),
                SqlArg::Text(id.to_string()),
                SqlArg::Text(source.to_string()),
                SqlArg::Text("{}".to_string()),
                SqlArg::Text(format!("hash:{id}")),
                SqlArg::Timestamp(now),
            ],
        )
        .await
        .expect("seed revision");
        if let Some(library_id) = library_id {
            SqlRuntime::execute_write(
                datastore,
                "seed_maintenance_rule_scope",
                "INSERT INTO maintenance_rule_set_libraries (rule_set_id, library_id, position)
                 VALUES ({}, {}, 0)",
                vec![
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text(library_id.to_string()),
                ],
            )
            .await
            .expect("seed rule scope");
        }
    }

    async fn rule_state(datastore: &StoreDatastore, id: &str) -> (String, bool) {
        let row = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT effect_arming, destructive_rearm_required
               FROM maintenance_rule_sets WHERE id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await
        .expect("read rule")
        .expect("rule exists");
        (
            row.text("effect_arming").expect("arming"),
            row.bool("destructive_rearm_required").expect("review flag"),
        )
    }

    #[tokio::test]
    async fn global_title_rules_are_reviewed_even_before_a_show_library_exists() {
        let temp = tempfile::tempdir().expect("tempdir");
        let services = SqliteServices::new_with_mode(
            temp.path()
                .join("maintenance.db")
                .to_string_lossy()
                .into_owned(),
            MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let datastore = services.datastore();
        // A movie-only install can add a show library after upgrade. A global
        // title rule must therefore be considered show-capable today.
        SqlRuntime::execute_write(
            &datastore,
            "remove_default_show_libraries",
            "DELETE FROM libraries WHERE facet IN ('series', 'anime')",
            vec![],
        )
        .await
        .expect("remove default show libraries");
        let show_fact_source =
            "package rules\nimport rego.v1\n\nmatch if input.facts.episode_count > 3\n";
        let unrelated_source = "package rules\nimport rego.v1\n\nmatch if input.facts.has_file\n";
        seed_rule(&datastore, "global-show-fact", show_fact_source, None).await;
        seed_rule(&datastore, "global-unrelated", unrelated_source, None).await;
        seed_rule(
            &datastore,
            "movie-only-show-fact",
            show_fact_source,
            Some("movie_default_library"),
        )
        .await;

        migrate(&datastore).await.expect("migration");

        assert_eq!(
            rule_state(&datastore, "global-show-fact").await,
            ("none".into(), true)
        );
        assert_eq!(
            rule_state(&datastore, "global-unrelated").await,
            ("destructive".into(), false)
        );
        assert_eq!(
            rule_state(&datastore, "movie-only-show-fact").await,
            ("destructive".into(), false)
        );

        // The compatibility journal makes a failed outer migration-ledger
        // retry harmless after an operator has acknowledged and re-armed.
        SqlRuntime::execute_write(
            &datastore,
            "operator_rearm",
            "UPDATE maintenance_rule_sets
                SET effect_arming = 'destructive', destructive_rearm_required = {} WHERE id = {}",
            vec![SqlArg::Bool(false), SqlArg::Text("global-show-fact".into())],
        )
        .await
        .expect("operator rearm");
        migrate(&datastore).await.expect("idempotent migration");
        assert_eq!(
            rule_state(&datastore, "global-show-fact").await,
            ("destructive".into(), false)
        );
    }
}
