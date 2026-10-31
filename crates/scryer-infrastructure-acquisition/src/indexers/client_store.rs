use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::indexer_clients::{
    IndexerClientConfiguration, IndexerClientGrant, IndexerClientRecord, IndexerClientRepository,
};
use scryer_application::{AppError, AppResult};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

#[cfg(test)]
#[path = "client_store_tests.rs"]
mod tests;

#[derive(Clone)]
pub struct IndexerClientStore {
    datastore: StoreDatastore,
}

impl IndexerClientStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    async fn select(
        &self,
        predicate: &str,
        args: &[SqlArg],
    ) -> AppResult<Vec<IndexerClientRecord>> {
        // One statement reads configuration, key generation and grants from
        // the same database snapshot, including clients with no grants.
        let sql = format!("{SELECT} {predicate} ORDER BY client.id, grant_row.indexer_id");
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, args).await?;
        let mut clients: Vec<IndexerClientRecord> = Vec::new();
        for row in &rows {
            let id = row.text("id")?;
            if clients.last().is_none_or(|client| client.id != id) {
                clients.push(decode_client(row)?);
            }
            if let Some(indexer_id) = row.opt_text("grant_indexer_id")? {
                let categories = row
                    .opt_text("categories_json")?
                    .map(|value| serde_json::from_str(&value).map_err(repository_error))
                    .transpose()?;
                clients
                    .last_mut()
                    .expect("decoded client")
                    .configuration
                    .grants
                    .push(IndexerClientGrant {
                        indexer_id,
                        categories,
                    });
            }
        }
        Ok(clients)
    }
}

const SELECT: &str = "SELECT client.id, client.name, client.enabled, client.protocols_json,
    client.lookup_id, client.secret_hash, client.key_generation, client.revision,
    client.last_used_at, client.last_outcome_code, client.created_at, client.updated_at,
    grant_row.indexer_id AS grant_indexer_id, grant_row.categories_json
    FROM indexer_api_clients client
    LEFT JOIN indexer_api_client_grants grant_row ON grant_row.client_id = client.id";

fn repository_error(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(format!("Invalid stored indexer client data: {error}"))
}

fn counter(value: u64) -> AppResult<i64> {
    i64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| repository_error("revision is out of range"))
}

fn decode_client(row: &SqlRow) -> AppResult<IndexerClientRecord> {
    Ok(IndexerClientRecord {
        id: row.text("id")?,
        configuration: IndexerClientConfiguration {
            name: row.text("name")?,
            enabled: row.bool("enabled")?,
            protocols: serde_json::from_str(&row.text("protocols_json")?)
                .map_err(repository_error)?,
            grants: Vec::new(),
        },
        lookup_id: row.opt_text("lookup_id")?,
        secret_hash: row.opt_text("secret_hash")?,
        key_generation: u64::try_from(row.i64("key_generation")?).map_err(repository_error)?,
        revision: u64::try_from(row.i64("revision")?).map_err(repository_error)?,
        last_used_at: row.opt_timestamp("last_used_at")?,
        last_outcome_code: row.opt_text("last_outcome_code")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

#[async_trait]
impl IndexerClientRepository for IndexerClientStore {
    async fn list(&self) -> AppResult<Vec<IndexerClientRecord>> {
        self.select("", &[]).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerClientRecord>> {
        Ok(self
            .select("WHERE client.id = {}", &[SqlArg::Text(id.into())])
            .await?
            .into_iter()
            .next())
    }

    async fn get_by_lookup_id(&self, lookup: &str) -> AppResult<Option<IndexerClientRecord>> {
        Ok(self
            .select(
                "WHERE client.lookup_id = {}",
                &[SqlArg::Text(lookup.into())],
            )
            .await?
            .into_iter()
            .next())
    }

    async fn save(
        &self,
        record: &IndexerClientRecord,
        expected_revision: Option<u64>,
    ) -> AppResult<bool> {
        let record = record.clone();
        let mut args = vec![
            SqlArg::Text(record.configuration.name.clone()),
            SqlArg::Bool(record.configuration.enabled),
            SqlArg::Text(
                serde_json::to_string(&record.configuration.protocols).map_err(repository_error)?,
            ),
            SqlArg::OptText(record.lookup_id.clone()),
            SqlArg::OptText(record.secret_hash.clone()),
            SqlArg::I64(counter(record.key_generation)?),
            SqlArg::I64(counter(record.revision)?),
            SqlArg::Timestamp(record.updated_at),
            SqlArg::Text(record.id.clone()),
        ];
        let sql = if let Some(previous) = expected_revision {
            if previous.checked_add(1) != Some(record.revision) {
                return Err(repository_error("revision must advance once"));
            }
            args.push(SqlArg::I64(counter(previous)?));
            "UPDATE indexer_api_clients SET name = {}, enabled = {}, protocols_json = {},
                lookup_id = {}, secret_hash = {}, key_generation = {}, revision = {}, updated_at = {}
                WHERE id = {} AND revision = {}"
        } else {
            if record.revision != 1 || record.key_generation != 1 {
                return Err(repository_error("new client counters must start at one"));
            }
            args.push(SqlArg::Timestamp(record.created_at));
            "INSERT INTO indexer_api_clients (name, enabled, protocols_json, lookup_id, secret_hash,
                key_generation, revision, updated_at, id, created_at)
                VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}) ON CONFLICT(id) DO NOTHING"
        };
        let grants = record
            .configuration
            .grants
            .iter()
            .map(|grant| {
                Ok(vec![
                    SqlArg::Text(record.id.clone()),
                    SqlArg::Text(grant.indexer_id.clone()),
                    SqlArg::OptText(
                        grant
                            .categories
                            .as_ref()
                            .map(serde_json::to_string)
                            .transpose()
                            .map_err(repository_error)?,
                    ),
                ])
            })
            .collect::<AppResult<Vec<_>>>()?;
        SqlRuntime::run_in_transaction(&self.datastore, "save_indexer_api_client", move |tx| {
            let args = args.clone();
            let id = record.id.clone();
            let grants = grants.clone();
            Box::pin(async move {
                if SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await? == 0 {
                    return Ok(false);
                }
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM indexer_api_client_grants WHERE client_id = {}",
                    &[SqlArg::Text(id)],
                )
                .await?;
                for grant in grants {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO indexer_api_client_grants
                        (client_id, indexer_id, categories_json) VALUES ({}, {}, {})",
                        &grant,
                    )
                    .await?;
                }
                Ok(true)
            })
        })
        .await
    }

    async fn record_use(
        &self,
        id: &str,
        revision: u64,
        at: DateTime<Utc>,
        outcome_code: &str,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "record_indexer_api_client_use",
            "UPDATE indexer_api_clients SET last_used_at = {}, last_outcome_code = {}
             WHERE id = {} AND revision = {} AND (last_used_at IS NULL OR last_used_at <= {})",
            vec![
                SqlArg::Timestamp(at),
                SqlArg::Text(outcome_code.into()),
                SqlArg::Text(id.into()),
                SqlArg::I64(counter(revision)?),
                SqlArg::Timestamp(at),
            ],
        )
        .await?;
        Ok(())
    }
}
