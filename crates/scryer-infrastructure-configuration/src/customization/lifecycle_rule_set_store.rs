use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, LifecycleRuleSetRepository};
use scryer_domain::{
    LifecycleEffectArming, LifecycleEvaluationMode, LifecycleRuleRevision, LifecycleRuleSet,
};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};

#[derive(Clone)]
pub struct LifecycleRuleSetStore {
    datastore: StoreDatastore,
}

impl LifecycleRuleSetStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl LifecycleRuleSetRepository for LifecycleRuleSetStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<LifecycleRuleSet>> {
        let sql = format!("{RULE_SET_SELECT} ORDER BY name ASC, id ASC, scope.position ASC");
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &[]).await?;
        rows_to_rule_sets(&rows)
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<LifecycleRuleSet>> {
        let sql = format!("{RULE_SET_SELECT} WHERE id = {{}} ORDER BY scope.position ASC");
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(rows_to_rule_sets(&rows)?.into_iter().next())
    }

    async fn create_rule_set(
        &self,
        rule_set: &LifecycleRuleSet,
        revision: &LifecycleRuleRevision,
    ) -> AppResult<()> {
        let rule_set_args = rule_set_args(rule_set)?;
        let revision_args = revision_args(revision);
        let rule_set_id = rule_set.id.clone();
        let library_ids = rule_set.library_ids.clone();

        SqlRuntime::run_in_transaction(&self.datastore, "create_lifecycle_rule_set", move |tx| {
            let rule_set_args = rule_set_args.clone();
            let revision_args = revision_args.clone();
            let rule_set_id = rule_set_id.clone();
            let library_ids = library_ids.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_RULE_SET_SQL, &rule_set_args).await?;
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_REVISION_SQL, &revision_args).await?;
                replace_library_scope(tx, &rule_set_id, &library_ids).await?;
                Ok(())
            })
        })
        .await
    }

    async fn add_revision(
        &self,
        revision: &LifecycleRuleRevision,
        max_titles_per_run: Option<Option<u32>>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let revision_args = revision_args(revision);
        // The disarm rides in the pointer update rather than in a second call:
        // arming acknowledges one specific matcher's blast radius, so there must
        // be no instant at which the new revision is in force under the old
        // revision's arming. The pointer moves first and only from the
        // revision this one follows, as in `add_revision_keeping_arming`.
        // A new maximum rides in the same statement, so a source that gains a
        // delete head is never stored without the maximum it was saved with.
        let rule_set_id = revision.rule_set_id.clone();
        let mut pointer_args = vec![SqlArg::I64(revision.revision_number)];
        pointer_args.extend(disarm_args());
        let max_set = max_titles_per_run_set(max_titles_per_run, &mut pointer_args);
        pointer_args.extend([
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(revision.rule_set_id.clone()),
            SqlArg::I64(revision.revision_number - 1),
        ]);
        let pointer_sql = format!(
            "UPDATE lifecycle_rule_sets
                SET current_revision_number = {{}}, {DISARM_AFTER_EDIT_SET}{max_set}, updated_at = {{}}
              WHERE id = {{}} AND current_revision_number = {{}}"
        );

        SqlRuntime::run_in_transaction(&self.datastore, "add_lifecycle_rule_revision", move |tx| {
            let revision_args = revision_args.clone();
            let pointer_args = pointer_args.clone();
            let pointer_sql = pointer_sql.clone();
            let rule_set_id = rule_set_id.clone();
            Box::pin(async move {
                let moved =
                    SqlRuntime::execute(SqlExec::Tx(tx), &pointer_sql, &pointer_args).await?;
                if moved != 1 {
                    return Err(revision_conflict());
                }
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_REVISION_SQL, &revision_args).await?;
                require_stored_maximum(tx, &rule_set_id).await
            })
        })
        .await
    }

    async fn add_revision_keeping_arming(
        &self,
        revision: &LifecycleRuleRevision,
        effect_arming: LifecycleEffectArming,
        max_titles_per_run: Option<Option<u32>>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let revision_args = revision_args(revision);
        // The pointer moves first and only from the revision this one
        // follows, so a concurrent save that got there first makes this one
        // write nothing rather than collide on the revision number.
        //
        // The arming written is the lower of the stored one and the caller's:
        // the caller read the row earlier, and an edit must never raise an
        // arming another admin lowered in between. Only `effect_arming` and
        // the pointer move, so a concurrent state change (a disable) stands.
        let below: Vec<LifecycleEffectArming> = [
            LifecycleEffectArming::None,
            LifecycleEffectArming::Reversible,
            LifecycleEffectArming::Destructive,
        ]
        .into_iter()
        .take_while(|level| *level != effect_arming)
        .collect();
        let rule_set_id = revision.rule_set_id.clone();
        let mut pointer_args = vec![SqlArg::I64(revision.revision_number)];
        let arming_set = if below.is_empty() {
            pointer_args.push(SqlArg::Text(effect_arming.as_storage_str().to_string()));
            "effect_arming = {}".to_string()
        } else {
            pointer_args.extend(
                below
                    .iter()
                    .map(|level| SqlArg::Text(level.as_storage_str().to_string())),
            );
            pointer_args.push(SqlArg::Text(effect_arming.as_storage_str().to_string()));
            format!(
                "effect_arming = CASE WHEN effect_arming IN ({}) THEN effect_arming ELSE {{}} END",
                vec!["{}"; below.len()].join(", ")
            )
        };
        let max_set = max_titles_per_run_set(max_titles_per_run, &mut pointer_args);
        pointer_args.extend([
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(revision.rule_set_id.clone()),
            SqlArg::I64(revision.revision_number - 1),
        ]);
        let pointer_sql = format!(
            "UPDATE lifecycle_rule_sets
                SET current_revision_number = {{}}, {arming_set}{max_set}, updated_at = {{}}
              WHERE id = {{}} AND current_revision_number = {{}}"
        );

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "add_lifecycle_rule_revision_keeping_arming",
            move |tx| {
                let revision_args = revision_args.clone();
                let pointer_args = pointer_args.clone();
                let pointer_sql = pointer_sql.clone();
                let rule_set_id = rule_set_id.clone();
                Box::pin(async move {
                    let moved =
                        SqlRuntime::execute(SqlExec::Tx(tx), &pointer_sql, &pointer_args).await?;
                    if moved != 1 {
                        return Err(revision_conflict());
                    }
                    SqlRuntime::execute(SqlExec::Tx(tx), INSERT_REVISION_SQL, &revision_args)
                        .await?;
                    require_stored_maximum(tx, &rule_set_id).await
                })
            },
        )
        .await
    }

    async fn get_revision(
        &self,
        rule_set_id: &str,
        revision_number: i64,
    ) -> AppResult<Option<LifecycleRuleRevision>> {
        let sql = format!(
            "SELECT {REVISION_COLUMNS}
               FROM lifecycle_rule_revisions
              WHERE rule_set_id = {{}} AND revision_number = {{}}"
        );
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::I64(revision_number),
            ],
        )
        .await?
        .as_ref()
        .map(row_to_revision)
        .transpose()
    }

    async fn get_revisions(
        &self,
        keys: &[(String, i64)],
    ) -> AppResult<Vec<Option<LifecycleRuleRevision>>> {
        let mut found = std::collections::HashMap::new();
        for chunk in keys.chunks(REVISION_KEY_CHUNK) {
            let predicate =
                vec!["(rule_set_id = {} AND revision_number = {})"; chunk.len()].join(" OR ");
            let sql = format!(
                "SELECT {REVISION_COLUMNS}
                   FROM lifecycle_rule_revisions
                  WHERE {predicate}"
            );
            let args: Vec<SqlArg> = chunk
                .iter()
                .flat_map(|(rule_set_id, revision_number)| {
                    [
                        SqlArg::Text(rule_set_id.clone()),
                        SqlArg::I64(*revision_number),
                    ]
                })
                .collect();
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
                let revision = row_to_revision(&row)?;
                found.insert(
                    (revision.rule_set_id.clone(), revision.revision_number),
                    revision,
                );
            }
        }
        Ok(keys.iter().map(|key| found.get(key).cloned()).collect())
    }

    async fn list_revisions(&self, rule_set_id: &str) -> AppResult<Vec<LifecycleRuleRevision>> {
        let sql = format!(
            "SELECT {REVISION_COLUMNS}
               FROM lifecycle_rule_revisions
              WHERE rule_set_id = {{}}
              ORDER BY revision_number DESC"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(rule_set_id.to_string())],
        )
        .await?
        .iter()
        .map(row_to_revision)
        .collect()
    }

    async fn update_rule_set_metadata(
        &self,
        id: &str,
        name: &str,
        description: &str,
        library_ids: &[String],
        schedule: Option<&scryer_domain::ScriptSchedule>,
        max_titles_per_run: Option<Option<u32>>,
        disarm: bool,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut args = vec![
            SqlArg::Text(name.to_string()),
            SqlArg::Text(description.to_string()),
        ];
        let mut set = String::from("name = {}, description = {}");
        if let Some(schedule) = schedule {
            set.push_str(", schedule_json = {}");
            args.push(SqlArg::Text(schedule_json(schedule)?));
        }
        set.push_str(&max_titles_per_run_set(max_titles_per_run, &mut args));
        // The disarm rides in the same transaction as the scope that invalidated it,
        // exactly as `add_revision` carries its own: there must be no instant at
        // which a re-scoped rule is in force under the previous scope's arming.
        if disarm {
            set.push_str(", ");
            set.push_str(DISARM_AFTER_EDIT_SET);
            args.extend(disarm_args());
        }
        let sql = format!(
            "UPDATE lifecycle_rule_sets
                SET {set}, updated_at = {{}}
              WHERE id = {{}}"
        );
        args.push(SqlArg::Timestamp(updated_at));
        args.push(SqlArg::Text(id.to_string()));
        let id = id.to_string();
        let library_ids = library_ids.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_lifecycle_rule_set_metadata",
            move |tx| {
                let args = args.clone();
                let id = id.clone();
                let library_ids = library_ids.clone();
                let sql = sql.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                    require_stored_maximum(tx, &id).await?;
                    replace_library_scope(tx, &id, &library_ids).await
                })
            },
        )
        .await
    }

    async fn set_rule_set_max_titles_per_run(
        &self,
        id: &str,
        max_titles_per_run: Option<u32>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "set_lifecycle_rule_set_max_titles_per_run",
            "UPDATE lifecycle_rule_sets SET max_titles_per_run = {}, updated_at = {} WHERE id = {}",
            vec![
                SqlArg::OptI64(max_titles_per_run.map(i64::from)),
                SqlArg::Timestamp(updated_at),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_lifecycle_rule_set",
            "DELETE FROM lifecycle_rule_sets WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn delete_rule_set_if_unused(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_unused_lifecycle_rule", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let args = [SqlArg::Text(id)];
                // Take a parent-row write lock before the reads. PostgreSQL
                // child FK inserts take a conflicting key-share lock; SQLite
                // serializes the transaction's writers. Either insertion wins
                // and is observed here, or deletion wins and insertion fails.
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE lifecycle_rule_sets SET id = id WHERE id = {}", &args).await?;
                let used = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT id FROM lifecycle_action_runs WHERE rule_set_id = {} LIMIT 1", &args).await?.is_some()
                    || SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                        "SELECT id FROM lifecycle_candidates WHERE rule_set_id = {} AND state NOT IN ('canceled', 'excluded', 'succeeded', 'failed') LIMIT 1", &args).await?.is_some();
                if used {
                    return Err(scryer_application::AppError::validation_refused(crate::error_reasons::CONFIGURATION_LIFECYCLE_RULE_HAS_HISTORY, "This lifecycle rule has live candidates or action history; disable it instead."));
                }
                SqlRuntime::execute(SqlExec::Tx(tx), "DELETE FROM lifecycle_rule_sets WHERE id = {}", &args).await?;
                Ok(())
            })
        }).await
    }

    async fn update_rule_set_state(
        &self,
        id: &str,
        mode: LifecycleEvaluationMode,
        arming: LifecycleEffectArming,
        confirmation: Option<&scryer_domain::LifecycleRuleArmingConfirmation>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        // Every column moves in one statement: a row that is enabled while its
        // mode says disabled has no reading the evaluator could act on.
        let sql = "UPDATE lifecycle_rule_sets
                SET evaluation_mode = {}, enabled = {}, effect_arming = {},
                    destructive_rearm_required = {}, updated_at = {}
              WHERE id = {}";
        let args = vec![
            SqlArg::Text(mode.as_storage_str().to_string()),
            SqlArg::Bool(mode != LifecycleEvaluationMode::Disabled),
            SqlArg::Text(arming.as_storage_str().to_string()),
            SqlArg::Bool(false),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(id.to_string()),
        ];
        let id = id.to_string();
        let confirmation = confirmation.cloned();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_lifecycle_rule_set_state",
            move |tx| {
                let id = id.clone();
                let confirmation = confirmation.clone();
                let args = args.clone();
                Box::pin(async move {
                    // Revision and scope writers lock this parent row before
                    // changing scope rows. Take the same lock before reading:
                    // a pre-lock snapshot can miss a concurrent scope change
                    // under PostgreSQL READ COMMITTED. This also serializes
                    // SQLite writers without engine-specific lock syntax.
                    let locked = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE lifecycle_rule_sets SET id = id WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                    if locked != 1 {
                        return Ok(false);
                    }
                    if arming == LifecycleEffectArming::Destructive {
                        let rows = SqlRuntime::fetch_all(
                            SqlExec::Tx(tx),
                            &format!(
                                "{RULE_SET_SELECT} WHERE id = {{}} ORDER BY scope.position ASC"
                            ),
                            &[SqlArg::Text(id)],
                        )
                        .await?;
                        let rules = rows_to_rule_sets(&rows)?;
                        if !rules.first().is_some_and(|rule| {
                            confirmation
                                .as_ref()
                                .is_some_and(|reviewed| reviewed.matches(rule))
                        }) {
                            return Ok(false);
                        }
                    }
                    Ok(SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await? == 1)
                })
            },
        )
        .await
    }
}

// Disarms a rule after an edit, flagging it for review when it was armed. SET
// expressions read the row as it was before the update on both engines.
const DISARM_AFTER_EDIT_SET: &str = "destructive_rearm_required = CASE WHEN effect_arming <> {} THEN {} ELSE destructive_rearm_required END, effect_arming = {}";

/// A revision save that did not follow the revision in force.
fn revision_conflict() -> AppError {
    AppError::validation_refused(
        scryer_application::error_reasons::lifecycle::RULE_REVISION_CONFLICT,
        "the rule was changed by another save; reload it and save again".to_string(),
    )
}

/// Refuse, inside the write's own transaction, a rule whose revision in force
/// deletes while it carries no per-fire maximum.
///
/// Runs after the write's UPDATE of the rule-set row, so the row is already
/// write-locked (PostgreSQL) or the writer is exclusive (SQLite): a concurrent
/// save that clears the maximum or adds a delete head is ordered strictly
/// before or after this check, never between it and the write. A refusal rolls
/// the whole write back.
async fn require_stored_maximum(tx: &mut SqlTx<'_>, rule_set_id: &str) -> AppResult<()> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT s.max_titles_per_run, r.action_spec
           FROM lifecycle_rule_sets s
           JOIN lifecycle_rule_revisions r
             ON r.rule_set_id = s.id AND r.revision_number = s.current_revision_number
          WHERE s.id = {}",
        &[SqlArg::Text(rule_set_id.to_string())],
    )
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let max = row
        .opt_i64("max_titles_per_run")?
        .map(|max| u32::try_from(max).unwrap_or(u32::MAX));
    let declared =
        scryer_application::lifecycle_rules::LifecycleDeclaredActions::from_persisted_json(
            &row.text("action_spec")?,
        )
        .ok()
        .flatten();
    match declared {
        Some(declared) => {
            scryer_application::lifecycle_rules::service::require_max_titles_per_run(&declared, max)
        }
        None => Ok(()),
    }
}

/// The `, max_titles_per_run = {}` assignment a write adds when it replaces
/// the rule's per-fire maximum, pushing its bind onto `args`; empty when the
/// write keeps the stored one.
fn max_titles_per_run_set(
    max_titles_per_run: Option<Option<u32>>,
    args: &mut Vec<SqlArg>,
) -> String {
    match max_titles_per_run {
        Some(max) => {
            args.push(SqlArg::OptI64(max.map(i64::from)));
            ", max_titles_per_run = {}".to_string()
        }
        None => String::new(),
    }
}

fn disarm_args() -> [SqlArg; 3] {
    let none = LifecycleEffectArming::None.as_storage_str();
    [
        SqlArg::Text(none.to_string()),
        SqlArg::Bool(true),
        SqlArg::Text(none.to_string()),
    ]
}

// Fetch metadata and scope in one snapshot, including global rules with no scope rows.
const RULE_SET_SELECT: &str = "SELECT id, name, description, enabled, evaluation_mode, effect_arming, destructive_rearm_required,
    system_owned, current_revision_number, schedule_json, max_titles_per_run, created_at, updated_at, scope.library_id AS scope_library_id
    FROM lifecycle_rule_sets
    LEFT JOIN lifecycle_rule_set_libraries scope ON scope.rule_set_id = lifecycle_rule_sets.id";

const REVISION_COLUMNS: &str = "id, rule_set_id, revision_number, rego_source, action_spec,
    grace_days, matcher_content_hash, created_by, created_at, builder_model_json";

/// Revision keys per statement: two binds each, well under every engine's ceiling.
const REVISION_KEY_CHUNK: usize = 400;

const INSERT_RULE_SET_SQL: &str = "INSERT INTO lifecycle_rule_sets
        (id, name, description, enabled, evaluation_mode, effect_arming, destructive_rearm_required,
         system_owned, current_revision_number, schedule_json, max_titles_per_run, created_at, updated_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const INSERT_REVISION_SQL: &str = "INSERT INTO lifecycle_rule_revisions
        (id, rule_set_id, revision_number, rego_source, action_spec,
         grace_days, matcher_content_hash, created_by, created_at,
         builder_model_json)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: &'static str,
    args: Vec<SqlArg>,
) -> AppResult<()> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let args = args.clone();
        Box::pin(async move {
            SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await?;
            Ok(())
        })
    })
    .await
}

fn rule_set_args(rule_set: &LifecycleRuleSet) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(rule_set.id.clone()),
        SqlArg::Text(rule_set.name.clone()),
        SqlArg::Text(rule_set.description.clone()),
        SqlArg::Bool(rule_set.enabled),
        SqlArg::Text(rule_set.evaluation_mode.as_storage_str().to_string()),
        SqlArg::Text(rule_set.effect_arming.as_storage_str().to_string()),
        SqlArg::Bool(rule_set.destructive_rearm_required),
        SqlArg::Bool(rule_set.system_owned),
        SqlArg::I64(rule_set.current_revision_number),
        SqlArg::Text(schedule_json(&rule_set.schedule)?),
        SqlArg::OptI64(rule_set.max_titles_per_run.map(i64::from)),
        SqlArg::Timestamp(rule_set.created_at),
        SqlArg::Timestamp(rule_set.updated_at),
    ])
}

fn schedule_json(schedule: &scryer_domain::ScriptSchedule) -> AppResult<String> {
    serde_json::to_string(schedule).map_err(repo_err)
}

/// A rule with no stored schedule runs on the default one. A schedule this
/// build cannot decode was written by a newer build; it reads as manual so
/// this build never fires a rule on timing it does not understand.
fn stored_schedule(row: &SqlRow) -> AppResult<scryer_domain::ScriptSchedule> {
    let Some(raw) = row
        .opt_text("schedule_json")?
        .filter(|raw| !raw.trim().is_empty())
    else {
        return Ok(scryer_domain::default_lifecycle_rule_schedule());
    };
    Ok(serde_json::from_str(&raw).unwrap_or_else(|error| {
        tracing::warn!(%error, "lifecycle rule schedule could not be decoded; treating it as manual");
        scryer_domain::ScriptSchedule::Manual
    }))
}

fn revision_args(revision: &LifecycleRuleRevision) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(revision.id.clone()),
        SqlArg::Text(revision.rule_set_id.clone()),
        SqlArg::I64(revision.revision_number),
        SqlArg::Text(revision.rego_source.clone()),
        SqlArg::Text(revision.action_spec_json.clone()),
        SqlArg::I64(revision.grace_days),
        SqlArg::Text(revision.source_content_hash.clone()),
        SqlArg::OptText(revision.created_by.clone()),
        SqlArg::Timestamp(revision.created_at),
        SqlArg::OptText(revision.builder_model_json.clone()),
    ]
}

fn row_to_rule_set(row: &SqlRow) -> AppResult<LifecycleRuleSet> {
    Ok(LifecycleRuleSet {
        id: row.text("id")?,
        name: row.text("name")?,
        description: row.text("description")?,
        enabled: row.bool("enabled")?,
        // An unrecognized stored mode is a rule written by a newer build. It
        // reads back as disabled so this build never acts on semantics it does
        // not implement.
        evaluation_mode: LifecycleEvaluationMode::parse_storage(&row.text("evaluation_mode")?)
            .unwrap_or_default(),
        // Same newer-build rule as the mode: an unknown arming level reads as
        // none, so this build never acts under an arming it cannot interpret.
        effect_arming: LifecycleEffectArming::parse_storage(&row.text("effect_arming")?)
            .unwrap_or_default(),
        destructive_rearm_required: row.bool("destructive_rearm_required")?,
        library_ids: row.opt_text("scope_library_id")?.into_iter().collect(),
        system_owned: row.bool("system_owned")?,
        current_revision_number: row.i64("current_revision_number")?,
        schedule: stored_schedule(row)?,
        // A stored value below one acts on nothing; it was never written
        // through the service, which refuses it.
        max_titles_per_run: row
            .opt_i64("max_titles_per_run")?
            .map(|max| u32::try_from(max.max(0)).unwrap_or(u32::MAX)),
        created_at: row.timestamp_or_now("created_at")?,
        updated_at: row.timestamp_or_now("updated_at")?,
    })
}

fn row_to_revision(row: &SqlRow) -> AppResult<LifecycleRuleRevision> {
    Ok(LifecycleRuleRevision {
        id: row.text("id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        rego_source: row.text("rego_source")?,
        action_spec_json: row.text("action_spec")?,
        grace_days: row.i64("grace_days")?,
        source_content_hash: row.text("matcher_content_hash")?,
        created_by: row.opt_text("created_by")?,
        created_at: row.timestamp_or_now("created_at")?,
        builder_model_json: row.opt_text("builder_model_json")?,
    })
}

fn rows_to_rule_sets(rows: &[SqlRow]) -> AppResult<Vec<LifecycleRuleSet>> {
    let mut rules: Vec<LifecycleRuleSet> = Vec::new();
    for row in rows {
        let id = row.text("id")?;
        if let Some(rule) = rules.last_mut().filter(|rule| rule.id == id) {
            rule.library_ids.extend(row.opt_text("scope_library_id")?);
        } else {
            rules.push(row_to_rule_set(row)?);
        }
    }
    Ok(rules)
}

async fn replace_library_scope(
    tx: &mut SqlTx<'_>,
    rule_set_id: &str,
    library_ids: &[String],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM lifecycle_rule_set_libraries WHERE rule_set_id = {}",
        &[SqlArg::Text(rule_set_id.to_string())],
    )
    .await?;
    let mut seen = std::collections::HashSet::new();
    for (position, library_id) in library_ids.iter().enumerate() {
        if !seen.insert(library_id) {
            continue;
        }
        tx.execute(
            "INSERT INTO lifecycle_rule_set_libraries (rule_set_id, library_id, position)
             VALUES ({}, {}, {})",
            &[
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::Text(library_id.clone()),
                SqlArg::I64(position as i64),
            ],
        )
        .await?;
    }
    Ok(())
}
