//! Scryer's own HTTP downloader (client type `http`).
//!
//! It fetches the direct-file resources an HTTP-family indexer resolved: one
//! playable media file per release plus the wanted subtitle sidecars, each
//! streamed into a `.part` file and renamed into place once its size checks
//! out. Jobs run sequentially within a job and concurrently across jobs, up
//! to the configured limits.
//!
//! The router rebuilds client objects on every call, so the transfers live in
//! a long-lived engine per configured client, kept in a process-wide registry
//! keyed by the configuration id. The engine's queue is a JSON document under
//! `<download_dir>/.scryer-http/`, next to the files it describes, so a
//! restart resumes or re-lists every job without a database migration.
//!
//! This client never deletes anything. Removing a job drops it from the queue
//! and leaves its files in place; partial files from an untrusted resume stay
//! on disk and the next attempt writes a fresh `.part` name.

mod engine;
mod selection;
mod state;
mod transfer;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, ClientJobLocator, DOWNLOAD_ID_PARAMETER, DownloadClient,
    DownloadClientAddRequest, DownloadClientObservation, DownloadGrabResult, DownloadSourceKind,
};
use scryer_domain::{
    CompletedDownload, DownloadClientConfig, DownloadQueueItem, DownloadQueueState, ProxyConfig,
};
use serde_json::Value;
use tokio_util::task::TaskTracker;
use tracing::info;

use self::engine::{HttpEngine, HttpEngineLimits, LiveSnapshot};
use self::state::{HttpFileRole, HttpJob, HttpJobFile, HttpJobState};
use self::transfer::Egress;
use super::{extract_i64_value, parse_download_client_config_json, read_config_string};

pub(crate) const CLIENT_TYPE: &str = "http";

pub(crate) const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 2;
pub(crate) const MAX_CONCURRENT_DOWNLOADS: usize = 8;
pub(crate) const DEFAULT_MAX_CONNECTIONS_PER_HOST: usize = 2;
pub(crate) const MAX_CONNECTIONS_PER_HOST: usize = 8;

/// The parsed `http` client configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpClientSettings {
    pub download_dir: PathBuf,
    pub limits: HttpEngineLimits,
}

impl HttpClientSettings {
    pub(crate) fn from_config_json(raw: &str) -> AppResult<Self> {
        let config = parse_download_client_config_json(raw)?;
        Self::from_config(&config)
    }

    fn from_config(config: &Value) -> AppResult<Self> {
        let download_dir = read_config_string(config, &["download_dir", "downloadDir"])
            .ok_or_else(|| {
                AppError::Validation("the HTTP downloader needs a download directory".into())
            })?;
        let download_dir = PathBuf::from(download_dir);
        if !download_dir.is_absolute() {
            return Err(AppError::Validation(format!(
                "the HTTP downloader's download directory must be an absolute path, not {}",
                download_dir.display()
            )));
        }
        let max_concurrent_downloads = read_bounded(
            config,
            &["max_concurrent_downloads", "maxConcurrentDownloads"],
            DEFAULT_MAX_CONCURRENT_DOWNLOADS,
            MAX_CONCURRENT_DOWNLOADS,
            "max_concurrent_downloads",
        )?;
        let max_connections_per_host = read_bounded(
            config,
            &["max_connections_per_host", "maxConnectionsPerHost"],
            DEFAULT_MAX_CONNECTIONS_PER_HOST,
            MAX_CONNECTIONS_PER_HOST,
            "max_connections_per_host",
        )?;
        let bandwidth_limit_kbps =
            match read_i64(config, &["bandwidth_limit_kbps", "bandwidthLimitKbps"]) {
                None | Some(0) => None,
                Some(value) if value > 0 => Some(value as u64),
                Some(value) => {
                    return Err(AppError::Validation(format!(
                        "bandwidth_limit_kbps must be zero or more, not {value}"
                    )));
                }
            };
        Ok(Self {
            download_dir,
            limits: HttpEngineLimits {
                max_concurrent_downloads,
                max_connections_per_host,
                bandwidth_limit_kbps,
            },
        })
    }
}

fn read_i64(config: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let value = config.get(*key)?;
        extract_i64_value(Some(value)).or_else(|| value.as_str()?.trim().parse().ok())
    })
}

fn read_bounded(
    config: &Value,
    keys: &[&str],
    default_value: usize,
    max: usize,
    label: &str,
) -> AppResult<usize> {
    match read_i64(config, keys) {
        None => Ok(default_value),
        Some(value) if value >= 1 && value as usize <= max => Ok(value as usize),
        Some(value) => Err(AppError::Validation(format!(
            "{label} must be between 1 and {max}, not {value}"
        ))),
    }
}

/// Engines by download client configuration id.
static ENGINES: LazyLock<std::sync::Mutex<HashMap<String, Arc<HttpEngine>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// The workers of engines whose configuration was deleted or stopped being
/// an HTTP downloader, kept only until they stop, so an engine created
/// again under the same id waits for them before it reads their queue.
static RETIRED: LazyLock<std::sync::Mutex<HashMap<String, TaskTracker>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Stop and drop the engine of a configuration that was deleted or is no
/// longer an HTTP downloader. Its workers checkpoint and stop; its queue
/// and files stay on disk.
pub(crate) fn forget_engine(config_id: &str) {
    let forgotten = ENGINES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(config_id);
    let Some(engine) = forgotten else {
        return;
    };
    engine.begin_shutdown();
    info!(
        client_id = config_id,
        "HTTP downloader stopped for a removed configuration"
    );
    let mut retired = RETIRED.lock().unwrap_or_else(|p| p.into_inner());
    retired.retain(|_, workers| !(workers.is_closed() && workers.is_empty()));
    retired.insert(config_id.to_string(), engine.workers());
}

/// Bring a running engine in line with a saved configuration: stop it when
/// the configuration is no longer an HTTP downloader, otherwise apply its
/// enabled flag. A configuration without an engine has nothing running.
pub(crate) fn apply_saved_config(config: &DownloadClientConfig) {
    if !config.client_type.eq_ignore_ascii_case(CLIENT_TYPE) {
        forget_engine(&config.id);
        return;
    }
    let engine = ENGINES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&config.id)
        .cloned();
    if let Some(engine) = engine {
        engine.set_enabled(config.is_enabled);
    }
}

#[cfg(test)]
pub(crate) fn registered_engine(config_id: &str) -> Option<Arc<HttpEngine>> {
    ENGINES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(config_id)
        .cloned()
}

/// The engine for a configuration, created on first use. A changed download
/// directory replaces the engine: the old one stops its workers (they
/// checkpoint first) and the new one reads the queue kept in the new
/// directory. Limits apply in place; the egress client is rebuilt, outside
/// the registry lock, only when the proxy it was built from changed, so its
/// connection pool survives the router's per-call client objects.
fn engine_for(
    config_id: &str,
    settings: &HttpClientSettings,
    proxy_config: Option<&ProxyConfig>,
) -> AppResult<Arc<HttpEngine>> {
    let key = egress_key(proxy_config)?;
    let current = ENGINES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(config_id)
        .filter(|engine| engine.download_dir == settings.download_dir)
        .cloned();
    if let Some(engine) = current
        && engine.has_egress_key(&key)
    {
        engine.set_limits(settings.limits);
        return Ok(engine);
    }
    let egress = egress_for(proxy_config)?;
    let mut engines = ENGINES.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(engine) = engines.get(config_id)
        && engine.download_dir == settings.download_dir
    {
        engine.set_limits(settings.limits);
        engine.set_egress(egress, key);
        return Ok(Arc::clone(engine));
    }
    let predecessor = match engines.remove(config_id) {
        Some(predecessor) => {
            predecessor.begin_shutdown();
            Some(predecessor.workers())
        }
        None => RETIRED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(config_id),
    };
    let engine = HttpEngine::new(
        config_id.to_string(),
        settings.download_dir.clone(),
        settings.limits,
        egress,
        key,
        predecessor,
    );
    engines.insert(config_id.to_string(), Arc::clone(&engine));
    Ok(engine)
}

/// The native HTTP downloader for one configuration.
pub struct HttpDownloadClient {
    engine: Arc<HttpEngine>,
}

impl HttpDownloadClient {
    pub(crate) fn for_config(
        config: &DownloadClientConfig,
        proxy_config: Option<&ProxyConfig>,
        // The router's client for this configuration follows redirects
        // itself, which would hide them from the transfer's own redirect
        // checks; the proxied egress builds its own client instead.
        _http_client: reqwest::Client,
    ) -> AppResult<Self> {
        let settings = HttpClientSettings::from_config_json(&config.config_json)?;
        let engine = engine_for(&config.id, &settings, proxy_config)?;
        engine.set_enabled(config.is_enabled);
        Ok(Self { engine })
    }

    #[cfg(test)]
    pub(crate) fn engine(&self) -> &Arc<HttpEngine> {
        &self.engine
    }

    fn queue_item(&self, job: &HttpJob, live: Option<LiveSnapshot>) -> DownloadQueueItem {
        job_to_queue_item(job, live)
    }
}

/// What an egress client is built from: nothing for the direct path, else
/// the proxy's revision and the endpoint it egresses through. A restarted
/// tunnel front changes the endpoint even when the settings did not.
fn egress_key(proxy_config: Option<&ProxyConfig>) -> AppResult<String> {
    let Some(proxy) = proxy_config.filter(|proxy| !proxy.is_challenge_solver()) else {
        return Ok("direct".to_string());
    };
    let endpoint =
        scryer_application::transport_proxy::proxy_egress_url(proxy).map_err(|message| {
            scryer_application::transport_proxy::record_transport_proxy_failure(proxy, &message);
            AppError::Validation(message)
        })?;
    Ok(format!(
        "{}|{endpoint}",
        scryer_application::transport_proxy::transport_proxy_revision(proxy)
    ))
}

/// A proxy that carries traffic gets a proxied client; otherwise every hop
/// is checked and pinned by the direct egress path.
///
/// The proxied client follows no redirects: the transfer follows them one
/// hop at a time, so the hop limit, the https downgrade refusal and the
/// origin scoping of the resource's headers hold through a proxy too.
fn egress_for(proxy_config: Option<&ProxyConfig>) -> AppResult<Egress> {
    let Some(proxy) = proxy_config.filter(|proxy| !proxy.is_challenge_solver()) else {
        return Ok(Egress::Direct);
    };
    scryer_application::transport_proxy::transport_proxied_reqwest_client_with_options(
        proxy,
        "",
        reqwest::redirect::Policy::none(),
        false,
    )
    .map(Egress::Proxied)
    .map_err(|message| {
        scryer_application::transport_proxy::record_transport_proxy_failure(proxy, &message);
        AppError::Validation(message)
    })
}

/// The connection test for a configuration that may not be saved yet: the
/// directory exists, is a directory, and accepts a file. Writability is
/// proven by creating (or reopening, without truncating) one fixed empty
/// marker file in the client's state directory, so repeated tests leave
/// exactly one file behind and nothing ever has to be removed.
pub(crate) async fn test_http_download_dir(config_json: &str) -> AppResult<()> {
    let settings = HttpClientSettings::from_config_json(config_json)?;
    check_download_dir(&settings.download_dir).await?;
    let state_dir = settings.download_dir.join(state::STATE_DIR_NAME);
    tokio::fs::create_dir_all(&state_dir)
        .await
        .map_err(|error| {
            AppError::Validation(format!(
                "the download directory {} is not writable: {error}",
                settings.download_dir.display()
            ))
        })?;
    let marker = state_dir.join(WRITE_CHECK_FILE_NAME);
    tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&marker)
        .await
        .map(|_| ())
        .map_err(|error| {
            AppError::Validation(format!(
                "the download directory {} is not writable: {error}",
                settings.download_dir.display()
            ))
        })
}

const WRITE_CHECK_FILE_NAME: &str = "write-check";

async fn check_download_dir(dir: &Path) -> AppResult<()> {
    let metadata = tokio::fs::metadata(dir).await.map_err(|error| {
        AppError::Validation(format!(
            "the download directory {} is not reachable: {error}",
            dir.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(AppError::Validation(format!(
            "the download directory {} is not a directory",
            dir.display()
        )));
    }
    Ok(())
}

#[async_trait]
impl DownloadClient for HttpDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        if let Some(kind) = request.source_kind
            && kind != DownloadSourceKind::HttpResource
        {
            return Err(AppError::Validation(format!(
                "the HTTP downloader only accepts HTTP resources, not {kind:?}"
            )));
        }
        let source = request.http.as_ref().ok_or_else(|| {
            AppError::Validation(
                "the HTTP downloader needs the release's HTTP resources; none were sent".into(),
            )
        })?;
        check_download_dir(&self.engine.download_dir).await?;
        let selection::PlannedDownload {
            output_dir,
            files: planned,
            mut media_fallbacks,
        } = selection::plan_download(source, &self.engine.download_dir)?;
        if let Some(file) = planned.iter().find(|file| file.resource.auth_ref.is_some()) {
            return Err(AppError::Validation(format!(
                "unsupported_auth: {} needs the provider's own sign-in, which the HTTP downloader cannot supply",
                file.final_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )));
        }
        for file in &planned {
            if tokio::fs::try_exists(&file.final_path)
                .await
                .unwrap_or(false)
            {
                return Err(AppError::Validation(format!(
                    "{} already exists; the HTTP downloader does not overwrite files",
                    file.final_path.display()
                )));
            }
        }

        // A fallback that needs the provider's sign-in could only fail later.
        media_fallbacks.retain(|resource| resource.auth_ref.is_none());
        let geo_restricted =
            source.release.availability == scryer_plugin_sdk::HttpAvailability::GeoRestricted;

        let now = Utc::now();
        let item_id = uuid::Uuid::new_v4().to_string();
        let facet = request
            .search_facet
            .clone()
            .unwrap_or_else(|| request.title.facet.clone());
        let release_name = request
            .release_title_without_password()
            .or_else(|| request.source_title_without_password())
            .unwrap_or_else(|| source.policy.output_basename.clone());
        let job = HttpJob {
            id: item_id.clone(),
            // The wire form is what the grab records as this job's identity
            // and what the tracker and import resolve the job by.
            download_id: request
                .download_id
                .as_ref()
                .map(scryer_domain::download_identity::DownloadId::to_wire),
            title_id: request.title.id.clone(),
            title_name: request.title.name.clone(),
            facet: facet.as_str().to_string(),
            release_name,
            category: request
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            output_basename: source.policy.output_basename.trim().to_string(),
            output_dir,
            files: planned
                .into_iter()
                .map(|file| HttpJobFile {
                    role: file.role,
                    resource: file.resource,
                    final_path: file.final_path,
                    attempt: 1,
                    bytes_done: 0,
                    total_bytes: None,
                    validator: None,
                    complete: false,
                    language: file.language,
                    forbidden_answers: 0,
                    skipped_error: None,
                })
                .collect(),
            state: HttpJobState::Queued,
            error: None,
            expected_size_bytes: source.policy.expected_size_bytes,
            expected_duration_seconds: source.policy.expected_duration_seconds,
            verify: source.policy.verify.clone(),
            queued_at: now,
            updated_at: now,
            completed_at: None,
            media_fallbacks,
            candidate_index: 0,
            forbidden_requests: 0,
            refused_variants: Vec::new(),
            geo_restricted,
            subtitle_required: source.selected.subtitle_mode
                == scryer_plugin_sdk::HttpSubtitleMode::SidecarRequired,
            progress_revision: 0,
        };
        self.engine.add_job(job).await?;
        if geo_restricted {
            // Only a hint: the operator may well be inside the region.
            info!(
                item_id = item_id.as_str(),
                provider = source.release.provider.as_str(),
                "release is declared geo-restricted; attempting anyway"
            );
        }
        Ok(DownloadGrabResult {
            job_id: item_id,
            client_id: None,
            client_type: CLIENT_TYPE.to_string(),
            info_hash: None,
            download_id: request.download_id,
            seed_goals: None,
            grabbed_file_names: Vec::new(),
        })
    }

    fn supports_native_data_removal(&self) -> bool {
        false
    }

    /// The queue document is the client's complete record, so a job missing
    /// from it is absent.
    async fn observe_download(
        &self,
        locator: &ClientJobLocator,
        _history_offset: usize,
    ) -> AppResult<DownloadClientObservation> {
        let found = self.engine.job_with_live(&locator.item_id).await?;
        Ok(match found {
            Some((job, live)) => {
                DownloadClientObservation::Present(Box::new(self.queue_item(&job, live)))
            }
            None => DownloadClientObservation::Absent,
        })
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Ok(self
            .engine
            .jobs_where(|job| !job.state.is_terminal())
            .await?
            .into_iter()
            .map(|(job, live)| self.queue_item(&job, live))
            .collect())
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        let mut items = self.list_queue().await?;
        items.retain(|item| item.title_id.as_deref() == Some(title_id));
        Ok(items)
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let mut jobs: Vec<HttpJob> = self
            .engine
            .jobs_where(|job| job.state.is_terminal())
            .await?
            .into_iter()
            .map(|(job, _)| job)
            .collect();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.updated_at));
        Ok(jobs.iter().map(|job| self.queue_item(job, None)).collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let mut jobs: Vec<HttpJob> = self
            .engine
            .jobs_where(|job| job.state == HttpJobState::Completed)
            .await?
            .into_iter()
            .map(|(job, _)| job)
            .collect();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.completed_at));
        Ok(jobs.iter().filter_map(job_to_completed_download).collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        self.engine.pause(id).await
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        self.engine.resume(id).await
    }

    /// Drops the job from the queue. `remove_data` is not honored: this
    /// client leaves every file it wrote where it is.
    async fn delete_queue_item(
        &self,
        id: &str,
        _is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.engine.remove(id, remove_data).await
    }

    async fn get_client_status(&self) -> AppResult<scryer_application::DownloadClientStatus> {
        self.engine.ensure_started().await?;
        Ok(scryer_application::DownloadClientStatus {
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            is_localhost: Some(true),
            remote_output_roots: vec![self.engine.download_dir.to_string_lossy().into_owned()],
            removes_completed_downloads: Some(false),
            sorting_mode: None,
            warnings: Vec::new(),
        })
    }

    async fn test_connection(&self) -> AppResult<String> {
        check_download_dir(&self.engine.download_dir).await?;
        self.engine.check_writable().await?;
        Ok(format!(
            "Scryer HTTP downloader {}",
            env!("CARGO_PKG_VERSION")
        ))
    }
}

fn map_state(state: HttpJobState) -> DownloadQueueState {
    match state {
        HttpJobState::Queued => DownloadQueueState::Queued,
        HttpJobState::Downloading => DownloadQueueState::Downloading,
        HttpJobState::Paused => DownloadQueueState::Paused,
        HttpJobState::Completed => DownloadQueueState::Completed,
        HttpJobState::Failed => DownloadQueueState::Failed,
    }
}

fn job_to_queue_item(job: &HttpJob, live: Option<LiveSnapshot>) -> DownloadQueueItem {
    let state = map_state(job.state);
    let total = job.known_total_bytes();
    let done = live
        .map(|live| live.bytes_done)
        .unwrap_or_else(|| job.durable_bytes());
    let progress_percent = match (state, total) {
        (DownloadQueueState::Completed, _) => 100,
        (_, Some(total)) if total > 0 => ((done.min(total) as f64 / total as f64) * 100.0)
            .floor()
            .clamp(0.0, 99.0) as u8,
        _ => 0,
    };
    let remaining_seconds = match (live.and_then(|live| live.bytes_per_second), total) {
        (Some(speed), Some(total)) if speed > 0.0 => {
            Some((total.saturating_sub(done) as f64 / speed).ceil() as i64)
        }
        _ => None,
    };
    let failed = job.state == HttpJobState::Failed;
    DownloadQueueItem {
        id: job.id.clone(),
        title_id: Some(job.title_id.clone()).filter(|id| !id.is_empty()),
        episode_id: None,
        title_name: job.title_name.clone(),
        facet: Some(job.facet.clone()).filter(|facet| !facet.is_empty()),
        category: job.category.clone(),
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
        size_bytes: total.and_then(|total| i64::try_from(total).ok()),
        remaining_seconds,
        queued_at: Some(job.queued_at.to_rfc3339()),
        last_updated_at: Some(job.updated_at.to_rfc3339()),
        attention_required: failed,
        attention_reason: if failed { job.error.clone() } else { None },
        download_client_item_id: job.id.clone(),
        download_id: job.download_id.clone(),
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
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
        seeding: None,
    }
}

fn job_to_completed_download(job: &HttpJob) -> Option<CompletedDownload> {
    let media = job.media_file()?;
    let mut parameters = vec![
        ("*scryer_title_id".to_string(), job.title_id.clone()),
        ("*scryer_facet".to_string(), job.facet.clone()),
    ];
    if let Some(download_id) = &job.download_id {
        parameters.push((DOWNLOAD_ID_PARAMETER.to_string(), download_id.clone()));
    }
    let size = job
        .files
        .iter()
        .map(|file| file.total_bytes.unwrap_or(file.bytes_done))
        .sum::<u64>();
    Some(CompletedDownload {
        client_type: CLIENT_TYPE.to_string(),
        client_id: String::new(),
        download_client_item_id: job.id.clone(),
        download_id: job.download_id.clone(),
        name: job.output_basename.clone(),
        release_name: Some(job.release_name.clone()),
        dest_dir: media.final_path.to_string_lossy().into_owned(),
        category: job.category.clone(),
        size_bytes: i64::try_from(size).ok(),
        completed_at: job.completed_at,
        parameters,
        posted_file_names: Vec::new(),
        subtitle_paths: job
            .files
            .iter()
            .filter(|file| file.role == HttpFileRole::Subtitle && file.complete)
            .map(|file| file.final_path.to_string_lossy().into_owned())
            .collect(),
    })
}
