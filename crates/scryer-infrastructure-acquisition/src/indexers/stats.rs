use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use scryer_application::{IndexerDispatchGate, IndexerQueryStats, IndexerStatsTracker};

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

const INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY: &str =
    "indexer.request_accounting_v2_started_at";

const QUOTA_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

struct IndexerEntry {
    indexer_name: String,
    queries: Vec<(DateTime<Utc>, bool)>,
    /// Timestamps of releases Scryer grabbed through this indexer. Shares the
    /// rolling 24-hour window and in-memory-only lifetime of `queries`; unlike
    /// `grab_current` below, this is Scryer's own count rather than a
    /// provider-reported quota reading, so it is never persisted.
    grabs: Vec<DateTime<Utc>>,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
    /// Requests sent to this indexer in the current quota window. Mirrors the
    /// persisted `indexer_api_quotas.queries_today` so the dashboard can be
    /// read without touching the database, and is seeded from it at boot by
    /// [`InMemoryIndexerStatsTracker::hydrate`].
    api_requests_today: u32,
    /// UTC day the current `api_requests_today` window opened on.
    api_requests_window_started_on: Option<NaiveDate>,
}

impl IndexerEntry {
    fn new(indexer_name: &str) -> Self {
        Self {
            indexer_name: indexer_name.to_string(),
            queries: Vec::new(),
            grabs: Vec::new(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
            api_requests_today: 0,
            api_requests_window_started_on: None,
        }
    }

    /// Roll the request counter over when the UTC day has changed, matching
    /// the reset the persisted row performs on its next write.
    fn roll_request_window(&mut self, today: NaiveDate) {
        match self.api_requests_window_started_on {
            Some(started_on) if started_on >= today => {}
            Some(_) => {
                self.api_requests_today = 0;
                self.api_requests_window_started_on = Some(today);
            }
            None => self.api_requests_window_started_on = Some(today),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PendingQuotaUpdate {
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
    /// Requests sent since the last flush. The only thing that advances the
    /// persisted `queries_today`.
    query_delta: u32,
}

impl PendingQuotaUpdate {
    fn merge(
        &mut self,
        api_current: Option<u32>,
        api_max: Option<u32>,
        grab_current: Option<u32>,
        grab_max: Option<u32>,
    ) {
        if api_current.is_some() {
            self.api_current = api_current;
        }
        if api_max.is_some() {
            self.api_max = api_max;
        }
        if grab_current.is_some() {
            self.grab_current = grab_current;
        }
        if grab_max.is_some() {
            self.grab_max = grab_max;
        }
        // Reporting a provider quota reading is not itself an API hit. Hits are
        // counted at the send site by `record_api_request_sent`; counting them
        // here too both double-counted successful searches and missed every
        // request that never produced a parsed response.
    }

    fn merge_request_sent(&mut self) {
        self.query_delta = self.query_delta.saturating_add(1);
    }

    fn is_empty(&self) -> bool {
        self.query_delta == 0
            && self.api_current.is_none()
            && self.api_max.is_none()
            && self.grab_current.is_none()
            && self.grab_max.is_none()
    }

    fn merge_requeued(&mut self, older: Self) {
        self.api_current = self.api_current.or(older.api_current);
        self.api_max = self.api_max.or(older.api_max);
        self.grab_current = self.grab_current.or(older.grab_current);
        self.grab_max = self.grab_max.or(older.grab_max);
        self.query_delta = self.query_delta.saturating_add(older.query_delta);
    }
}

/// Thread-safe indexer stats tracker with in-memory 24-hour rolling window
/// and optional SQLite persistence for API quota enforcement.
#[derive(Clone)]
pub struct InMemoryIndexerStatsTracker {
    entries: Arc<Mutex<HashMap<String, IndexerEntry>>>,
    datastore: Option<StoreDatastore>,
    pending_quota_updates: Arc<Mutex<BTreeMap<(String, NaiveDate), PendingQuotaUpdate>>>,
    /// Ensures an explicit shutdown drain cannot overlap the scheduled flush.
    quota_flush_lock: Arc<tokio::sync::Mutex<()>>,
    dispatch_gate: IndexerDispatchGate,
    quota_flush_scheduled: Arc<AtomicBool>,
    quota_runtime: Arc<Mutex<Option<tokio::runtime::Handle>>>,
    observation_started_at: Arc<Mutex<Option<String>>>,
}

impl Default for InMemoryIndexerStatsTracker {
    fn default() -> Self {
        Self::new(None)
    }
}

impl InMemoryIndexerStatsTracker {
    pub fn new(datastore: Option<StoreDatastore>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            datastore,
            pending_quota_updates: Arc::new(Mutex::new(BTreeMap::new())),
            quota_flush_lock: Arc::new(tokio::sync::Mutex::new(())),
            dispatch_gate: IndexerDispatchGate::default(),
            quota_flush_scheduled: Arc::new(AtomicBool::new(false)),
            quota_runtime: Arc::new(Mutex::new(tokio::runtime::Handle::try_current().ok())),
            observation_started_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Seed the in-memory counters from the persisted quota rows.
    ///
    /// Called once at boot, before any indexer traffic. Without it the
    /// dashboard restarts at zero every time the process does while the
    /// indexer's own counter keeps climbing, which is exactly the discrepancy
    /// operators were reporting. Rows from an earlier UTC day are ignored: the
    /// window has already rolled over and the next persisted write resets them.
    pub async fn hydrate(&self) {
        self.quota_runtime_handle();
        let Some(datastore) = &self.datastore else {
            return;
        };
        let quotas = match crate::queries::indexer::load_indexer_quotas(datastore).await {
            Ok(quotas) => quotas,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to load persisted indexer quotas; API hit counts start from zero"
                );
                return;
            }
        };
        let observation_started_at = match SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT CAST(settings_values.value_json AS TEXT) AS value_json
               FROM settings_values
               JOIN settings_definitions
                 ON settings_definitions.id = settings_values.setting_definition_id
              WHERE settings_definitions.category = {}
                AND settings_definitions.scope = {}
                AND settings_definitions.key_name = {}
                AND settings_values.scope = {}
                AND settings_values.scope_id IS NULL",
            &[
                SqlArg::Text("service".to_string()),
                SqlArg::Text("system".to_string()),
                SqlArg::Text(INDEXER_REQUEST_ACCOUNTING_V2_STARTED_AT_KEY.to_string()),
                SqlArg::Text("system".to_string()),
            ],
        )
        .await
        {
            Ok(Some(row)) => row
                .text("value_json")
                .ok()
                .and_then(|value| serde_json::from_str::<Option<String>>(&value).ok())
                .flatten(),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(error = %error, "failed to load indexer request accounting observation epoch");
                None
            }
        };
        *self.observation_started_at.lock().unwrap() = observation_started_at;

        let today = Utc::now().date_naive();
        let mut entries = self.entries.lock().unwrap();
        for quota in quotas {
            let window_started_on = quota
                .window_started_on
                .as_deref()
                .and_then(|day| day.parse::<NaiveDate>().ok());
            let entry = entries
                .entry(quota.indexer_id.clone())
                // The name is unknown until this indexer is first used; the
                // first send overwrites this placeholder.
                .or_insert_with(|| IndexerEntry::new(&quota.indexer_id));
            entry.api_current = quota.api_current;
            entry.api_max = quota.api_max;
            entry.grab_current = quota.grab_current;
            entry.grab_max = quota.grab_max;
            match window_started_on {
                Some(started_on) if started_on == today => {
                    entry.api_requests_today = quota.api_requests_today;
                    entry.api_requests_window_started_on = Some(started_on);
                }
                _ => {
                    entry.api_requests_today = 0;
                    entry.api_requests_window_started_on = Some(today);
                }
            }
        }
    }

    fn prune_old(entry: &mut IndexerEntry) {
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        entry.queries.retain(|(ts, _)| *ts > cutoff);
        entry.grabs.retain(|ts| *ts > cutoff);
    }

    fn quota_runtime_handle(&self) -> Option<tokio::runtime::Handle> {
        let mut runtime = self.quota_runtime.lock().unwrap();
        if runtime.is_none() {
            *runtime = tokio::runtime::Handle::try_current().ok();
        }
        runtime.clone()
    }

    fn schedule_quota_flush(&self) {
        if self.datastore.is_none() {
            return;
        }
        // Legacy plugins invoke the tracker from ordinary worker threads. Use
        // the application runtime captured at construction or hydration there.
        let Some(runtime) = self.quota_runtime_handle() else {
            // Keep the updates queued for a later flush if no runtime has
            // been attached yet, without wedging the scheduled flag.
            return;
        };
        if self.quota_flush_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }

        let tracker = self.clone();
        runtime.spawn(async move {
            tokio::time::sleep(QUOTA_FLUSH_INTERVAL).await;
            tracker.flush_pending_quota_updates().await;
        });
    }

    pub async fn flush_pending_quota_updates(&self) {
        let _flush_guard = self.quota_flush_lock.lock().await;
        let Some(datastore) = &self.datastore else {
            self.quota_flush_scheduled.store(false, Ordering::Release);
            return;
        };
        let datastore = datastore.clone();
        let updates = {
            let mut pending = self.pending_quota_updates.lock().unwrap();
            if pending.is_empty() {
                self.quota_flush_scheduled.store(false, Ordering::Release);
                return;
            }
            std::mem::take(&mut *pending)
        };

        let mut failed_updates = Vec::new();
        for ((indexer_id, sent_on), update) in updates {
            if update.is_empty() {
                continue;
            }
            if let Err(error) = crate::queries::indexer::upsert_indexer_quota(
                &datastore,
                &indexer_id,
                update.api_current,
                update.api_max,
                update.grab_current,
                update.grab_max,
                crate::queries::indexer::DailyRequestCount {
                    sent_on,
                    count: update.query_delta,
                },
            )
            .await
            {
                tracing::warn!(
                    indexer_id,
                    error = %error,
                    "failed to flush coalesced indexer quota update; requeueing"
                );
                failed_updates.push(((indexer_id, sent_on), update));
            }
        }

        if !failed_updates.is_empty() {
            let mut pending = self.pending_quota_updates.lock().unwrap();
            for (indexer_id, update) in failed_updates {
                pending
                    .entry(indexer_id)
                    .or_default()
                    .merge_requeued(update);
            }
        }

        self.quota_flush_scheduled.store(false, Ordering::Release);
        if !self.pending_quota_updates.lock().unwrap().is_empty() {
            self.schedule_quota_flush();
        }
    }

    /// Final, awaited drain for orderly shutdown. Call only after request
    /// producers have stopped; a failed write remains queued and is surfaced
    /// to the caller instead of being mistaken for durable accounting.
    pub async fn drain_pending_quota_updates(&self) -> scryer_application::AppResult<()> {
        self.flush_pending_quota_updates().await;
        if self.pending_quota_updates.lock().unwrap().is_empty() {
            Ok(())
        } else {
            Err(scryer_application::AppError::Repository(
                "indexer request accounting could not be persisted during shutdown".into(),
            ))
        }
    }

    /// Stop admitting new observed indexer HTTP dispatches before final drain.
    pub fn close_dispatch_gate(&self) {
        self.dispatch_gate.close();
    }

    /// Await completion of dispatches admitted before the gate was closed.
    pub async fn wait_for_admitted_dispatches(&self) {
        self.dispatch_gate.wait_for_idle().await;
    }
}

impl IndexerStatsTracker for InMemoryIndexerStatsTracker {
    fn dispatch_gate(&self) -> Option<IndexerDispatchGate> {
        Some(self.dispatch_gate.clone())
    }

    fn record_query(&self, indexer_id: &str, indexer_name: &str, success: bool) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .entry(indexer_id.to_string())
            .or_insert_with(|| IndexerEntry::new(indexer_name));
        entry.indexer_name = indexer_name.to_string();
        entry.queries.push((Utc::now(), success));
        Self::prune_old(entry);
    }

    fn record_grab(&self, indexer_id: &str, indexer_name: &str) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .entry(indexer_id.to_string())
            .or_insert_with(|| IndexerEntry::new(indexer_name));
        entry.indexer_name = indexer_name.to_string();
        entry.grabs.push(Utc::now());
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
            if api_current.is_some() {
                entry.api_current = api_current;
            }
            if api_max.is_some() {
                entry.api_max = api_max;
            }
            if grab_current.is_some() {
                entry.grab_current = grab_current;
            }
            if grab_max.is_some() {
                entry.grab_max = grab_max;
            }
        }
        drop(entries);

        if self.datastore.is_some() {
            let mut pending = self.pending_quota_updates.lock().unwrap();
            pending
                .entry((indexer_id.to_string(), Utc::now().date_naive()))
                .or_default()
                .merge(api_current, api_max, grab_current, grab_max);
            drop(pending);
            self.schedule_quota_flush();
        }
    }

    fn record_api_request_sent(&self, indexer_id: &str, indexer_name: &str) {
        self.record_api_request_sent_on(indexer_id, indexer_name, Utc::now().date_naive());
    }

    fn record_api_request_sent_at(
        &self,
        indexer_id: &str,
        indexer_name: &str,
        dispatched_at: DateTime<Utc>,
    ) {
        self.record_api_request_sent_on(indexer_id, indexer_name, dispatched_at.date_naive());
    }

    fn all_stats(&self) -> Vec<IndexerQueryStats> {
        self.collect_stats()
    }
}

impl InMemoryIndexerStatsTracker {
    fn record_api_request_sent_on(&self, indexer_id: &str, indexer_name: &str, today: NaiveDate) {
        if !scryer_application::indexer_error_history_is_persistable(indexer_id) {
            return;
        }
        {
            let mut entries = self.entries.lock().unwrap();
            let entry = entries
                .entry(indexer_id.to_string())
                .or_insert_with(|| IndexerEntry::new(indexer_name));
            entry.indexer_name = indexer_name.to_string();
            entry.roll_request_window(today);
            if entry.api_requests_window_started_on == Some(today) {
                entry.api_requests_today = entry.api_requests_today.saturating_add(1);
            }
        }

        if self.datastore.is_some() {
            self.pending_quota_updates
                .lock()
                .unwrap()
                .entry((indexer_id.to_string(), today))
                .or_default()
                .merge_request_sent();
            self.schedule_quota_flush();
        }
    }

    fn collect_stats(&self) -> Vec<IndexerQueryStats> {
        let observation_started_at = self.observation_started_at.lock().unwrap().clone();
        let mut entries = self.entries.lock().unwrap();
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        let today = Utc::now().date_naive();
        entries
            .iter_mut()
            .map(|(id, entry)| {
                entry.roll_request_window(today);
                entry.queries.retain(|(ts, _)| *ts > cutoff);
                entry.grabs.retain(|ts| *ts > cutoff);
                let successful = entry.queries.iter().filter(|(_, s)| *s).count() as u32;
                let total = entry.queries.len() as u32;
                let last_query_at = entry.queries.last().map(|(ts, _)| ts.to_rfc3339());
                IndexerQueryStats {
                    indexer_id: id.clone(),
                    indexer_name: entry.indexer_name.clone(),
                    queries_last_24h: total,
                    successful_last_24h: successful,
                    failed_last_24h: total - successful,
                    grabs_last_24h: entry.grabs.len() as u32,
                    last_query_at,
                    api_requests_today: entry.api_requests_today,
                    api_requests_window_started_at: entry
                        .api_requests_window_started_on
                        .map(|day| day.to_string()),
                    api_requests_observed_since: observation_started_at.clone(),
                    api_current: entry.api_current,
                    api_max: entry.api_max,
                    grab_current: entry.grab_current,
                    grab_max: entry.grab_max,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
    use tokio::time::{Duration, timeout};

    fn tracker_with_gate(
        pool: &SqlitePool,
        writer_gate: Arc<tokio::sync::Mutex<()>>,
    ) -> InMemoryIndexerStatsTracker {
        InMemoryIndexerStatsTracker::new(Some(StoreDatastore::sqlite(pool.clone(), writer_gate)))
    }

    /// Backdate every recorded grab for one indexer, standing in for time
    /// passing without making the test sleep.
    fn backdate_grabs(
        tracker: &InMemoryIndexerStatsTracker,
        indexer_id: &str,
        age: chrono::Duration,
    ) {
        let mut entries = tracker.entries.lock().unwrap();
        let entry = entries.get_mut(indexer_id).expect("indexer entry");
        for grab in entry.grabs.iter_mut() {
            *grab -= age;
        }
    }

    fn grabs_for(tracker: &InMemoryIndexerStatsTracker, indexer_id: &str) -> u32 {
        tracker
            .all_stats()
            .into_iter()
            .find(|stats| stats.indexer_id == indexer_id)
            .map(|stats| stats.grabs_last_24h)
            .unwrap_or(0)
    }

    #[test]
    fn record_grab_increments_the_rolling_24h_count() {
        let tracker = InMemoryIndexerStatsTracker::default();
        assert_eq!(grabs_for(&tracker, "idx-grab"), 0);

        tracker.record_grab("idx-grab", "Grabby Indexer");
        tracker.record_grab("idx-grab", "Grabby Indexer");

        assert_eq!(grabs_for(&tracker, "idx-grab"), 2);
        let stats = tracker.all_stats();
        let entry = stats
            .iter()
            .find(|stats| stats.indexer_id == "idx-grab")
            .expect("grab-only indexer should appear in stats");
        assert_eq!(entry.indexer_name, "Grabby Indexer");
        // Grabs must not be confused with queries or with provider quota counters.
        assert_eq!(entry.queries_last_24h, 0);
        assert_eq!(entry.grab_current, None);
        assert_eq!(entry.grab_max, None);
    }

    #[test]
    fn grabs_older_than_the_window_are_pruned() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_grab("idx-prune", "Pruned Indexer");
        assert_eq!(grabs_for(&tracker, "idx-prune"), 1);

        backdate_grabs(&tracker, "idx-prune", chrono::Duration::hours(25));
        assert_eq!(
            grabs_for(&tracker, "idx-prune"),
            0,
            "a grab older than 24 hours must leave the window"
        );

        // A fresh grab after pruning still counts, and recording prunes too.
        tracker.record_grab("idx-prune", "Pruned Indexer");
        assert_eq!(grabs_for(&tracker, "idx-prune"), 1);
    }

    #[test]
    fn recording_a_grab_prunes_expired_grabs_for_that_indexer() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_grab("idx-prune-on-write", "Indexer");
        backdate_grabs(&tracker, "idx-prune-on-write", chrono::Duration::hours(25));

        tracker.record_grab("idx-prune-on-write", "Indexer");

        let entries = tracker.entries.lock().unwrap();
        assert_eq!(
            entries
                .get("idx-prune-on-write")
                .expect("indexer entry")
                .grabs
                .len(),
            1,
            "record_grab should drop expired grabs like record_query does"
        );
    }

    #[test]
    fn grabs_do_not_bleed_between_indexers() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_grab("idx-a", "Indexer A");
        tracker.record_grab("idx-a", "Indexer A");
        tracker.record_grab("idx-b", "Indexer B");
        tracker.record_query("idx-c", "Indexer C", true);

        assert_eq!(grabs_for(&tracker, "idx-a"), 2);
        assert_eq!(grabs_for(&tracker, "idx-b"), 1);
        assert_eq!(
            grabs_for(&tracker, "idx-c"),
            0,
            "an indexer that was only queried has no grabs"
        );
    }

    #[test]
    fn grabs_and_queries_share_an_entry_without_overwriting_each_other() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_query("idx-mixed", "Mixed Indexer", true);
        tracker.record_grab("idx-mixed", "Mixed Indexer");
        tracker.record_query("idx-mixed", "Mixed Indexer", false);

        let stats = tracker.all_stats();
        let entry = stats
            .iter()
            .find(|stats| stats.indexer_id == "idx-mixed")
            .expect("mixed indexer stats");
        assert_eq!(entry.queries_last_24h, 2);
        assert_eq!(entry.successful_last_24h, 1);
        assert_eq!(entry.failed_last_24h, 1);
        assert_eq!(entry.grabs_last_24h, 1);
    }

    #[test]
    fn requeued_updates_preserve_newer_limits_and_sum_deltas() {
        let mut older = PendingQuotaUpdate::default();
        older.merge(Some(1), Some(100), Some(2), Some(20));
        older.merge_request_sent();

        let mut newer = PendingQuotaUpdate::default();
        newer.merge(Some(5), None, None, Some(50));
        newer.merge_request_sent();
        newer.merge_requeued(older);

        assert_eq!(newer.api_current, Some(5));
        assert_eq!(newer.api_max, Some(100));
        assert_eq!(newer.grab_current, Some(2));
        assert_eq!(newer.grab_max, Some(50));
        assert_eq!(newer.query_delta, 2);
    }

    async fn create_quota_table(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE indexer_api_quotas (
                indexer_id TEXT PRIMARY KEY NOT NULL,
                api_current INTEGER,
                api_max INTEGER,
                grab_current INTEGER,
                grab_max INTEGER,
                queries_today INTEGER NOT NULL DEFAULT 0,
                last_query_at TEXT,
                last_reset_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(pool)
        .await
        .expect("quota table should be created");
    }

    #[tokio::test]
    async fn hydrate_exposes_the_observation_epoch_and_ignores_synthetic_test_requests() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;
        sqlx::query(
            "CREATE TABLE settings_definitions (
                id TEXT PRIMARY KEY, category TEXT NOT NULL, scope TEXT NOT NULL,
                key_name TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("settings definitions table");
        sqlx::query(
            "CREATE TABLE settings_values (
                id TEXT PRIMARY KEY, setting_definition_id TEXT NOT NULL, scope TEXT NOT NULL,
                scope_id TEXT, value_json TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("settings values table");
        sqlx::query(
            "INSERT INTO settings_definitions (id, category, scope, key_name)
             VALUES ('epoch', 'service', 'system', 'indexer.request_accounting_v2_started_at')",
        )
        .execute(&pool)
        .await
        .expect("epoch definition");
        sqlx::query(
            "INSERT INTO settings_values (id, setting_definition_id, scope, scope_id, value_json)
             VALUES ('epoch-value', 'epoch', 'system', NULL, '\"2026-09-06T12:00:00Z\"')",
        )
        .execute(&pool)
        .await
        .expect("epoch value");

        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.hydrate().await;
        tracker.record_api_request_sent("test-connection", "Test connection");
        assert!(tracker.all_stats().is_empty());

        tracker.record_api_request_sent("idx-observed", "Observed Indexer");
        let stat = tracker.all_stats().pop().expect("observed stat");
        assert_eq!(
            stat.api_requests_observed_since.as_deref(),
            Some("2026-09-06T12:00:00Z")
        );
    }

    #[tokio::test]
    async fn worker_thread_requests_flush_on_the_application_runtime() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;

        for constructed_on_worker in [false, true] {
            let datastore =
                StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
            let tracker = if constructed_on_worker {
                std::thread::spawn(move || InMemoryIndexerStatsTracker::new(Some(datastore)))
                    .join()
                    .expect("construct tracker outside the runtime")
            } else {
                InMemoryIndexerStatsTracker::new(Some(datastore))
            };
            tracker.hydrate().await;
            let indexer_id = format!("idx-worker-{constructed_on_worker}");
            let worker_id = indexer_id.clone();
            std::thread::spawn(move || {
                assert!(tokio::runtime::Handle::try_current().is_err());
                tracker.record_api_request_sent(&worker_id, "Worker Indexer");
                tracker.record_api_limits(&worker_id, Some(7), Some(100), None, None);
            })
            .join()
            .expect("worker callback must not require an entered runtime");

            let row = timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(row) = sqlx::query(
                        "SELECT queries_today, api_current FROM indexer_api_quotas WHERE indexer_id = ?",
                    )
                    .bind(&indexer_id)
                    .fetch_optional(&pool)
                    .await
                    .expect("read worker quota")
                    {
                        break row;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("worker update should flush without another callback");
            assert_eq!(row.get::<i64, _>("queries_today"), 1);
            assert_eq!(row.get::<i64, _>("api_current"), 7);
        }
    }

    #[tokio::test]
    async fn quota_flush_coalesces_updates_and_preserves_query_delta() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;

        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.record_query("idx-1", "Indexer One", true);
        // Two requests sent, then the quota readings those responses carried.
        tracker.record_api_request_sent("idx-1", "Indexer One");
        tracker.record_api_request_sent("idx-1", "Indexer One");
        tracker.record_api_limits("idx-1", Some(1), Some(100), None, None);
        tracker.record_api_limits("idx-1", Some(2), None, Some(3), Some(50));
        tracker.flush_pending_quota_updates().await;

        let row = sqlx::query(
            "SELECT api_current, api_max, grab_current, grab_max, queries_today
             FROM indexer_api_quotas WHERE indexer_id = ?",
        )
        .bind("idx-1")
        .fetch_one(&pool)
        .await
        .expect("quota row should exist");

        assert_eq!(row.get::<i64, _>("api_current"), 2);
        assert_eq!(row.get::<i64, _>("api_max"), 100);
        assert_eq!(row.get::<i64, _>("grab_current"), 3);
        assert_eq!(row.get::<i64, _>("grab_max"), 50);
        assert_eq!(row.get::<i64, _>("queries_today"), 2);

        tracker.record_api_request_sent("idx-1", "Indexer One");
        tracker.record_api_limits("idx-1", None, Some(101), None, None);
        tracker.flush_pending_quota_updates().await;

        let row = sqlx::query(
            "SELECT api_current, api_max, queries_today
             FROM indexer_api_quotas WHERE indexer_id = ?",
        )
        .bind("idx-1")
        .fetch_one(&pool)
        .await
        .expect("quota row should still exist");

        assert_eq!(row.get::<i64, _>("api_current"), 2);
        assert_eq!(row.get::<i64, _>("api_max"), 101);
        assert_eq!(row.get::<i64, _>("queries_today"), 3);
    }

    #[tokio::test]
    async fn sent_requests_are_the_persisted_hit_count_and_quota_readings_are_not() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;

        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        // Four requests left for this indexer: two search pages, a caps
        // refresh and a retry that timed out. None of them parsed a quota.
        for _ in 0..4 {
            tracker.record_api_request_sent("idx-hits", "Hit Counter");
        }
        // A quota reading is a report about the account, not a request.
        tracker.record_api_limits("idx-hits", Some(11), Some(1000), None, None);
        tracker.flush_pending_quota_updates().await;

        let row = sqlx::query(
            "SELECT queries_today, api_current, api_max
             FROM indexer_api_quotas WHERE indexer_id = ?",
        )
        .bind("idx-hits")
        .fetch_one(&pool)
        .await
        .expect("quota row should exist");
        assert_eq!(row.get::<i64, _>("queries_today"), 4);
        assert_eq!(row.get::<i64, _>("api_current"), 11);
        assert_eq!(row.get::<i64, _>("api_max"), 1000);

        let stats = tracker
            .all_stats()
            .into_iter()
            .find(|stats| stats.indexer_id == "idx-hits")
            .expect("indexer stats");
        assert_eq!(stats.api_requests_today, 4);
        assert_eq!(
            stats.queries_last_24h, 0,
            "search runs and API hits are different numbers"
        );
    }

    #[tokio::test]
    async fn a_quota_reading_alone_still_persists() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;

        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.record_api_limits("idx-quota-only", Some(7), Some(100), None, None);
        tracker.flush_pending_quota_updates().await;

        let row = sqlx::query(
            "SELECT queries_today, api_current FROM indexer_api_quotas WHERE indexer_id = ?",
        )
        .bind("idx-quota-only")
        .fetch_one(&pool)
        .await
        .expect("a quota-only update must not be skipped by the flush");
        assert_eq!(row.get::<i64, _>("api_current"), 7);
        assert_eq!(
            row.get::<i64, _>("queries_today"),
            0,
            "reporting a quota reading must not invent an API hit"
        );
    }

    #[tokio::test]
    async fn hydrate_restores_todays_count_and_drops_a_stale_window() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;
        sqlx::query(
            "INSERT INTO indexer_api_quotas
                (indexer_id, api_current, api_max, queries_today, last_reset_at)
             VALUES ('idx-today', 5, 500, 123, datetime('now')),
                    ('idx-stale', 9, 900, 456, datetime('now', '-3 days'))",
        )
        .execute(&pool)
        .await
        .expect("seed quota rows");

        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.hydrate().await;

        let stats = tracker.all_stats();
        let today = stats
            .iter()
            .find(|stats| stats.indexer_id == "idx-today")
            .expect("today's row");
        assert_eq!(
            today.api_requests_today, 123,
            "a restart must not zero the day's spend"
        );
        assert_eq!(today.api_current, Some(5));
        assert_eq!(today.api_max, Some(500));

        let stale = stats
            .iter()
            .find(|stats| stats.indexer_id == "idx-stale")
            .expect("stale row");
        assert_eq!(
            stale.api_requests_today, 0,
            "a window from an earlier UTC day has already rolled over"
        );

        // Counting continues from the hydrated value rather than from zero.
        tracker.record_api_request_sent("idx-today", "Today Indexer");
        let resumed = tracker
            .all_stats()
            .into_iter()
            .find(|stats| stats.indexer_id == "idx-today")
            .expect("today's row");
        assert_eq!(resumed.api_requests_today, 124);
    }

    #[tokio::test]
    async fn midnight_batches_and_retries_preserve_the_send_day_through_hydration() {
        for fail_first_flush in [false, true] {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory sqlite should open");
            if !fail_first_flush {
                create_quota_table(&pool).await;
            }
            let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
            let today = Utc::now().date_naive();
            let yesterday = today.pred_opt().unwrap();
            tracker.record_api_request_sent_on("idx-midnight", "Midnight Indexer", yesterday);
            tracker.record_api_request_sent_on("idx-midnight", "Midnight Indexer", yesterday);
            if fail_first_flush {
                tracker.flush_pending_quota_updates().await;
                assert_eq!(tracker.pending_quota_updates.lock().unwrap().len(), 1);
                create_quota_table(&pool).await;
            }
            tracker.record_api_request_sent_on("idx-midnight", "Midnight Indexer", today);
            assert_eq!(tracker.pending_quota_updates.lock().unwrap().len(), 2);
            assert_eq!(tracker.all_stats()[0].api_requests_today, 1);
            tracker.flush_pending_quota_updates().await;

            let restored = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
            restored.hydrate().await;
            assert_eq!(
                restored.all_stats()[0].api_requests_today,
                1,
                "yesterday's requests must not become today's after restart"
            );
            assert!(tracker.pending_quota_updates.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn explicit_send_timestamps_survive_out_of_order_requeue_and_reopen() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        let today = Utc::now().date_naive();
        let yesterday = today.pred_opt().expect("previous UTC day");
        let at_noon = |day: NaiveDate| {
            DateTime::from_naive_utc_and_offset(day.and_hms_opt(12, 0, 0).expect("valid noon"), Utc)
        };

        // Arrival order is intentionally inverted. The queued key retains the
        // transport send day, so the eventual database replay is day-correct.
        tracker.record_api_request_sent_at("idx-order", "Ordered Indexer", at_noon(today));
        tracker.record_api_request_sent_at("idx-order", "Ordered Indexer", at_noon(yesterday));
        assert_eq!(
            tracker
                .all_stats()
                .pop()
                .expect("live quota stat")
                .api_requests_today,
            1,
            "a delayed callback from yesterday must not reset today's live count"
        );
        assert!(
            tracker.drain_pending_quota_updates().await.is_err(),
            "a failed final drain must report failure rather than discard its batch"
        );
        assert_eq!(tracker.pending_quota_updates.lock().unwrap().len(), 2);

        create_quota_table(&pool).await;
        tracker
            .drain_pending_quota_updates()
            .await
            .expect("final drain should replay both queued UTC days");
        let reopened = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        reopened.hydrate().await;
        let stat = reopened.all_stats().pop().expect("reopened quota stat");
        assert_eq!(
            stat.api_requests_today, 1,
            "yesterday's replay must not become today's usage"
        );
    }

    #[tokio::test]
    async fn final_drain_persists_concurrent_requests_and_reopens_their_total() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;
        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        let mut workers = Vec::new();
        for _ in 0..24 {
            let tracker = tracker.clone();
            workers.push(std::thread::spawn(move || {
                tracker.record_api_request_sent("idx-drain", "Drain Indexer");
            }));
        }
        for worker in workers {
            worker.join().expect("request worker should finish");
        }

        tracker
            .drain_pending_quota_updates()
            .await
            .expect("final drain should persist every admitted request");
        let reopened = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        reopened.hydrate().await;
        let stat = reopened.all_stats().pop().expect("reopened quota stat");
        assert_eq!(stat.api_requests_today, 24);
    }

    #[tokio::test]
    async fn the_persisted_window_resets_on_a_utc_day_boundary_not_a_rolling_day() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;
        // Opened 20 hours ago but on the previous UTC day: a rolling 24-hour
        // window would keep accumulating, a calendar day resets.
        sqlx::query(
            "INSERT INTO indexer_api_quotas (indexer_id, queries_today, last_reset_at)
             VALUES ('idx-day', 40, datetime('now', 'start of day', '-4 hours'))",
        )
        .execute(&pool)
        .await
        .expect("seed quota row");

        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.record_api_request_sent("idx-day", "Day Indexer");
        tracker.flush_pending_quota_updates().await;

        let queries_today: i64 =
            sqlx::query_scalar("SELECT queries_today FROM indexer_api_quotas WHERE indexer_id = ?")
                .bind("idx-day")
                .fetch_one(&pool)
                .await
                .expect("quota row should exist");
        assert_eq!(
            queries_today, 1,
            "a new UTC day starts the count over instead of adding to yesterday's"
        );
    }

    #[tokio::test]
    async fn quota_flush_waits_for_the_shared_sqlite_writer_gate() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;

        let writer_gate = Arc::new(tokio::sync::Mutex::new(()));
        let tracker = tracker_with_gate(&pool, Arc::clone(&writer_gate));
        let guard = writer_gate.lock().await;
        tracker.record_api_request_sent("idx-gated", "Gated Indexer");
        tracker.record_api_limits("idx-gated", Some(7), Some(100), None, None);

        let flushing_tracker = tracker.clone();
        let mut flush_task = tokio::spawn(async move {
            flushing_tracker.flush_pending_quota_updates().await;
        });
        assert!(
            timeout(Duration::from_millis(100), &mut flush_task)
                .await
                .is_err(),
            "quota persistence must wait for the shared writer gate"
        );

        let count_while_held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM indexer_api_quotas")
            .fetch_one(&pool)
            .await
            .expect("quota table should remain readable");
        assert_eq!(count_while_held, 0);

        drop(guard);
        timeout(Duration::from_secs(5), flush_task)
            .await
            .expect("quota flush should finish after releasing the writer gate")
            .expect("quota flush task should not panic");

        let queries_today: i64 =
            sqlx::query_scalar("SELECT queries_today FROM indexer_api_quotas WHERE indexer_id = ?")
                .bind("idx-gated")
                .fetch_one(&pool)
                .await
                .expect("gated quota update should persist");
        assert_eq!(queries_today, 1);
    }

    #[tokio::test]
    async fn quota_flush_requeues_failed_updates() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.record_api_request_sent("idx-retry", "Retry Indexer");
        tracker.record_api_limits("idx-retry", Some(9), Some(200), None, None);

        tracker.flush_pending_quota_updates().await;
        assert_eq!(tracker.pending_quota_updates.lock().unwrap().len(), 1);

        create_quota_table(&pool).await;
        let row = timeout(Duration::from_secs(5), async {
            loop {
                if let Some(row) = sqlx::query(
                    "SELECT api_current, api_max, queries_today
                     FROM indexer_api_quotas WHERE indexer_id = ?",
                )
                .bind("idx-retry")
                .fetch_optional(&pool)
                .await
                .expect("query requeued quota update")
                {
                    break row;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("requeued quota update should flush without new indexer activity");
        assert_eq!(row.get::<i64, _>("api_current"), 9);
        assert_eq!(row.get::<i64, _>("api_max"), 200);
        assert_eq!(row.get::<i64, _>("queries_today"), 1);
        assert!(tracker.pending_quota_updates.lock().unwrap().is_empty());
    }
}
