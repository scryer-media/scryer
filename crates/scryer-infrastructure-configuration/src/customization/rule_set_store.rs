use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, RuleSetHistoryChange, RuleSetRepository};
use scryer_domain::{Id, MediaFacet, RulePackInstallation, RulePackMember, RuleSet};
use sqlx::Row;
use std::sync::Arc;

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::storage::sql::json::{
    canonical_json_arg, canonical_json_text, json_text_or, opt_json_text,
};

#[derive(Clone)]
pub struct RuleSetStore {
    datastore: StoreDatastore,
}

impl RuleSetStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl RuleSetRepository for RuleSetStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let sql =
            format!("SELECT {RULE_SET_COLUMNS} FROM rule_sets ORDER BY priority DESC, name ASC");
        fetch_rule_sets(self.datastore.read_exec(), &sql, &[]).await
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let sql = format!(
            "SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE enabled = {{}} ORDER BY priority DESC, name ASC"
        );
        fetch_rule_sets(self.datastore.read_exec(), &sql, &[SqlArg::Bool(true)]).await
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        let sql = format!("SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE id = {{}}");
        fetch_optional_rule_set(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let args = rule_set_args(rule_set)?;
        execute_write(
            &self.datastore,
            "create_rule_set",
            "INSERT INTO rule_sets
                (id, name, description, rego_source, enabled, priority,
                 applied_facets, created_at, updated_at, is_managed, managed_key,
                 managed_tag_filter)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            args,
        )
        .await
    }

    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let args = rule_set_args(rule_set)?;
        execute_write(
            &self.datastore,
            "update_rule_set",
            "UPDATE rule_sets
                SET name = {}, description = {}, rego_source = {}, enabled = {},
                    priority = {}, applied_facets = {}, updated_at = {},
                    is_managed = {}, managed_key = {}, managed_tag_filter = {}
              WHERE id = {}",
            vec![
                args[1].clone(),
                args[2].clone(),
                args[3].clone(),
                args[4].clone(),
                args[5].clone(),
                args[6].clone(),
                args[8].clone(),
                args[9].clone(),
                args[10].clone(),
                args[11].clone(),
                args[0].clone(),
            ],
        )
        .await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_rule_set",
            "DELETE FROM rule_sets WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "record_rule_set_history",
            "INSERT INTO rule_set_history
                (id, rule_set_id, action, rego_source, actor_id, created_at)
             VALUES ({}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(Id::new().0),
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::Text(action.to_string()),
                SqlArg::OptText(rego_source.map(str::to_string)),
                SqlArg::OptText(actor_id.map(str::to_string)),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
    }

    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
        let sql =
            format!("SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE managed_key = {{}} LIMIT 1");
        fetch_optional_rule_set(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(key.to_string())],
        )
        .await
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_rule_set_by_managed_key",
            "DELETE FROM rule_sets WHERE managed_key = {}",
            vec![SqlArg::Text(key.to_string())],
        )
        .await
    }

    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>> {
        let pattern = format!("{prefix}%");
        let sql = format!(
            "SELECT {RULE_SET_COLUMNS}
               FROM rule_sets
              WHERE managed_key LIKE {{}}
              ORDER BY managed_key"
        );
        fetch_rule_sets(self.datastore.read_exec(), &sql, &[SqlArg::Text(pattern)]).await
    }

    async fn list_rule_pack_installations(&self) -> AppResult<Vec<RulePackInstallation>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT pack_id, name, version, digest, auto_update, revision, last_updated, last_error
               FROM rule_pack_installations ORDER BY name ASC, pack_id ASC",
            &[],
        )
        .await?;
        let mut installations = Vec::with_capacity(rows.len());
        for row in rows {
            installations.push(row_to_rule_pack_installation(&self.datastore, &row).await?);
        }
        Ok(installations)
    }

    async fn get_rule_pack_installation(
        &self,
        pack_id: &str,
    ) -> AppResult<Option<RulePackInstallation>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT pack_id, name, version, digest, auto_update, revision, last_updated, last_error
               FROM rule_pack_installations WHERE pack_id = {}",
            &[SqlArg::Text(pack_id.to_string())],
        )
        .await?;
        match row {
            Some(row) => row_to_rule_pack_installation(&self.datastore, &row)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    async fn find_rule_pack_installation_by_rule_set_id(
        &self,
        rule_set_id: &str,
    ) -> AppResult<Option<RulePackInstallation>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT i.pack_id, i.name, i.version, i.digest, i.auto_update, i.revision,
                    i.last_updated, i.last_error
               FROM rule_pack_installations i
               JOIN rule_pack_members m ON m.pack_id = i.pack_id
              WHERE m.rule_set_id = {}",
            &[SqlArg::Text(rule_set_id.to_string())],
        )
        .await?;
        match row {
            Some(row) => row_to_rule_pack_installation(&self.datastore, &row)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    async fn apply_rule_pack_installation(
        &self,
        installation: &RulePackInstallation,
        expected_revision: Option<i64>,
        changed_rule_sets: &[RuleSet],
        history: &[RuleSetHistoryChange],
    ) -> AppResult<bool> {
        let installation = installation.clone();
        let changed_rule_sets: Arc<[RuleSet]> = changed_rule_sets.into();
        let history: Arc<[RuleSetHistoryChange]> = history.into();
        SqlRuntime::run_in_transaction(&self.datastore, "apply_rule_pack_installation", move |tx| {
            let installation = installation.clone();
            let changed_rule_sets = changed_rule_sets.clone();
            let history = history.clone();
            Box::pin(async move {
                let install_args = rule_pack_installation_args(&installation);
                let applied = match expected_revision {
                    Some(expected_revision) => SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE rule_pack_installations
                            SET name = {}, version = {}, digest = {}, auto_update = {},
                                revision = {}, last_updated = {}, last_error = {}
                          WHERE pack_id = {} AND revision = {}",
                        &[
                            install_args[1].clone(), install_args[2].clone(), install_args[3].clone(),
                            install_args[4].clone(), install_args[5].clone(), install_args[6].clone(),
                            install_args[7].clone(), install_args[0].clone(), SqlArg::I64(expected_revision),
                        ],
                    ).await?,
                    None => SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO rule_pack_installations
                            (pack_id, name, version, digest, auto_update, revision, last_updated, last_error)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}) ON CONFLICT(pack_id) DO NOTHING",
                        &install_args,
                    ).await?,
                };
                if applied != 1 {
                    return Ok(false);
                }

                // Members reference rule rows. Write rule rows first so a
                // first install never violates that foreign key.
                for rule_set in changed_rule_sets.iter() {
                    SqlRuntime::execute(SqlExec::Tx(tx), UPSERT_RULE_SET_SQL, &rule_set_args(rule_set)?).await?;
                }
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE rule_pack_members SET removed = {} WHERE pack_id = {}",
                    &[SqlArg::Bool(true), SqlArg::Text(installation.pack_id.clone())],
                ).await?;
                for member in &installation.members {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO rule_pack_members (pack_id, template_id, rule_set_id, removed)
                         VALUES ({}, {}, {}, {})
                         ON CONFLICT(pack_id, template_id) DO UPDATE
                            SET rule_set_id = excluded.rule_set_id, removed = excluded.removed",
                        &rule_pack_member_args(&installation.pack_id, member),
                    ).await?;
                }
                insert_rule_set_history(tx, &history).await?;
                Ok(true)
            })
        }).await
    }

    async fn uninstall_rule_pack(
        &self,
        pack_id: &str,
        expected_revision: i64,
        history: &[RuleSetHistoryChange],
    ) -> AppResult<bool> {
        let pack_id = pack_id.to_string();
        let history = history.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "uninstall_rule_pack", move |tx| {
            let pack_id = pack_id.clone();
            let history = history.clone();
            Box::pin(async move {
                let claimed = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE rule_pack_installations SET revision = revision
                      WHERE pack_id = {} AND revision = {}",
                    &[
                        SqlArg::Text(pack_id.clone()),
                        SqlArg::I64(expected_revision),
                    ],
                )
                .await?;
                if claimed != 1 {
                    return Ok(false);
                }
                insert_rule_set_history(tx, &history).await?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM rule_sets WHERE id IN
                        (SELECT rule_set_id FROM rule_pack_members WHERE pack_id = {})",
                    &[SqlArg::Text(pack_id.clone())],
                )
                .await?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM rule_pack_installations WHERE pack_id = {}",
                    &[SqlArg::Text(pack_id)],
                )
                .await?;
                Ok(true)
            })
        })
        .await
    }

    async fn copy_rule_pack_rule_set_to_custom(
        &self,
        pack_id: &str,
        source_rule_set_id: &str,
        custom_rule_set: &RuleSet,
        expected_revision: i64,
        history: &[RuleSetHistoryChange],
    ) -> AppResult<bool> {
        if custom_rule_set.is_managed {
            return Err(AppError::Validation(
                "copied rule sets must be ordinary user rules".to_string(),
            ));
        }
        let pack_id = pack_id.to_string();
        let source_rule_set_id = source_rule_set_id.to_string();
        let custom_rule_set = custom_rule_set.clone();
        let history = history.to_vec();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "copy_rule_pack_rule_set_to_custom",
            move |tx| {
                let pack_id = pack_id.clone();
                let source_rule_set_id = source_rule_set_id.clone();
                let custom_rule_set = custom_rule_set.clone();
                let history = history.clone();
                Box::pin(async move {
                    let updated = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE rule_pack_installations
                        SET revision = revision + 1, last_updated = {}
                      WHERE pack_id = {} AND revision = {}",
                        &[
                            SqlArg::Timestamp(Utc::now()),
                            SqlArg::Text(pack_id.clone()),
                            SqlArg::I64(expected_revision),
                        ],
                    )
                    .await?;
                    if updated != 1 {
                        return Ok(false);
                    }
                    let disabled = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE rule_sets SET enabled = {}, updated_at = {}
                      WHERE id = {} AND EXISTS (
                          SELECT 1 FROM rule_pack_members
                           WHERE pack_id = {} AND rule_set_id = {}
                      )",
                        &[
                            SqlArg::Bool(false),
                            SqlArg::Timestamp(Utc::now()),
                            SqlArg::Text(source_rule_set_id.clone()),
                            SqlArg::Text(pack_id),
                            SqlArg::Text(source_rule_set_id),
                        ],
                    )
                    .await?;
                    if disabled != 1 {
                        return Err(AppError::Validation(
                            "rule set is not owned by the requested pack".to_string(),
                        ));
                    }
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        INSERT_RULE_SET_SQL,
                        &rule_set_args(&custom_rule_set)?,
                    )
                    .await?;
                    insert_rule_set_history(tx, &history).await?;
                    Ok(true)
                })
            },
        )
        .await
    }
}

const RULE_SET_COLUMNS: &str = "id, name, description, rego_source, enabled, priority,
    applied_facets, created_at, updated_at, is_managed, managed_key, managed_tag_filter";

const UPSERT_RULE_SET_SQL: &str = "INSERT INTO rule_sets
    (id, name, description, rego_source, enabled, priority, applied_facets, created_at,
     updated_at, is_managed, managed_key, managed_tag_filter)
 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
 ON CONFLICT(id) DO UPDATE SET
    name = excluded.name, description = excluded.description, rego_source = excluded.rego_source,
    enabled = excluded.enabled, priority = excluded.priority, applied_facets = excluded.applied_facets,
    updated_at = excluded.updated_at, is_managed = excluded.is_managed,
    managed_key = excluded.managed_key, managed_tag_filter = excluded.managed_tag_filter";

const INSERT_RULE_SET_SQL: &str = "INSERT INTO rule_sets
    (id, name, description, rego_source, enabled, priority, applied_facets, created_at,
     updated_at, is_managed, managed_key, managed_tag_filter)
 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

async fn fetch_rule_sets(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<RuleSet>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(row_to_rule_set)
        .collect()
}

async fn fetch_optional_rule_set(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<RuleSet>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .as_ref()
        .map(row_to_rule_set)
        .transpose()
}

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

fn rule_set_args(rule_set: &RuleSet) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(rule_set.id.clone()),
        SqlArg::Text(rule_set.name.clone()),
        SqlArg::Text(rule_set.description.clone()),
        SqlArg::Text(rule_set.rego_source.clone()),
        SqlArg::Bool(rule_set.enabled),
        SqlArg::I32(rule_set.priority),
        canonical_json_arg(&rule_set.applied_facets)?,
        SqlArg::Timestamp(rule_set.created_at),
        SqlArg::Timestamp(rule_set.updated_at),
        SqlArg::Bool(rule_set.is_managed),
        SqlArg::OptText(rule_set.managed_key.clone()),
        managed_tag_filter_arg(rule_set.managed_tag_filter.as_deref())?,
    ])
}

/// Stored as a JSON text array, or SQL NULL when the pack is unfiltered.
fn managed_tag_filter_arg(tags: Option<&[String]>) -> AppResult<SqlArg> {
    match tags {
        Some(tags) => canonical_json_text(&tags).map(|json| SqlArg::OptText(Some(json))),
        None => Ok(SqlArg::OptText(None)),
    }
}

fn row_to_rule_set(row: &SqlRow) -> AppResult<RuleSet> {
    Ok(RuleSet {
        id: row.text("id")?,
        name: row.text("name")?,
        description: row.text("description")?,
        rego_source: row.text("rego_source")?,
        enabled: row.bool("enabled")?,
        priority: row.i32("priority")?,
        applied_facets: applied_facets(row)?,
        created_at: timestamp_or_now(row, "created_at")?,
        updated_at: timestamp_or_now(row, "updated_at")?,
        is_managed: row.bool("is_managed")?,
        managed_key: row.opt_text("managed_key")?,
        managed_tag_filter: managed_tag_filter(row)?,
    })
}

fn managed_tag_filter(row: &SqlRow) -> AppResult<Option<Vec<String>>> {
    let Some(raw) = opt_json_text(row, "managed_tag_filter")? else {
        return Ok(None);
    };
    Ok(serde_json::from_str::<Vec<String>>(&raw)
        .ok()
        .filter(|tags| !tags.is_empty()))
}

fn applied_facets(row: &SqlRow) -> AppResult<Vec<MediaFacet>> {
    let raw = json_text_or(row, "applied_facets", "[]")?;
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

fn rule_pack_installation_args(installation: &RulePackInstallation) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(installation.pack_id.clone()),
        SqlArg::Text(installation.name.clone()),
        SqlArg::Text(installation.version.clone()),
        SqlArg::Text(installation.digest.clone()),
        SqlArg::Bool(installation.auto_update),
        SqlArg::I64(installation.revision),
        SqlArg::Timestamp(installation.last_updated),
        SqlArg::OptText(installation.last_error.clone()),
    ]
}

fn rule_pack_member_args(pack_id: &str, member: &RulePackMember) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(pack_id.to_string()),
        SqlArg::Text(member.template_id.clone()),
        SqlArg::Text(member.rule_set_id.clone()),
        SqlArg::Bool(member.removed),
    ]
}

async fn row_to_rule_pack_installation(
    datastore: &StoreDatastore,
    row: &SqlRow,
) -> AppResult<RulePackInstallation> {
    let pack_id = row.text("pack_id")?;
    let member_rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT template_id, rule_set_id, removed FROM rule_pack_members
          WHERE pack_id = {} ORDER BY template_id ASC",
        &[SqlArg::Text(pack_id.clone())],
    )
    .await?;
    let members = member_rows
        .iter()
        .map(|member| {
            Ok(RulePackMember {
                template_id: member.text("template_id")?,
                rule_set_id: member.text("rule_set_id")?,
                removed: member.bool("removed")?,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(RulePackInstallation {
        pack_id,
        name: row.text("name")?,
        version: row.text("version")?,
        digest: row.text("digest")?,
        auto_update: row.bool("auto_update")?,
        revision: row.i64("revision")?,
        last_updated: timestamp_or_now(row, "last_updated")?,
        last_error: row.opt_text("last_error")?,
        members,
    })
}

async fn insert_rule_set_history(
    tx: &mut SqlTx<'_>,
    history: &[RuleSetHistoryChange],
) -> AppResult<()> {
    for change in history {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO rule_set_history
                (id, rule_set_id, action, rego_source, actor_id, created_at)
             VALUES ({}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(change.rule_set_id.clone()),
                SqlArg::Text(change.action.clone()),
                SqlArg::OptText(change.rego_source.clone()),
                SqlArg::OptText(change.actor_id.clone()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await?;
    }
    Ok(())
}
