//! Instance-level list exclusions and the external ids they match on.
//!
//! Ids live in a child table so a match is an indexed lookup on (source,
//! value) rather than a scan of JSON, and so one exclusion can carry every id
//! the request pipeline knows for the title.

use async_trait::async_trait;
use scryer_application::lists::ListExclusionRepository;
use scryer_application::{AppError, AppResult};
use scryer_domain::{ExternalId, ListExclusion, ListExclusionScope, MediaFacet};
use std::collections::HashMap;

use super::{ListStore, parse_or_repo_err, placeholders};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime};

const EXCLUSION_COLUMNS: &str =
    "id, kind, display_title, year, scope, subscription_id, created_by_user_id, created_at";

#[async_trait]
impl ListExclusionRepository for ListStore {
    async fn create(&self, exclusion: ListExclusion) -> AppResult<ListExclusion> {
        let args = vec![
            SqlArg::Text(exclusion.id.clone()),
            SqlArg::Text(exclusion.kind.as_str().to_string()),
            SqlArg::Text(exclusion.display_title.clone()),
            SqlArg::OptI32(exclusion.year),
            SqlArg::Text(exclusion.scope.as_str().to_string()),
            SqlArg::OptText(exclusion.scope.subscription_id().map(str::to_string)),
            SqlArg::OptText(exclusion.created_by_user_id.clone()),
            SqlArg::Timestamp(exclusion.created_at),
        ];
        let id_rows: Vec<Vec<SqlArg>> = dedupe_ids(&exclusion.external_ids)
            .into_iter()
            .map(|(source, value)| {
                vec![
                    SqlArg::Text(exclusion.id.clone()),
                    SqlArg::Text(source),
                    SqlArg::Text(value),
                ]
            })
            .collect();
        SqlRuntime::run_in_transaction(&self.datastore, "create_list_exclusion", move |tx| {
            let args = args.clone();
            let id_rows = id_rows.clone();
            let exclusion = exclusion.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    &format!(
                        "INSERT INTO list_exclusions ({EXCLUSION_COLUMNS}) VALUES ({})",
                        placeholders(8)
                    ),
                    &args,
                )
                .await?;
                if !id_rows.is_empty() {
                    SqlRuntime::execute_batch_insert(
                        tx,
                        "INSERT INTO list_exclusion_external_ids (exclusion_id, source, value)",
                        3,
                        id_rows,
                        "",
                    )
                    .await?;
                }
                Ok(exclusion)
            })
        })
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<ListExclusion>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!("SELECT {EXCLUSION_COLUMNS} FROM list_exclusions WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(self.hydrate_exclusions(rows).await?.pop())
    }

    async fn list(&self) -> AppResult<Vec<ListExclusion>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {EXCLUSION_COLUMNS} FROM list_exclusions
                  ORDER BY CASE scope WHEN 'all_lists' THEN 0 ELSE 1 END, created_at DESC, id"
            ),
            &[],
        )
        .await?;
        self.hydrate_exclusions(rows).await
    }

    async fn find_matching(
        &self,
        kind: MediaFacet,
        external_ids: &[ExternalId],
        subscription_id: Option<&str>,
    ) -> AppResult<Vec<ListExclusion>> {
        let ids = dedupe_ids(external_ids);
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = vec![SqlArg::Text(kind.as_str().to_string())];
        let id_predicates: Vec<&str> = ids
            .iter()
            .map(|(source, value)| {
                args.push(SqlArg::Text(source.clone()));
                args.push(SqlArg::Text(value.clone()));
                "(LOWER(x.source) = {} AND LOWER(x.value) = {})"
            })
            .collect();
        let scope_clause = match subscription_id {
            Some(subscription_id) => {
                args.push(SqlArg::Text(subscription_id.to_string()));
                "(e.scope = 'all_lists' OR e.subscription_id = {})"
            }
            None => "e.scope = 'all_lists'",
        };
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT DISTINCT e.id, e.kind, e.display_title, e.year, e.scope,
                        e.subscription_id, e.created_by_user_id, e.created_at
                   FROM list_exclusions e
                   JOIN list_exclusion_external_ids x ON x.exclusion_id = e.id
                  WHERE e.kind = {{}}
                    AND ({})
                    AND {scope_clause}
                  ORDER BY e.created_at, e.id",
                id_predicates.join(" OR ")
            ),
            &args,
        )
        .await?;
        self.hydrate_exclusions(rows).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let changed = SqlRuntime::execute_write(
            &self.datastore,
            "delete_list_exclusion",
            "DELETE FROM list_exclusions WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("list exclusion {id}")));
        }
        Ok(())
    }
}

impl ListStore {
    async fn hydrate_exclusions(&self, rows: Vec<SqlRow>) -> AppResult<Vec<ListExclusion>> {
        let mut exclusions = rows
            .iter()
            .map(row_to_exclusion)
            .collect::<AppResult<Vec<_>>>()?;
        if exclusions.is_empty() {
            return Ok(exclusions);
        }
        let ids: Vec<SqlArg> = exclusions
            .iter()
            .map(|exclusion| SqlArg::Text(exclusion.id.clone()))
            .collect();
        let id_rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT exclusion_id, source, value FROM list_exclusion_external_ids
                  WHERE exclusion_id IN ({})
                  ORDER BY exclusion_id, source, value",
                placeholders(ids.len())
            ),
            &ids,
        )
        .await?;
        let mut by_exclusion: HashMap<String, Vec<ExternalId>> = HashMap::new();
        for row in &id_rows {
            by_exclusion
                .entry(row.text("exclusion_id")?)
                .or_default()
                .push(ExternalId::new(row.text("source")?, row.text("value")?));
        }
        for exclusion in &mut exclusions {
            exclusion.external_ids = by_exclusion.remove(&exclusion.id).unwrap_or_default();
        }
        Ok(exclusions)
    }
}

/// Lower-cased, de-duplicated (source, value) pairs; blank ids are dropped.
fn dedupe_ids(external_ids: &[ExternalId]) -> Vec<(String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    external_ids
        .iter()
        .filter_map(|id| {
            let source = id.source.trim().to_ascii_lowercase();
            let value = id.value.trim().to_ascii_lowercase();
            if source.is_empty() || value.is_empty() {
                return None;
            }
            seen.insert((source.clone(), value.clone()))
                .then_some((source, value))
        })
        .collect()
}

fn row_to_exclusion(row: &SqlRow) -> AppResult<ListExclusion> {
    let scope = match row.text("scope")?.as_str() {
        "all_lists" => ListExclusionScope::AllLists,
        "list" => ListExclusionScope::List {
            subscription_id: row.opt_text("subscription_id")?.unwrap_or_default(),
        },
        other => {
            return Err(AppError::Repository(format!(
                "unknown exclusion scope value '{other}'"
            )));
        }
    };
    Ok(ListExclusion {
        id: row.text("id")?,
        kind: parse_or_repo_err("kind", &row.text("kind")?, MediaFacet::parse)?,
        external_ids: Vec::new(),
        display_title: row.text("display_title")?,
        year: row.opt_i32("year")?,
        scope,
        created_by_user_id: row.opt_text("created_by_user_id")?,
        created_at: row.timestamp("created_at")?,
    })
}
