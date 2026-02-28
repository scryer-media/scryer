use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use scryer_application::{IndexerQueryStats, IndexerStatsTracker};

struct IndexerEntry {
    indexer_name: String,
    queries: Vec<(DateTime<Utc>, bool)>,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
}

/// Thread-safe in-memory tracker for per-indexer query counts and API quotas.
/// Data is kept for a rolling 24-hour window and lost on restart.
#[derive(Clone)]
pub struct InMemoryIndexerStatsTracker {
    entries: std::sync::Arc<Mutex<HashMap<String, IndexerEntry>>>,
}

impl InMemoryIndexerStatsTracker {
    pub fn new() -> Self {
        Self {
            entries: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn prune_old(entry: &mut IndexerEntry) {
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        entry.queries.retain(|(ts, _)| *ts > cutoff);
    }
}

impl IndexerStatsTracker for InMemoryIndexerStatsTracker {
    fn record_query(&self, indexer_id: &str, indexer_name: &str, success: bool) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .entry(indexer_id.to_string())
            .or_insert_with(|| IndexerEntry {
                indexer_name: indexer_name.to_string(),
                queries: Vec::new(),
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            });
        entry.indexer_name = indexer_name.to_string();
        entry.queries.push((Utc::now(), success));
        Self::prune_old(entry);
    }

    fn record_api_limits(
        &self,
        indexer_id: &str,
        api_current: Option<u32>,
        api_max: Option<u32>,
        grab_current: Option<u32>,
        grab_max: Option<u32>,
    ) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(indexer_id) {
            if api_current.is_some() || api_max.is_some() {
                entry.api_current = api_current;
                entry.api_max = api_max;
            }
            if grab_current.is_some() || grab_max.is_some() {
                entry.grab_current = grab_current;
                entry.grab_max = grab_max;
            }
        }
    }

    fn all_stats(&self) -> Vec<IndexerQueryStats> {
        let mut entries = self.entries.lock().unwrap();
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        entries
            .iter_mut()
            .map(|(id, entry)| {
                entry.queries.retain(|(ts, _)| *ts > cutoff);
                let successful = entry.queries.iter().filter(|(_, s)| *s).count() as u32;
                let total = entry.queries.len() as u32;
                let last_query_at = entry.queries.last().map(|(ts, _)| ts.to_rfc3339());
                IndexerQueryStats {
                    indexer_id: id.clone(),
                    indexer_name: entry.indexer_name.clone(),
                    queries_last_24h: total,
                    successful_last_24h: successful,
                    failed_last_24h: total - successful,
                    last_query_at,
                    api_current: entry.api_current,
                    api_max: entry.api_max,
                    grab_current: entry.grab_current,
                    grab_max: entry.grab_max,
                }
            })
            .collect()
    }
}
