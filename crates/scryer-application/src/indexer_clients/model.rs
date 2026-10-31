use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::AppResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexerClientProtocol {
    Newznab,
    Torznab,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerClientGrant {
    /// One existing source identity, including an explicitly selected managed child.
    pub indexer_id: String,
    /// None is unrestricted. An explicit empty set permits no categories;
    /// it must never be converted into an unconstrained upstream request.
    pub categories: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerClientConfiguration {
    pub name: String,
    pub enabled: bool,
    pub protocols: Vec<IndexerClientProtocol>,
    pub grants: Vec<IndexerClientGrant>,
}

#[derive(Clone)]
pub struct IndexerClientRecord {
    pub id: String,
    pub configuration: IndexerClientConfiguration,
    pub lookup_id: Option<String>,
    pub secret_hash: Option<String>,
    pub key_generation: u64,
    /// Every management mutation advances this revision atomically. It is
    /// also the grant/configuration generation checked before feed delivery.
    pub revision: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_outcome_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for IndexerClientRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IndexerClientRecord")
            .field("id", &self.id)
            .field("configuration", &self.configuration)
            .field("key_generation", &self.key_generation)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// Persistence for client administration only. No scheduler, timeout, retry,
/// source quota, or duplicate indexer configuration belongs to this repository.
#[async_trait::async_trait]
pub trait IndexerClientRepository: Send + Sync {
    async fn list(&self) -> AppResult<Vec<IndexerClientRecord>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerClientRecord>>;
    async fn get_by_lookup_id(&self, lookup_id: &str) -> AppResult<Option<IndexerClientRecord>>;
    /// None inserts a new record. Some compares the previous revision and
    /// atomically replaces the record and grants, returning false on a race.
    async fn save(
        &self,
        record: &IndexerClientRecord,
        expected_revision: Option<u64>,
    ) -> AppResult<bool>;
    /// A stale in-flight request cannot overwrite current-generation status.
    async fn record_use(
        &self,
        id: &str,
        revision: u64,
        at: DateTime<Utc>,
        outcome_code: &str,
    ) -> AppResult<()>;
}
