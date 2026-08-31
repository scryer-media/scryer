use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppResult, ScopeCoverageRow, ScopeIndexerCoverageRepository};

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

fn placeholders(count: usize) -> String {
    (0..count).map(|_| "{}").collect::<Vec<_>>().join(", ")
}

/// Convergence ledger store (SQLite + Postgres via the shared runtime).
///
/// Upsert uses portable `INSERT … ON CONFLICT (pk) DO UPDATE SET … = excluded.…`
/// which both dialects support; a re-search under a new fingerprint overwrites
/// the prior row's fingerprint + timestamp.
#[derive(Clone)]
pub struct ScopeIndexerCoverageStore {
    datastore: StoreDatastore,
}

impl ScopeIndexerCoverageStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl ScopeIndexerCoverageRepository for ScopeIndexerCoverageStore {
    async fn record_coverage(
        &self,
        scope_key: &str,
        facet: &str,
        indexer_id: &str,
        fingerprint: &str,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "record_scope_indexer_coverage",
            "INSERT INTO scope_indexer_coverage (scope_key, facet, indexer_id, fingerprint, searched_at)
             VALUES ({}, {}, {}, {}, {})
             ON CONFLICT (scope_key, facet, indexer_id)
             DO UPDATE SET fingerprint = excluded.fingerprint, searched_at = excluded.searched_at",
            vec![
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(facet.to_string()),
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(fingerprint.to_string()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn covered_indexers(
        &self,
        scope_key: &str,
        facet: &str,
        fingerprint: &str,
        stale_before: Option<DateTime<Utc>>,
    ) -> AppResult<Vec<String>> {
        let mut sql = String::from(
            "SELECT indexer_id FROM scope_indexer_coverage
             WHERE scope_key = {} AND facet = {} AND fingerprint = {}",
        );
        let mut args = vec![
            SqlArg::Text(scope_key.to_string()),
            SqlArg::Text(facet.to_string()),
            SqlArg::Text(fingerprint.to_string()),
        ];
        if let Some(stale_before) = stale_before {
            sql.push_str(" AND searched_at >= {}");
            args.push(SqlArg::Timestamp(stale_before));
        }

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .into_iter()
            .map(|row| row.text("indexer_id"))
            .collect()
    }

    async fn prune_scope(&self, scope_key: &str) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "prune_scope_indexer_coverage",
            "DELETE FROM scope_indexer_coverage WHERE scope_key = {}",
            vec![SqlArg::Text(scope_key.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn prune_scope_indexer(&self, scope_key: &str, indexer_id: &str) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "prune_scope_indexer_coverage_for_indexer",
            "DELETE FROM scope_indexer_coverage WHERE scope_key = {} AND indexer_id = {}",
            vec![
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(indexer_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn prune_indexer(&self, indexer_id: &str) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "prune_scope_indexer_coverage_for_indexer_globally",
            "DELETE FROM scope_indexer_coverage WHERE indexer_id = {}",
            vec![SqlArg::Text(indexer_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn list_coverage_for_scope_keys(
        &self,
        scope_keys: &[String],
    ) -> AppResult<Vec<ScopeCoverageRow>> {
        if scope_keys.is_empty() {
            return Ok(Vec::new());
        }
        // One round-trip for a whole page: fingerprint staleness
        // is decided in memory against the live per-scope fingerprint, so the query
        // returns every row and does no fingerprint filtering itself.
        let sql = format!(
            "SELECT scope_key, indexer_id, fingerprint, searched_at
             FROM scope_indexer_coverage
             WHERE scope_key IN ({})",
            placeholders(scope_keys.len())
        );
        let args = scope_keys
            .iter()
            .cloned()
            .map(SqlArg::Text)
            .collect::<Vec<_>>();
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .into_iter()
            .map(|row| {
                Ok(ScopeCoverageRow {
                    scope_key: row.text("scope_key")?,
                    indexer_id: row.text("indexer_id")?,
                    fingerprint: row.text("fingerprint")?,
                    searched_at: row.timestamp("searched_at")?.to_rfc3339(),
                })
            })
            .collect()
    }
}
