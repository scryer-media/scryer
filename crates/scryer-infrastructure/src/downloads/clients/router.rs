use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Component, Path};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use scryer_application::challenge_solver as solver;
use scryer_application::{
    AppError, AppResult, DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE, DownloadClient,
    DownloadClientAddRequest, DownloadClientConfigRepository, DownloadClientPluginProvider,
    DownloadClientRemotePathMapping, DownloadClientStatus, DownloadGrabResult, DownloadSourceKind,
    IndexerConfigRepository, IndexerProxyConfigRepository, RateLimitCooldownAction,
    ResolvedDownloadArtifact, SettingsRepository, StagedNzbRef, StagedNzbStore,
    accepted_inputs_for_client, apply_remote_path_mappings_to_completed_download,
    apply_remote_path_mappings_to_status, parse_download_client_remote_path_mappings,
};
use scryer_domain::{DownloadClientConfig, DownloadQueueItem, IndexerProxyConfig, MediaFacet};
use scryer_outbound_http::{
    OutboundHttpClient, RateLimitRegistry, generic_reqwest_client, prepare_plugin_http_target,
    send_reqwest_request,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, warn};

use super::nzbget::NzbgetDownloadClient;
use super::sabnzbd::SabnzbdDownloadClient;
use super::weaver::WeaverDownloadClient;
use super::{
    parse_download_client_config_json, read_config_string, request_source_hint_for_nzb,
    resolve_download_client_base_url, stage_nzb_from_bytes, stage_nzb_from_url,
};

const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
const DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS: u64 = 10;
const PROXIED_TORRENT_FILE_MAX_BYTES: usize = 32 * 1024 * 1024;
const SOLVER_RESPONSE_MAX_BYTES: usize = PROXIED_TORRENT_FILE_MAX_BYTES * 2;

#[derive(Debug)]
enum BoundedResponseBodyError {
    Read(reqwest::Error),
    TooLarge,
}

async fn read_response_body_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, BoundedResponseBodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(BoundedResponseBodyError::TooLarge);
    }

    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(max_bytes);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(BoundedResponseBodyError::Read)?
    {
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or(BoundedResponseBodyError::TooLarge)?;
        if next_len > max_bytes {
            return Err(BoundedResponseBodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn content_disposition_filename(headers: Option<&serde_json::Value>) -> Option<String> {
    let value = solver::solution_header_string(headers, "content-disposition")?;
    value.split(';').find_map(|part| {
        let part = part.trim();
        let filename = part.strip_prefix("filename=")?;
        let filename = filename.trim_matches('"').trim();
        let safe = filename
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' '))
            .collect::<String>();
        (!safe.trim().is_empty()).then_some(safe)
    })
}

fn looks_like_torrent_metainfo(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > PROXIED_TORRENT_FILE_MAX_BYTES {
        return false;
    }
    matches!(
        parse_bencode_dict(bytes, 0, 0),
        Ok((consumed, true)) if consumed == bytes.len()
    )
}

fn parse_bencode_value(bytes: &[u8], offset: usize, depth: usize) -> Result<usize, ()> {
    if depth > 64 || offset >= bytes.len() {
        return Err(());
    }
    match bytes[offset] {
        b'i' => {
            let end = bytes[offset + 1..]
                .iter()
                .position(|byte| *byte == b'e')
                .map(|position| offset + 1 + position)
                .ok_or(())?;
            if end == offset + 1 {
                return Err(());
            }
            Ok(end + 1)
        }
        b'l' => {
            let mut cursor = offset + 1;
            while cursor < bytes.len() && bytes[cursor] != b'e' {
                cursor = parse_bencode_value(bytes, cursor, depth + 1)?;
            }
            if cursor >= bytes.len() {
                return Err(());
            }
            Ok(cursor + 1)
        }
        b'd' => parse_bencode_dict(bytes, offset, depth + 1).map(|(cursor, _)| cursor),
        b'0'..=b'9' => parse_bencode_string(bytes, offset).map(|(cursor, _, _)| cursor),
        _ => Err(()),
    }
}

fn parse_bencode_dict(bytes: &[u8], offset: usize, depth: usize) -> Result<(usize, bool), ()> {
    if depth > 64 || bytes.get(offset) != Some(&b'd') {
        return Err(());
    }
    let mut cursor = offset + 1;
    let mut has_info_dict = false;
    while cursor < bytes.len() && bytes[cursor] != b'e' {
        let (after_key, key_start, key_end) = parse_bencode_string(bytes, cursor)?;
        let is_top_level_info = depth == 0 && &bytes[key_start..key_end] == b"info";
        if is_top_level_info && bytes.get(after_key) != Some(&b'd') {
            return Err(());
        }
        cursor = parse_bencode_value(bytes, after_key, depth + 1)?;
        has_info_dict |= is_top_level_info;
    }
    if cursor >= bytes.len() {
        return Err(());
    }
    Ok((cursor + 1, has_info_dict))
}

fn parse_bencode_string(bytes: &[u8], offset: usize) -> Result<(usize, usize, usize), ()> {
    let colon = bytes[offset..]
        .iter()
        .position(|byte| *byte == b':')
        .map(|position| offset + position)
        .ok_or(())?;
    if colon == offset {
        return Err(());
    }
    let length = std::str::from_utf8(&bytes[offset..colon])
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or(())?;
    let start = colon + 1;
    let end = start.checked_add(length).ok_or(())?;
    if end > bytes.len() {
        return Err(());
    }
    Ok((end, start, end))
}

fn looks_like_nzb(bytes: &[u8]) -> bool {
    let preview = &bytes[..bytes.len().min(4096)];
    let Ok(text) = std::str::from_utf8(preview) else {
        return false;
    };
    let text = text.trim_start().to_ascii_lowercase();
    text.starts_with("<?xml") && text.contains("<nzb")
        || text.starts_with("<nzb")
        || text.contains("<!doctype nzb")
}

fn looks_like_rejected_download_document(bytes: &[u8]) -> bool {
    let preview = &bytes[..bytes.len().min(256 * 1024)];
    let Ok(text) = std::str::from_utf8(preview) else {
        return false;
    };
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.contains("cf-chl")
        || lower.contains("checking your browser")
        || lower.contains("just a moment")
        || lower.contains("captcha")
        || lower.contains("<form") && lower.contains("login")
        || lower.starts_with("<error")
        || lower.contains("<error ")
}

fn magnet_info_hash_hint(uri: &str) -> Option<String> {
    uri.split(['?', '&'])
        .find_map(|part| part.strip_prefix("xt=urn:btih:"))
        .map(str::to_string)
        .and_then(|value| scryer_application::normalize_torrent_info_hash(Some(&value)))
}

fn target_rate_limit_error(headers: Option<&serde_json::Value>) -> AppError {
    let retry_after = solver::retry_after_from_solution_headers(headers);
    AppError::TemporaryUnavailable {
        message: solver::rate_limit_message_with_retry_after(retry_after),
        retry_after,
        rate_limit_cooldown: RateLimitCooldownAction::RecordFallback,
    }
}

struct FetchedDownloadArtifact {
    bytes: Vec<u8>,
    headers: Option<serde_json::Value>,
    final_url: Option<String>,
}

const DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_INITIAL_SECS: u64 = 15;
const DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_MAX_SECS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DownloadFeedbackReadKind {
    Queue,
    TitleQueue,
    History,
    RecentActivity,
    TitleRecentActivity,
    RecentCompletedDownloads,
}

#[derive(Clone, Copy, Debug)]
struct FeedbackReadBackoffState {
    consecutive_failures: u32,
    blocked_until: Instant,
}

fn download_client_remote_path_mappings(
    config: &DownloadClientConfig,
) -> Option<Vec<DownloadClientRemotePathMapping>> {
    match parse_download_client_remote_path_mappings(&config.config_json) {
        Ok(mappings) => Some(mappings),
        Err(error) => {
            warn!(
                client_id = %config.id,
                client = %config.name,
                error = %error,
                "failed to parse remote path mappings for download client"
            );
            None
        }
    }
}

fn normalize_completed_download_import_dir(item: &mut scryer_domain::CompletedDownload) {
    let reported_dir = Path::new(item.dest_dir.trim());
    if !reported_dir.is_dir() {
        return;
    }

    let name = item.name.trim();
    if name.is_empty() || name.contains(['/', '\\']) {
        return;
    }
    let mut components = Path::new(name).components();
    let Some(Component::Normal(child_name)) = components.next() else {
        return;
    };
    if components.next().is_some() || reported_dir.file_name() == Some(child_name) {
        return;
    }

    let release_dir = reported_dir.join(child_name);
    if !release_dir.is_dir() {
        return;
    }

    tracing::debug!(
        client_id = %item.client_id,
        client_type = %item.client_type,
        reported_dir = %reported_dir.display(),
        release_dir = %release_dir.display(),
        "resolved completed download release directory from client-reported parent"
    );
    item.dest_dir = release_dir.to_string_lossy().into_owned();
}

#[derive(Clone)]
pub struct PrioritizedDownloadClientRouter {
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    indexer_configs: Option<Arc<dyn IndexerConfigRepository>>,
    indexer_proxy_configs: Option<Arc<dyn IndexerProxyConfigRepository>>,
    settings: Arc<dyn SettingsRepository>,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
    plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    outbound_http: OutboundHttpClient,
    feedback_read_timeout: Duration,
    feedback_read_backoff:
        Arc<Mutex<HashMap<(String, DownloadFeedbackReadKind), FeedbackReadBackoffState>>>,
}

#[derive(Clone)]
struct FeedbackTimeoutDownloadClient {
    inner: Arc<dyn DownloadClient>,
    read_timeout: Duration,
}

impl FeedbackTimeoutDownloadClient {
    fn new(inner: Arc<dyn DownloadClient>, read_timeout: Duration) -> Self {
        Self {
            inner,
            read_timeout,
        }
    }

    async fn run_feedback_read<T, F>(&self, future: F) -> AppResult<T>
    where
        F: Future<Output = AppResult<T>> + Send,
        T: Send,
    {
        timeout(self.read_timeout, future).await.map_err(|_| {
            AppError::DownloadFeedbackTimeout(DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE.to_string())
        })?
    }
}

#[async_trait]
impl DownloadClient for FeedbackTimeoutDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        self.inner.submit_download(request).await
    }

    async fn submit_to_download_queue(
        &self,
        title: &scryer_domain::Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> AppResult<DownloadGrabResult> {
        self.inner
            .submit_to_download_queue(
                title,
                source_hint,
                source_kind,
                source_title,
                source_password,
                category,
            )
            .await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_queue()).await
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_queue_for_title(title_id))
            .await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_history()).await
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_history_page(offset, limit))
            .await
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_recent_activity(limit))
            .await
    }

    async fn list_recent_activity_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_recent_activity_for_title(title_id, limit))
            .await
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.inner.list_completed_downloads().await
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.inner.list_recent_completed_downloads(limit).await
    }

    async fn list_queue_excluding_client_types(
        &self,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(
            self.inner
                .list_queue_excluding_client_types(excluded_client_types),
        )
        .await
    }

    async fn list_recent_activity_excluding_client_types(
        &self,
        limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(
            self.inner
                .list_recent_activity_excluding_client_types(limit, excluded_client_types),
        )
        .await
    }

    async fn list_recent_activity_for_client_types(
        &self,
        limit: usize,
        client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(
            self.inner
                .list_recent_activity_for_client_types(limit, client_types),
        )
        .await
    }

    async fn list_recent_completed_downloads_excluding_client_types(
        &self,
        limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.inner
            .list_recent_completed_downloads_excluding_client_types(limit, excluded_client_types)
            .await
    }

    async fn list_recent_completed_downloads_for_client_scope(
        &self,
        limit: usize,
        client_ids: &[String],
        client_types: &[String],
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.inner
            .list_recent_completed_downloads_for_client_scope(
                limit,
                client_ids,
                client_types,
                excluded_client_types,
            )
            .await
    }

    async fn get_completed_download_for_source(
        &self,
        client_id: &str,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<scryer_domain::CompletedDownload>> {
        self.run_feedback_read(self.inner.get_completed_download_for_source(
            client_id,
            client_type,
            download_client_item_id,
        ))
        .await
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        self.inner.pause_queue_item(id).await
    }

    async fn pause_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        self.inner.pause_queue_item_for_client(client_id, id).await
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        self.inner.resume_queue_item(id).await
    }

    async fn resume_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        self.inner.resume_queue_item_for_client(client_id, id).await
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        self.inner.delete_queue_item(id, is_history).await
    }

    async fn delete_queue_item_for_client_id(
        &self,
        client_id: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        self.inner
            .delete_queue_item_for_client_id(client_id, id, is_history)
            .await
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        self.inner
            .delete_queue_item_for_client(client_type, id, is_history)
            .await
    }

    async fn mark_imported(
        &self,
        request: &scryer_application::DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        self.inner.mark_imported(request).await
    }

    async fn get_client_status(&self) -> AppResult<scryer_application::DownloadClientStatus> {
        self.inner.get_client_status().await
    }

    async fn get_client_status_for_client_id(
        &self,
        client_id: &str,
    ) -> AppResult<scryer_application::DownloadClientStatus> {
        self.inner.get_client_status_for_client_id(client_id).await
    }

    async fn test_connection(&self) -> AppResult<String> {
        self.inner.test_connection().await
    }
}

#[derive(Default)]
struct FeedbackReadSummary {
    successful_clients: usize,
    timed_out_clients: usize,
}

impl FeedbackReadSummary {
    fn record_success(&mut self) {
        self.successful_clients += 1;
    }

    fn record_error(&mut self, error: &AppError) {
        if matches!(error, AppError::DownloadFeedbackTimeout(_)) {
            self.timed_out_clients += 1;
        }
    }

    fn finish(self) -> AppResult<()> {
        if self.successful_clients == 0 && self.timed_out_clients > 0 {
            return Err(AppError::DownloadFeedbackTimeout(
                DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE.to_string(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DownloadArtifactKind {
    NzbBytes,
    MagnetUri,
    TorrentBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DownloadClientRoutingScope {
    Library,
    Facet,
}

struct FacetClientSelection {
    clients: Vec<DownloadClientConfig>,
    disabled_scope: Option<DownloadClientRoutingScope>,
}

struct ResolvedDownloadClientRouting {
    scope: DownloadClientRoutingScope,
    routing_object: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DownloadClientRoutingEntry {
    enabled: bool,
    category: Option<String>,
    recent_queue_priority: Option<String>,
    older_queue_priority: Option<String>,
    remove_completed: bool,
    remove_failed: bool,
}

impl PrioritizedDownloadClientRouter {
    pub fn new(
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        settings: Arc<dyn SettingsRepository>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
        plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    ) -> Self {
        Self::with_feedback_read_timeout(
            download_client_configs,
            settings,
            staged_nzb_store,
            staged_nzb_pipeline_limit,
            plugin_provider,
            Duration::from_secs(DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS),
        )
    }

    fn with_feedback_read_timeout(
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        settings: Arc<dyn SettingsRepository>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
        plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
        feedback_read_timeout: Duration,
    ) -> Self {
        let http_client = generic_reqwest_client();
        Self {
            download_client_configs,
            indexer_configs: None,
            indexer_proxy_configs: None,
            settings,
            staged_nzb_store,
            staged_nzb_pipeline_limit,
            plugin_provider,
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            feedback_read_timeout,
            feedback_read_backoff: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_indexer_config_repositories(
        mut self,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_proxy_configs: Arc<dyn IndexerProxyConfigRepository>,
    ) -> Self {
        self.indexer_configs = Some(indexer_configs);
        self.indexer_proxy_configs = Some(indexer_proxy_configs);
        self
    }

    fn wrap_feedback_client(
        client: Arc<dyn DownloadClient>,
        feedback_read_timeout: Duration,
    ) -> Arc<dyn DownloadClient> {
        Arc::new(FeedbackTimeoutDownloadClient::new(
            client,
            feedback_read_timeout,
        ))
    }

    fn feedback_read_kind_label(kind: DownloadFeedbackReadKind) -> &'static str {
        match kind {
            DownloadFeedbackReadKind::Queue => "queue",
            DownloadFeedbackReadKind::TitleQueue => "title_queue",
            DownloadFeedbackReadKind::History => "history",
            DownloadFeedbackReadKind::RecentActivity => "recent_activity",
            DownloadFeedbackReadKind::TitleRecentActivity => "title_recent_activity",
            DownloadFeedbackReadKind::RecentCompletedDownloads => "recent_completed_downloads",
        }
    }

    fn feedback_read_bypasses_backoff(kind: DownloadFeedbackReadKind) -> bool {
        matches!(
            kind,
            DownloadFeedbackReadKind::TitleQueue | DownloadFeedbackReadKind::TitleRecentActivity
        )
    }

    fn feedback_backoff_duration(consecutive_failures: u32) -> Duration {
        let mut seconds = DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_INITIAL_SECS;
        for _ in 1..consecutive_failures {
            seconds = seconds
                .saturating_mul(2)
                .min(DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_MAX_SECS);
        }
        Duration::from_secs(seconds)
    }

    fn feedback_backoff_remaining(
        &self,
        client_id: &str,
        kind: DownloadFeedbackReadKind,
    ) -> Option<Duration> {
        if Self::feedback_read_bypasses_backoff(kind) {
            return None;
        }

        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        let key = (client_id.to_string(), kind);
        let now = Instant::now();
        match backoff.get(&key).copied() {
            Some(state) if state.blocked_until > now => {
                Some(state.blocked_until.saturating_duration_since(now))
            }
            Some(_) => {
                backoff.remove(&key);
                None
            }
            None => None,
        }
    }

    fn record_feedback_read_success(&self, client_id: &str, kind: DownloadFeedbackReadKind) {
        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        backoff.remove(&(client_id.to_string(), kind));
    }

    fn record_feedback_read_failure(&self, client_id: &str, kind: DownloadFeedbackReadKind) {
        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        let key = (client_id.to_string(), kind);
        let failures = backoff
            .get(&key)
            .map(|state| state.consecutive_failures.saturating_add(1))
            .unwrap_or(1);
        let delay = Self::feedback_backoff_duration(failures);
        backoff.insert(
            key,
            FeedbackReadBackoffState {
                consecutive_failures: failures,
                blocked_until: Instant::now() + delay,
            },
        );
    }

    async fn list_enabled_clients_by_priority(&self) -> AppResult<Vec<DownloadClientConfig>> {
        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        clients.sort_by_key(|config| config.client_priority);
        Ok(clients)
    }

    async fn list_enabled_clients_by_priority_excluding(
        &self,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadClientConfig>> {
        let mut clients = self.list_enabled_clients_by_priority().await?;
        clients.retain(|config| {
            !excluded_client_types
                .iter()
                .any(|client_type| config.client_type.eq_ignore_ascii_case(client_type.trim()))
        });
        Ok(clients)
    }

    fn request_source_kind(request: &DownloadClientAddRequest) -> Option<DownloadSourceKind> {
        request
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(request.source_hint.as_deref()))
            .or_else(|| {
                request
                    .info_hash_hint
                    .as_ref()
                    .map(|_| DownloadSourceKind::TorrentFile)
            })
    }

    fn source_kind_label(kind: DownloadSourceKind) -> &'static str {
        match kind {
            DownloadSourceKind::NzbFile => "NZB file",
            DownloadSourceKind::NzbUrl => "NZB URL",
            DownloadSourceKind::TorrentFile => "torrent file",
            DownloadSourceKind::MagnetUri => "magnet",
        }
    }

    fn config_accepts_source_kind(
        config: &DownloadClientConfig,
        source_kind: DownloadSourceKind,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> bool {
        let accepted_inputs = accepted_inputs_for_client(&config.client_type, plugin_provider);
        if accepted_inputs.is_empty() {
            return false;
        }
        accepted_inputs.iter().any(|&accepted_kind| {
            // NzbFile and NzbUrl are interchangeable — scryer fetches the URL
            // and sends the file content, so any NZB-capable client handles both.
            match (accepted_kind, source_kind) {
                (DownloadSourceKind::NzbFile, DownloadSourceKind::NzbUrl)
                | (DownloadSourceKind::NzbUrl, DownloadSourceKind::NzbFile) => true,
                _ => accepted_kind == source_kind,
            }
        })
    }

    fn request_artifact_kind(request: &DownloadClientAddRequest) -> Option<DownloadArtifactKind> {
        match request.resolved_download_artifact.as_ref()? {
            ResolvedDownloadArtifact::Nzb { .. } => Some(DownloadArtifactKind::NzbBytes),
            ResolvedDownloadArtifact::Magnet { .. } => Some(DownloadArtifactKind::MagnetUri),
            ResolvedDownloadArtifact::TorrentFile { .. } => {
                Some(DownloadArtifactKind::TorrentBytes)
            }
        }
    }

    fn artifact_kind_label(kind: DownloadArtifactKind) -> &'static str {
        match kind {
            DownloadArtifactKind::NzbBytes => "NZB payload",
            DownloadArtifactKind::MagnetUri => "magnet URI",
            DownloadArtifactKind::TorrentBytes => "torrent file",
        }
    }

    fn config_accepts_artifact_kind(
        config: &DownloadClientConfig,
        artifact_kind: DownloadArtifactKind,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> bool {
        if Self::is_native_nzb_client_type(&config.client_type) {
            return artifact_kind == DownloadArtifactKind::NzbBytes;
        }
        let Some(provider) = plugin_provider else {
            return false;
        };
        let accepted_inputs = provider.accepted_inputs_for_provider(&config.client_type);
        accepted_inputs.iter().any(|input| {
            let input = input.trim().to_ascii_lowercase();
            match artifact_kind {
                DownloadArtifactKind::NzbBytes => matches!(input.as_str(), "nzb" | "nzb_file"),
                DownloadArtifactKind::MagnetUri => {
                    matches!(input.as_str(), "magnet" | "magnet_uri")
                }
                DownloadArtifactKind::TorrentBytes => {
                    matches!(input.as_str(), "torrent_bytes" | "torrent_file")
                }
            }
        })
    }

    fn download_url_matches_indexer_origin(
        indexer: &scryer_domain::IndexerConfig,
        raw: &str,
    ) -> bool {
        let Ok(download_url) = url::Url::parse(raw) else {
            return false;
        };
        if !matches!(download_url.scheme(), "http" | "https") {
            return false;
        }
        let Ok(base_url) = url::Url::parse(&indexer.base_url) else {
            return false;
        };
        if !matches!(base_url.scheme(), "http" | "https")
            || download_url.scheme() != base_url.scheme()
            || download_url.port_or_known_default() != base_url.port_or_known_default()
        {
            return false;
        }
        download_url.host_str().is_some_and(|host| {
            base_url
                .host_str()
                .is_some_and(|base_host| host.eq_ignore_ascii_case(base_host))
        })
    }

    async fn prepare_proxied_download_request(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadClientAddRequest> {
        let Some(indexer_id) = request.indexer_id.as_deref() else {
            return Ok(request.clone());
        };
        let (Some(indexer_configs), Some(indexer_proxy_configs)) =
            (&self.indexer_configs, &self.indexer_proxy_configs)
        else {
            return Ok(request.clone());
        };
        let indexer = indexer_configs
            .get_by_id(indexer_id)
            .await?
            .ok_or_else(|| AppError::Validation("Indexer configuration was not found.".into()))?;
        let Some(proxy_config_id) = indexer.indexer_proxy_config_id.as_deref() else {
            return Ok(request.clone());
        };
        let proxy_config = indexer_proxy_configs
            .get_by_id(proxy_config_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation("Indexer proxy configuration was not found.".into())
            })?;
        if !proxy_config.is_enabled {
            return Err(AppError::Validation(
                "Indexer proxy is disabled for this indexer.".into(),
            ));
        }
        let download_url = request
            .source_hint
            .as_deref()
            .ok_or_else(|| AppError::Validation("Proxied download is missing a URL.".into()))?;
        if download_url.trim_start().starts_with("magnet:?") {
            let uri = download_url.trim().to_string();
            let mut prepared = request.clone();
            prepared.resolved_download_artifact = Some(ResolvedDownloadArtifact::Magnet {
                info_hash_hint: request
                    .info_hash_hint
                    .clone()
                    .or_else(|| magnet_info_hash_hint(&uri)),
                uri: uri.clone(),
            });
            prepared.source_kind = Some(DownloadSourceKind::MagnetUri);
            prepared.source_hint = Some(uri);
            return Ok(prepared);
        }
        if !Self::download_url_matches_indexer_origin(&indexer, download_url) {
            return Err(AppError::Validation(
                "Proxied download URL does not match the assigned indexer origin.".into(),
            ));
        }
        let artifact_result = self
            .resolve_download_artifact_via_indexer_proxy(
                &proxy_config,
                download_url,
                request.info_hash_hint.clone(),
            )
            .await;
        if let Some(repo) = self.indexer_proxy_configs.as_ref() {
            solver::flush_solver_health(repo.as_ref()).await;
        }
        let artifact = artifact_result?;

        let mut prepared = request.clone();
        prepared.resolved_download_artifact = Some(artifact.clone());
        match artifact {
            ResolvedDownloadArtifact::Nzb { .. } => {
                prepared.source_kind = Some(DownloadSourceKind::NzbFile);
                prepared.source_hint = None;
            }
            ResolvedDownloadArtifact::Magnet {
                uri,
                info_hash_hint,
            } => {
                prepared.source_kind = Some(DownloadSourceKind::MagnetUri);
                prepared.source_hint = Some(uri);
                prepared.info_hash_hint = info_hash_hint.or(prepared.info_hash_hint);
            }
            ResolvedDownloadArtifact::TorrentFile { info_hash_hint, .. } => {
                prepared.source_kind = Some(DownloadSourceKind::TorrentFile);
                prepared.source_hint = None;
                prepared.info_hash_hint = info_hash_hint.or(prepared.info_hash_hint);
            }
        }
        Ok(prepared)
    }

    async fn resolve_download_artifact_via_indexer_proxy(
        &self,
        proxy_config: &IndexerProxyConfig,
        download_url: &str,
        info_hash_hint: Option<String>,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let provider = proxy_config.provider_type;
        let provider_name = solver::solver_provider_name(provider);

        // Validate before delegating to the solver as well as before any direct
        // retry. Otherwise the solver itself becomes a deputy for blocked
        // link-local or cloud-metadata destinations.
        drop(
            prepare_plugin_http_target(download_url, "indexer download artifact")
                .await
                .map_err(|error| {
                    warn!(error = %error, "blocked unsafe indexer download artifact URL");
                    AppError::DownloadSubmitUnavailable(
                        "Scryer refused an unsafe download artifact destination.".into(),
                    )
                })?,
        );

        // Keep artifact resolution direct-first, just like indexer searches.
        // A fresh solved session is applied when available; otherwise this is
        // an ordinary guarded fetch. Direct rate limits still propagate and
        // challenge/error responses fall back to a full solve.
        let session_headers =
            solver::SolvedSessionCache::shared().session_headers(&proxy_config.id, download_url);
        let had_solved_session = !session_headers.is_empty();
        match self
            .fetch_download_artifact_direct(
                provider_name,
                download_url,
                &session_headers,
                proxy_config.request_timeout_seconds,
            )
            .await
        {
            Ok(fetched) => {
                match Self::classify_resolved_download_artifact(
                    provider_name,
                    fetched.final_url.as_deref(),
                    fetched.headers.as_ref(),
                    fetched.bytes,
                    info_hash_hint.clone(),
                ) {
                    Ok(artifact) => return Ok(artifact),
                    Err(error) => {
                        debug!(
                            proxy_config_id = proxy_config.id.as_str(),
                            error = %error,
                            had_solved_session,
                            "direct artifact fetch not usable; falling back to solver"
                        );
                        if had_solved_session {
                            solver::SolvedSessionCache::shared()
                                .invalidate(&proxy_config.id, download_url);
                        }
                    }
                }
            }
            Err(error @ AppError::TemporaryUnavailable { .. }) => return Err(error),
            Err(error) => {
                debug!(
                    proxy_config_id = proxy_config.id.as_str(),
                    error = %error,
                    had_solved_session,
                    "direct artifact fetch failed; falling back to solver"
                );
                if had_solved_session {
                    solver::SolvedSessionCache::shared().invalidate(&proxy_config.id, download_url);
                }
            }
        }

        let endpoint = solver::solver_solve_endpoint(&proxy_config.base_url);
        let response = generic_reqwest_client()
            .post(endpoint)
            .timeout(Duration::from_secs(
                proxy_config.request_timeout_seconds as u64 + 5,
            ))
            .json(&solver::solver_solve_request(
                provider,
                download_url,
                proxy_config.request_timeout_seconds,
            ))
            .send()
            .await
            .map_err(|error| {
                let message = if error.is_timeout() {
                    solver::solver_error_message(provider, solver::SolverErrorKind::Timeout)
                } else {
                    solver::solver_error_message(provider, solver::SolverErrorKind::Unreachable)
                };
                solver::SolverHealthLedger::shared().record_failure(&proxy_config.id, message);
                AppError::DownloadSubmitUnavailable(message.into())
            })?;
        let solver_status = response.status();
        if solver_status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || solver_status.is_server_error()
        {
            let message =
                solver::solver_error_message(provider, solver::SolverErrorKind::Unavailable);
            solver::SolverHealthLedger::shared().record_failure(&proxy_config.id, message);
            return Err(AppError::DownloadSubmitUnavailable(message.into()));
        }
        let body = match read_response_body_bounded(response, SOLVER_RESPONSE_MAX_BYTES).await {
            Ok(body) => body,
            Err(BoundedResponseBodyError::Read(error)) => {
                debug!(
                    proxy_config_id = proxy_config.id.as_str(),
                    is_timeout = error.is_timeout(),
                    is_body = error.is_body(),
                    is_decode = error.is_decode(),
                    "failed to read challenge solver response body"
                );
                let message =
                    solver::solver_error_message(provider, solver::SolverErrorKind::Unreadable);
                solver::SolverHealthLedger::shared().record_failure(&proxy_config.id, message);
                return Err(AppError::DownloadSubmitUnavailable(message.into()));
            }
            Err(BoundedResponseBodyError::TooLarge) => {
                let message = format!(
                    "{provider_name} returned a response larger than Scryer's download artifact limit."
                );
                solver::SolverHealthLedger::shared().record_failure(&proxy_config.id, &message);
                return Err(AppError::DownloadSubmitUnavailable(message));
            }
        };
        let solution = solver::parse_solver_solution(&body).map_err(|error| {
            if matches!(
                error,
                solver::ChallengeSolverParseError::Malformed
                    | solver::ChallengeSolverParseError::ServiceError
            ) {
                solver::SolverHealthLedger::shared()
                    .record_failure(&proxy_config.id, error.message(provider));
            }
            AppError::DownloadSubmitUnavailable(error.message(provider).into())
        })?;
        solver::SolverHealthLedger::shared().record_success(&proxy_config.id);
        let solution_status = solution.status.unwrap_or_else(|| solver_status.as_u16());
        if solution_status == reqwest::StatusCode::TOO_MANY_REQUESTS.as_u16() {
            return Err(target_rate_limit_error(solution.headers.as_ref()));
        }
        let solution_body = solution.response.as_deref().unwrap_or_default();
        if solution_body.len() > PROXIED_TORRENT_FILE_MAX_BYTES {
            return Err(AppError::DownloadSubmitUnavailable(format!(
                "The resolved download artifact exceeded Scryer's {} MiB limit.",
                PROXIED_TORRENT_FILE_MAX_BYTES / (1024 * 1024)
            )));
        }
        if solver::solved_body_looks_rate_limited(solution_body.as_bytes()) {
            return Err(target_rate_limit_error(solution.headers.as_ref()));
        }
        let retry_headers = solver::solution_retry_headers(&solution);
        if !(200..300).contains(&solution_status) {
            if retry_headers.is_empty() {
                return Err(AppError::DownloadSubmitUnavailable(format!(
                    "{provider_name} target request returned HTTP {solution_status}."
                )));
            }
            let fetched = self
                .fetch_download_artifact_direct(
                    provider_name,
                    download_url,
                    &retry_headers,
                    proxy_config.request_timeout_seconds,
                )
                .await?;
            solver::SolvedSessionCache::shared().store_solution(
                &proxy_config.id,
                download_url,
                &solution,
            );
            return Self::classify_resolved_download_artifact(
                provider_name,
                fetched.final_url.as_deref(),
                fetched.headers.as_ref(),
                fetched.bytes,
                info_hash_hint,
            );
        }
        solver::SolvedSessionCache::shared().store_solution(
            &proxy_config.id,
            download_url,
            &solution,
        );
        if Self::should_refetch_binary_download_artifact(
            download_url,
            solution.url.as_deref(),
            solution.headers.as_ref(),
        ) && !retry_headers.is_empty()
        {
            let fetched = self
                .fetch_download_artifact_direct(
                    provider_name,
                    download_url,
                    &retry_headers,
                    proxy_config.request_timeout_seconds,
                )
                .await?;
            return Self::classify_resolved_download_artifact(
                provider_name,
                fetched.final_url.as_deref(),
                fetched.headers.as_ref(),
                fetched.bytes,
                info_hash_hint,
            );
        }
        let bytes = solution.response.unwrap_or_default().into_bytes();
        let embedded_artifact = Self::classify_resolved_download_artifact(
            provider_name,
            solution.url.as_deref(),
            solution.headers.as_ref(),
            bytes,
            info_hash_hint.clone(),
        );
        match embedded_artifact {
            Ok(artifact) => Ok(artifact),
            Err(error) if retry_headers.is_empty() => Err(error),
            Err(error) => {
                debug!(
                    proxy_config_id = proxy_config.id.as_str(),
                    error = %error,
                    "embedded solver artifact was not usable; retrying original URL with solved session"
                );
                let fetched = self
                    .fetch_download_artifact_direct(
                        provider_name,
                        download_url,
                        &retry_headers,
                        proxy_config.request_timeout_seconds,
                    )
                    .await?;
                Self::classify_resolved_download_artifact(
                    provider_name,
                    fetched.final_url.as_deref(),
                    fetched.headers.as_ref(),
                    fetched.bytes,
                    info_hash_hint,
                )
            }
        }
    }

    fn should_refetch_binary_download_artifact(
        original_url: &str,
        final_url: Option<&str>,
        headers: Option<&serde_json::Value>,
    ) -> bool {
        let content_type = solver::solution_header_string(headers, "content-type")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if content_type.contains("application/x-bittorrent")
            || content_type.contains("application/octet-stream")
        {
            return true;
        }
        [Some(original_url), final_url]
            .into_iter()
            .flatten()
            .any(|raw| {
                url::Url::parse(raw)
                    .ok()
                    .is_some_and(|url| url.path().to_ascii_lowercase().ends_with(".torrent"))
            })
    }

    async fn fetch_download_artifact_direct(
        &self,
        provider_name: &str,
        download_url: &str,
        session_headers: &[(String, String)],
        request_timeout_seconds: u32,
    ) -> AppResult<FetchedDownloadArtifact> {
        let target = prepare_plugin_http_target(download_url, "indexer download artifact")
            .await
            .map_err(|error| {
                warn!(error = %error, "blocked unsafe indexer download artifact URL");
                AppError::DownloadSubmitUnavailable(
                    "Scryer refused an unsafe download artifact destination.".into(),
                )
            })?;
        let mut builder = target
            .client()
            .get(target.url().clone())
            .timeout(Duration::from_secs(u64::from(
                request_timeout_seconds.saturating_add(5),
            )));
        for (name, value) in session_headers {
            builder = builder.header(name, value);
        }
        let response = send_reqwest_request(builder).await.map_err(|error| {
            debug!(
                proxy_provider = provider_name,
                is_timeout = error.is_timeout(),
                "download artifact fetch failed"
            );
            if error.is_timeout() {
                AppError::DownloadSubmitUnavailable("The download artifact fetch timed out.".into())
            } else {
                AppError::DownloadSubmitUnavailable(
                    "Scryer could not fetch the download artifact.".into(),
                )
            }
        })?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| {
                    scryer_outbound_http::parse_retry_after(value).map(|(delay, _)| delay)
                });
            return Err(AppError::TemporaryUnavailable {
                message: solver::rate_limit_message_with_retry_after(retry_after),
                retry_after,
                rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
            });
        }
        if !response.status().is_success() {
            return Err(AppError::DownloadSubmitUnavailable(format!(
                "The download artifact fetch returned HTTP {}.",
                response.status().as_u16()
            )));
        }
        let final_url = Some(response.url().to_string());
        let mut header_map = serde_json::Map::new();
        for name in ["content-type", "content-disposition"] {
            if let Some(value) = response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
            {
                header_map.insert(name.to_string(), serde_json::Value::from(value));
            }
        }
        let headers = (!header_map.is_empty()).then_some(serde_json::Value::Object(header_map));
        let bytes = match read_response_body_bounded(response, PROXIED_TORRENT_FILE_MAX_BYTES).await
        {
            Ok(bytes) => bytes,
            Err(BoundedResponseBodyError::Read(error)) => {
                debug!(
                    is_timeout = error.is_timeout(),
                    is_body = error.is_body(),
                    is_decode = error.is_decode(),
                    "failed to read proxied download artifact body"
                );
                return Err(AppError::DownloadSubmitUnavailable(
                    "Scryer could not read the download artifact.".into(),
                ));
            }
            Err(BoundedResponseBodyError::TooLarge) => {
                return Err(AppError::DownloadSubmitUnavailable(format!(
                    "The resolved download artifact exceeded Scryer's {} MiB limit.",
                    PROXIED_TORRENT_FILE_MAX_BYTES / (1024 * 1024)
                )));
            }
        };
        Ok(FetchedDownloadArtifact {
            bytes,
            headers,
            final_url,
        })
    }

    fn classify_resolved_download_artifact(
        provider_name: &str,
        final_url: Option<&str>,
        headers: Option<&serde_json::Value>,
        bytes: Vec<u8>,
        info_hash_hint: Option<String>,
    ) -> AppResult<ResolvedDownloadArtifact> {
        if final_url.is_some_and(|url| url.starts_with("magnet:")) {
            let uri = final_url.unwrap().trim().to_string();
            return Ok(ResolvedDownloadArtifact::Magnet {
                info_hash_hint: info_hash_hint.or_else(|| magnet_info_hash_hint(&uri)),
                uri,
            });
        }
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let trimmed = text.trim();
            if trimmed.starts_with("magnet:?") {
                let uri = trimmed.to_string();
                return Ok(ResolvedDownloadArtifact::Magnet {
                    info_hash_hint: info_hash_hint.or_else(|| magnet_info_hash_hint(&uri)),
                    uri,
                });
            }
        }

        let content_type = solver::solution_header_string(headers, "content-type");
        let file_name = content_disposition_filename(headers);
        let final_path = final_url
            .and_then(|value| url::Url::parse(value).ok())
            .map(|url| url.path().to_ascii_lowercase());
        let file_name_lower = file_name.as_ref().map(|value| value.to_ascii_lowercase());
        if looks_like_rejected_download_document(&bytes) {
            return Err(AppError::Validation(format!(
                "{provider_name} did not resolve download artifact."
            )));
        }

        if content_type.as_deref().is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("application/x-bittorrent")
                || value.contains("application/octet-stream") && looks_like_torrent_metainfo(&bytes)
        }) || final_path
            .as_deref()
            .is_some_and(|path| path.ends_with(".torrent"))
            || file_name_lower
                .as_deref()
                .is_some_and(|name| name.ends_with(".torrent"))
        {
            if !looks_like_torrent_metainfo(&bytes) {
                return Err(AppError::Validation(format!(
                    "{provider_name} resolved invalid torrent file bytes."
                )));
            }
            if bytes.len() > PROXIED_TORRENT_FILE_MAX_BYTES {
                return Err(AppError::Validation(format!(
                    "{provider_name} resolved torrent file is too large."
                )));
            }
            return Ok(ResolvedDownloadArtifact::TorrentFile {
                bytes,
                file_name,
                content_type,
                info_hash_hint,
            });
        }

        if content_type.as_deref().is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().split(';').next().unwrap_or(""),
                "application/x-nzb"
            )
        }) || looks_like_nzb(&bytes)
        {
            if !looks_like_nzb(&bytes) {
                return Err(AppError::Validation(format!(
                    "{provider_name} resolved invalid NZB bytes."
                )));
            }
            return Ok(ResolvedDownloadArtifact::Nzb {
                bytes,
                file_name,
                content_type,
            });
        }

        Err(AppError::Validation(format!(
            "{provider_name} resolved the download URL, but the result was not an NZB, magnet URI, or torrent file."
        )))
    }

    fn read_trimmed_string(raw_value: Option<&serde_json::Value>) -> Option<String> {
        raw_value
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn read_bool(raw_value: Option<&serde_json::Value>, default: bool) -> bool {
        match raw_value {
            Some(serde_json::Value::Bool(value)) => *value,
            Some(serde_json::Value::String(value)) => !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no"
            ),
            Some(serde_json::Value::Number(value)) => value.as_i64() != Some(0),
            _ => default,
        }
    }

    fn parse_routing_object(raw_json: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        serde_json::from_str::<serde_json::Value>(raw_json)
            .ok()?
            .as_object()
            .cloned()
    }

    fn parse_routing_entry(config: &serde_json::Value) -> DownloadClientRoutingEntry {
        DownloadClientRoutingEntry {
            enabled: Self::read_bool(config.get("enabled"), true),
            category: Self::read_trimmed_string(config.get("category")),
            recent_queue_priority: Self::read_trimmed_string(
                config
                    .get("recentQueuePriority")
                    .or_else(|| config.get("recentPriority"))
                    .or_else(|| config.get("recent_priority")),
            ),
            older_queue_priority: Self::read_trimmed_string(
                config
                    .get("olderQueuePriority")
                    .or_else(|| config.get("olderPriority"))
                    .or_else(|| config.get("older_priority")),
            ),
            remove_completed: Self::read_bool(
                config
                    .get("removeCompleted")
                    .or_else(|| config.get("remove_completed"))
                    .or_else(|| config.get("removeComplete")),
                false,
            ),
            remove_failed: Self::read_bool(
                config
                    .get("removeFailed")
                    .or_else(|| config.get("remove_failed"))
                    .or_else(|| config.get("removeFailure")),
                false,
            ),
        }
    }

    fn facet_scope_id(facet: &MediaFacet) -> &'static str {
        facet.as_str()
    }

    async fn get_download_client_routing_json(&self, scope_id: &str) -> AppResult<Option<String>> {
        if let Some(routing_json) = self
            .settings
            .get_setting_json(
                "system",
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await?
        {
            return Ok(Some(routing_json));
        }

        self.settings
            .get_setting_json(
                "system",
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await
    }

    async fn get_explicit_download_client_routing_json(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(routing_json) = self
            .settings
            .get_setting_json_explicit(
                "system",
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await?
        {
            return Ok(Some(routing_json));
        }

        self.settings
            .get_setting_json_explicit(
                "system",
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await
    }

    async fn resolve_routing_object_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<Option<ResolvedDownloadClientRouting>> {
        if let Some(raw_json) = self
            .get_explicit_download_client_routing_json(&title.library_id)
            .await?
        {
            if let Some(routing_object) = Self::parse_routing_object(&raw_json) {
                return Ok(Some(ResolvedDownloadClientRouting {
                    scope: DownloadClientRoutingScope::Library,
                    routing_object,
                }));
            }

            warn!(
                library_id = title.library_id.as_str(),
                title = title.name.as_str(),
                "ignoring invalid library-scoped download client routing override"
            );
        }

        let scope_id = Self::facet_scope_id(&title.facet);
        if let Some(raw_json) = self.get_download_client_routing_json(scope_id).await? {
            if let Some(routing_object) = Self::parse_routing_object(&raw_json) {
                return Ok(Some(ResolvedDownloadClientRouting {
                    scope: DownloadClientRoutingScope::Facet,
                    routing_object,
                }));
            }

            warn!(
                facet = ?title.facet,
                title = title.name.as_str(),
                "ignoring invalid facet-scoped download client routing settings"
            );
        }

        Ok(None)
    }

    /// Return enabled clients ordered by effective routing priority for this title.
    /// Falls back to global `client_priority` if no routing config applies.
    async fn list_clients_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<FacetClientSelection> {
        let resolved_routing = self.resolve_routing_object_for_title(title).await?;

        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        let any_globally_enabled = !clients.is_empty();
        let mut disabled_scope = None;

        match resolved_routing {
            Some(resolved_routing) => {
                let ordered_ids: Vec<String> =
                    resolved_routing.routing_object.keys().cloned().collect();
                let missing_client_default_enabled =
                    !matches!(resolved_routing.scope, DownloadClientRoutingScope::Library);

                clients.retain(|client| {
                    resolved_routing
                        .routing_object
                        .get(&client.id)
                        .map(|entry| Self::read_bool(entry.get("enabled"), true))
                        .unwrap_or(missing_client_default_enabled)
                });

                if any_globally_enabled && clients.is_empty() {
                    disabled_scope = Some(resolved_routing.scope);
                }

                if ordered_ids.is_empty() {
                    clients.sort_by_key(|c| c.client_priority);
                } else {
                    clients.sort_by_key(|c| {
                        ordered_ids
                            .iter()
                            .position(|id| id == &c.id)
                            .unwrap_or(usize::MAX)
                    });
                }
            }
            None => {
                clients.sort_by_key(|c| c.client_priority);
            }
        }

        Ok(FacetClientSelection {
            clients,
            disabled_scope,
        })
    }

    async fn routing_entry_for_client(
        &self,
        title: &scryer_domain::Title,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientRoutingEntry>> {
        let Some(resolved_routing) = self.resolve_routing_object_for_title(title).await? else {
            return Ok(None);
        };

        Ok(resolved_routing
            .routing_object
            .get(client_id)
            .map(Self::parse_routing_entry))
    }

    fn normalized_request_category(request: &DownloadClientAddRequest) -> Option<String> {
        request
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    async fn apply_selected_client_routing(
        &self,
        request: &DownloadClientAddRequest,
        client_id: &str,
    ) -> AppResult<DownloadClientAddRequest> {
        let mut effective_request = request.clone();
        let routing_entry = self
            .routing_entry_for_client(&request.title, client_id)
            .await?;

        effective_request.category = routing_entry
            .as_ref()
            .and_then(|entry| entry.category.clone())
            .or_else(|| Self::normalized_request_category(request));

        let is_recent = request.is_recent.unwrap_or(false);
        effective_request.queue_priority = routing_entry.and_then(|entry| {
            if is_recent {
                entry.recent_queue_priority
            } else {
                entry.older_queue_priority
            }
        });

        Ok(effective_request)
    }

    fn is_native_nzb_client_type(client_type: &str) -> bool {
        matches!(client_type, "nzbget" | "sabnzbd" | "weaver")
    }

    fn request_uses_nzb_payload(request: &DownloadClientAddRequest) -> bool {
        matches!(
            Self::request_source_kind(request),
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl)
        )
    }

    async fn delete_staged_nzb(&self, staged_nzb: Option<&StagedNzbRef>, reason: &str) {
        let Some(staged_nzb) = staged_nzb else {
            return;
        };

        if let Err(error) = self.staged_nzb_store.delete_staged_nzb(staged_nzb).await {
            warn!(
                staged_nzb_id = staged_nzb.id.as_str(),
                error = %error,
                reason,
                "failed to delete staged nzb artifact"
            );
        }
    }

    fn client_from_config(
        config: &DownloadClientConfig,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
        feedback_read_timeout: Duration,
    ) -> AppResult<Arc<dyn DownloadClient>> {
        if let Some(provider) = plugin_provider
            && let Some(client) = provider.client_for_config(config)
        {
            return Ok(Self::wrap_feedback_client(client, feedback_read_timeout));
        }

        let client = match config.client_type.as_str() {
            "nzbget" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url =
                    resolve_download_client_base_url(&parsed_config).ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let username = read_config_string(&parsed_config, &["username"]);
                let password = read_config_string(&parsed_config, &["password"]);
                let dupe_mode = read_config_string(&parsed_config, &["dupe_mode", "dupeMode"])
                    .unwrap_or_else(|| "SCORE".to_string());
                let client = NzbgetDownloadClient::with_staged_nzb_store(
                    base_url,
                    username,
                    password,
                    dupe_mode,
                    staged_nzb_store,
                    staged_nzb_pipeline_limit,
                );
                Self::wrap_feedback_client(Arc::new(client), feedback_read_timeout)
            }
            "sabnzbd" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url =
                    resolve_download_client_base_url(&parsed_config).ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let api_key = read_config_string(&parsed_config, &["api_key", "apiKey", "apikey"]);
                let username = read_config_string(&parsed_config, &["username"]);
                let password = read_config_string(&parsed_config, &["password"]);
                if api_key.is_none() && (username.is_none() || password.is_none()) {
                    return Err(AppError::Validation(format!(
                        "download client {} (sabnzbd) requires an API key or username/password",
                        config.id
                    )));
                }
                let client = SabnzbdDownloadClient::with_auth_and_staged_nzb_store(
                    base_url,
                    api_key,
                    username,
                    password,
                    staged_nzb_store,
                    staged_nzb_pipeline_limit,
                );
                Self::wrap_feedback_client(Arc::new(client), feedback_read_timeout)
            }
            "weaver" => {
                let client = WeaverDownloadClient::from_config_with_staged_nzb_store(
                    config,
                    staged_nzb_store,
                    staged_nzb_pipeline_limit,
                )?;
                Self::wrap_feedback_client(Arc::new(client), feedback_read_timeout)
            }
            _ => {
                return Err(AppError::Validation(format!(
                    "unsupported download client type '{}' for config {}",
                    config.client_type, config.id
                )));
            }
        };

        Ok(client)
    }

    async fn resolve_client_for_queue_action(
        &self,
        id: &str,
        is_history: bool,
    ) -> AppResult<Option<Arc<dyn DownloadClient>>> {
        let configs = self.list_enabled_clients_by_priority().await?;
        if configs.is_empty() {
            return Ok(None);
        }

        let mut clients = Vec::new();
        for config in configs {
            match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => clients.push((config, client)),
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped while routing queue action"
                    );
                }
            }
        }

        if clients.is_empty() {
            return Ok(None);
        }

        for (config, client) in &clients {
            let items = if is_history {
                client.list_history().await
            } else {
                client.list_queue().await
            };

            match items {
                Ok(items) => {
                    if items.iter().any(|item| item.download_client_item_id == id) {
                        return Ok(Some(Arc::clone(client)));
                    }
                }
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        queue_item_id = id,
                        history = is_history,
                        error = %error,
                        "failed to inspect download client while routing queue action"
                    );
                }
            }
        }

        if clients.len() == 1 {
            return Ok(Some(Arc::clone(&clients[0].1)));
        }

        Err(AppError::Validation(format!(
            "download client item not found: {id}"
        )))
    }

    async fn resolve_client_for_id(
        &self,
        client_id: &str,
    ) -> AppResult<Option<Arc<dyn DownloadClient>>> {
        let normalized = client_id.trim();
        if normalized.is_empty() {
            return Ok(None);
        }

        let configs = self.list_enabled_clients_by_priority().await?;
        for config in configs {
            if config.id != normalized {
                continue;
            }

            return Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            )
            .map(Some);
        }

        Ok(None)
    }

    async fn resolve_client_for_type(
        &self,
        client_type: &str,
    ) -> AppResult<Option<Arc<dyn DownloadClient>>> {
        let normalized = client_type.trim();
        if normalized.is_empty() {
            return Ok(None);
        }

        let configs = self.list_enabled_clients_by_priority().await?;
        if configs.is_empty() {
            return Ok(None);
        }

        for config in configs {
            if !config.client_type.eq_ignore_ascii_case(normalized) {
                continue;
            }

            return Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            )
            .map(Some);
        }

        Ok(None)
    }
}

#[async_trait]
impl DownloadClient for PrioritizedDownloadClientRouter {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let request = self.prepare_proxied_download_request(request).await?;
        let request = &request;
        // Pillar D1 for NZB bytes Scryer already holds (indexer-proxied
        // artifacts). URL-sourced NZBs are gated as they stream in, inside
        // `stage_nzb_from_url`, so no payload is ever fetched twice.
        if let Some(ResolvedDownloadArtifact::Nzb { bytes, .. }) =
            request.resolved_download_artifact.as_ref()
        {
            let head_len = bytes.len().min(scryer_application::NZB_HEAD_PROBE_BYTES);
            scryer_application::enforce_nzb_category_gate(
                &bytes[..head_len],
                request
                    .search_facet
                    .as_ref()
                    .unwrap_or(&request.title.facet),
            )?;
        }
        let resolved_artifact_kind = Self::request_artifact_kind(request);
        let selection = match self.list_clients_for_title(&request.title).await {
            Ok(configs) => configs,
            Err(error) => {
                warn!(
                    error = %error,
                    title = request.title.name.as_str(),
                    facet = ?request.title.facet,
                    "failed to load prioritized download clients"
                );
                return Err(error.into_download_submit_unavailable());
            }
        };

        if let Some(disabled_scope) = selection.disabled_scope {
            let message = match disabled_scope {
                DownloadClientRoutingScope::Library => format!(
                    "no download client enabled for library {}",
                    request.title.library_id
                ),
                DownloadClientRoutingScope::Facet => {
                    "no download client enabled for this facet".to_string()
                }
            };
            return Err(AppError::Validation(message));
        }

        let mut clients = selection.clients;

        if clients.is_empty() {
            return Err(AppError::download_submit_unavailable(
                "no enabled download clients configured",
            ));
        }

        if let Some(artifact_kind) = resolved_artifact_kind {
            clients.retain(|config| {
                let compatible = Self::config_accepts_artifact_kind(
                    config,
                    artifact_kind,
                    self.plugin_provider.as_ref(),
                );
                if !compatible {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        artifact_kind = Self::artifact_kind_label(artifact_kind),
                        "download client skipped because it cannot accept the resolved artifact"
                    );
                }
                compatible
            });

            if clients.is_empty() {
                return Err(AppError::Validation(format!(
                    "no enabled download client can accept the resolved {}",
                    Self::artifact_kind_label(artifact_kind)
                )));
            }
        } else if let Some(source_kind) = Self::request_source_kind(request) {
            clients.retain(|config| {
                let compatible = Self::config_accepts_source_kind(
                    config,
                    source_kind,
                    self.plugin_provider.as_ref(),
                );
                if !compatible {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        source_kind = source_kind.as_str(),
                        "download client skipped because it cannot handle this release type"
                    );
                }
                compatible
            });

            if clients.is_empty() {
                return Err(AppError::Validation(format!(
                    "no enabled download client can handle {} releases",
                    Self::source_kind_label(source_kind)
                )));
            }
        }

        let mut last_error: Option<AppError> = None;
        let mut staged_nzb = if let Some(staged_nzb) = request.staged_nzb.clone() {
            self.staged_nzb_store
                .mark_artifact_active(&staged_nzb.compressed_path)?;
            Some(super::StagedNzbLease {
                staged_nzb,
                self_staged: false,
                store: self.staged_nzb_store.clone(),
                _permit: None,
            })
        } else {
            None
        };
        for config in clients {
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped due to invalid configuration"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            let effective_request = match self
                .apply_selected_client_routing(request, &config.id)
                .await
            {
                Ok(mut effective_request) => {
                    if Self::is_native_nzb_client_type(&config.client_type)
                        && Self::request_uses_nzb_payload(&effective_request)
                    {
                        if staged_nzb.is_none() {
                            if let Some(ResolvedDownloadArtifact::Nzb { bytes, .. }) =
                                effective_request.resolved_download_artifact.clone()
                            {
                                let source_label = effective_request
                                    .download_id
                                    .as_deref()
                                    .or(effective_request.source_title.as_deref())
                                    .unwrap_or("proxied-nzb");
                                staged_nzb = Some(
                                    stage_nzb_from_bytes(
                                        &self.staged_nzb_store,
                                        &self.staged_nzb_pipeline_limit,
                                        source_label,
                                        Some(&request.title.id),
                                        bytes,
                                    )
                                    .await?,
                                );
                            } else {
                                let source_hint = request_source_hint_for_nzb(&effective_request)?;
                                staged_nzb = Some(
                                    stage_nzb_from_url(
                                        &self.outbound_http,
                                        &self.staged_nzb_store,
                                        &self.staged_nzb_pipeline_limit,
                                        &source_hint,
                                        Some(&request.title.id),
                                        request
                                            .search_facet
                                            .as_ref()
                                            .unwrap_or(&request.title.facet),
                                    )
                                    .await?,
                                );
                            }
                        }
                        effective_request.staged_nzb =
                            staged_nzb.as_ref().map(|lease| lease.staged_nzb.clone());
                    }
                    effective_request
                }
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped because routing configuration could not be resolved"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            match client.submit_download(&effective_request).await {
                Ok(result) => {
                    self.delete_staged_nzb(
                        staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                        "submit_success",
                    )
                    .await;
                    return Ok(DownloadGrabResult {
                        job_id: result.job_id,
                        client_id: Some(config.id.clone()),
                        client_type: config.client_type.clone(),
                        info_hash: result.info_hash,
                    });
                }
                Err(error) => {
                    let should_failover = matches!(
                        error,
                        AppError::Repository(_) | AppError::DownloadSubmitUnavailable(_)
                    );
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        failover = should_failover,
                        "download client enqueue failed"
                    );
                    if should_failover {
                        last_error = Some(error);
                        continue;
                    }
                    self.delete_staged_nzb(
                        staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                        "submit_failure",
                    )
                    .await;
                    return Err(error);
                }
            }
        }

        self.delete_staged_nzb(
            staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
            "submit_failure",
        )
        .await;

        Err(last_error
            .unwrap_or_else(|| {
                AppError::Repository(
                    "all prioritized download clients failed to enqueue this release".to_string(),
                )
            })
            .into_download_submit_unavailable())
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue_excluding_client_types(&[]).await
    }

    async fn list_queue_excluding_client_types(
        &self,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self
            .list_enabled_clients_by_priority_excluding(excluded_client_types)
            .await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::Queue)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(DownloadFeedbackReadKind::Queue),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for queue listing");
                    continue;
                }
            };
            match client.list_queue().await {
                Ok(mut items) => {
                    self.record_feedback_read_success(&config.id, DownloadFeedbackReadKind::Queue);
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(&config.id, DownloadFeedbackReadKind::Queue);
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list queue");
                }
            }
        }
        read_summary.finish()?;
        Ok(all_items)
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            let _ =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::TitleQueue);
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for title-scoped queue listing");
                    continue;
                }
            };
            match client.list_queue_for_title(title_id).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::TitleQueue,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::TitleQueue,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list title-scoped queue");
                }
            }
        }
        read_summary.finish()?;
        Ok(all_items)
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::History)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(DownloadFeedbackReadKind::History),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for history listing");
                    continue;
                }
            };
            match client.list_history().await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list history");
                }
            }
        }
        read_summary.finish()?;
        Ok(all_items)
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_recent_activity_excluding_client_types(limit, &[])
            .await
    }

    async fn list_recent_activity_excluding_client_types(
        &self,
        limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self
            .list_enabled_clients_by_priority_excluding(excluded_client_types)
            .await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) = self
                .feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::RecentActivity)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(
                        DownloadFeedbackReadKind::RecentActivity
                    ),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for recent activity listing");
                    continue;
                }
            };
            match client.list_recent_activity(limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);
        all_items.truncate(limit);
        Ok(all_items)
    }

    async fn list_recent_activity_for_client_types(
        &self,
        limit: usize,
        client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 || client_types.is_empty() {
            return Ok(Vec::new());
        }

        let clients = self
            .list_enabled_clients_by_priority()
            .await?
            .into_iter()
            .filter(|config| {
                client_types.iter().any(|client_type| {
                    config
                        .client_type
                        .trim()
                        .eq_ignore_ascii_case(client_type.trim())
                })
            })
            .collect::<Vec<_>>();
        if clients.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) = self
                .feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::RecentActivity)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(
                        DownloadFeedbackReadKind::RecentActivity
                    ),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for type-scoped recent activity listing");
                    continue;
                }
            };
            match client.list_recent_activity(limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list type-scoped recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);
        all_items.truncate(limit);
        Ok(all_items)
    }

    async fn list_recent_activity_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            let _ = self.feedback_backoff_remaining(
                &config.id,
                DownloadFeedbackReadKind::TitleRecentActivity,
            );
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for title-scoped recent activity listing");
                    continue;
                }
            };
            match client.list_recent_activity_for_title(title_id, limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::TitleRecentActivity,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::TitleRecentActivity,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list title-scoped recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);
        all_items.truncate(limit);
        Ok(all_items)
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }

        let fetch_limit = offset.saturating_add(limit);
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::History)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(DownloadFeedbackReadKind::History),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for paged history listing");
                    continue;
                }
            };
            match client.list_history_page(0, fetch_limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list paged history");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);

        Ok(all_items.into_iter().skip(offset).take(limit).collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }
        let mut all_items = Vec::new();
        for config in clients {
            if let Some(remaining) = self.feedback_backoff_remaining(
                &config.id,
                DownloadFeedbackReadKind::RecentCompletedDownloads,
            ) {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(
                        DownloadFeedbackReadKind::RecentCompletedDownloads
                    ),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for completed downloads");
                    continue;
                }
            };
            let mappings = download_client_remote_path_mappings(&config);
            let accepts_torrents = Self::config_accepts_source_kind(
                &config,
                DownloadSourceKind::TorrentFile,
                self.plugin_provider.as_ref(),
            ) || Self::config_accepts_source_kind(
                &config,
                DownloadSourceKind::MagnetUri,
                self.plugin_provider.as_ref(),
            );
            match client.list_completed_downloads().await {
                Ok(mut items) => {
                    tracing::debug!(
                        client = %config.name,
                        client_type = %config.client_type,
                        count = items.len(),
                        "completed downloads from client"
                    );
                    let mut accepted_items = Vec::with_capacity(items.len());
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        if let Some(mappings) = mappings.as_deref() {
                            apply_remote_path_mappings_to_completed_download(item, mappings);
                        }
                        if accepts_torrents {
                            normalize_completed_download_import_dir(item);
                        }
                        accepted_items.push(item.clone());
                    }
                    all_items.extend(accepted_items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, client = %config.name, error = %error, "failed to list completed downloads");
                }
            }
        }
        Ok(all_items)
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.list_recent_completed_downloads_excluding_client_types(limit, &[])
            .await
    }

    async fn list_recent_completed_downloads_excluding_client_types(
        &self,
        limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.list_recent_completed_downloads_for_client_scope(
            limit,
            &[],
            &[],
            excluded_client_types,
        )
        .await
    }

    async fn list_recent_completed_downloads_for_client_scope(
        &self,
        limit: usize,
        client_ids: &[String],
        client_types: &[String],
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self
            .list_enabled_clients_by_priority_excluding(excluded_client_types)
            .await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }

        let scoped_client_ids = client_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        let scoped_client_types = client_types
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        let has_scope = !scoped_client_ids.is_empty() || !scoped_client_types.is_empty();
        let mut all_items = Vec::new();
        for config in clients {
            if has_scope {
                let type_key = config.client_type.trim().to_ascii_lowercase();
                let id_matches =
                    !scoped_client_ids.is_empty() && scoped_client_ids.contains(config.id.trim());
                let type_matches = !scoped_client_types.is_empty()
                    && scoped_client_types.contains(type_key.as_str());
                let matches_scope = id_matches || type_matches;
                if !matches_scope {
                    continue;
                }
            }
            if let Some(remaining) = self.feedback_backoff_remaining(
                &config.id,
                DownloadFeedbackReadKind::RecentCompletedDownloads,
            ) {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(
                        DownloadFeedbackReadKind::RecentCompletedDownloads
                    ),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for recent completed downloads");
                    continue;
                }
            };
            let mappings = download_client_remote_path_mappings(&config);
            let accepts_torrents = Self::config_accepts_source_kind(
                &config,
                DownloadSourceKind::TorrentFile,
                self.plugin_provider.as_ref(),
            ) || Self::config_accepts_source_kind(
                &config,
                DownloadSourceKind::MagnetUri,
                self.plugin_provider.as_ref(),
            );
            match client.list_recent_completed_downloads(limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentCompletedDownloads,
                    );
                    tracing::debug!(
                        client = %config.name,
                        client_type = %config.client_type,
                        recent_completed_strategy = recent_completed_strategy_label(&config.client_type),
                        count = items.len(),
                        "recent completed downloads from client"
                    );
                    let mut accepted_items = Vec::with_capacity(items.len());
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        if let Some(mappings) = mappings.as_deref() {
                            apply_remote_path_mappings_to_completed_download(item, mappings);
                        }
                        if accepts_torrents {
                            normalize_completed_download_import_dir(item);
                        }
                        accepted_items.push(item.clone());
                    }
                    all_items.extend(accepted_items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::RecentCompletedDownloads,
                    );
                    tracing::warn!(client_id = %config.id, client = %config.name, error = %error, "failed to list recent completed downloads");
                }
            }
        }

        all_items.sort_by(compare_completed_downloads_desc);
        all_items.truncate(limit);
        Ok(all_items)
    }

    async fn get_completed_download_for_source(
        &self,
        client_id: &str,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<scryer_domain::CompletedDownload>> {
        let reference = download_client_item_id.trim();
        if reference.is_empty() {
            return Ok(None);
        }

        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Ok(None);
        }

        let clients = self.list_enabled_clients_by_priority_excluding(&[]).await?;
        let config = clients.iter().find(|config| {
            config.id.trim() == client_id
                && config
                    .client_type
                    .trim()
                    .eq_ignore_ascii_case(client_type.trim())
        });
        let Some(config) = config else {
            return Ok(None);
        };

        if let Some(remaining) = self.feedback_backoff_remaining(
            &config.id,
            DownloadFeedbackReadKind::RecentCompletedDownloads,
        ) {
            debug!(
                client_id = %config.id,
                client = %config.name,
                remaining_ms = remaining.as_millis(),
                "skipping targeted completed download lookup during backoff"
            );
            return Ok(None);
        }

        let client = Self::client_from_config(
            config,
            self.staged_nzb_store.clone(),
            self.staged_nzb_pipeline_limit.clone(),
            self.plugin_provider.as_ref(),
            self.feedback_read_timeout,
        )?;
        match client
            .get_completed_download_for_source(&config.id, &config.client_type, reference)
            .await
        {
            Ok(item) => {
                self.record_feedback_read_success(
                    &config.id,
                    DownloadFeedbackReadKind::RecentCompletedDownloads,
                );
                Ok(item.map(|mut item| {
                    item.client_id = config.id.clone();
                    if let Some(mappings) = download_client_remote_path_mappings(config).as_deref()
                    {
                        apply_remote_path_mappings_to_completed_download(&mut item, mappings);
                    }
                    let accepts_torrents = Self::config_accepts_source_kind(
                        config,
                        DownloadSourceKind::TorrentFile,
                        self.plugin_provider.as_ref(),
                    ) || Self::config_accepts_source_kind(
                        config,
                        DownloadSourceKind::MagnetUri,
                        self.plugin_provider.as_ref(),
                    );
                    if accepts_torrents {
                        normalize_completed_download_import_dir(&mut item);
                    }
                    item
                }))
            }
            Err(error) => {
                self.record_feedback_read_failure(
                    &config.id,
                    DownloadFeedbackReadKind::RecentCompletedDownloads,
                );
                Err(error)
            }
        }
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, false).await? {
            return client.pause_queue_item(id).await;
        }
        Err(AppError::Validation(format!(
            "download client item not found: {id}"
        )))
    }

    async fn pause_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.pause_queue_item(id).await;
        }
        Err(AppError::Validation(format!(
            "download client not found: {client_id}"
        )))
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, false).await? {
            return client.resume_queue_item(id).await;
        }
        Err(AppError::Validation(format!(
            "download client item not found: {id}"
        )))
    }

    async fn resume_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.resume_queue_item(id).await;
        }
        Err(AppError::Validation(format!(
            "download client not found: {client_id}"
        )))
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, is_history).await? {
            return client.delete_queue_item(id, is_history).await;
        }
        Err(AppError::Validation(format!(
            "download client item not found: {id}"
        )))
    }

    async fn delete_queue_item_for_client_id(
        &self,
        client_id: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.delete_queue_item(id, is_history).await;
        }
        Err(AppError::Validation(format!(
            "download client not found: {client_id}"
        )))
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_type(client_type).await? {
            return client.delete_queue_item(id, is_history).await;
        }
        Err(AppError::Validation(format!(
            "download client not found for type: {client_type}"
        )))
    }

    async fn get_client_status_for_client_id(
        &self,
        client_id: &str,
    ) -> AppResult<DownloadClientStatus> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        let config = self
            .download_client_configs
            .get_by_id(client_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!("download client not found: {client_id}"))
            })?;
        let client = Self::client_from_config(
            &config,
            self.staged_nzb_store.clone(),
            self.staged_nzb_pipeline_limit.clone(),
            self.plugin_provider.as_ref(),
            self.feedback_read_timeout,
        )?;
        let mut status = client.get_client_status().await?;
        let mappings = download_client_remote_path_mappings(&config);
        if let Some(mappings) = mappings.as_deref() {
            apply_remote_path_mappings_to_status(&mut status, mappings);
        }
        Ok(status)
    }
}

fn recent_completed_strategy_label(client_type: &str) -> &'static str {
    match client_type {
        "sabnzbd" | "weaver" => "bounded",
        _ => "full_fallback_or_client_default",
    }
}

fn parse_history_timestamp(value: Option<&str>) -> i64 {
    value
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
}

fn compare_history_items_desc(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
) -> std::cmp::Ordering {
    parse_history_timestamp(right.last_updated_at.as_deref())
        .cmp(&parse_history_timestamp(left.last_updated_at.as_deref()))
        .then_with(|| right.id.cmp(&left.id))
}

fn download_queue_history_key(item: &DownloadQueueItem) -> String {
    if item.client_type.is_empty() && item.download_client_item_id.is_empty() {
        return item.id.clone();
    }

    if item.client_id.trim().is_empty() {
        return format!("{}:{}", item.client_type, item.download_client_item_id);
    }

    format!("{}:{}", item.client_id, item.download_client_item_id)
}

fn compare_completed_downloads_desc(
    left: &scryer_domain::CompletedDownload,
    right: &scryer_domain::CompletedDownload,
) -> std::cmp::Ordering {
    right.completed_at.cmp(&left.completed_at).then_with(|| {
        right
            .download_client_item_id
            .cmp(&left.download_client_item_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncWriteExt;

    fn test_indexer_config(base_url: &str) -> scryer_domain::IndexerConfig {
        let now = Utc::now();
        scryer_domain::IndexerConfig {
            id: "indexer-1".to_string(),
            name: "Indexer".to_string(),
            provider_type: "torznab".to_string(),
            base_url: base_url.to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn torrent_metainfo_detection_requires_valid_bencoded_info_dict() {
        assert!(looks_like_torrent_metainfo(b"d4:infod4:name4:testee"));
        assert!(!looks_like_torrent_metainfo(b"not a torrent"));
        assert!(!looks_like_torrent_metainfo(b"d4:name4:testee"));
    }

    #[test]
    fn classifier_rejects_invalid_torrent_bytes_before_submission() {
        let headers = serde_json::json!({
            "content-type": "application/x-bittorrent",
        });

        let error = PrioritizedDownloadClientRouter::classify_resolved_download_artifact(
            "Byparr",
            Some("https://indexer.example/download/thing.torrent"),
            Some(&headers),
            b"not a torrent".to_vec(),
            None,
        )
        .expect_err("invalid torrent bytes must fail");

        assert_eq!(
            error.to_string(),
            "validation: Byparr resolved invalid torrent file bytes.",
        );
    }

    #[tokio::test]
    async fn trawl_resolves_embedded_nzb_with_millisecond_timeout() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let download_url = format!("{}/download/release.nzb", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": download_url,
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": download_url,
                    "status": 200,
                    "headers": { "content-type": "application/x-nzb" },
                    "cookies": [{ "name": "cf_clearance", "value": "abc" }],
                    "userAgent": "Trawl",
                    "response": "<nzb></nzb>"
                }
            })))
            .mount(&server)
            .await;

        let now = Utc::now();
        let proxy = IndexerProxyConfig {
            id: "trawl-1".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_indexer_proxy(&proxy, &download_url, None)
            .await
            .expect("Trawl should resolve embedded NZB content");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::Nzb { bytes, .. } if bytes == b"<nzb></nzb>"
        ));
    }

    #[tokio::test]
    async fn trawl_artifact_resolution_uses_direct_response_before_solver() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let download_path = "/download";
        let download_url = format!("{}{download_path}?id=direct", server.uri());
        Mock::given(method("GET"))
            .and(path(download_path))
            .and(query_param("id", "direct"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-bittorrent")
                    .set_body_bytes(b"d4:infod4:name6:directee"),
            )
            .mount(&server)
            .await;

        let now = Utc::now();
        let proxy = IndexerProxyConfig {
            id: "trawl-direct-first".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_indexer_proxy(&proxy, &download_url, None)
            .await
            .expect("direct artifact should bypass Trawl");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::TorrentFile { bytes, .. }
                if bytes == b"d4:infod4:name6:directee"
        ));
        let requests = server
            .received_requests()
            .await
            .expect("direct request should be captured");
        assert_eq!(requests.len(), 1, "the solver endpoint must not be called");
        assert_eq!(requests[0].url.path(), download_path);
    }

    #[tokio::test]
    async fn trawl_refetches_opaque_binary_artifact_when_solution_headers_are_empty() {
        use wiremock::matchers::{body_json, header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let download_path = "/download";
        let download_url = format!("{}{download_path}?id=release", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": download_url,
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": download_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [],
                    "userAgent": "Trawl UA",
                    "response": ""
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(download_path))
            .and(query_param("id", "release"))
            .and(header("user-agent", "Trawl UA"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-bittorrent")
                    .set_body_bytes(b"d4:infod4:name4:testee"),
            )
            .mount(&server)
            .await;

        let now = Utc::now();
        let proxy = IndexerProxyConfig {
            id: "trawl-1".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_indexer_proxy(&proxy, &download_url, None)
            .await
            .expect("Trawl should refetch binary torrent content");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::TorrentFile { bytes, .. }
                if bytes == b"d4:infod4:name4:testee"
        ));
    }

    #[tokio::test]
    async fn byparr_non_success_solution_refetches_download_with_clearance_session() {
        use wiremock::matchers::{body_json, header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let download_path = "/download";
        let download_url = format!("{}{download_path}?id=release", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": download_url,
                "maxTimeout": 60
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": download_url,
                    "status": 503,
                    "headers": { "content-type": "text/html" },
                    "cookies": [{ "name": "e2e_clearance", "value": "solved" }],
                    "userAgent": "Byparr UA",
                    "response": "<html>Just a moment</html>"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(download_path))
            .and(query_param("id", "release"))
            .and(header("cookie", "e2e_clearance=solved"))
            .and(header("user-agent", "Byparr UA"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-nzb")
                    .set_body_bytes(b"<nzb></nzb>"),
            )
            .mount(&server)
            .await;

        let now = Utc::now();
        let proxy = IndexerProxyConfig {
            id: "byparr-refetch".into(),
            name: "Byparr".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_indexer_proxy(&proxy, &download_url, None)
            .await
            .expect("Byparr should refetch the NZB with its clearance session");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::Nzb { bytes, .. } if bytes == b"<nzb></nzb>"
        ));
    }

    #[tokio::test]
    async fn trawl_server_error_is_classified_as_solver_unavailable() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let download_url = format!("{}/download?id=release", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": download_url,
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
                "status": "error",
                "message": "Browser pool initializing, retry in a few seconds",
                "solution": {
                    "url": download_url,
                    "status": 0,
                    "headers": {},
                    "response": "",
                    "cookies": [],
                    "userAgent": ""
                }
            })))
            .mount(&server)
            .await;

        let now = Utc::now();
        let proxy = IndexerProxyConfig {
            id: "trawl-unavailable".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };

        let error = no_client_router()
            .resolve_download_artifact_via_indexer_proxy(&proxy, &download_url, None)
            .await
            .expect_err("Trawl server errors must fail as solver unavailable");

        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message == solver::TRAWL_UNAVAILABLE_MESSAGE
        ));
    }

    #[test]
    fn proxied_download_url_requires_the_full_indexer_origin() {
        let indexer = test_indexer_config("https://indexer.example/api");

        assert!(
            PrioritizedDownloadClientRouter::download_url_matches_indexer_origin(
                &indexer,
                "https://INDEXER.example/download/release.torrent",
            )
        );
        assert!(
            !PrioritizedDownloadClientRouter::download_url_matches_indexer_origin(
                &indexer,
                "http://indexer.example/download/release.torrent",
            )
        );
        assert!(
            !PrioritizedDownloadClientRouter::download_url_matches_indexer_origin(
                &indexer,
                "https://indexer.example:8443/download/release.torrent",
            )
        );

        let default_port_indexer = test_indexer_config("http://indexer.example:80/api");
        assert!(
            PrioritizedDownloadClientRouter::download_url_matches_indexer_origin(
                &default_port_indexer,
                "http://indexer.example/download/release.torrent",
            )
        );
    }

    #[tokio::test]
    async fn artifact_resolution_blocks_metadata_destinations_before_network_or_solver() {
        let router = no_client_router();
        let now = Utc::now();
        for provider_type in [
            scryer_domain::IndexerProxyProviderType::Byparr,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ] {
            let proxy = IndexerProxyConfig {
                id: format!("{}-1", provider_type.as_str()),
                name: solver::solver_provider_name(provider_type).to_string(),
                provider_type,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: "http://127.0.0.1:1".to_string(),
                request_timeout_seconds: 1,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            };
            for target in [
                "http://169.254.169.254/latest/meta-data/",
                "http://[::ffff:169.254.169.254]/latest/meta-data/",
                "http://100.100.100.200/latest/meta-data/",
            ] {
                let error = match router
                    .fetch_download_artifact_direct(
                        solver::solver_provider_name(provider_type),
                        target,
                        &[],
                        1,
                    )
                    .await
                {
                    Ok(_) => panic!("metadata destination must be rejected before fetch"),
                    Err(error) => error,
                };
                assert!(
                    matches!(&error, AppError::DownloadSubmitUnavailable(message)
                        if message.contains("unsafe download artifact destination")),
                    "unexpected error for {target}: {error}"
                );

                let error = match router
                    .resolve_download_artifact_via_indexer_proxy(&proxy, target, None)
                    .await
                {
                    Ok(_) => panic!("metadata destination must not be delegated to the solver"),
                    Err(error) => error,
                };
                assert!(
                    matches!(&error, AppError::DownloadSubmitUnavailable(message)
                        if message.contains("unsafe download artifact destination")),
                    "unexpected solver error for {target}: {error}"
                );
            }
        }
    }

    #[tokio::test]
    async fn bounded_response_reader_rejects_chunked_body_over_limit() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n",
                )
                .await
                .expect("write response");
        });

        let response = generic_reqwest_client()
            .get(format!("http://{address}/artifact"))
            .send()
            .await
            .expect("fetch test response");
        let error = read_response_body_bounded(response, 4)
            .await
            .expect_err("five-byte chunked response must exceed four-byte limit");

        assert!(matches!(error, BoundedResponseBodyError::TooLarge));
        server.await.expect("test server task");
    }

    struct MockDownloadClientConfigRepository {
        configs: Vec<DownloadClientConfig>,
    }

    #[async_trait]
    impl DownloadClientConfigRepository for MockDownloadClientConfigRepository {
        async fn list(
            &self,
            _provider_type: Option<String>,
        ) -> AppResult<Vec<DownloadClientConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
            Ok(self.configs.iter().find(|config| config.id == id).cloned())
        }

        async fn create(&self, _config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn update(
            &self,
            _update: scryer_application::DownloadClientConfigUpdate,
        ) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            unreachable!("not used in router tests")
        }

        async fn reorder(&self, _ordered_ids: Vec<String>) -> AppResult<()> {
            unreachable!("not used in router tests")
        }
    }

    #[derive(Default)]
    struct MockSettingsRepository {
        routing_by_scope: HashMap<String, String>,
    }

    #[async_trait]
    impl SettingsRepository for MockSettingsRepository {
        async fn get_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(scope_id.and_then(|id| self.routing_by_scope.get(&id).cloned()))
        }

        async fn get_setting_json_explicit(
            &self,
            _scope: &str,
            _key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(scope_id.and_then(|id| self.routing_by_scope.get(&id).cloned()))
        }

        async fn upsert_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
            _value_json: String,
            _source: &str,
            _updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct MockDownloadClient {
        submissions: Mutex<Vec<DownloadClientAddRequest>>,
        submit_error: Mutex<Option<MockSubmitError>>,
        queue_items: Mutex<Vec<DownloadQueueItem>>,
        history_items: Mutex<Vec<DownloadQueueItem>>,
        completed_downloads: Mutex<Vec<scryer_domain::CompletedDownload>>,
        status: Mutex<DownloadClientStatus>,
        paused: Mutex<Vec<String>>,
        resumed: Mutex<Vec<String>>,
        deleted: Mutex<Vec<(String, bool)>>,
    }

    #[derive(Clone, Copy)]
    enum MockSubmitError {
        Ambiguous,
        Rejected,
        Repository,
        SubmitUnavailable,
    }

    #[async_trait]
    impl DownloadClient for MockDownloadClient {
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.submissions.lock().unwrap().push(request.clone());
            match *self.submit_error.lock().unwrap() {
                Some(MockSubmitError::Ambiguous) => {
                    return Err(AppError::DownloadSubmitAmbiguous(
                        "submit result is ambiguous".to_string(),
                    ));
                }
                Some(MockSubmitError::Rejected) => {
                    return Err(AppError::DownloadSubmitRejected(
                        "submit was rejected".to_string(),
                    ));
                }
                Some(MockSubmitError::Repository) => {
                    return Err(AppError::Repository("client enqueue failed".to_string()));
                }
                Some(MockSubmitError::SubmitUnavailable) => {
                    return Err(AppError::download_submit_unavailable(
                        "client submit unavailable",
                    ));
                }
                None => {}
            }
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "mock".to_string(),
                info_hash: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.queue_items.lock().unwrap().clone())
        }

        async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.history_items.lock().unwrap().clone())
        }

        async fn list_completed_downloads(
            &self,
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            Ok(self.completed_downloads.lock().unwrap().clone())
        }

        async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
            Ok(self.status.lock().unwrap().clone())
        }

        async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
            self.paused.lock().unwrap().push(id.to_string());
            Ok(())
        }

        async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
            self.resumed.lock().unwrap().push(id.to_string());
            Ok(())
        }

        async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
            self.deleted
                .lock()
                .unwrap()
                .push((id.to_string(), is_history));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailingCompletedDownloadClient {
        recent_completed_calls: AtomicUsize,
    }

    impl FailingCompletedDownloadClient {
        fn recent_completed_call_count(&self) -> usize {
            self.recent_completed_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DownloadClient for FailingCompletedDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".to_string()))
        }

        async fn list_recent_completed_downloads(
            &self,
            _limit: usize,
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            self.recent_completed_calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Repository(
                "completed history unavailable".to_string(),
            ))
        }
    }

    #[derive(Default)]
    struct ScopedRecentCompletedDownloadClient {
        scoped_calls: Mutex<Vec<(Vec<String>, Vec<String>)>>,
    }

    #[async_trait]
    impl DownloadClient for ScopedRecentCompletedDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".to_string()))
        }

        async fn list_recent_completed_downloads(
            &self,
            _limit: usize,
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            Err(AppError::Repository(
                "unscoped completed history should not be used".to_string(),
            ))
        }

        async fn list_recent_completed_downloads_for_client_scope(
            &self,
            _limit: usize,
            client_ids: &[String],
            client_types: &[String],
            _excluded_client_types: &[&str],
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            self.scoped_calls
                .lock()
                .unwrap()
                .push((client_ids.to_vec(), client_types.to_vec()));
            Ok(vec![scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: "default".to_string(),
                download_client_item_id: "qbit-1".to_string(),
                download_id: None,
                name: "Qbit Complete".to_string(),
                dest_dir: "/downloads/qbit".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now()),
                parameters: Vec::new(),
            }])
        }
    }

    struct MockDownloadClientPluginProvider {
        accepted_inputs: Vec<String>,
        clients: Vec<(String, Arc<dyn DownloadClient>)>,
    }

    impl DownloadClientPluginProvider for MockDownloadClientPluginProvider {
        fn client_for_config(
            &self,
            config: &DownloadClientConfig,
        ) -> Option<Arc<dyn DownloadClient>> {
            self.clients
                .iter()
                .find(|(id, _)| id == &config.id)
                .map(|(_, client)| Arc::clone(client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["qbittorrent".to_string()]
        }

        fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
            self.accepted_inputs.clone()
        }
    }

    struct DelayedQueueDownloadClient {
        delay: Duration,
        queue_items: Vec<DownloadQueueItem>,
    }

    #[async_trait]
    impl DownloadClient for DelayedQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "delayed".to_string(),
                info_hash: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            tokio::time::sleep(self.delay).await;
            Ok(self.queue_items.clone())
        }
    }

    #[derive(Default)]
    struct FailingQueueDownloadClient {
        list_queue_calls: AtomicUsize,
        list_queue_for_title_calls: AtomicUsize,
    }

    impl FailingQueueDownloadClient {
        fn list_queue_call_count(&self) -> usize {
            self.list_queue_calls.load(Ordering::SeqCst)
        }

        fn list_queue_for_title_call_count(&self) -> usize {
            self.list_queue_for_title_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DownloadClient for FailingQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "failing".to_string(),
                info_hash: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            self.list_queue_calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Repository("queue unavailable".to_string()))
        }

        async fn list_queue_for_title(&self, _title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
            self.list_queue_for_title_calls
                .fetch_add(1, Ordering::SeqCst);
            Err(AppError::Repository("title queue unavailable".to_string()))
        }
    }

    fn test_title_for_facet(facet: MediaFacet) -> scryer_domain::Title {
        let root_folder_id = scryer_domain::root_folder_id_for_path(match facet {
            MediaFacet::Movie => "/data/movies",
            MediaFacet::Series => "/data/series",
            MediaFacet::Anime => "/data/anime",
        });
        scryer_domain::Title {
            id: "title-1".to_string(),
            name: "Test Title".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            facet,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            root_folder_id,
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn test_title() -> scryer_domain::Title {
        test_title_for_facet(MediaFacet::Movie)
    }

    fn test_config(id: &str, name: &str, client_type: &str, priority: i64) -> DownloadClientConfig {
        DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            client_priority: priority,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn null_staged_nzb_store() -> Arc<dyn StagedNzbStore> {
        Arc::new(scryer_application::NullStagedNzbStore)
    }

    fn test_pipeline_limit() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(4))
    }

    fn disabled_test_config(
        id: &str,
        name: &str,
        client_type: &str,
        priority: i64,
    ) -> DownloadClientConfig {
        DownloadClientConfig {
            is_enabled: false,
            ..test_config(id, name, client_type, priority)
        }
    }

    fn test_queue_item(id: &str) -> DownloadQueueItem {
        DownloadQueueItem {
            id: format!("queue-{id}"),
            title_id: None,
            episode_id: None,
            title_name: "Test Download".to_string(),
            facet: None,
            category: None,
            client_id: String::new(),
            client_name: String::new(),
            client_type: "mock".to_string(),
            state: scryer_domain::DownloadQueueState::Queued,
            progress_percent: 0,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        }
    }

    fn no_client_router() -> PrioritizedDownloadClientRouter {
        PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository { configs: vec![] }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            None,
        )
    }

    #[tokio::test]
    async fn no_configured_clients_return_empty_feedback_reads() {
        let router = no_client_router();

        assert!(router.list_queue().await.unwrap().is_empty());
        assert!(
            router
                .list_queue_excluding_client_types(&["weaver"])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            router
                .list_queue_for_title("title-1")
                .await
                .unwrap()
                .is_empty()
        );
        assert!(router.list_history().await.unwrap().is_empty());
        assert!(router.list_history_page(0, 10).await.unwrap().is_empty());
        assert!(router.list_recent_activity(10).await.unwrap().is_empty());
        assert!(
            router
                .list_recent_activity_for_title("title-1", 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(router.list_completed_downloads().await.unwrap().is_empty());
        assert!(
            router
                .list_recent_completed_downloads_for_client_scope(10, &[], &[], &[])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn no_configured_clients_do_not_submit_or_route_queue_actions() {
        let router = no_client_router();

        let submit_error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("no configured clients should make submit unavailable");

        assert!(matches!(
            submit_error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("no enabled download clients configured")
        ));
        assert!(matches!(
            router.pause_queue_item("job-1").await,
            Err(AppError::Validation(message)) if message.contains("download client item not found")
        ));
        assert!(matches!(
            router.resume_queue_item("job-1").await,
            Err(AppError::Validation(message)) if message.contains("download client item not found")
        ));
        assert!(matches!(
            router.delete_queue_item("job-1", false).await,
            Err(AppError::Validation(message)) if message.contains("download client item not found")
        ));
    }

    #[tokio::test]
    async fn submit_download_skips_incompatible_clients_by_source_kind() {
        let torrent_client = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string(), "magnet_uri".to_string()],
                clients: vec![("torrent".to_string(), torrent_client.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("torrent", "qBittorrent", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let result = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("torrent request should route to torrent client");

        assert_eq!(result.client_type, "qbittorrent");
        assert_eq!(torrent_client.submissions.lock().unwrap().len(), 1);
    }

    /// Head shape taken from the incident NZB: the indexer declared the
    /// release as anime, and Scryer submitted it for a live-action series.
    const ANIME_CATEGORY_NZB: &[u8] = br#"<?xml version="1.0" encoding="iso-8859-1" ?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
<head>
 <meta type="name">One.Piece.S02.DANiSH.JAPANESE.1080p.WEB.H264</meta>
 <meta type="category">TV &gt; Anime</meta>
</head>
<file poster="poster@example.invalid" date="1700000000" subject="[1/1] - &quot;one.piece.par2&quot;"></file>
</nzb>"#;

    fn nzb_bytes_router(client: Arc<MockDownloadClient>) -> PrioritizedDownloadClientRouter {
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb".to_string()],
                clients: vec![("nzb".to_string(), client)],
            });
        PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("nzb", "Plugin NZB", "plugin-nzb", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
    }

    fn resolved_nzb_request(facet: MediaFacet) -> DownloadClientAddRequest {
        DownloadClientAddRequest {
            title: test_title_for_facet(facet),
            search_facet: None,
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            download_id: None,
            source_hint: Some("https://example.invalid/release.nzb".to_string()),
            staged_nzb: None,
            resolved_download_artifact: Some(ResolvedDownloadArtifact::Nzb {
                bytes: ANIME_CATEGORY_NZB.to_vec(),
                file_name: Some("release.nzb".to_string()),
                content_type: Some("application/x-nzb".to_string()),
            }),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("One.Piece.S02.DANiSH.JAPANESE.1080p.WEB.H264".to_string()),
            source_password: None,
            category: None,
            queue_priority: None,
            download_directory: None,
            release_title: None,
            indexer_name: None,
            indexer_id: None,
            info_hash_hint: None,
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            is_recent: None,
            season_pack: None,
        }
    }

    #[tokio::test]
    async fn submit_download_blocks_nzb_bytes_whose_category_contradicts_the_subject() {
        let client = Arc::new(MockDownloadClient::default());
        let router = nzb_bytes_router(client.clone());

        let error = router
            .submit_download(&resolved_nzb_request(MediaFacet::Series))
            .await
            .expect_err("an anime-categorized nzb must not be submitted for a series subject");

        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("category_mismatch")),
            "expected a definitive category_mismatch veto, got {error:?}"
        );
        assert!(
            client.submissions.lock().unwrap().is_empty(),
            "the download client must never receive a vetoed release"
        );
    }

    #[tokio::test]
    async fn submit_download_passes_nzb_bytes_whose_category_matches_the_subject() {
        let client = Arc::new(MockDownloadClient::default());
        let router = nzb_bytes_router(client.clone());

        router
            .submit_download(&resolved_nzb_request(MediaFacet::Anime))
            .await
            .expect("the same bytes are exactly right for an anime subject");

        assert_eq!(client.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_gate_honors_search_facet_over_owner_facet() {
        // Series-movie grabs: the owning title is a series, but the release
        // was searched and validated as a movie. The gate must compare the
        // search facet, or every correctly categorized linked film is vetoed.
        let client = Arc::new(MockDownloadClient::default());
        let router = nzb_bytes_router(client.clone());

        let mut request = resolved_nzb_request(MediaFacet::Series);
        request.search_facet = Some(MediaFacet::Anime);
        router
            .submit_download(&request)
            .await
            .expect("an anime-categorized nzb is right for an anime-faceted search");

        assert_eq!(client.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_does_not_failover_ambiguous_submit_errors() {
        let primary = Arc::new(MockDownloadClient::default());
        *primary.submit_error.lock().unwrap() = Some(MockSubmitError::Ambiguous);
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("ambiguous submit errors should stop router failover");

        assert!(matches!(error, AppError::DownloadSubmitAmbiguous(_)));
        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn submit_download_does_not_failover_rejected_submit_errors() {
        let primary = Arc::new(MockDownloadClient::default());
        *primary.submit_error.lock().unwrap() = Some(MockSubmitError::Rejected);
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("rejected submit errors should stop router failover");

        assert!(matches!(error, AppError::DownloadSubmitRejected(_)));
        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn submit_download_all_failover_clients_failed_returns_submit_unavailable() {
        let primary = Arc::new(MockDownloadClient::default());
        *primary.submit_error.lock().unwrap() = Some(MockSubmitError::Repository);
        let secondary = Arc::new(MockDownloadClient::default());
        *secondary.submit_error.lock().unwrap() = Some(MockSubmitError::SubmitUnavailable);
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("exhausted failover clients should fail");

        assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_errors_when_no_enabled_client_can_handle_source_kind() {
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("nzb", "NZBGet", "nzbget", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            None,
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("magnet:?xt=urn:btih:abcdef".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::MagnetUri),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("magnet request should fail when only nzb clients are enabled");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("magnet"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_download_skips_clients_disabled_for_facet() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": false },
                        "secondary": { "enabled": true }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let result = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("secondary client should be used when primary is disabled for facet");

        assert_eq!(result.client_type, "qbittorrent");
        assert!(primary.submissions.lock().unwrap().is_empty());
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_respects_facet_specific_enablement_per_scope() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([
                    (
                        "movie".to_string(),
                        r#"{
                            "primary": { "enabled": false },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                    (
                        "anime".to_string(),
                        r#"{
                            "primary": { "enabled": true },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                ]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title_for_facet(MediaFacet::Movie),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/movie.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Movie Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("movie request should use secondary");

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title_for_facet(MediaFacet::Anime),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/anime.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Anime Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("anime request should use primary");

        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_ignores_facet_enabled_flag_for_globally_disabled_clients() {
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("secondary".to_string(), secondary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    disabled_test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": true },
                        "secondary": { "enabled": true }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("secondary client should be used because primary is globally disabled");

        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_applies_selected_client_category_and_recent_queue_priority() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "category": "Movies",
                            "recentQueuePriority": "high",
                            "olderQueuePriority": "low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: Some("Fallback".to_string()),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(true),
                season_pack: None,
            })
            .await
            .expect("request should be routed");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.category.as_deref(), Some("Movies"));
        assert_eq!(request.queue_priority.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn submit_download_uses_older_queue_priority_when_request_is_not_recent() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "olderPriority": "very low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(false),
                season_pack: None,
            })
            .await
            .expect("request should be routed");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.queue_priority.as_deref(), Some("very low"));
    }

    #[tokio::test]
    async fn submit_download_fails_when_all_clients_disabled_for_facet() {
        let primary = Arc::new(MockDownloadClient::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": false }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            })),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("facet-disabled clients should fail fast");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("no download client enabled"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }

        assert!(primary.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn submit_download_library_override_beats_facet_routing_for_eligibility() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let title = test_title();
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([
                    (
                        "movie".to_string(),
                        r#"{
                            "primary": { "enabled": true },
                            "secondary": { "enabled": false }
                        }"#
                        .to_string(),
                    ),
                    (
                        title.library_id.clone(),
                        r#"{
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                ]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title,
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("library override should use the secondary client");

        assert!(primary.submissions.lock().unwrap().is_empty());
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_treats_missing_library_override_clients_as_disabled() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let title = test_title();
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([
                    (
                        "movie".to_string(),
                        r#"{
                            "primary": { "enabled": true },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                    (
                        title.library_id.clone(),
                        r#"{
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                ]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title,
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("omitted clients should be treated as disabled for this library");

        assert!(primary.submissions.lock().unwrap().is_empty());
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_library_override_applies_category_and_queue_priority() {
        let primary = Arc::new(MockDownloadClient::default());
        let title = test_title();
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    title.library_id.clone(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "category": "Library Movies",
                            "recentQueuePriority": "high",
                            "olderQueuePriority": "low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title,
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: Some("Fallback".to_string()),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(true),
                season_pack: None,
            })
            .await
            .expect("library override should route the request");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.category.as_deref(), Some("Library Movies"));
        assert_eq!(request.queue_priority.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn submit_download_fails_when_all_clients_disabled_for_library_override() {
        let title = test_title();
        let primary = Arc::new(MockDownloadClient::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    title.library_id.clone(),
                    r#"{
                        "primary": { "enabled": false }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            })),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title,
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                indexer_id: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("library override should fail fast when every client is disabled");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("no download client enabled for library"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
        assert!(primary.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn pause_queue_item_routes_to_matching_client_item_id() {
        let nzb_client = Arc::new(MockDownloadClient::default());
        nzb_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("123"));

        let sab_client = Arc::new(MockDownloadClient::default());
        sab_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("SABnzbd_nzo_95u9pco9"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("nzb".to_string(), nzb_client.clone()),
                    ("sab".to_string(), sab_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("sab", "SABnzbd", "sabnzbd", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .pause_queue_item("SABnzbd_nzo_95u9pco9")
            .await
            .expect("pause should route to sabnzbd client");

        assert!(nzb_client.paused.lock().unwrap().is_empty());
        assert_eq!(
            sab_client.paused.lock().unwrap().as_slice(),
            ["SABnzbd_nzo_95u9pco9"]
        );
    }

    #[tokio::test]
    async fn delete_history_item_routes_to_matching_client_item_id() {
        let nzb_client = Arc::new(MockDownloadClient::default());
        nzb_client
            .history_items
            .lock()
            .unwrap()
            .push(test_queue_item("42"));

        let sab_client = Arc::new(MockDownloadClient::default());
        sab_client
            .history_items
            .lock()
            .unwrap()
            .push(test_queue_item("SABnzbd_nzo_hist01"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("nzb".to_string(), nzb_client.clone()),
                    ("sab".to_string(), sab_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("sab", "SABnzbd", "sabnzbd", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .delete_queue_item("SABnzbd_nzo_hist01", true)
            .await
            .expect("history delete should route to sabnzbd client");

        assert!(nzb_client.deleted.lock().unwrap().is_empty());
        assert_eq!(
            sab_client.deleted.lock().unwrap().as_slice(),
            [("SABnzbd_nzo_hist01".to_string(), true)]
        );
    }

    #[tokio::test]
    async fn list_history_page_merges_clients_before_slicing() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        let mut a1 = test_queue_item("a-1");
        a1.last_updated_at = Some("300".to_string());
        let mut a2 = test_queue_item("a-2");
        a2.last_updated_at = Some("100".to_string());
        client_a.history_items.lock().unwrap().extend([a1, a2]);

        let mut b1 = test_queue_item("b-1");
        b1.last_updated_at = Some("200".to_string());
        let mut b2 = test_queue_item("b-2");
        b2.last_updated_at = Some("50".to_string());
        client_b.history_items.lock().unwrap().extend([b1, b2]);

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a.clone()),
                    ("client-b".to_string(), client_b.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("client-a", "Client A", "qbittorrent", 0),
                    test_config("client-b", "Client B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let page = router
            .list_history_page(1, 2)
            .await
            .expect("paged history should succeed");

        let ids = page
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b-1".to_string(), "a-2".to_string()]);
    }

    #[tokio::test]
    async fn list_recent_activity_merges_clients_before_truncating() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        let mut a1 = test_queue_item("a-1");
        a1.last_updated_at = Some("300".to_string());
        let mut a2 = test_queue_item("a-2");
        a2.last_updated_at = Some("100".to_string());
        client_a.history_items.lock().unwrap().extend([a1, a2]);

        let mut b1 = test_queue_item("b-1");
        b1.last_updated_at = Some("200".to_string());
        let mut b2 = test_queue_item("b-2");
        b2.last_updated_at = Some("50".to_string());
        client_b.history_items.lock().unwrap().extend([b1, b2]);

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a.clone()),
                    ("client-b".to_string(), client_b.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("client-a", "Client A", "qbittorrent", 0),
                    test_config("client-b", "Client B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_recent_activity(2)
            .await
            .expect("recent activity should succeed");

        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a-1".to_string(), "b-1".to_string()]);
    }

    #[tokio::test]
    async fn list_queue_excluding_weaver_keeps_non_weaver_clients() {
        let qbit_client = Arc::new(MockDownloadClient::default());
        qbit_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("qbit-1"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("qbit-client".to_string(), qbit_client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("weaver-client", "Weaver", "weaver", 0),
                    test_config("qbit-client", "qBittorrent", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_queue_excluding_client_types(&["weaver"])
            .await
            .expect("queue listing should succeed");

        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["qbit-1".to_string()]);
    }

    #[tokio::test]
    async fn list_recent_completed_excluding_only_weaver_does_not_use_fallback_client() {
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("weaver-client", "Weaver", "weaver", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            None,
        );

        let items = router
            .list_recent_completed_downloads_excluding_client_types(10, &["weaver"])
            .await
            .expect("recent completed listing should succeed");

        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn list_recent_completed_downloads_merges_clients_before_truncating() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        client_a.completed_downloads.lock().unwrap().extend([
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "a-1".to_string(),
                download_id: None,
                name: "A 1".to_string(),
                dest_dir: "/downloads/a-1".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now()),
                parameters: Vec::new(),
            },
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "a-2".to_string(),
                download_id: None,
                name: "A 2".to_string(),
                dest_dir: "/downloads/a-2".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now() - chrono::Duration::minutes(2)),
                parameters: Vec::new(),
            },
        ]);
        client_b.completed_downloads.lock().unwrap().extend([
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "b-1".to_string(),
                download_id: None,
                name: "B 1".to_string(),
                dest_dir: "/downloads/b-1".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now() - chrono::Duration::minutes(1)),
                parameters: Vec::new(),
            },
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "b-2".to_string(),
                download_id: None,
                name: "B 2".to_string(),
                dest_dir: "/downloads/b-2".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now() - chrono::Duration::minutes(3)),
                parameters: Vec::new(),
            },
        ]);

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a.clone()),
                    ("client-b".to_string(), client_b.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("client-a", "Client A", "qbittorrent", 0),
                    test_config("client-b", "Client B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_recent_completed_downloads(2)
            .await
            .expect("recent completed downloads should succeed");

        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a-1".to_string(), "b-1".to_string()]);
    }

    #[tokio::test]
    async fn list_recent_completed_downloads_for_client_scope_skips_unrelated_clients() {
        let qbit_client = Arc::new(MockDownloadClient::default());
        qbit_client
            .completed_downloads
            .lock()
            .unwrap()
            .push(scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "qbit-1".to_string(),
                download_id: None,
                name: "Qbit Complete".to_string(),
                dest_dir: "/downloads/qbit".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now()),
                parameters: Vec::new(),
            });
        let nzbget_client = Arc::new(FailingCompletedDownloadClient::default());

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string(), "nzb_file".to_string()],
                clients: vec![
                    ("qbit-client".to_string(), qbit_client.clone()),
                    ("nzbget-client".to_string(), nzbget_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("qbit-client", "qBittorrent", "qbittorrent", 0),
                    test_config("nzbget-client", "NZBGet", "nzbget", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_recent_completed_downloads_for_client_scope(
                10,
                &["qbit-client".to_string()],
                &[],
                &[],
            )
            .await
            .expect("scoped completed downloads should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].download_client_item_id, "qbit-1");
        assert_eq!(items[0].client_id, "qbit-client");
        assert_eq!(nzbget_client.recent_completed_call_count(), 0);
    }

    #[tokio::test]
    async fn list_recent_completed_downloads_for_client_scope_matches_exact_id_with_stale_type() {
        let qbit_client = Arc::new(MockDownloadClient::default());
        qbit_client
            .completed_downloads
            .lock()
            .unwrap()
            .push(scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "qbit-1".to_string(),
                download_id: None,
                name: "Qbit Complete".to_string(),
                dest_dir: "/downloads/qbit".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now()),
                parameters: Vec::new(),
            });
        let nzbget_client = Arc::new(FailingCompletedDownloadClient::default());

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string(), "nzb_file".to_string()],
                clients: vec![
                    ("qbit-client".to_string(), qbit_client.clone()),
                    ("nzbget-client".to_string(), nzbget_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("qbit-client", "qBittorrent", "stale-type", 0),
                    test_config("nzbget-client", "NZBGet", "nzbget", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_recent_completed_downloads_for_client_scope(
                10,
                &["qbit-client".to_string()],
                &[],
                &[],
            )
            .await
            .expect("exact-id scoped completed downloads should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].download_client_item_id, "qbit-1");
        assert_eq!(items[0].client_id, "qbit-client");
        assert_eq!(nzbget_client.recent_completed_call_count(), 0);
    }

    #[tokio::test]
    async fn list_recent_completed_downloads_backs_off_failing_clients() {
        let failing_client = Arc::new(FailingCompletedDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_file".to_string()],
                clients: vec![("nzbget-client".to_string(), failing_client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("nzbget-client", "NZBGet", "nzbget", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let first = router
            .list_recent_completed_downloads(10)
            .await
            .expect("failed client should produce partial empty results");
        let second = router
            .list_recent_completed_downloads(10)
            .await
            .expect("client in backoff should still produce partial empty results");

        assert!(first.is_empty());
        assert!(second.is_empty());
        assert_eq!(failing_client.recent_completed_call_count(), 1);
    }

    #[test]
    fn completed_download_parent_resolves_archive_suffixed_release_directory() {
        for release_name in ["Paperman.2012.7z", "Paperman.2012.zip", "Paperman.2012.rar"] {
            let root = tempfile::tempdir().unwrap();
            let release_dir = root.path().join(release_name);
            std::fs::create_dir(&release_dir).unwrap();
            let mut item = scryer_domain::CompletedDownload {
                client_type: "any-torrent-client".to_string(),
                client_id: "client-a".to_string(),
                download_client_item_id: "archive-1".to_string(),
                download_id: None,
                name: release_name.to_string(),
                dest_dir: root.path().to_string_lossy().into_owned(),
                category: None,
                size_bytes: None,
                completed_at: None,
                parameters: Vec::new(),
            };

            normalize_completed_download_import_dir(&mut item);

            assert_eq!(item.dest_dir, release_dir.to_string_lossy());
        }
    }

    #[test]
    fn completed_download_parent_does_not_follow_unsafe_or_missing_child_names() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "../escape",
            "nested/release",
            "nested\\release",
            "missing.7z",
        ] {
            let mut item = scryer_domain::CompletedDownload {
                client_type: "any-torrent-client".to_string(),
                client_id: "client-a".to_string(),
                download_client_item_id: "archive-1".to_string(),
                download_id: None,
                name: name.to_string(),
                dest_dir: root.path().to_string_lossy().into_owned(),
                category: None,
                size_bytes: None,
                completed_at: None,
                parameters: Vec::new(),
            };

            normalize_completed_download_import_dir(&mut item);

            assert_eq!(item.dest_dir, root.path().to_string_lossy());
        }
    }

    #[tokio::test]
    async fn torrent_client_completed_parent_is_resolved_before_import() {
        let root = tempfile::tempdir().unwrap();
        let release_name = "Paperman.2012.7z";
        let release_dir = root.path().join(release_name);
        std::fs::create_dir(&release_dir).unwrap();

        let client = Arc::new(MockDownloadClient::default());
        client
            .completed_downloads
            .lock()
            .unwrap()
            .push(scryer_domain::CompletedDownload {
                client_type: "torrent-client".to_string(),
                client_id: String::new(),
                download_client_item_id: "archive-1".to_string(),
                download_id: None,
                name: release_name.to_string(),
                dest_dir: root.path().to_string_lossy().into_owned(),
                category: None,
                size_bytes: None,
                completed_at: None,
                parameters: Vec::new(),
            });
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![("client-a".to_string(), client)],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("client-a", "Client A", "torrent-client", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router.list_completed_downloads().await.unwrap();

        assert_eq!(items[0].dest_dir, release_dir.to_string_lossy());
    }

    #[tokio::test]
    async fn list_completed_downloads_applies_remote_path_mappings_from_client_config() {
        let client = Arc::new(MockDownloadClient::default());
        client
            .completed_downloads
            .lock()
            .unwrap()
            .push(scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "remote-1".to_string(),
                download_id: None,
                name: "Remote Download".to_string(),
                dest_dir: "D:\\Data\\Completed\\Remote Download".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now()),
                parameters: Vec::new(),
            });

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![("client-a".to_string(), client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![DownloadClientConfig {
                    config_json:
                        r#"{"remote_path_mappings":"D:\\Data\\Completed => /Volumes/downloads"}"#
                            .to_string(),
                    ..test_config("client-a", "Client A", "qbittorrent", 0)
                }],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_completed_downloads()
            .await
            .expect("completed downloads should succeed");

        assert_eq!(items[0].client_id, "client-a");
        assert_eq!(items[0].dest_dir, "/Volumes/downloads/Remote Download");
    }

    #[tokio::test]
    async fn get_client_status_for_client_id_applies_remote_path_mappings_to_output_roots() {
        let client = Arc::new(MockDownloadClient::default());
        *client.status.lock().unwrap() = DownloadClientStatus {
            is_localhost: Some(false),
            remote_output_roots: vec!["/downloads/complete".to_string()],
            ..DownloadClientStatus::default()
        };

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![("client-a".to_string(), client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![DownloadClientConfig {
                    config_json: r#"{"remote_path_mappings":"/downloads => /Volumes/downloads"}"#
                        .to_string(),
                    ..test_config("client-a", "Client A", "qbittorrent", 0)
                }],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let status = router
            .get_client_status_for_client_id("client-a")
            .await
            .expect("client status should succeed");

        assert_eq!(
            status.remote_output_roots,
            vec!["/Volumes/downloads/complete".to_string()]
        );
    }

    #[tokio::test]
    async fn download_client_timeout_wrapper_times_out_feedback_reads() {
        let wrapped = FeedbackTimeoutDownloadClient::new(
            Arc::new(DelayedQueueDownloadClient {
                delay: Duration::from_millis(25),
                queue_items: vec![test_queue_item("slow")],
            }),
            Duration::from_millis(5),
        );

        let error = wrapped
            .list_queue()
            .await
            .expect_err("slow feedback reads should time out");

        assert!(matches!(
            error,
            AppError::DownloadFeedbackTimeout(ref message)
                if message == DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE
        ));
    }

    #[tokio::test]
    async fn download_client_timeout_wrapper_forwards_scoped_recent_completed_reads() {
        let inner = Arc::new(ScopedRecentCompletedDownloadClient::default());
        let wrapped = FeedbackTimeoutDownloadClient::new(inner.clone(), Duration::from_millis(100));
        let client_ids = vec!["default".to_string()];
        let client_types = vec!["qbittorrent".to_string()];

        let items = wrapped
            .list_recent_completed_downloads_for_client_scope(10, &client_ids, &client_types, &[])
            .await
            .expect("scoped recent completed reads should forward to the wrapped client");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].download_client_item_id, "qbit-1");
        let calls = inner.scoped_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, client_ids);
        assert_eq!(calls[0].1, client_types);
    }

    #[tokio::test]
    async fn list_queue_returns_partial_data_when_a_client_times_out() {
        let fast_client = Arc::new(MockDownloadClient::default());
        fast_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("fast"));

        let slow_client: Arc<dyn DownloadClient> = Arc::new(DelayedQueueDownloadClient {
            delay: Duration::from_millis(25),
            queue_items: vec![test_queue_item("slow")],
        });

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("fast".to_string(), fast_client.clone()),
                    ("slow".to_string(), slow_client),
                ],
            });

        let router = PrioritizedDownloadClientRouter::with_feedback_read_timeout(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("fast", "Fast", "qbittorrent", 0),
                    test_config("slow", "Slow", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
            Duration::from_millis(5),
        );

        let items = router
            .list_queue()
            .await
            .expect("partial data should still succeed when one client times out");

        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["fast".to_string()]);
    }

    #[tokio::test]
    async fn list_queue_returns_timeout_error_when_all_clients_time_out() {
        let slow_a: Arc<dyn DownloadClient> = Arc::new(DelayedQueueDownloadClient {
            delay: Duration::from_millis(25),
            queue_items: vec![test_queue_item("slow-a")],
        });
        let slow_b: Arc<dyn DownloadClient> = Arc::new(DelayedQueueDownloadClient {
            delay: Duration::from_millis(25),
            queue_items: vec![test_queue_item("slow-b")],
        });

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("slow-a".to_string(), slow_a),
                    ("slow-b".to_string(), slow_b),
                ],
            });

        let router = PrioritizedDownloadClientRouter::with_feedback_read_timeout(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("slow-a", "Slow A", "qbittorrent", 0),
                    test_config("slow-b", "Slow B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
            Duration::from_millis(5),
        );

        let error = router
            .list_queue()
            .await
            .expect_err("timeout-only outages should surface as typed timeout errors");

        assert!(matches!(
            error,
            AppError::DownloadFeedbackTimeout(ref message)
                if message == DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE
        ));
    }

    #[tokio::test]
    async fn list_queue_backs_off_after_feedback_failures() {
        let fast_client = Arc::new(MockDownloadClient::default());
        fast_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("fast"));
        let failing_client = Arc::new(FailingQueueDownloadClient::default());

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("fast".to_string(), fast_client.clone()),
                    ("failing".to_string(), failing_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("fast", "Fast", "qbittorrent", 0),
                    test_config("failing", "Failing", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let first = router
            .list_queue()
            .await
            .expect("first queue read should succeed");
        let second = router
            .list_queue()
            .await
            .expect("backed off queue read should succeed");

        assert_eq!(failing_client.list_queue_call_count(), 1);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].download_client_item_id, "fast");
        assert_eq!(second[0].download_client_item_id, "fast");
    }

    #[tokio::test]
    async fn list_queue_for_title_bypasses_feedback_backoff() {
        let failing_client = Arc::new(FailingQueueDownloadClient::default());

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("failing".to_string(), failing_client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("failing", "Failing", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let _ = router
            .list_queue()
            .await
            .expect("queue read should degrade to empty");
        let _ = router
            .list_queue_for_title("title-1")
            .await
            .expect("title-scoped queue read should bypass backoff");

        assert_eq!(failing_client.list_queue_call_count(), 1);
        assert_eq!(failing_client.list_queue_for_title_call_count(), 1);
    }
}
