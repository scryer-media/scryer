//! The `torbox` download client: TorBox fetches the torrent or NZB, and
//! Scryer then downloads the finished files itself.
//!
//! A job passes through three phases. **Submit** records the intent, sends
//! the create, and records the TorBox id from the answer. **Remote** waits
//! for TorBox to hold the whole item, read by one poller per configuration
//! with one list call per item family per tick. **Fetch** plans the item's
//! video and subtitle files as a linked job on Scryer's own HTTP engine,
//! whose link source asks TorBox for a fresh signed link per transfer.
//!
//! Every phase reports the job under one stable id minted at submit. The
//! durable record ([`super::records`]) maps that id to the TorBox item and to
//! the engine job, and it is what lets a restart pick up any phase.
//!
//! Removal deletes exactly the TorBox item this client created, by the id
//! it recorded, and drops the engine job. No local file is ever deleted
//! here; the files the engine wrote stay where they are.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, ClientJobLocator, DownloadClient, DownloadClientAddRequest,
    DownloadClientObservation, DownloadGrabResult, ResolvedDownloadArtifact, torbox_refusal,
};
use scryer_domain::{
    CompletedDownload, DownloadClientConfig, DownloadQueueItem, DownloadQueueState,
    DownloadSeedingSnapshot, ProxyConfig,
};
use tokio::sync::{Notify, OnceCell, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, info, warn};

use super::api::{
    TorboxApi, TorboxCreated, TorboxDownloadTarget, TorboxError, TorboxItem, TorboxItemKind,
    TorboxListQuery, TorboxSeedPreference, TorboxTorrentCreate, TorboxTorrentSource,
    TorboxUsenetCreate, TorboxUsenetSource,
};
use super::budget::{TorboxBudgetLane, TorboxHourlyBudget};
use super::fetch::{self, FetchRefusal};
use super::records::{TorboxJobPhase, TorboxJobRecord, TorboxRecordStore};
use super::state::TorboxRemoteState;
use crate::downloads::clients::http::owned::{
    CONNECTIONS_PER_LINK, HttpClientSettings, HttpEngine, HttpJob, HttpJobState, LinkOutcome,
    LinkRequest, LinkSource, LinkedJobRequest, LiveSnapshot, ResolvedLink, check_download_dir,
    completed_download_for, egress_for, egress_key, queue_item_for,
};
use crate::downloads::clients::{
    MAX_NZB_BYTES, parse_download_client_config_json, read_config_bool, read_config_string,
};

pub(crate) const CLIENT_TYPE: &str = "torbox";

/// How often the poller reads TorBox while a job waits on it.
pub(crate) const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(15);
/// How often the poller reads TorBox while the only thing left to learn is
/// whether a finished torrent is still seeding.
pub(crate) const DEFAULT_SEED_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// How long a create whose answer was lost may stay unmatched before the
/// job is failed as missing.
const SUBMIT_GRACE: chrono::Duration = chrono::Duration::minutes(2);

/// The parsed `torbox` client configuration.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TorboxSettings {
    token: String,
    pub http: HttpClientSettings,
    pub cached_only: bool,
}

impl std::fmt::Debug for TorboxSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorboxSettings")
            .field("token", &"<redacted>")
            .field("http", &self.http)
            .field("cached_only", &self.cached_only)
            .finish()
    }
}

impl TorboxSettings {
    pub(crate) fn from_config_json(raw: &str) -> AppResult<Self> {
        let config = parse_download_client_config_json(raw)?;
        let token =
            read_config_string(&config, &["api_key", "apiKey", "apikey"]).ok_or_else(|| {
                AppError::validation_refused(
                    torbox_refusal::TOKEN_REQUIRED,
                    "the TorBox client needs a TorBox API token",
                )
            })?;
        Ok(Self {
            token,
            http: HttpClientSettings::from_config_json(raw)?,
            cached_only: read_config_bool(&config, &["cached_only", "cachedOnly"], false),
        })
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

// ---------------------------------------------------------------------------
// registry

static RUNTIMES: LazyLock<std::sync::Mutex<HashMap<String, Arc<TorboxRuntime>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// The tasks of runtimes whose configuration was deleted, changed type or
/// moved its download directory, kept until they stop so a successor waits
/// for them before it reads what they wrote.
static RETIRED: LazyLock<std::sync::Mutex<HashMap<String, TaskTracker>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Stop and drop the runtime of a configuration that was deleted or is no
/// longer a TorBox client. Its records, queue and files stay on disk.
pub(crate) fn forget_runtime(config_id: &str) {
    let forgotten = RUNTIMES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(config_id);
    let Some(runtime) = forgotten else {
        return;
    };
    runtime.begin_shutdown();
    info!(
        client_id = config_id,
        "TorBox client stopped for a removed configuration"
    );
    let mut retired = RETIRED.lock().unwrap_or_else(|p| p.into_inner());
    retired.retain(|_, tasks| !(tasks.is_closed() && tasks.is_empty()));
    retired.insert(config_id.to_string(), runtime.all_tasks());
}

/// Bring a running runtime in line with a saved configuration.
pub(crate) fn apply_saved_config(config: &DownloadClientConfig) {
    if !config.client_type.eq_ignore_ascii_case(CLIENT_TYPE) {
        forget_runtime(&config.id);
        return;
    }
    let runtime = RUNTIMES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&config.id)
        .cloned();
    if let Some(runtime) = runtime {
        runtime.set_enabled(config.is_enabled);
    }
}

/// The runtime for a configuration, created on first use. A changed
/// download directory replaces it; everything else applies in place.
fn runtime_for(
    config_id: &str,
    settings: &TorboxSettings,
    proxy_config: Option<&ProxyConfig>,
    http_client: reqwest::Client,
) -> AppResult<Arc<TorboxRuntime>> {
    let key = egress_key(proxy_config)?;
    let api = TorboxApi::new(http_client, settings.token());
    let current = RUNTIMES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(config_id)
        .filter(|runtime| runtime.download_dir == settings.http.download_dir)
        .cloned();
    if let Some(runtime) = current {
        runtime.reconfigure(api, settings);
        if !runtime.engine.has_egress_key(&key) {
            runtime.engine.set_egress(egress_for(proxy_config)?, key);
        }
        return Ok(runtime);
    }
    let egress = egress_for(proxy_config)?;
    let mut runtimes = RUNTIMES.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(runtime) = runtimes.get(config_id)
        && runtime.download_dir == settings.http.download_dir
    {
        runtime.reconfigure(api, settings);
        runtime.engine.set_egress(egress, key);
        return Ok(Arc::clone(runtime));
    }
    let predecessor = match runtimes.remove(config_id) {
        Some(predecessor) => {
            predecessor.begin_shutdown();
            Some(predecessor.all_tasks())
        }
        None => RETIRED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(config_id),
    };
    let runtime = TorboxRuntime::new(
        RuntimeParts {
            client_id: config_id.to_string(),
            settings: settings.clone(),
            api,
            egress,
            egress_key: key,
            predecessor,
            poll_interval: DEFAULT_POLL_INTERVAL,
            seed_poll_interval: DEFAULT_SEED_POLL_INTERVAL,
            budget: TorboxHourlyBudget::default(),
        },
        true,
    );
    runtimes.insert(config_id.to_string(), Arc::clone(&runtime));
    Ok(runtime)
}

// ---------------------------------------------------------------------------
// runtime

/// The shared, swappable API handle: the runtime and the engine's link
/// source read the same one, so a new token applies to both.
type SharedApi = Arc<std::sync::RwLock<TorboxApi>>;

fn current_api(api: &SharedApi) -> TorboxApi {
    api.read().unwrap_or_else(|p| p.into_inner()).clone()
}

pub(crate) struct RuntimeParts {
    pub client_id: String,
    pub settings: TorboxSettings,
    pub api: TorboxApi,
    pub egress: crate::downloads::clients::http::owned::Egress,
    pub egress_key: String,
    pub predecessor: Option<TaskTracker>,
    pub poll_interval: Duration,
    pub seed_poll_interval: Duration,
    pub budget: TorboxHourlyBudget,
}

/// What the poller last read about one job's TorBox item. Kept in memory
/// only: it changes on every tick, and writing it would make an idle
/// download write.
#[derive(Clone, Debug)]
struct RemoteSnapshot {
    state: TorboxRemoteState,
    raw_state: Option<String>,
    progress: Option<f64>,
    size: Option<i64>,
    eta: Option<i64>,
    ratio: Option<f64>,
    private: bool,
    downloaded: bool,
}

impl RemoteSnapshot {
    fn of(item: &TorboxItem) -> Self {
        Self {
            state: TorboxRemoteState::parse(item.download_state.as_deref()),
            raw_state: item.download_state.clone(),
            progress: item.progress,
            size: item.size,
            eta: item.eta,
            ratio: item.ratio,
            private: item.private,
            downloaded: item.is_downloaded(),
        }
    }
}

#[derive(Default)]
struct RuntimeState {
    records: Vec<TorboxJobRecord>,
    snapshots: HashMap<String, RemoteSnapshot>,
    /// Records whose create is in flight in this process, which the poller
    /// must not treat as orphaned.
    in_flight: HashSet<String>,
    /// Finished torrents TorBox no longer seeds (or no longer holds), so the
    /// poller has nothing left to learn about them.
    seeding_ended: HashSet<String>,
}

/// One TorBox client configuration's long-lived state.
pub(crate) struct TorboxRuntime {
    pub(crate) client_id: String,
    pub(crate) download_dir: PathBuf,
    pub(crate) engine: Arc<HttpEngine>,
    api: SharedApi,
    cached_only: AtomicBool,
    enabled: AtomicBool,
    budget: TorboxHourlyBudget,
    store: TorboxRecordStore,
    state: tokio::sync::Mutex<RuntimeState>,
    /// Held by the one record writer.
    save_lock: tokio::sync::Mutex<()>,
    loaded: OnceCell<()>,
    predecessor: std::sync::Mutex<Option<TaskTracker>>,
    wake: Notify,
    parked: watch::Sender<bool>,
    poll_interval: Duration,
    seed_poll_interval: Duration,
    auto_poll: bool,
    poller_started: AtomicBool,
    shutdown: CancellationToken,
    tasks: TaskTracker,
    #[cfg(test)]
    record_writes: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for TorboxRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorboxRuntime")
            .field("client_id", &self.client_id)
            .field("download_dir", &self.download_dir)
            .finish_non_exhaustive()
    }
}

impl TorboxRuntime {
    /// A runtime. With `auto_poll` off the poller never runs on its own and
    /// a test drives it with [`Self::poll_once`].
    pub(crate) fn new(parts: RuntimeParts, auto_poll: bool) -> Arc<Self> {
        let api: SharedApi = Arc::new(std::sync::RwLock::new(parts.api));
        let links: Arc<dyn LinkSource> = Arc::new(TorboxLinks {
            api: Arc::clone(&api),
        });
        let download_dir = parts.settings.http.download_dir.clone();
        let engine = HttpEngine::new_with_link_source(
            parts.client_id.clone(),
            download_dir.clone(),
            parts.settings.http.limits,
            parts.egress,
            parts.egress_key,
            parts.predecessor.clone(),
            Some(links),
        );
        Arc::new(Self {
            store: TorboxRecordStore::for_client(&download_dir, &parts.client_id),
            client_id: parts.client_id,
            download_dir,
            engine,
            api,
            cached_only: AtomicBool::new(parts.settings.cached_only),
            enabled: AtomicBool::new(true),
            budget: parts.budget,
            state: tokio::sync::Mutex::new(RuntimeState::default()),
            save_lock: tokio::sync::Mutex::new(()),
            loaded: OnceCell::new(),
            predecessor: std::sync::Mutex::new(parts.predecessor),
            wake: Notify::new(),
            parked: watch::channel(false).0,
            poll_interval: parts.poll_interval,
            seed_poll_interval: parts.seed_poll_interval,
            auto_poll,
            poller_started: AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            tasks: TaskTracker::new(),
            #[cfg(test)]
            record_writes: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn api(&self) -> TorboxApi {
        current_api(&self.api)
    }

    fn reconfigure(&self, api: TorboxApi, settings: &TorboxSettings) {
        *self.api.write().unwrap_or_else(|p| p.into_inner()) = api;
        self.cached_only
            .store(settings.cached_only, Ordering::Release);
        self.engine.set_limits(settings.http.limits);
    }

    fn cached_only(&self) -> bool {
        self.cached_only.load(Ordering::Acquire)
    }

    pub(crate) fn set_enabled(self: &Arc<Self>, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        self.engine.set_enabled(enabled);
        self.wake.notify_one();
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    fn begin_shutdown(&self) {
        self.shutdown.cancel();
        self.tasks.close();
        self.engine.begin_shutdown();
    }

    /// The runtime's own tasks and the engine's workers, as one tracker a
    /// successor can wait on.
    fn all_tasks(&self) -> TaskTracker {
        let combined = TaskTracker::new();
        let (own, workers) = (self.tasks.clone(), self.engine.workers());
        if tokio::runtime::Handle::try_current().is_ok() {
            combined.spawn(async move {
                own.wait().await;
                workers.wait().await;
            });
        }
        combined.close();
        combined
    }

    /// Stop the runtime the way a configuration change does, returning what
    /// a successor waits on.
    #[cfg(test)]
    pub(crate) fn retire(&self) -> TaskTracker {
        self.begin_shutdown();
        self.all_tasks()
    }

    /// Records written since the runtime started.
    #[cfg(test)]
    pub(crate) fn record_writes(&self) -> u64 {
        self.record_writes.load(Ordering::Relaxed)
    }

    /// A receiver that is `true` while the poller is parked with nothing to
    /// wait for.
    #[cfg(test)]
    pub(crate) fn parked(&self) -> watch::Receiver<bool> {
        self.parked.subscribe()
    }

    #[cfg(test)]
    pub(crate) async fn records(&self) -> Vec<TorboxJobRecord> {
        self.state.lock().await.records.clone()
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> &TorboxRecordStore {
        &self.store
    }

    /// Load the records once, after any predecessor has stopped writing,
    /// and start the poller when a job waits on TorBox.
    async fn ensure_loaded(self: &Arc<Self>) -> AppResult<()> {
        if self.shutdown.is_cancelled() {
            return Err(AppError::download_submit_unavailable(
                "the TorBox client's configuration changed; try again",
            ));
        }
        self.loaded
            .get_or_try_init(|| async {
                let predecessor = self
                    .predecessor
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                if let Some(predecessor) = predecessor {
                    predecessor.wait().await;
                }
                let records = self.store.load().await?;
                self.state.lock().await.records = records;
                Ok::<(), AppError>(())
            })
            .await?;
        self.engine.ensure_started().await?;
        self.start_poller();
        Ok(())
    }

    fn start_poller(self: &Arc<Self>) {
        if !self.auto_poll
            || self.shutdown.is_cancelled()
            || self.poller_started.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let runtime = Arc::clone(self);
        self.tasks.spawn(async move { runtime.run_poller().await });
    }

    /// Write the records. One writer at a time, and the snapshot is taken
    /// after the writer's turn comes, so the last write holds the newest
    /// state.
    async fn persist(&self) -> AppResult<()> {
        let _turn = self.save_lock.lock().await;
        let snapshot = self.state.lock().await.records.clone();
        self.store.save(&snapshot).await?;
        #[cfg(test)]
        self.record_writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// How long the poller waits before its next read, or `None` when
    /// nothing waits on TorBox and it should park.
    async fn next_wait(&self) -> Option<Duration> {
        if !self.is_enabled() {
            return None;
        }
        let state = self.state.lock().await;
        let mut seeding = false;
        for record in &state.records {
            match record.phase {
                TorboxJobPhase::Submitting | TorboxJobPhase::Remote => {
                    return Some(self.poll_interval);
                }
                TorboxJobPhase::Fetching
                    if record.watches_seeding() && !state.seeding_ended.contains(&record.id) =>
                {
                    seeding = true;
                }
                _ => {}
            }
        }
        seeding.then_some(self.seed_poll_interval)
    }

    async fn run_poller(self: Arc<Self>) {
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let Some(wait) = self.next_wait().await else {
                deadline = None;
                self.parked.send_replace(true);
                tokio::select! {
                    _ = self.wake.notified() => {}
                    _ = self.shutdown.cancelled() => return,
                }
                self.parked.send_replace(false);
                continue;
            };
            let soonest = tokio::time::Instant::now() + wait;
            let at = deadline.map_or(soonest, |at| at.min(soonest));
            deadline = Some(at);
            tokio::select! {
                _ = tokio::time::sleep_until(at) => {
                    deadline = None;
                    if let Err(error) = self.poll_once().await {
                        warn!(
                            client_id = self.client_id.as_str(),
                            error = %error,
                            "TorBox poll failed; retrying on the next tick"
                        );
                    }
                }
                // A new job or a removal: work out the wait again.
                _ = self.wake.notified() => {}
                _ = self.shutdown.cancelled() => return,
            }
        }
    }

    /// One poll: one list call per item family something waits on, then
    /// every waiting job is brought up to date. Records are written only
    /// when a job changes phase.
    pub(crate) async fn poll_once(self: &Arc<Self>) -> AppResult<()> {
        self.ensure_loaded().await?;
        let kinds = {
            let state = self.state.lock().await;
            let mut kinds: Vec<TorboxItemKind> = Vec::new();
            for record in &state.records {
                let waits = match record.phase {
                    TorboxJobPhase::Submitting => !state.in_flight.contains(&record.id),
                    TorboxJobPhase::Remote => true,
                    TorboxJobPhase::Fetching => {
                        record.watches_seeding() && !state.seeding_ended.contains(&record.id)
                    }
                    TorboxJobPhase::Failed => false,
                };
                if waits && !kinds.contains(&record.kind) {
                    kinds.push(record.kind);
                }
            }
            kinds
        };
        if kinds.is_empty() {
            return Ok(());
        }
        let api = self.api();
        let mut listed: HashMap<TorboxItemKind, Vec<TorboxItem>> = HashMap::new();
        for kind in kinds {
            let items = api
                .list(
                    kind,
                    TorboxListQuery {
                        bypass_cache: true,
                        ..TorboxListQuery::default()
                    },
                )
                .await
                .map_err(TorboxError::into_app_error)?;
            listed.insert(kind, items);
        }

        // Items the list did not show are confirmed one by one before a job
        // is failed or its seeding is taken as ended; the list may be paged,
        // and otherwise this only happens to a job whose item vanished.
        let missing: Vec<(TorboxItemKind, i64)> = {
            let state = self.state.lock().await;
            state
                .records
                .iter()
                .filter(|record| {
                    record.phase == TorboxJobPhase::Remote
                        || (record.watches_seeding() && !state.seeding_ended.contains(&record.id))
                })
                .filter_map(|record| Some((record.kind, record.torbox_id?)))
                .filter(|(kind, id)| {
                    !listed
                        .get(kind)
                        .is_some_and(|items| items.iter().any(|item| item.id == *id))
                })
                .collect()
        };
        let mut confirmed: HashMap<(TorboxItemKind, i64), Option<TorboxItem>> = HashMap::new();
        for (kind, id) in missing {
            let item = api
                .item(kind, id, true)
                .await
                .map_err(TorboxError::into_app_error)?;
            confirmed.insert((kind, id), item);
        }

        let now = Utc::now();
        let mut changed = false;
        let mut ready: Vec<(String, TorboxItem)> = Vec::new();
        {
            let mut state = self.state.lock().await;
            let RuntimeState {
                records,
                snapshots,
                in_flight,
                seeding_ended,
            } = &mut *state;
            let claimed: HashSet<(TorboxItemKind, i64)> = records
                .iter()
                .filter_map(|record| Some((record.kind, record.torbox_id?)))
                .collect();
            let mut newly_claimed: HashSet<(TorboxItemKind, i64)> = HashSet::new();
            for record in records.iter_mut() {
                let items = listed.get(&record.kind).map(Vec::as_slice).unwrap_or(&[]);
                match record.phase {
                    TorboxJobPhase::Submitting if !in_flight.contains(&record.id) => {
                        let found = match_submission(record, items, |id| {
                            claimed.contains(&(record.kind, id))
                                || newly_claimed.contains(&(record.kind, id))
                        });
                        if let Some(item) = found {
                            newly_claimed.insert((record.kind, item.id));
                            record.torbox_id = Some(item.id);
                            record.phase = TorboxJobPhase::Remote;
                            record.updated_at = now;
                            changed = true;
                            info!(
                                client_id = self.client_id.as_str(),
                                item_id = record.id.as_str(),
                                torbox_id = item.id,
                                "TorBox item found for a create whose answer was lost"
                            );
                        } else if now - record.created_at >= SUBMIT_GRACE {
                            record.fail(
                                torbox_refusal::ITEM_MISSING,
                                "TorBox shows no item for this download; its create may not have arrived".into(),
                                now,
                            );
                            changed = true;
                        }
                    }
                    TorboxJobPhase::Remote => {
                        let Some(torbox_id) = record.torbox_id else {
                            continue;
                        };
                        let item = items.iter().find(|item| item.id == torbox_id).or_else(|| {
                            confirmed
                                .get(&(record.kind, torbox_id))
                                .and_then(Option::as_ref)
                        });
                        let Some(item) = item else {
                            record.fail(
                                torbox_refusal::ITEM_MISSING,
                                format!("TorBox no longer holds item {torbox_id}"),
                                now,
                            );
                            snapshots.remove(&record.id);
                            changed = true;
                            continue;
                        };
                        let snapshot = RemoteSnapshot::of(item);
                        if snapshot.state == TorboxRemoteState::Failed && !snapshot.downloaded {
                            record.fail(
                                torbox_refusal::REMOTE_FAILED,
                                format!(
                                    "TorBox reports the download as {}",
                                    snapshot.raw_state.as_deref().unwrap_or("failed")
                                ),
                                now,
                            );
                            snapshots.remove(&record.id);
                            changed = true;
                            continue;
                        }
                        if snapshot.downloaded {
                            if record.remote_finished_at.is_none() {
                                record.remote_finished_at = Some(now);
                            }
                            ready.push((record.id.clone(), item.clone()));
                        }
                        snapshots.insert(record.id.clone(), snapshot);
                    }
                    TorboxJobPhase::Fetching
                        if record.watches_seeding() && !seeding_ended.contains(&record.id) =>
                    {
                        let Some(torbox_id) = record.torbox_id else {
                            continue;
                        };
                        let item = items.iter().find(|item| item.id == torbox_id).or_else(|| {
                            confirmed
                                .get(&(record.kind, torbox_id))
                                .and_then(Option::as_ref)
                        });
                        match item {
                            Some(item) => {
                                let snapshot = RemoteSnapshot::of(item);
                                if !snapshot.state.is_seeding() {
                                    seeding_ended.insert(record.id.clone());
                                }
                                snapshots.insert(record.id.clone(), snapshot);
                            }
                            // Confirmed gone by the single-item read.
                            None => {
                                seeding_ended.insert(record.id.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if changed {
            self.persist().await?;
        }
        for (record_id, item) in ready {
            self.start_fetch(&record_id, &item).await?;
        }
        Ok(())
    }

    /// Plan a finished item's files as an engine job. The job folder is
    /// recorded before the engine job is added, so a restart between the
    /// two finds that job again instead of adding a second one.
    async fn start_fetch(self: &Arc<Self>, record_id: &str, item: &TorboxItem) -> AppResult<()> {
        let Some(record) = self.record(record_id).await else {
            return Ok(());
        };
        let files = match fetch::plan_fetch(record.kind, item) {
            Ok(files) => files,
            Err(FetchRefusal { reason, message }) => {
                return self.fail_record(record_id, reason, message).await;
            }
        };
        let job_folder = match record.job_folder.clone() {
            Some(folder) => folder,
            None => {
                let folder = self.choose_job_folder(&record).await?;
                self.update_record(record_id, |record| {
                    record.job_folder = Some(folder.clone());
                })
                .await?;
                folder
            }
        };
        let adopted = self
            .engine
            .jobs_where(|job| {
                job.output_basename == job_folder && job.download_id == record.download_id
            })
            .await?
            .into_iter()
            .next()
            .map(|(job, _)| job.id);
        let engine_job_id = match adopted {
            Some(id) => id,
            None => {
                let request = LinkedJobRequest {
                    download_id: record.download_id.clone(),
                    title_id: record.title_id.clone(),
                    title_name: record.title_name.clone(),
                    facet: record.facet.clone(),
                    release_name: record.release_name.clone(),
                    category: record.category.clone(),
                    job_folder: job_folder.clone(),
                    files,
                    expected_size_bytes: None,
                    expected_duration_seconds: None,
                    verify: Default::default(),
                    subtitle_required: false,
                };
                match self.engine.add_linked_job(request).await {
                    Ok(id) => id,
                    Err(error) => {
                        let reason = error
                            .reason()
                            .unwrap_or(torbox_refusal::FILE_PATHS_UNUSABLE);
                        return self.fail_record(record_id, reason, error.to_string()).await;
                    }
                }
            }
        };
        let now = Utc::now();
        self.update_record(record_id, |record| {
            record.engine_job_id = Some(engine_job_id.clone());
            record.phase = TorboxJobPhase::Fetching;
            record.remote_finished_at.get_or_insert(now);
            record.updated_at = now;
        })
        .await?;
        info!(
            client_id = self.client_id.as_str(),
            item_id = record_id,
            "TorBox finished the download; fetching its files"
        );
        Ok(())
    }

    /// The release name as a folder no other job of this client uses.
    async fn choose_job_folder(self: &Arc<Self>, record: &TorboxJobRecord) -> AppResult<String> {
        let base = fetch::job_folder_for(&record.release_name);
        let prefix = base.to_lowercase();
        let mut taken: HashSet<String> = {
            let state = self.state.lock().await;
            state
                .records
                .iter()
                .filter(|other| other.id != record.id)
                .filter_map(|other| other.job_folder.as_deref())
                .map(str::to_lowercase)
                .collect()
        };
        taken.extend(
            self.engine
                .jobs_where(|job| job.output_basename.to_lowercase().starts_with(&prefix))
                .await?
                .into_iter()
                .map(|(job, _)| job.output_basename.to_lowercase()),
        );
        Ok(fetch::unique_job_folder(&base, &taken))
    }

    async fn record(&self, id: &str) -> Option<TorboxJobRecord> {
        self.state
            .lock()
            .await
            .records
            .iter()
            .find(|record| record.id == id)
            .cloned()
    }

    async fn update_record(
        &self,
        id: &str,
        change: impl FnOnce(&mut TorboxJobRecord),
    ) -> AppResult<()> {
        {
            let mut state = self.state.lock().await;
            let Some(record) = state.records.iter_mut().find(|record| record.id == id) else {
                return Ok(());
            };
            change(record);
        }
        self.persist().await
    }

    async fn fail_record(&self, id: &str, reason: &'static str, message: String) -> AppResult<()> {
        warn!(
            client_id = self.client_id.as_str(),
            item_id = id,
            reason,
            "TorBox download failed: {message}"
        );
        let now = Utc::now();
        {
            let mut state = self.state.lock().await;
            state.snapshots.remove(id);
            if let Some(record) = state.records.iter_mut().find(|record| record.id == id) {
                record.fail(reason, message, now);
            }
        }
        self.persist().await
    }
}

/// The TorBox item a create whose answer was lost made: the torrent with
/// the recorded info hash, or the one unclaimed usenet item with the name
/// the create sent.
fn match_submission<'a>(
    record: &TorboxJobRecord,
    items: &'a [TorboxItem],
    claimed: impl Fn(i64) -> bool,
) -> Option<&'a TorboxItem> {
    match record.kind {
        TorboxItemKind::Torrent => {
            let hash = record.info_hash.as_deref()?;
            items.iter().find(|item| {
                item.hash
                    .as_deref()
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(hash))
            })
        }
        TorboxItemKind::Usenet => {
            let mut matches = items.iter().filter(|item| {
                !claimed(item.id) && item.name.as_deref() == Some(record.submitted_name.as_str())
            });
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        }
    }
}

// ---------------------------------------------------------------------------
// link source

/// Resolves a planned file to a fresh signed TorBox link, per transfer.
struct TorboxLinks {
    api: SharedApi,
}

// TorBox caps the connections one link may serve; the engine never opens
// more than one per link.
const _: () = assert!(CONNECTIONS_PER_LINK == 1);

#[async_trait]
impl LinkSource for TorboxLinks {
    async fn resolve(&self, request: &LinkRequest<'_>) -> LinkOutcome {
        debug!(
            engine_job_id = request.job_id,
            purpose = ?request.purpose,
            "requesting a TorBox download link"
        );
        let Some((kind, item_id, file_id)) = fetch::parse_link_key(request.key) else {
            return LinkOutcome::Refused {
                reason: torbox_refusal::REMOTE_FAILED,
                message: "the file's TorBox reference is unreadable".to_string(),
            };
        };
        let api = current_api(&self.api);
        match api
            .request_download(kind, item_id, TorboxDownloadTarget::File(file_id))
            .await
        {
            Ok(link) => match reqwest::Url::parse(link.expose()) {
                Ok(url) => LinkOutcome::Ready(ResolvedLink::new(url)),
                Err(_) => LinkOutcome::Refused {
                    reason: torbox_refusal::REMOTE_FAILED,
                    message: "TorBox answered with an unusable download link".to_string(),
                },
            },
            Err(error) => link_outcome_for(error),
        }
    }
}

fn link_outcome_for(error: TorboxError) -> LinkOutcome {
    let message = error.to_string();
    let transient_wait = match &error {
        TorboxError::RateLimited { retry_after, .. } => {
            Some(retry_after.unwrap_or(super::api::TORBOX_DEFAULT_RATE_LIMIT_RETRY))
        }
        TorboxError::Transport { .. } => Some(super::api::TORBOX_DEFAULT_SERVER_FAULT_RETRY),
        _ => None,
    };
    if let Some(wait) = transient_wait {
        return LinkOutcome::RetryAfter { wait, message };
    }
    match error.into_app_error() {
        AppError::TemporaryUnavailable { retry_after, .. } => LinkOutcome::RetryAfter {
            wait: retry_after.unwrap_or(super::api::TORBOX_DEFAULT_SERVER_FAULT_RETRY),
            message,
        },
        other => LinkOutcome::Refused {
            reason: other.reason().unwrap_or(torbox_refusal::REMOTE_FAILED),
            message,
        },
    }
}

// ---------------------------------------------------------------------------
// the client

/// The `torbox` download client for one configuration.
pub struct TorboxDownloadClient {
    runtime: Arc<TorboxRuntime>,
}

impl TorboxDownloadClient {
    pub(crate) fn for_config(
        config: &DownloadClientConfig,
        proxy_config: Option<&ProxyConfig>,
        http_client: reqwest::Client,
    ) -> AppResult<Self> {
        let settings = TorboxSettings::from_config_json(&config.config_json)?;
        let runtime = runtime_for(&config.id, &settings, proxy_config, http_client)?;
        runtime.set_enabled(config.is_enabled);
        Ok(Self { runtime })
    }

    #[cfg(test)]
    pub(crate) fn with_runtime(runtime: Arc<TorboxRuntime>) -> Self {
        Self { runtime }
    }

    /// Queue items for records, reading the engine once for every record
    /// in the fetch phase.
    async fn items_for(
        &self,
        records: Vec<TorboxJobRecord>,
        snapshots: &HashMap<String, RemoteSnapshot>,
        seeding_ended: &HashSet<String>,
        want: impl Fn(&TorboxJobRecord, Option<&HttpJob>) -> bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let engine_ids: HashSet<&str> = records
            .iter()
            .filter(|record| record.phase == TorboxJobPhase::Fetching)
            .filter_map(|record| record.engine_job_id.as_deref())
            .collect();
        let jobs: HashMap<String, (HttpJob, Option<LiveSnapshot>)> = if engine_ids.is_empty() {
            HashMap::new()
        } else {
            self.runtime
                .engine
                .jobs_where(|job| engine_ids.contains(job.id.as_str()))
                .await?
                .into_iter()
                .map(|(job, live)| (job.id.clone(), (job, live)))
                .collect()
        };
        let mut items = Vec::new();
        for record in &records {
            let engine = match record.phase {
                TorboxJobPhase::Fetching => {
                    // A fetch whose engine job is gone has nothing to show.
                    let Some(found) = record.engine_job_id.as_deref().and_then(|id| jobs.get(id))
                    else {
                        continue;
                    };
                    Some(found)
                }
                _ => None,
            };
            if !want(record, engine.map(|(job, _)| job)) {
                continue;
            }
            items.push(queue_item(
                record,
                engine.map(|(job, live)| (job, *live)),
                snapshots.get(&record.id),
                seeding_ended.contains(&record.id),
            ));
        }
        Ok(items)
    }

    async fn listing(
        &self,
        want: impl Fn(&TorboxJobRecord, Option<&HttpJob>) -> bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.runtime.ensure_loaded().await?;
        let (records, snapshots, seeding_ended) = {
            let state = self.runtime.state.lock().await;
            (
                state.records.clone(),
                state.snapshots.clone(),
                state.seeding_ended.clone(),
            )
        };
        self.items_for(records, &snapshots, &seeding_ended, want)
            .await
    }

    async fn submit(&self, request: &DownloadClientAddRequest) -> AppResult<DownloadGrabResult> {
        let runtime = &self.runtime;
        runtime.ensure_loaded().await?;
        let source = submission_source(request).await?;
        let cached_only = runtime.cached_only();
        match source.kind() {
            TorboxItemKind::Usenet => runtime.budget.check(TorboxBudgetLane::UsenetCreate)?,
            TorboxItemKind::Torrent if !cached_only => {
                runtime.budget.check(TorboxBudgetLane::UncachedTorrent)?
            }
            TorboxItemKind::Torrent => {}
        }
        check_download_dir(&runtime.download_dir).await?;

        let now = Utc::now();
        let facet = request
            .search_facet
            .clone()
            .unwrap_or_else(|| request.title.facet.clone());
        let release_name = request
            .release_title_without_password()
            .or_else(|| request.source_title_without_password())
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| request.title.name.clone());
        let record = TorboxJobRecord {
            id: uuid::Uuid::new_v4().to_string(),
            download_id: request
                .download_id
                .as_ref()
                .map(scryer_domain::download_identity::DownloadId::to_wire),
            title_id: request.title.id.clone(),
            title_name: request.title.name.clone(),
            facet: facet.as_str().to_string(),
            release_name: release_name.clone(),
            category: request
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            kind: source.kind(),
            torbox_id: None,
            info_hash: source.info_hash(),
            submitted_name: release_name.clone(),
            phase: TorboxJobPhase::Submitting,
            job_folder: None,
            engine_job_id: None,
            seed_goal_ratio: request.seed_goal_ratio,
            seed_goal_seconds: request.seed_goal_seconds,
            created_at: now,
            updated_at: now,
            remote_finished_at: None,
            error: None,
            error_reason: None,
        };
        let record_id = record.id.clone();
        let info_hash = record.info_hash.clone();
        {
            let mut state = runtime.state.lock().await;
            state.in_flight.insert(record_id.clone());
            state.records.push(record);
        }
        if let Err(error) = runtime.persist().await {
            self.drop_record(&record_id).await;
            return Err(error);
        }

        let api = runtime.api();
        let seeds = request.seed_goal_ratio.is_some() || request.seed_goal_seconds.is_some();
        let kind = source.kind();
        let outcome = match source {
            SubmissionSource::Torrent { source, .. } => {
                api.create_torrent(TorboxTorrentCreate {
                    source,
                    seed: seeds.then_some(TorboxSeedPreference::Seed),
                    allow_zip: false,
                    name: Some(release_name.clone()),
                    as_queued: None,
                    add_only_if_cached: cached_only,
                })
                .await
            }
            SubmissionSource::Usenet { bytes, file_name } => {
                api.create_usenet(TorboxUsenetCreate {
                    source: TorboxUsenetSource::File { file_name, bytes },
                    name: Some(release_name.clone()),
                    password: request.password_candidates().first().map(str::to_string),
                    post_processing: None,
                    as_queued: None,
                    add_only_if_cached: cached_only,
                })
                .await
            }
        };

        let created: Option<TorboxCreated> = match outcome {
            Ok(created) => {
                self.record_budget(kind, cached_only, created.cached);
                Some(created)
            }
            Err(error) if error.outcome_unknown() => {
                self.record_budget(kind, cached_only, None);
                warn!(
                    client_id = runtime.client_id.as_str(),
                    item_id = record_id.as_str(),
                    error = %error,
                    "TorBox create answer was lost; looking for the item"
                );
                match self.reconcile(&record_id).await {
                    Some(item_id) => Some(TorboxCreated {
                        id: item_id,
                        hash: None,
                        auth_id: None,
                        cached: None,
                    }),
                    None => {
                        runtime.state.lock().await.in_flight.remove(&record_id);
                        runtime.wake.notify_one();
                        return Err(AppError::DownloadSubmitAmbiguous(format!(
                            "TorBox may or may not have accepted the download ({error}); Scryer keeps looking for it"
                        )));
                    }
                }
            }
            Err(error) => {
                self.drop_record(&record_id).await;
                return Err(error.into_app_error());
            }
        };
        let created = created.expect("a create outcome");
        let info_hash = info_hash.or_else(|| {
            scryer_application::normalize_torrent_info_hash(created.hash.as_deref())
                .filter(|hash| hash.len() == 40)
        });
        let accepted = {
            let mut state = runtime.state.lock().await;
            state.in_flight.remove(&record_id);
            match state
                .records
                .iter_mut()
                .find(|record| record.id == record_id)
            {
                Some(record) => {
                    record.torbox_id = Some(created.id);
                    record.info_hash = info_hash.clone();
                    record.phase = TorboxJobPhase::Remote;
                    record.updated_at = Utc::now();
                    true
                }
                None => false,
            }
        };
        if accepted {
            if let Err(error) = runtime.persist().await {
                // The item exists on TorBox; the next write records it, and
                // until then a restart finds it as an unanswered create.
                warn!(
                    client_id = runtime.client_id.as_str(),
                    item_id = record_id.as_str(),
                    error = %error,
                    "the TorBox id could not be recorded yet"
                );
            }
        }
        runtime.wake.notify_one();
        info!(
            client_id = runtime.client_id.as_str(),
            item_id = record_id.as_str(),
            torbox_id = created.id,
            "download sent to TorBox"
        );
        Ok(DownloadGrabResult {
            job_id: record_id,
            client_id: None,
            client_type: CLIENT_TYPE.to_string(),
            info_hash,
            download_id: request.download_id,
            seed_goals: None,
            grabbed_file_names: Vec::new(),
        })
    }

    fn record_budget(&self, kind: TorboxItemKind, cached_only: bool, cached: Option<bool>) {
        match kind {
            TorboxItemKind::Usenet => self.runtime.budget.record(TorboxBudgetLane::UsenetCreate),
            TorboxItemKind::Torrent if !cached_only => {
                self.runtime.budget.record_torrent_create(cached)
            }
            TorboxItemKind::Torrent => {}
        }
    }

    /// Look for the item a create whose answer was lost made.
    async fn reconcile(&self, record_id: &str) -> Option<i64> {
        let record = self.runtime.record(record_id).await?;
        let items = self
            .runtime
            .api()
            .list(
                record.kind,
                TorboxListQuery {
                    bypass_cache: true,
                    ..TorboxListQuery::default()
                },
            )
            .await
            .ok()?;
        let claimed: HashSet<i64> = {
            let state = self.runtime.state.lock().await;
            state
                .records
                .iter()
                .filter(|other| other.kind == record.kind)
                .filter_map(|other| other.torbox_id)
                .collect()
        };
        match_submission(&record, &items, |id| claimed.contains(&id)).map(|item| item.id)
    }

    /// Forget a record whose create never reached TorBox.
    async fn drop_record(&self, record_id: &str) {
        {
            let mut state = self.runtime.state.lock().await;
            state.in_flight.remove(record_id);
            state.records.retain(|record| record.id != record_id);
        }
        if let Err(error) = self.runtime.persist().await {
            warn!(
                client_id = self.runtime.client_id.as_str(),
                item_id = record_id,
                error = %error,
                "a refused TorBox download could not be dropped from the records"
            );
        }
    }
}

/// What a grab sends to TorBox.
enum SubmissionSource {
    Torrent {
        source: TorboxTorrentSource,
        info_hash: Option<String>,
    },
    Usenet {
        bytes: Vec<u8>,
        file_name: String,
    },
}

impl SubmissionSource {
    fn kind(&self) -> TorboxItemKind {
        match self {
            Self::Torrent { .. } => TorboxItemKind::Torrent,
            Self::Usenet { .. } => TorboxItemKind::Usenet,
        }
    }

    fn info_hash(&self) -> Option<String> {
        match self {
            Self::Torrent { info_hash, .. } => info_hash.clone(),
            Self::Usenet { .. } => None,
        }
    }
}

fn v1_info_hash(raw: Option<&str>) -> Option<String> {
    scryer_application::normalize_torrent_info_hash(raw).filter(|hash| hash.len() == 40)
}

/// The torrent or NZB a grab carries, from bytes Scryer already holds.
/// Nothing here fetches an indexer URL.
async fn submission_source(request: &DownloadClientAddRequest) -> AppResult<SubmissionSource> {
    match &request.resolved_download_artifact {
        Some(ResolvedDownloadArtifact::Magnet {
            uri,
            info_hash_hint,
        }) => Ok(SubmissionSource::Torrent {
            info_hash: v1_info_hash(info_hash_hint.as_deref())
                .or_else(|| {
                    v1_info_hash(scryer_application::extract_magnet_info_hash(uri).as_deref())
                })
                .or_else(|| v1_info_hash(request.info_hash_hint.as_deref())),
            source: TorboxTorrentSource::Magnet(uri.clone()),
        }),
        Some(ResolvedDownloadArtifact::TorrentFile {
            bytes,
            file_name,
            info_hash_hint,
            ..
        }) => Ok(SubmissionSource::Torrent {
            info_hash: v1_info_hash(info_hash_hint.as_deref())
                .or_else(|| v1_info_hash(request.info_hash_hint.as_deref())),
            source: TorboxTorrentSource::File {
                file_name: file_name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "release.torrent".to_string()),
                bytes: bytes.clone(),
            },
        }),
        Some(ResolvedDownloadArtifact::Nzb {
            bytes, file_name, ..
        }) => {
            check_nzb_size(bytes)?;
            let facet = request
                .search_facet
                .clone()
                .unwrap_or_else(|| request.title.facet.clone());
            let head = &bytes[..bytes.len().min(scryer_application::NZB_HEAD_PROBE_BYTES)];
            scryer_application::enforce_nzb_category_gate(head, &facet)?;
            Ok(SubmissionSource::Usenet {
                bytes: bytes.clone(),
                file_name: nzb_file_name(file_name.as_deref()),
            })
        }
        None => {
            if let Some(staged) = request.staged_nzb.as_ref() {
                let bytes =
                    crate::downloads::clients::router::read_staged_nzb_bytes(staged).await?;
                check_nzb_size(&bytes)?;
                return Ok(SubmissionSource::Usenet {
                    bytes,
                    file_name: nzb_file_name(None),
                });
            }
            let hint = request
                .source_hint
                .as_deref()
                .map(str::trim)
                .filter(|hint| scryer_application::is_valid_magnet_uri(hint));
            if let Some(uri) = hint {
                return Ok(SubmissionSource::Torrent {
                    info_hash: v1_info_hash(
                        scryer_application::extract_magnet_info_hash(uri).as_deref(),
                    )
                    .or_else(|| v1_info_hash(request.info_hash_hint.as_deref())),
                    source: TorboxTorrentSource::Magnet(uri.to_string()),
                });
            }
            Err(AppError::validation_refused(
                torbox_refusal::SOURCE_MISSING,
                "the TorBox client needs a magnet, a torrent file or an NZB; none was resolved",
            ))
        }
    }
}

fn check_nzb_size(bytes: &[u8]) -> AppResult<()> {
    if bytes.is_empty() {
        return Err(AppError::validation_refused(
            torbox_refusal::INVALID_NZB,
            "the NZB is empty",
        ));
    }
    if bytes.len() as u64 > MAX_NZB_BYTES {
        return Err(AppError::validation_refused(
            crate::error_reasons::ACQUISITION_NZB_ARTIFACT_TOO_LARGE,
            format!("the NZB is larger than {MAX_NZB_BYTES} bytes"),
        ));
    }
    Ok(())
}

fn nzb_file_name(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "release.nzb".to_string())
}

// ---------------------------------------------------------------------------
// reporting

fn queue_item(
    record: &TorboxJobRecord,
    engine: Option<(&HttpJob, Option<LiveSnapshot>)>,
    snapshot: Option<&RemoteSnapshot>,
    seeding_ended: bool,
) -> DownloadQueueItem {
    let mut item = match engine {
        Some((job, live)) => queue_item_for(job, live, CLIENT_TYPE),
        None => remote_queue_item(record, snapshot),
    };
    item.id = record.id.clone();
    item.download_client_item_id = record.id.clone();
    item.download_id = record.download_id.clone();
    item.queued_at = Some(record.created_at.to_rfc3339());
    if record.kind == TorboxItemKind::Torrent {
        item.seeding = Some(seeding_snapshot(
            record,
            engine.map(|(job, _)| job),
            snapshot,
            seeding_ended,
        ));
    }
    item
}

/// A job TorBox still works on, before any file is fetched.
fn remote_queue_item(
    record: &TorboxJobRecord,
    snapshot: Option<&RemoteSnapshot>,
) -> DownloadQueueItem {
    let failed = record.phase == TorboxJobPhase::Failed;
    let remote_state = snapshot.map(|snapshot| snapshot.state);
    let state = if failed {
        DownloadQueueState::Failed
    } else {
        match remote_state {
            None | Some(TorboxRemoteState::Queued) => DownloadQueueState::Queued,
            Some(TorboxRemoteState::Paused) => DownloadQueueState::Paused,
            Some(TorboxRemoteState::Stalled) => DownloadQueueState::Warning,
            Some(_) => DownloadQueueState::Downloading,
        }
    };
    let stalled = state == DownloadQueueState::Warning;
    let (attention_reason, attention_reason_code) = if failed {
        (record.error.clone(), record.error_reason.clone())
    } else if stalled {
        (
            Some("TorBox has no peers or sources to download this from".to_string()),
            Some(torbox_refusal::STALLED.to_string()),
        )
    } else {
        (None, None)
    };
    let progress_percent = snapshot
        .and_then(|snapshot| snapshot.progress)
        .map(|progress| (progress * 100.0).floor().clamp(0.0, 99.0) as u8)
        .unwrap_or(0);
    DownloadQueueItem {
        id: record.id.clone(),
        title_id: Some(record.title_id.clone()).filter(|id| !id.is_empty()),
        episode_id: None,
        title_name: record.title_name.clone(),
        facet: Some(record.facet.clone()).filter(|facet| !facet.is_empty()),
        category: record.category.clone(),
        client_id: String::new(),
        client_name: String::new(),
        client_type: CLIENT_TYPE.to_string(),
        state,
        progress_percent,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: snapshot.and_then(|snapshot| snapshot.size),
        remaining_seconds: snapshot
            .and_then(|snapshot| snapshot.eta)
            .filter(|eta| *eta > 0),
        queued_at: Some(record.created_at.to_rfc3339()),
        last_updated_at: Some(record.updated_at.to_rfc3339()),
        attention_required: failed || stalled,
        attention_reason: attention_reason.clone(),
        attention_reason_code,
        download_client_item_id: record.id.clone(),
        download_id: record.download_id.clone(),
        import_status: None,
        import_type: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: true,
        source_provider: None,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: if failed && record.error_reason.is_some() {
            attention_reason.into_iter().collect()
        } else {
            Vec::new()
        },
        tracked_match_type: None,
        seeding: None,
    }
}

/// TorBox seeds its own copy, so the local files are free to move once
/// fetched; only removing the TorBox item ends the seeding. A torrent with
/// no seed goal asked nothing of TorBox and may go once TorBox holds it; a
/// torrent with a goal may go once TorBox stops seeding.
fn seeding_snapshot(
    record: &TorboxJobRecord,
    engine: Option<&HttpJob>,
    snapshot: Option<&RemoteSnapshot>,
    seeding_ended: bool,
) -> DownloadSeedingSnapshot {
    let downloaded = record.phase == TorboxJobPhase::Fetching
        || snapshot.is_some_and(|snapshot| snapshot.downloaded);
    let can_remove = if !record.has_seed_goal() {
        downloaded.then_some(true)
    } else if seeding_ended {
        Some(true)
    } else {
        snapshot
            .filter(|snapshot| snapshot.downloaded)
            .map(|snapshot| !snapshot.state.is_seeding())
    };
    DownloadSeedingSnapshot {
        can_remove,
        can_move_files: engine.map(|job| job.state == HttpJobState::Completed),
        seed_ratio: snapshot.and_then(|snapshot| snapshot.ratio),
        seed_time_seconds: None,
        is_private: snapshot.map(|snapshot| snapshot.private),
        uploaded_bytes: None,
        completed_at: record
            .remote_finished_at
            .map(|at: DateTime<Utc>| at.to_rfc3339()),
        seed_goal_ratio: record.seed_goal_ratio,
        seed_goal_seconds: record.seed_goal_seconds,
        never_remove: false,
    }
}

fn is_engine_not_found(error: &AppError) -> bool {
    error.reason() == Some(crate::error_reasons::ACQUISITION_HTTP_DOWNLOAD_NOT_FOUND)
}

#[async_trait]
impl DownloadClient for TorboxDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        self.submit(request).await
    }

    /// The host removes the fetched files itself; TorBox's copy goes with
    /// the item when the job is removed.
    fn supports_native_data_removal(&self) -> bool {
        false
    }

    /// The records are the client's complete account of its jobs, so a job
    /// missing from them is absent.
    async fn observe_download(
        &self,
        locator: &ClientJobLocator,
        _history_offset: usize,
    ) -> AppResult<DownloadClientObservation> {
        let id = locator.item_id.as_str();
        let mut items = self.listing(|record, _| record.id == id).await?;
        Ok(match items.pop() {
            Some(item) => DownloadClientObservation::Present(Box::new(item)),
            None => DownloadClientObservation::Absent,
        })
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.listing(|record, job| match record.phase {
            TorboxJobPhase::Submitting | TorboxJobPhase::Remote => true,
            TorboxJobPhase::Fetching => job.is_some_and(|job| !job.state.is_terminal()),
            TorboxJobPhase::Failed => false,
        })
        .await
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        let mut items = self.list_queue().await?;
        items.retain(|item| item.title_id.as_deref() == Some(title_id));
        Ok(items)
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let mut items = self
            .listing(|record, job| match record.phase {
                TorboxJobPhase::Failed => true,
                TorboxJobPhase::Fetching => job.is_some_and(|job| job.state.is_terminal()),
                _ => false,
            })
            .await?;
        items.sort_by(|a, b| b.last_updated_at.cmp(&a.last_updated_at));
        Ok(items)
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        self.runtime.ensure_loaded().await?;
        let fetching: HashMap<String, String> = {
            let state = self.runtime.state.lock().await;
            state
                .records
                .iter()
                .filter(|record| record.phase == TorboxJobPhase::Fetching)
                .filter_map(|record| Some((record.engine_job_id.clone()?, record.id.clone())))
                .collect()
        };
        if fetching.is_empty() {
            return Ok(Vec::new());
        }
        let mut jobs: Vec<HttpJob> = self
            .runtime
            .engine
            .jobs_where(|job| {
                job.state == HttpJobState::Completed && fetching.contains_key(&job.id)
            })
            .await?
            .into_iter()
            .map(|(job, _)| job)
            .collect();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.completed_at));
        Ok(jobs
            .iter()
            .filter_map(|job| {
                let mut completed = completed_download_for(job, CLIENT_TYPE)?;
                completed.download_client_item_id = fetching.get(&job.id)?.clone();
                Some(completed)
            })
            .collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let record = self.known_record(id).await?;
        match (
            record.phase,
            record.engine_job_id.as_deref(),
            record.torbox_id,
        ) {
            (TorboxJobPhase::Fetching, Some(engine_id), _) => {
                self.runtime.engine.pause(engine_id).await
            }
            (TorboxJobPhase::Remote, _, Some(torbox_id))
                if record.kind == TorboxItemKind::Usenet =>
            {
                self.runtime
                    .api()
                    .pause_usenet(torbox_id)
                    .await
                    .map_err(TorboxError::into_app_error)
            }
            _ => Err(AppError::validation_refused(
                torbox_refusal::ACTION_UNSUPPORTED,
                "TorBox cannot pause this download at this stage",
            )),
        }
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let record = self.known_record(id).await?;
        match (
            record.phase,
            record.engine_job_id.as_deref(),
            record.torbox_id,
        ) {
            (TorboxJobPhase::Fetching, Some(engine_id), _) => {
                self.runtime.engine.resume(engine_id).await
            }
            (TorboxJobPhase::Remote, _, Some(torbox_id)) => self
                .runtime
                .api()
                .resume(record.kind, torbox_id)
                .await
                .map_err(TorboxError::into_app_error),
            _ => Err(AppError::validation_refused(
                torbox_refusal::ACTION_UNSUPPORTED,
                "TorBox cannot resume this download at this stage",
            )),
        }
    }

    /// Deletes the one TorBox item this job created, by the id recorded for
    /// it, then drops the job. The fetched files stay where they are;
    /// `remove_data` is not honored here.
    async fn delete_queue_item(
        &self,
        id: &str,
        _is_history: bool,
        _remove_data: bool,
    ) -> AppResult<()> {
        let record = self.known_record(id).await?;
        if let Some(torbox_id) = record.torbox_id {
            match self.runtime.api().delete(record.kind, torbox_id).await {
                Ok(()) => {}
                Err(error) if error.is_item_not_found() => {}
                Err(error) => return Err(error.into_app_error()),
            }
        }
        if let Some(engine_id) = record.engine_job_id.as_deref() {
            match self.runtime.engine.remove(engine_id, false).await {
                Ok(()) => {}
                Err(error) if is_engine_not_found(&error) => {}
                Err(error) => return Err(error),
            }
        }
        {
            let mut state = self.runtime.state.lock().await;
            state.records.retain(|other| other.id != id);
            state.snapshots.remove(id);
            state.seeding_ended.remove(id);
        }
        self.runtime.persist().await?;
        self.runtime.wake.notify_one();
        Ok(())
    }

    async fn get_client_status(&self) -> AppResult<scryer_application::DownloadClientStatus> {
        self.runtime.ensure_loaded().await?;
        Ok(scryer_application::DownloadClientStatus {
            version: None,
            is_localhost: Some(true),
            remote_output_roots: vec![self.runtime.download_dir.to_string_lossy().into_owned()],
            removes_completed_downloads: Some(false),
            sorting_mode: None,
            warnings: Vec::new(),
        })
    }

    async fn test_connection(&self) -> AppResult<String> {
        let user = self
            .runtime
            .api()
            .user_me()
            .await
            .map_err(TorboxError::into_app_error)?;
        check_download_dir(&self.runtime.download_dir).await?;
        self.runtime.engine.check_writable().await?;
        Ok(match user.plan {
            Some(plan) => format!("TorBox ({plan:?} plan)"),
            None => "TorBox".to_string(),
        })
    }
}

impl TorboxDownloadClient {
    async fn known_record(&self, id: &str) -> AppResult<TorboxJobRecord> {
        self.runtime.ensure_loaded().await?;
        self.runtime.record(id).await.ok_or_else(|| {
            AppError::not_found_reason(
                torbox_refusal::JOB_NOT_FOUND,
                format!("the TorBox client has no download {id}"),
            )
        })
    }
}

/// The connection test for a configuration that may not be saved yet: the
/// token is accepted and the download directory is usable.
pub(crate) async fn test_unsaved_config(
    config_json: &str,
    http_client: reqwest::Client,
) -> AppResult<()> {
    let settings = TorboxSettings::from_config_json(config_json)?;
    crate::downloads::clients::http::owned::test_http_download_dir(config_json).await?;
    TorboxApi::new(http_client, settings.token())
        .user_me()
        .await
        .map(|_| ())
        .map_err(TorboxError::into_app_error)
}
