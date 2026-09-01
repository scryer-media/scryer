use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppResult, MaintenanceRuleSetRepository};
use scryer_domain::{
    MaintenanceEffectArming, MaintenanceEvaluationMode, MaintenanceRuleRevision,
    MaintenanceRuleSet, MaintenanceRuleSubjectKind,
};
use sqlx::Row;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};
use crate::storage::sql::json::{canonical_json_text, json_text_or};

#[derive(Clone)]
pub struct MaintenanceRuleSetStore {
    datastore: StoreDatastore,
}

impl MaintenanceRuleSetStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl MaintenanceRuleSetRepository for MaintenanceRuleSetStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<MaintenanceRuleSet>> {
        let sql = format!(
            "SELECT {RULE_SET_COLUMNS} FROM maintenance_rule_sets ORDER BY name ASC, id ASC"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &[])
            .await?
            .iter()
            .map(row_to_rule_set)
            .collect()
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<MaintenanceRuleSet>> {
        let sql = format!("SELECT {RULE_SET_COLUMNS} FROM maintenance_rule_sets WHERE id = {{}}");
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_rule_set)
        .transpose()
    }

    async fn create_rule_set(
        &self,
        rule_set: &MaintenanceRuleSet,
        revision: &MaintenanceRuleRevision,
    ) -> AppResult<()> {
        let rule_set_args = rule_set_args(rule_set)?;
        let revision_args = revision_args(revision);

        SqlRuntime::run_in_transaction(&self.datastore, "create_maintenance_rule_set", move |tx| {
            let rule_set_args = rule_set_args.clone();
            let revision_args = revision_args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_RULE_SET_SQL, &rule_set_args).await?;
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_REVISION_SQL, &revision_args).await?;
                Ok(())
            })
        })
        .await
    }

    async fn add_revision(
        &self,
        revision: &MaintenanceRuleRevision,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let revision_args = revision_args(revision);
        // The disarm rides in the pointer update rather than in a second call:
        // arming acknowledges one specific matcher's blast radius, so there must
        // be no instant at which the new revision is in force under the old
        // revision's arming.
        let pointer_args = vec![
            SqlArg::I64(revision.revision_number),
            SqlArg::Text(MaintenanceEffectArming::None.as_storage_str().to_string()),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(revision.rule_set_id.clone()),
        ];

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "add_maintenance_rule_revision",
            move |tx| {
                let revision_args = revision_args.clone();
                let pointer_args = pointer_args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), INSERT_REVISION_SQL, &revision_args)
                        .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE maintenance_rule_sets
                            SET current_revision_number = {}, effect_arming = {}, updated_at = {}
                          WHERE id = {}",
                        &pointer_args,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_revision(
        &self,
        rule_set_id: &str,
        revision_number: i64,
    ) -> AppResult<Option<MaintenanceRuleRevision>> {
        let sql = format!(
            "SELECT {REVISION_COLUMNS}
               FROM maintenance_rule_revisions
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

    async fn list_revisions(&self, rule_set_id: &str) -> AppResult<Vec<MaintenanceRuleRevision>> {
        let sql = format!(
            "SELECT {REVISION_COLUMNS}
               FROM maintenance_rule_revisions
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
        disarm: bool,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut args = vec![
            SqlArg::Text(name.to_string()),
            SqlArg::Text(description.to_string()),
            SqlArg::Text(canonical_json_text(&library_ids)?),
        ];
        // The disarm rides in the same UPDATE as the scope that invalidated it,
        // exactly as `add_revision` carries its own: there must be no instant at
        // which a re-scoped rule is in force under the previous scope's arming.
        let sql = if disarm {
            args.push(SqlArg::Text(
                MaintenanceEffectArming::None.as_storage_str().to_string(),
            ));
            "UPDATE maintenance_rule_sets
                SET name = {}, description = {}, library_ids = {}, effect_arming = {},
                    updated_at = {}
              WHERE id = {}"
        } else {
            "UPDATE maintenance_rule_sets
                SET name = {}, description = {}, library_ids = {}, updated_at = {}
              WHERE id = {}"
        };
        args.push(SqlArg::Timestamp(updated_at));
        args.push(SqlArg::Text(id.to_string()));
        execute_write(
            &self.datastore,
            "update_maintenance_rule_set_metadata",
            sql,
            args,
        )
        .await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_maintenance_rule_set",
            "DELETE FROM maintenance_rule_sets WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn update_rule_set_evaluation_mode(
        &self,
        id: &str,
        mode: MaintenanceEvaluationMode,
        enabled: bool,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        // Both columns move in one statement: a row that is enabled while its
        // mode says disabled has no reading the evaluator could act on.
        execute_write(
            &self.datastore,
            "update_maintenance_rule_set_evaluation_mode",
            "UPDATE maintenance_rule_sets
                SET evaluation_mode = {}, enabled = {}, updated_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(mode.as_storage_str().to_string()),
                SqlArg::Bool(enabled),
                SqlArg::Timestamp(updated_at),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await
    }

    async fn update_rule_set_arming(
        &self,
        id: &str,
        arming: MaintenanceEffectArming,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "update_maintenance_rule_set_arming",
            "UPDATE maintenance_rule_sets
                SET effect_arming = {}, updated_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(arming.as_storage_str().to_string()),
                SqlArg::Timestamp(updated_at),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await
    }
}

const RULE_SET_COLUMNS: &str = "id, name, description, enabled, evaluation_mode, effect_arming,
    subject_kind, library_ids, current_revision_number, created_at, updated_at";

const REVISION_COLUMNS: &str = "id, rule_set_id, revision_number, rego_source, action_spec,
    grace_days, matcher_content_hash, created_by, created_at";

const INSERT_RULE_SET_SQL: &str = "INSERT INTO maintenance_rule_sets
        (id, name, description, enabled, evaluation_mode, effect_arming, subject_kind,
         library_ids, current_revision_number, created_at, updated_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const INSERT_REVISION_SQL: &str = "INSERT INTO maintenance_rule_revisions
        (id, rule_set_id, revision_number, rego_source, action_spec,
         grace_days, matcher_content_hash, created_by, created_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})";

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

fn rule_set_args(rule_set: &MaintenanceRuleSet) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(rule_set.id.clone()),
        SqlArg::Text(rule_set.name.clone()),
        SqlArg::Text(rule_set.description.clone()),
        SqlArg::Bool(rule_set.enabled),
        SqlArg::Text(rule_set.evaluation_mode.as_storage_str().to_string()),
        SqlArg::Text(rule_set.effect_arming.as_storage_str().to_string()),
        SqlArg::Text(rule_set.subject_kind.as_storage_str().to_string()),
        SqlArg::Text(canonical_json_text(&rule_set.library_ids)?),
        SqlArg::I64(rule_set.current_revision_number),
        SqlArg::Timestamp(rule_set.created_at),
        SqlArg::Timestamp(rule_set.updated_at),
    ])
}

fn revision_args(revision: &MaintenanceRuleRevision) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(revision.id.clone()),
        SqlArg::Text(revision.rule_set_id.clone()),
        SqlArg::I64(revision.revision_number),
        SqlArg::Text(revision.rego_source.clone()),
        SqlArg::Text(revision.action_spec_json.clone()),
        SqlArg::I64(revision.grace_days),
        SqlArg::Text(revision.matcher_content_hash.clone()),
        SqlArg::OptText(revision.created_by.clone()),
        SqlArg::Timestamp(revision.created_at),
    ]
}

fn row_to_rule_set(row: &SqlRow) -> AppResult<MaintenanceRuleSet> {
    Ok(MaintenanceRuleSet {
        id: row.text("id")?,
        name: row.text("name")?,
        description: row.text("description")?,
        enabled: row.bool("enabled")?,
        // An unrecognized stored mode is a rule written by a newer build. It
        // reads back as disabled so this build never acts on semantics it does
        // not implement.
        evaluation_mode: MaintenanceEvaluationMode::parse_storage(&row.text("evaluation_mode")?)
            .unwrap_or_default(),
        // Same newer-build rule as the mode: an unknown arming level reads as
        // none, so this build never acts under an arming it cannot interpret.
        effect_arming: MaintenanceEffectArming::parse_storage(&row.text("effect_arming")?)
            .unwrap_or_default(),
        library_ids: library_ids(row)?,
        subject_kind: MaintenanceRuleSubjectKind::parse_storage(&row.text("subject_kind")?)
            .unwrap_or_default(),
        current_revision_number: row.i64("current_revision_number")?,
        created_at: timestamp_or_now(row, "created_at")?,
        updated_at: timestamp_or_now(row, "updated_at")?,
    })
}

fn row_to_revision(row: &SqlRow) -> AppResult<MaintenanceRuleRevision> {
    Ok(MaintenanceRuleRevision {
        id: row.text("id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        rego_source: row.text("rego_source")?,
        action_spec_json: row.text("action_spec")?,
        grace_days: row.i64("grace_days")?,
        matcher_content_hash: row.text("matcher_content_hash")?,
        created_by: row.opt_text("created_by")?,
        created_at: timestamp_or_now(row, "created_at")?,
    })
}

/// Stored as a JSON text array; empty means every library.
fn library_ids(row: &SqlRow) -> AppResult<Vec<String>> {
    let raw = json_text_or(row, "library_ids", "[]")?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn timestamp_or_now(row: &SqlRow, column: &str) -> AppResult<DateTime<Utc>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            Ok(DateTime::parse_from_rfc3339(&raw)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()))
        }
        SqlRow::Postgres(_) => row.timestamp(column),
    }
}
