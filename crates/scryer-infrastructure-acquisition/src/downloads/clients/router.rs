use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Component, Path};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::native_download_client_http_client;
use async_trait::async_trait;
use futures_util::StreamExt;
use scryer_application::challenge_solver as solver;
use scryer_application::transport_proxy;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest,
    DownloadClientCategorySnapshotStore, DownloadClientConfigRepository,
    DownloadClientFeedbackScope, DownloadClientListing, DownloadClientPluginProvider,
    DownloadClientRemotePathMapping, DownloadClientSnapshotOutcome, DownloadClientStatus,
    DownloadGrabResult, DownloadSourceKind, IndexerConfigRepository, IndexerPluginProvider,
    PersistedSeedGoals, ProxyConfigRepository, RateLimitCooldownAction, ResolvedDownloadArtifact,
    ResolvedSeedGoals, SeedGoalRequest, SeedGoalResolver, SeedingProfileRepository,
    SettingsRepository, StagedNzbRef, StagedNzbStore, accepted_inputs_for_client,
    apply_remote_path_mappings_to_completed_download, apply_remote_path_mappings_to_status,
    extract_magnet_info_hash, is_valid_magnet_uri, normalize_torrent_info_hash,
    parse_download_client_remote_path_mappings,
};
use scryer_domain::{DownloadClientConfig, DownloadQueueItem, MediaFacet, ProxyConfig};
use scryer_outbound_http::{
    AsyncOutboundHttpError, OutboundHttpClient, PluginEgressPolicy, RateLimitRegistry,
    generic_reqwest_client, prepare_plugin_http_target, prepare_plugin_http_target_from_url,
    proxy_reqwest_client, send_reqwest_request_with_cooldown_budget,
};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use super::nzbget::NzbgetDownloadClient;
use super::sabnzbd::SabnzbdDownloadClient;
use super::weaver::WeaverDownloadClient;
use super::{
    parse_download_client_config_json, read_config_string, request_source_hint_for_nzb,
    resolve_download_client_base_url, stage_nzb_from_bytes, stage_nzb_from_url,
};

const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
const DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS_ENV: &str =
    "SCRYER_DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS";
const DOWNLOAD_CLIENT_FEEDBACK_POLL_CONCURRENCY: usize = 4;
const PROXIED_TORRENT_FILE_MAX_BYTES: usize = 32 * 1024 * 1024;
const SOLVER_RESPONSE_MAX_BYTES: usize = PROXIED_TORRENT_FILE_MAX_BYTES * 2;

/// Who fetches an NZB URL during request preparation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArtifactFetch {
    /// The download client pulls the URL itself (the submit path).
    ClientSide,
    /// Scryer resolves the bytes so it holds the file (D17 browser downloads).
    HostSide,
}

pub fn download_client_feedback_timeout() -> Duration {
    static CACHED: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        let raw = std::env::var(DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS_ENV).ok();
        parse_download_client_feedback_timeout(
            raw.as_deref(),
            scryer_outbound_http::DEFAULT_DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT,
        )
    })
}

fn parse_download_client_feedback_timeout(raw: Option<&str>, default: Duration) -> Duration {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(default)
}

fn download_feedback_timeout_message(timeout: Duration) -> String {
    let elapsed = if timeout.subsec_nanos() == 0 {
        format!("{}s", timeout.as_secs())
    } else {
        format!("{}ms", timeout.as_millis())
    };
    format!("download feedback timed out after {elapsed}; queue status is temporarily unavailable")
}

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

fn hexadecimal_v1_magnet_info_hash(uri: &str) -> Option<String> {
    extract_magnet_info_hash(uri)
        .and_then(|hash| normalize_torrent_info_hash(Some(&hash)))
        .filter(|hash| hash.len() == 40)
}

fn resolved_magnet_info_hash_hint(info_hash_hint: Option<String>, uri: &str) -> Option<String> {
    hexadecimal_v1_magnet_info_hash(uri)
        .or(info_hash_hint)
        .or_else(|| extract_magnet_info_hash(uri))
}

fn preserves_v2_info_hash_hint(info_hash_hint: Option<&str>) -> bool {
    normalize_torrent_info_hash(info_hash_hint).is_some_and(|hash| hash.len() == 64)
}

fn resolved_v1_info_hash(request: &DownloadClientAddRequest) -> Option<String> {
    let info_hash_hint = match request.resolved_download_artifact.as_ref()? {
        ResolvedDownloadArtifact::Magnet { info_hash_hint, .. }
        | ResolvedDownloadArtifact::TorrentFile { info_hash_hint, .. } => info_hash_hint.as_deref(),
        ResolvedDownloadArtifact::Nzb { .. } => return None,
    };
    normalize_torrent_info_hash(info_hash_hint).filter(|hash| hash.len() == 40)
}

#[cfg(test)]
fn looks_like_torrent_metainfo(bytes: &[u8]) -> bool {
    torrent_info_hash_v1(bytes).is_some()
}

/// The BitTorrent v1 info hash is the SHA-1 of the raw bencoded `info`
/// dictionary. Never deserialize and re-encode it: bencode byte order is part
/// of the torrent identity.
fn torrent_info_hash_v1(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > PROXIED_TORRENT_FILE_MAX_BYTES {
        return None;
    }
    let (consumed, info_span) = parse_bencode_dict(bytes, 0, 0).ok()?;
    if consumed != bytes.len() {
        return None;
    }
    let (info_start, info_end) = info_span?;
    let digest = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY,
        &bytes[info_start..info_end],
    );
    Some(
        digest
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
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

fn parse_bencode_dict(
    bytes: &[u8],
    offset: usize,
    depth: usize,
) -> Result<(usize, Option<(usize, usize)>), ()> {
    if depth > 64 || bytes.get(offset) != Some(&b'd') {
        return Err(());
    }
    let mut cursor = offset + 1;
    let mut info_span = None;
    while cursor < bytes.len() && bytes[cursor] != b'e' {
        let (after_key, key_start, key_end) = parse_bencode_string(bytes, cursor)?;
        let is_top_level_info = depth == 0 && &bytes[key_start..key_end] == b"info";
        if is_top_level_info && bytes.get(after_key) != Some(&b'd') {
            return Err(());
        }
        let after_value = parse_bencode_value(bytes, after_key, depth + 1)?;
        if is_top_level_info && info_span.replace((after_key, after_value)).is_some() {
            return Err(());
        }
        cursor = after_value;
    }
    if cursor >= bytes.len() {
        return Err(());
    }
    Ok((cursor + 1, info_span))
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

fn target_rate_limit_error(headers: Option<&serde_json::Value>) -> AppError {
    let retry_after = solver::retry_after_from_solution_headers(headers);
    AppError::TemporaryUnavailable {
        message: solver::rate_limit_message_with_retry_after(retry_after),
        retry_after,
        rate_limit_cooldown: RateLimitCooldownAction::RecordFallback,
    }
}

#[derive(Debug)]
struct FetchedDownloadArtifact {
    bytes: Vec<u8>,
    headers: Option<serde_json::Value>,
    final_url: Option<String>,
}

/// How an artifact fetch leaves the host.
///
/// Solver-assigned indexers are `Direct` here too: a solver's own solve request
/// is a separate call, and its direct-first attempt and its post-solve replay
/// both dial the origin themselves.
#[derive(Clone, Copy, Debug)]
enum ArtifactFetchEgress<'a> {
    Direct,
    TransportProxy(&'a ProxyConfig),
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
    proxy_configs: Option<Arc<dyn ProxyConfigRepository>>,
    indexer_plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
    settings: Arc<dyn SettingsRepository>,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
    plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    /// Seeding-profile catalog for grab-time goal resolution. `None` leaves
    /// every grab without goals, i.e. today's behavior.
    seeding_profiles: Option<Arc<dyn SeedingProfileRepository>>,
    outbound_http: OutboundHttpClient,
    feedback_read_timeout: Duration,
    category_snapshot_store: DownloadClientCategorySnapshotStore,
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
            AppError::DownloadFeedbackTimeout(download_feedback_timeout_message(self.read_timeout))
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

    async fn list_queue_with_read_report(&self) -> AppResult<DownloadClientListing> {
        self.run_feedback_read(self.inner.list_queue_with_read_report())
            .await
    }

    async fn list_queue_with_feedback_scope(
        &self,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_queue_with_feedback_scope(scope))
            .await
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_queue_for_title(title_id))
            .await
    }

    async fn list_queue_for_title_with_feedback_scope(
        &self,
        title_id: &str,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(
            self.inner
                .list_queue_for_title_with_feedback_scope(title_id, scope),
        )
        .await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_history()).await
    }

    async fn list_history_with_read_report(&self) -> AppResult<DownloadClientListing> {
        self.run_feedback_read(self.inner.list_history_with_read_report())
            .await
    }

    async fn list_history_with_feedback_scope(
        &self,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_history_with_feedback_scope(scope))
            .await
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_history_page(offset, limit))
            .await
    }

    async fn list_history_page_with_feedback_scope(
        &self,
        offset: usize,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(
            self.inner
                .list_history_page_with_feedback_scope(offset, limit, scope),
        )
        .await
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_recent_activity(limit))
            .await
    }

    async fn list_recent_activity_with_feedback_scope(
        &self,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(
            self.inner
                .list_recent_activity_with_feedback_scope(limit, scope),
        )
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

    async fn list_recent_activity_for_title_with_feedback_scope(
        &self,
        title_id: &str,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(
            self.inner
                .list_recent_activity_for_title_with_feedback_scope(title_id, limit, scope),
        )
        .await
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.run_feedback_read(self.inner.list_completed_downloads())
            .await
    }

    async fn list_completed_downloads_with_feedback_scope(
        &self,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.run_feedback_read(
            self.inner
                .list_completed_downloads_with_feedback_scope(scope),
        )
        .await
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.run_feedback_read(self.inner.list_recent_completed_downloads(limit))
            .await
    }

    async fn list_recent_completed_downloads_with_feedback_scope(
        &self,
        limit: usize,
        scope: &DownloadClientFeedbackScope,
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.run_feedback_read(
            self.inner
                .list_recent_completed_downloads_with_feedback_scope(limit, scope),
        )
        .await
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

    async fn list_snapshot_outcome_excluding_client_types(
        &self,
        recent_activity_limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<DownloadClientSnapshotOutcome> {
        self.run_feedback_read(self.inner.list_snapshot_outcome_excluding_client_types(
            recent_activity_limit,
            excluded_client_types,
        ))
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
        self.run_feedback_read(
            self.inner
                .list_recent_completed_downloads_excluding_client_types(
                    limit,
                    excluded_client_types,
                ),
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
        self.run_feedback_read(self.inner.list_recent_completed_downloads_for_client_scope(
            limit,
            client_ids,
            client_types,
            excluded_client_types,
        ))
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

    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.inner
            .delete_queue_item(id, is_history, remove_data)
            .await
    }

    async fn delete_queue_item_for_client_id(
        &self,
        client_id: &str,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.inner
            .delete_queue_item_for_client_id(client_id, id, is_history, remove_data)
            .await
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.inner
            .delete_queue_item_for_client(client_type, id, is_history, remove_data)
            .await
    }

    async fn mark_imported(
        &self,
        request: &scryer_application::DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        self.inner.mark_imported(request).await
    }

    async fn mark_imported_non_destructive(
        &self,
        request: &scryer_application::DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        self.inner.mark_imported_non_destructive(request).await
    }

    async fn mark_imported_non_destructive_for_client_id(
        &self,
        client_id: &str,
        request: &scryer_application::DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        self.inner
            .mark_imported_non_destructive_for_client_id(client_id, request)
            .await
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
    timeout_message: Option<String>,
}

impl FeedbackReadSummary {
    fn record_success(&mut self) {
        self.successful_clients += 1;
    }

    fn record_error(&mut self, error: &AppError) {
        if let AppError::DownloadFeedbackTimeout(message) = error {
            self.timed_out_clients += 1;
            self.timeout_message.get_or_insert_with(|| message.clone());
        }
    }

    fn finish(self) -> AppResult<()> {
        if self.successful_clients == 0 && self.timed_out_clients > 0 {
            return Err(AppError::DownloadFeedbackTimeout(
                self.timeout_message.unwrap_or_else(|| {
                    download_feedback_timeout_message(download_client_feedback_timeout())
                }),
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
    all_clients: Vec<DownloadClientConfig>,
    disabled_scope: Option<DownloadClientRoutingScope>,
    routing: Option<ResolvedDownloadClientRouting>,
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
    seeding_profile_id: Option<String>,
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
            download_client_feedback_timeout(),
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
            proxy_configs: None,
            indexer_plugin_provider: None,
            settings,
            staged_nzb_store,
            staged_nzb_pipeline_limit,
            plugin_provider,
            seeding_profiles: None,
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            feedback_read_timeout,
            category_snapshot_store: DownloadClientCategorySnapshotStore::default(),
            feedback_read_backoff: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_indexer_config_repositories(
        mut self,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        proxy_configs: Arc<dyn ProxyConfigRepository>,
    ) -> Self {
        self.indexer_configs = Some(indexer_configs);
        self.proxy_configs = Some(proxy_configs);
        self
    }

    /// Enable indexer-owned grab resolution ahead of the router's direct and
    /// challenge-solver fallbacks. Indexers with no grab action return `None`
    /// and leave those fallbacks unchanged.
    pub fn with_indexer_plugin_provider(
        mut self,
        indexer_plugin_provider: Arc<dyn IndexerPluginProvider>,
    ) -> Self {
        self.indexer_plugin_provider = Some(indexer_plugin_provider);
        self
    }

    pub fn with_download_client_category_snapshot_store(
        mut self,
        store: DownloadClientCategorySnapshotStore,
    ) -> Self {
        self.category_snapshot_store = store;
        self
    }

    /// Turn on grab-time seeding-goal resolution. Order-independent with
    /// `with_indexer_config_repositories`: the resolver is built per grab from
    /// whatever repositories are wired at that point.
    pub fn with_seed_goal_resolution(
        mut self,
        seeding_profiles: Arc<dyn SeedingProfileRepository>,
    ) -> Self {
        self.seeding_profiles = Some(seeding_profiles);
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

    fn feedback_backoff_duration(
        consecutive_failures: u32,
        feedback_read_timeout: Duration,
        failed_read_elapsed: Duration,
    ) -> Duration {
        let maximum = Duration::from_secs(DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_MAX_SECS)
            .max(feedback_read_timeout);
        let mut delay = Duration::from_secs(DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_INITIAL_SECS);
        for _ in 1..consecutive_failures {
            delay = delay.saturating_mul(2).min(maximum);
        }
        delay.max(failed_read_elapsed.min(maximum))
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

    async fn poll_feedback_clients<T, F, Fut>(
        &self,
        clients: Vec<DownloadClientConfig>,
        read_kind: DownloadFeedbackReadKind,
        operation: &'static str,
        read: F,
    ) -> Vec<(DownloadClientConfig, Duration, AppResult<T>)>
    where
        T: Send,
        F: Fn(Arc<dyn DownloadClient>, DownloadClientFeedbackScope) -> Fut + Sync,
        Fut: Future<Output = AppResult<T>> + Send,
    {
        self.poll_feedback_clients_with_skips(clients, read_kind, operation, read)
            .await
            .0
    }

    /// Runs one feedback read against every client, returning the per-client
    /// results plus the ids of clients that were never asked (feedback backoff
    /// or a client that could not be built from its config). Callers deciding
    /// whether a download is *gone* need the skipped set: a client nobody
    /// asked has not said "no".
    async fn poll_feedback_clients_with_skips<T, F, Fut>(
        &self,
        clients: Vec<DownloadClientConfig>,
        read_kind: DownloadFeedbackReadKind,
        operation: &'static str,
        read: F,
    ) -> (
        Vec<(DownloadClientConfig, Duration, AppResult<T>)>,
        Vec<String>,
    )
    where
        T: Send,
        F: Fn(Arc<dyn DownloadClient>, DownloadClientFeedbackScope) -> Fut + Sync,
        Fut: Future<Output = AppResult<T>> + Send,
    {
        let category_snapshot = self.category_snapshot_store.snapshot().await;
        // Proxy assignments are resolved up front: the client build below runs
        // inside a synchronous closure and cannot await, and an unresolvable
        // assignment has to skip the client rather than read it unproxied.
        let mut proxy_configs_by_client: HashMap<String, AppResult<Option<ProxyConfig>>> =
            HashMap::new();
        for config in &clients {
            proxy_configs_by_client.insert(
                config.id.clone(),
                self.proxy_for_download_client(config).await,
            );
        }
        let mut skipped_client_ids = Vec::new();
        let reads = clients
            .into_iter()
            .enumerate()
            .filter_map(|(index, config)| {
                if let Some(remaining) = self.feedback_backoff_remaining(&config.id, read_kind) {
                    debug!(
                        client_id = %config.id,
                        client = %config.name,
                        read_kind = Self::feedback_read_kind_label(read_kind),
                        remaining_ms = remaining.as_millis(),
                        "skipping download client feedback read during backoff"
                    );
                    skipped_client_ids.push(config.id.clone());
                    return None;
                }

                let proxy_config = match proxy_configs_by_client.get(&config.id) {
                    Some(Ok(proxy_config)) => proxy_config.clone(),
                    Some(Err(error)) => {
                        tracing::warn!(
                            client_id = %config.id,
                            error = %error,
                            operation,
                            "skipping client for feedback read: assigned proxy is unusable"
                        );
                        return None;
                    }
                    None => None,
                };
                let client = match Self::client_from_config(
                    &config,
                    self.staged_nzb_store.clone(),
                    self.staged_nzb_pipeline_limit.clone(),
                    self.plugin_provider.as_ref(),
                    self.feedback_read_timeout,
                    proxy_config.as_ref(),
                ) {
                    Ok(client) => client,
                    Err(error) => {
                        tracing::warn!(
                            client_id = %config.id,
                            error = %error,
                            operation,
                            "skipping client for feedback read"
                        );
                        skipped_client_ids.push(config.id.clone());
                        return None;
                    }
                };
                let scope = category_snapshot
                    .as_deref()
                    .map(|snapshot| snapshot.feedback_scope_for_client(&config.id))
                    .unwrap_or_default();
                let read = read(client, scope);
                Some(async move {
                    let started_at = Instant::now();
                    let result = read.await;
                    (index, config, started_at.elapsed(), result)
                })
            });

        let mut reads = futures_util::stream::iter(reads)
            .buffer_unordered(DOWNLOAD_CLIENT_FEEDBACK_POLL_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        reads.sort_unstable_by_key(|(index, ..)| *index);
        let reads = reads
            .into_iter()
            .map(|(_, config, elapsed, result)| (config, elapsed, result))
            .collect();
        (reads, skipped_client_ids)
    }

    /// The shared body of the aggregate queue/history reads: poll every
    /// client, stamp items with the owning client, and report which clients
    /// did not answer. Only an all-clients timeout is an error; every other
    /// failure degrades that client to an empty contribution, so the
    /// `unreadable_client_ids` in the listing are the only trace of it.
    async fn poll_feedback_listing<F, Fut>(
        &self,
        clients: Vec<DownloadClientConfig>,
        read_kind: DownloadFeedbackReadKind,
        operation: &'static str,
        read: F,
    ) -> AppResult<DownloadClientListing>
    where
        F: Fn(Arc<dyn DownloadClient>, DownloadClientFeedbackScope) -> Fut + Sync,
        Fut: Future<Output = AppResult<Vec<DownloadQueueItem>>> + Send,
    {
        let mut listing = DownloadClientListing {
            polled_client_count: clients.len(),
            ..DownloadClientListing::default()
        };
        if clients.is_empty() {
            return Ok(listing);
        }
        let mut read_summary = FeedbackReadSummary::default();
        let (reads, skipped_client_ids) = self
            .poll_feedback_clients_with_skips(clients, read_kind, operation, read)
            .await;
        listing.unreadable_client_ids.extend(skipped_client_ids);
        for (config, elapsed, result) in reads {
            match result {
                Ok(mut items) => {
                    self.record_feedback_read_success(&config.id, read_kind);
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    listing.items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(&config.id, read_kind, elapsed);
                    read_summary.record_error(&error);
                    listing.unreadable_client_ids.push(config.id.clone());
                    match read_kind {
                        DownloadFeedbackReadKind::Queue => {
                            tracing::warn!(client_id = %config.id, error = %error, "failed to list queue");
                        }
                        DownloadFeedbackReadKind::History => {
                            tracing::warn!(client_id = %config.id, error = %error, "failed to list history");
                        }
                        _ => {
                            tracing::warn!(client_id = %config.id, error = %error, operation, "download client feedback read failed");
                        }
                    }
                }
            }
        }
        read_summary.finish()?;
        Ok(listing)
    }

    async fn queue_listing_excluding_client_types(
        &self,
        excluded_client_types: &[&str],
    ) -> AppResult<DownloadClientListing> {
        let clients = self
            .list_enabled_clients_by_priority_excluding(excluded_client_types)
            .await?;
        self.poll_feedback_listing(
            clients,
            DownloadFeedbackReadKind::Queue,
            "queue listing",
            |client, scope| async move { client.list_queue_with_feedback_scope(&scope).await },
        )
        .await
    }

    async fn history_listing(&self) -> AppResult<DownloadClientListing> {
        let clients = self.list_enabled_clients_by_priority().await?;
        self.poll_feedback_listing(
            clients,
            DownloadFeedbackReadKind::History,
            "history listing",
            |client, scope| async move { client.list_history_with_feedback_scope(&scope).await },
        )
        .await
    }

    fn record_feedback_read_success(&self, client_id: &str, kind: DownloadFeedbackReadKind) {
        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        backoff.remove(&(client_id.to_string(), kind));
    }

    fn record_feedback_read_failure(
        &self,
        client_id: &str,
        kind: DownloadFeedbackReadKind,
        failed_read_elapsed: Duration,
    ) {
        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        let key = (client_id.to_string(), kind);
        let failures = backoff
            .get(&key)
            .map(|state| state.consecutive_failures.saturating_add(1))
            .unwrap_or(1);
        let delay = Self::feedback_backoff_duration(
            failures,
            self.feedback_read_timeout,
            failed_read_elapsed,
        );
        backoff.insert(
            key,
            FeedbackReadBackoffState {
                consecutive_failures: failures,
                blocked_until: Instant::now() + delay,
            },
        );
    }

    async fn list_enabled_clients_by_priority(&self) -> AppResult<Vec<DownloadClientConfig>> {
        let all_clients = self.download_client_configs.list(None).await?;
        let mut clients = all_clients
            .iter()
            .filter(|config| config.is_enabled)
            .cloned()
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

    async fn load_indexer_config_for_submission(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<Option<scryer_domain::IndexerConfig>> {
        let Some(indexer_id) = request
            .indexer_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let Some(indexer_configs) = self.indexer_configs.as_ref() else {
            return Err(AppError::download_submit_unavailable(format!(
                "indexer {indexer_id} routing is unavailable: indexer configuration repository is not wired"
            )));
        };
        let indexer = indexer_configs.get_by_id(indexer_id).await.map_err(|error| {
            AppError::download_submit_unavailable(format!(
                "indexer {indexer_id} routing is unavailable: failed to load indexer configuration: {error}"
            ))
        })?;
        indexer
            .ok_or_else(|| {
                AppError::download_submit_unavailable(format!(
                    "indexer {indexer_id} routing is unavailable: indexer configuration was not found"
                ))
            })
            .map(Some)
    }

    async fn prepare_download_request(
        &self,
        request: &DownloadClientAddRequest,
        indexer: Option<&scryer_domain::IndexerConfig>,
    ) -> AppResult<DownloadClientAddRequest> {
        self.prepare_download_request_with(request, indexer, ArtifactFetch::ClientSide)
            .await
    }

    /// `prepare_download_request` with the NZB fetch policy made explicit.
    ///
    /// `ClientSide` is the submit path's historical behaviour: an NZB URL is
    /// handed to the download client untouched. `HostSide` (D17) makes Scryer
    /// resolve the bytes itself, which is the only way to put the file in the
    /// operator's browser.
    async fn prepare_download_request_with(
        &self,
        request: &DownloadClientAddRequest,
        indexer: Option<&scryer_domain::IndexerConfig>,
        artifact_fetch: ArtifactFetch,
    ) -> AppResult<DownloadClientAddRequest> {
        if request.resolved_download_artifact.is_some() {
            return Ok(request.clone());
        }
        let download_url = request
            .source_hint
            .as_deref()
            .map(str::trim)
            .unwrap_or_default();
        if download_url.is_empty() {
            return Ok(request.clone());
        }
        if is_valid_magnet_uri(download_url) {
            let uri = download_url.to_string();
            let resolved_v1_info_hash = hexadecimal_v1_magnet_info_hash(&uri);
            let mut prepared = request.clone();
            prepared.resolved_download_artifact = Some(ResolvedDownloadArtifact::Magnet {
                info_hash_hint: resolved_v1_info_hash
                    .clone()
                    .or_else(|| request.info_hash_hint.clone())
                    .or_else(|| extract_magnet_info_hash(&uri)),
                uri: uri.clone(),
            });
            prepared.source_kind = Some(DownloadSourceKind::MagnetUri);
            prepared.source_hint = Some(uri);
            prepared.info_hash_hint =
                resolved_v1_info_hash.or_else(|| request.info_hash_hint.clone());
            return Ok(prepared);
        }

        // NZBs do not need host-side fetching unless an assigned challenge
        // solver owns the indexer URL. Unlabelled HTTP is conservatively
        // treated as a torrent artifact, matching the adapter's historical
        // default and ensuring download clients never fetch arbitrary URLs.
        let has_proxy = indexer.and_then(|value| value.proxy_config_id.as_deref());
        if artifact_fetch == ArtifactFetch::ClientSide
            && has_proxy.is_none()
            && (request.staged_nzb.is_some()
                || matches!(
                    request.source_kind,
                    Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl)
                ))
        {
            return Ok(request.clone());
        }
        // The operator typed the indexer's base URL, so an artifact served from
        // that exact origin may sit on a rootless container's host bridge.
        let egress_policy = indexer
            .map(|indexer| PluginEgressPolicy::for_operator_configured_url(&indexer.base_url))
            .unwrap_or_default();
        // Resolve the assigned solver's proxy first: both the origin guard and
        // the indexer-owned grab below need it, and a misconfigured proxy must
        // fail the same way whichever path ends up resolving the artifact.
        let proxy_config = if let Some((indexer, proxy_config_id)) = indexer.zip(has_proxy) {
            let Some(proxy_configs) = self.proxy_configs.as_ref() else {
                return Err(AppError::download_submit_unavailable(format!(
                    "indexer {} routing is unavailable: proxy repository is not wired",
                    indexer.id
                )));
            };
            let proxy_config = proxy_configs
                .get_by_id(proxy_config_id)
                .await
                .map_err(|error| {
                    AppError::download_submit_unavailable(format!(
                        "indexer {} routing is unavailable: failed to load proxy configuration: {error}",
                        indexer.id
                    ))
                })?
                .ok_or_else(|| {
                    AppError::download_submit_unavailable(format!(
                        "indexer {} routing is unavailable: proxy configuration {} was not found",
                        indexer.id, proxy_config_id
                    ))
                })?;
            if !proxy_config.is_enabled {
                return Err(AppError::download_submit_unavailable(format!(
                    "indexer {} routing is unavailable: assigned proxy {} is disabled",
                    indexer.id, proxy_config_id
                )));
            }
            if !Self::download_url_matches_indexer_origin(indexer, download_url) {
                return Err(AppError::Validation(
                    "Proxied download URL does not match the assigned indexer origin.".into(),
                ));
            }
            Some(proxy_config)
        } else {
            None
        };

        // An indexer that owns an authenticated grab flow resolves the artifact
        // itself: a private tracker will not serve the file to a bare fetch, and
        // the plugin already holds that session. Providers without such a flow
        // answer `None` and leave the paths below untouched.
        if let (Some(indexer), Some(provider)) = (indexer, self.indexer_plugin_provider.as_ref())
            && let Some(client) =
                provider.client_for_provider_with_proxy(indexer, proxy_config.as_ref())
            && let Some(artifact) = client.resolve_download(download_url).await?
        {
            return self.prepare_resolved_request(request, artifact);
        }

        let Some(proxy_config) = proxy_config else {
            // No indexer, or an indexer without an assigned solver: fetch the
            // artifact directly and classify it.
            let fetched = self
                .fetch_download_artifact_direct(
                    "indexer",
                    download_url,
                    &[],
                    scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
                    &egress_policy,
                )
                .await?;
            return self.prepare_resolved_request(
                request,
                Self::classify_resolved_download_artifact(
                    "indexer",
                    fetched.final_url.as_deref(),
                    fetched.headers.as_ref(),
                    fetched.bytes,
                    request.info_hash_hint.clone(),
                )?,
            );
        };
        let artifact_result = if !proxy_config.is_challenge_solver() {
            // A transport proxy solves nothing: this is the direct fetch above,
            // dialled through the operator's proxy instead of straight out. A
            // tunnel takes the same arm and fails there until its engine
            // exists, rather than being handed to the solver path or fetched
            // directly.
            self.resolve_download_artifact_via_transport_proxy(
                &proxy_config,
                download_url,
                request.info_hash_hint.clone(),
                &egress_policy,
            )
            .await
        } else {
            self.resolve_download_artifact_via_proxy(
                &proxy_config,
                download_url,
                request.info_hash_hint.clone(),
                &egress_policy,
            )
            .await
        };
        if let Some(repo) = self.proxy_configs.as_ref() {
            solver::flush_solver_health(repo.as_ref()).await;
        }
        let artifact = artifact_result?;

        self.prepare_resolved_request(request, artifact)
    }

    /// Fetch and classify a download artifact through an assigned transport
    /// proxy. Same fetch, same classification, same redirect and size rules as
    /// the unproxied arm — only the egress client differs.
    async fn resolve_download_artifact_via_transport_proxy(
        &self,
        proxy_config: &ProxyConfig,
        download_url: &str,
        info_hash_hint: Option<String>,
        egress_policy: &PluginEgressPolicy,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let provider_name = solver::solver_provider_name(proxy_config.provider_type);
        let fetched = self
            .fetch_download_artifact(
                provider_name,
                download_url,
                &[],
                scryer_outbound_http::effective_proxy_request_timeout(
                    proxy_config.request_timeout_seconds,
                ),
                egress_policy,
                ArtifactFetchEgress::TransportProxy(proxy_config),
            )
            .await?;
        transport_proxy::record_transport_proxy_success(proxy_config);
        Self::classify_resolved_download_artifact(
            provider_name,
            fetched.final_url.as_deref(),
            fetched.headers.as_ref(),
            fetched.bytes,
            info_hash_hint,
        )
    }

    fn prepare_resolved_request(
        &self,
        request: &DownloadClientAddRequest,
        artifact: ResolvedDownloadArtifact,
    ) -> AppResult<DownloadClientAddRequest> {
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

    async fn resolve_download_artifact_via_proxy(
        &self,
        proxy_config: &ProxyConfig,
        download_url: &str,
        info_hash_hint: Option<String>,
        egress_policy: &PluginEgressPolicy,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let provider = proxy_config.provider_type;
        let provider_name = solver::solver_provider_name(provider);

        // Validate before delegating to the solver as well as before any direct
        // retry. Otherwise the solver itself becomes a deputy for blocked
        // link-local or cloud-metadata destinations.
        drop(
            prepare_plugin_http_target(download_url, "indexer download artifact", egress_policy)
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
                scryer_outbound_http::effective_proxy_request_timeout(
                    proxy_config.request_timeout_seconds,
                ),
                egress_policy,
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
        let solver_timeout = scryer_outbound_http::effective_proxy_request_timeout(
            proxy_config.request_timeout_seconds,
        );
        let solver_deadline = tokio::time::Instant::now() + solver_timeout;
        let response = tokio::time::timeout_at(
            solver_deadline,
            send_reqwest_request_with_cooldown_budget(
                proxy_reqwest_client()
                    .post(endpoint)
                    .timeout(solver_timeout)
                    .json(&solver::solver_solve_request(
                        provider,
                        download_url,
                        proxy_config.request_timeout_seconds,
                    )),
                Some(Duration::ZERO),
            ),
        )
        .await
        .map_err(|_| {
            let message = solver::solver_error_message(provider, solver::SolverErrorKind::Timeout);
            solver::SolverHealthLedger::shared().record_failure(&proxy_config.id, message);
            AppError::DownloadSubmitUnavailable(message.into())
        })?
        .map_err(|error| {
            let kind = match error {
                AsyncOutboundHttpError::Request(error) if error.is_timeout() => {
                    solver::SolverErrorKind::Timeout
                }
                AsyncOutboundHttpError::Request(_) => solver::SolverErrorKind::Unreachable,
                AsyncOutboundHttpError::CooldownBudgetExceeded { .. } => {
                    solver::SolverErrorKind::Unavailable
                }
            };
            let message = solver::solver_error_message(provider, kind);
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
        let body = match tokio::time::timeout_at(
            solver_deadline,
            read_response_body_bounded(response, SOLVER_RESPONSE_MAX_BYTES),
        )
        .await
        {
            Err(_) => {
                let message =
                    solver::solver_error_message(provider, solver::SolverErrorKind::Timeout);
                solver::SolverHealthLedger::shared().record_failure(&proxy_config.id, message);
                return Err(AppError::DownloadSubmitUnavailable(message.into()));
            }
            Ok(Ok(body)) => body,
            Ok(Err(BoundedResponseBodyError::Read(error))) => {
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
            Ok(Err(BoundedResponseBodyError::TooLarge)) => {
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
                    scryer_outbound_http::effective_proxy_request_timeout(
                        proxy_config.request_timeout_seconds,
                    ),
                    egress_policy,
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
                    scryer_outbound_http::effective_proxy_request_timeout(
                        proxy_config.request_timeout_seconds,
                    ),
                    egress_policy,
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
                        scryer_outbound_http::effective_proxy_request_timeout(
                            proxy_config.request_timeout_seconds,
                        ),
                        egress_policy,
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
        request_timeout: Duration,
        egress_policy: &PluginEgressPolicy,
    ) -> AppResult<FetchedDownloadArtifact> {
        self.fetch_download_artifact(
            provider_name,
            download_url,
            session_headers,
            request_timeout,
            egress_policy,
            ArtifactFetchEgress::Direct,
        )
        .await
    }

    /// The artifact fetch, parameterized on how it leaves the host.
    ///
    /// `Direct` resolves and pins every hop through the guarded plugin target.
    /// `TransportProxy` dials the operator's proxy instead: the destination is
    /// validated syntactically but not resolved here, because with `remote_dns`
    /// that name belongs to the proxy and may not resolve locally at all. The
    /// caller has already confirmed the URL matches the assigned indexer's
    /// origin (`download_url_matches_indexer_origin`).
    async fn fetch_download_artifact(
        &self,
        provider_name: &str,
        download_url: &str,
        session_headers: &[(String, String)],
        request_timeout: Duration,
        egress_policy: &PluginEgressPolicy,
        egress: ArtifactFetchEgress<'_>,
    ) -> AppResult<FetchedDownloadArtifact> {
        let original = url::Url::parse(download_url)
            .map_err(|_| AppError::Validation("Download artifact URL is invalid.".into()))?;
        let deadline = Instant::now() + request_timeout;
        let origin_scheme = original.scheme().to_string();
        let origin_host = original.host_str().map(str::to_string);
        let origin_port = original.port_or_known_default();
        let mut current = original;
        let mut visited = HashSet::from([current.clone()]);
        for redirect_count in 0..=5 {
            if current.scheme().eq_ignore_ascii_case("magnet")
                && is_valid_magnet_uri(current.as_str())
            {
                return Ok(FetchedDownloadArtifact {
                    bytes: Vec::new(),
                    headers: None,
                    final_url: Some(current.to_string()),
                });
            }
            if !matches!(current.scheme(), "http" | "https") {
                return Err(AppError::Validation(
                    "Download artifact redirects must use HTTP(S) or magnet URLs.".into(),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let (hop_client, hop_url) = match egress {
                ArtifactFetchEgress::Direct => {
                    let target = timeout(
                        remaining,
                        prepare_plugin_http_target_from_url(
                            current.clone(),
                            "indexer download artifact",
                            egress_policy,
                        ),
                    )
                    .await
                    .map_err(|_| {
                        AppError::DownloadSubmitUnavailable(
                            "The download artifact fetch timed out.".into(),
                        )
                    })?
                    .map_err(|error| {
                        warn!(error = %error, "blocked unsafe indexer download artifact URL");
                        AppError::DownloadSubmitUnavailable(
                            "Scryer refused an unsafe download artifact destination.".into(),
                        )
                    })?;
                    (target.client().clone(), target.url().clone())
                }
                ArtifactFetchEgress::TransportProxy(proxy_config) => {
                    let url = scryer_outbound_http::validate_operator_http_url(
                        current.as_str(),
                        "indexer download artifact",
                    )
                    .map_err(|error| {
                        warn!(error = %error, "blocked unsafe indexer download artifact URL");
                        AppError::DownloadSubmitUnavailable(
                            "Scryer refused an unsafe download artifact destination.".into(),
                        )
                    })?;
                    let client =
                        transport_proxy::transport_proxied_reqwest_client(proxy_config, "")
                            .map_err(|message| {
                                transport_proxy::record_transport_proxy_failure(
                                    proxy_config,
                                    &message,
                                );
                                AppError::DownloadSubmitUnavailable(message)
                            })?;
                    (client, url)
                }
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let mut builder = hop_client.get(hop_url.clone()).timeout(remaining);
            let same_origin = origin_scheme == hop_url.scheme()
                && origin_host
                    .as_deref()
                    .zip(hop_url.host_str())
                    .is_some_and(|(original, target)| original.eq_ignore_ascii_case(target))
                && origin_port == hop_url.port_or_known_default();
            if same_origin {
                for (name, value) in session_headers {
                    builder = builder.header(name, value);
                }
            }
            let response = timeout(
                remaining,
                send_reqwest_request_with_cooldown_budget(builder, Some(Duration::ZERO)),
            )
                .await
                .map_err(|_| {
                    AppError::DownloadSubmitUnavailable(
                        "The download artifact fetch timed out.".into(),
                    )
                })?
                .map_err(|error| {
                debug!(
                    proxy_provider = provider_name,
                    is_timeout = matches!(&error, AsyncOutboundHttpError::Request(value) if value.is_timeout()),
                    "download artifact fetch failed"
                );
                match error {
                    AsyncOutboundHttpError::CooldownBudgetExceeded { remaining, .. } => {
                        AppError::TemporaryUnavailable {
                            message: "The download artifact destination is rate limited.".into(),
                            retry_after: Some(remaining),
                            rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
                        }
                    }
                    AsyncOutboundHttpError::Request(error) if error.is_timeout() => {
                        AppError::DownloadSubmitUnavailable("The download artifact fetch timed out.".into())
                    }
                    // A connector failure on a proxied hop is the proxy's, not
                    // the indexer's, and says so by name.
                    AsyncOutboundHttpError::Request(error)
                        if matches!(egress, ArtifactFetchEgress::TransportProxy(_)) =>
                    {
                        let ArtifactFetchEgress::TransportProxy(proxy_config) = egress else {
                            unreachable!("guarded by the match arm")
                        };
                        match transport_proxy::transport_proxy_connect_failure(proxy_config, &error)
                        {
                            Some(message) => {
                                transport_proxy::record_transport_proxy_failure(
                                    proxy_config,
                                    &message,
                                );
                                AppError::DownloadSubmitUnavailable(message)
                            }
                            None => AppError::DownloadSubmitUnavailable(
                                "Scryer could not fetch the download artifact.".into(),
                            ),
                        }
                    }
                    AsyncOutboundHttpError::Request(_) => AppError::DownloadSubmitUnavailable(
                        "Scryer could not fetch the download artifact.".into(),
                    ),
                }
            })?;
            if response.status().is_redirection() {
                if !matches!(
                    response.status(),
                    reqwest::StatusCode::MOVED_PERMANENTLY
                        | reqwest::StatusCode::FOUND
                        | reqwest::StatusCode::SEE_OTHER
                        | reqwest::StatusCode::TEMPORARY_REDIRECT
                        | reqwest::StatusCode::PERMANENT_REDIRECT
                ) {
                    return Err(AppError::Validation(format!(
                        "Download artifact returned unsupported HTTP redirect {}.",
                        response.status().as_u16()
                    )));
                }
                if redirect_count == 5 {
                    return Err(AppError::Validation(
                        "The download artifact exceeded the redirect limit.".into(),
                    ));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "Download artifact redirect was missing a location.".into(),
                        )
                    })?;
                let next = if location
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("magnet:")
                {
                    let magnet = url::Url::parse(location.trim()).map_err(|_| {
                        AppError::Validation(
                            "Download artifact magnet redirect was invalid.".into(),
                        )
                    })?;
                    if !is_valid_magnet_uri(magnet.as_str()) {
                        return Err(AppError::Validation(
                            "Download artifact magnet redirect was invalid.".into(),
                        ));
                    }
                    magnet
                } else {
                    current.join(location).map_err(|_| {
                        AppError::Validation(
                            "Download artifact redirect location was invalid.".into(),
                        )
                    })?
                };
                if !visited.insert(next.clone()) {
                    return Err(AppError::Validation(
                        "The download artifact redirect looped.".into(),
                    ));
                }
                current = next;
                continue;
            }
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
                let status = response.status();
                return Err(
                    if matches!(
                        status,
                        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
                    ) {
                        AppError::DownloadSourceGone(format!(
                            "The download artifact is no longer available (HTTP {}).",
                            status.as_u16()
                        ))
                    } else if matches!(
                        status,
                        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
                    ) {
                        AppError::DownloadSubmitUnavailable(format!(
                            "The download artifact fetch was rejected by indexer '{provider_name}'; check its credentials (HTTP {}).",
                            status.as_u16()
                        ))
                    } else {
                        AppError::DownloadSubmitUnavailable(format!(
                            "The download artifact fetch returned HTTP {}.",
                            status.as_u16()
                        ))
                    },
                );
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
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let bytes = match timeout(
                remaining,
                read_response_body_bounded(response, PROXIED_TORRENT_FILE_MAX_BYTES),
            )
            .await
            {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(BoundedResponseBodyError::Read(error))) => {
                    debug!(
                        is_timeout = error.is_timeout(),
                        "failed to read download artifact body"
                    );
                    return Err(AppError::DownloadSubmitUnavailable(
                        "Scryer could not read the download artifact.".into(),
                    ));
                }
                Ok(Err(BoundedResponseBodyError::TooLarge)) => {
                    return Err(AppError::DownloadSubmitUnavailable(format!(
                        "The resolved download artifact exceeded Scryer's {} MiB limit.",
                        PROXIED_TORRENT_FILE_MAX_BYTES / (1024 * 1024)
                    )));
                }
                Err(_) => {
                    return Err(AppError::DownloadSubmitUnavailable(
                        "The download artifact fetch timed out.".into(),
                    ));
                }
            };
            return Ok(FetchedDownloadArtifact {
                bytes,
                headers,
                final_url,
            });
        }
        Err(AppError::Validation(
            "The download artifact redirect loop was exhausted.".into(),
        ))
    }

    fn classify_resolved_download_artifact(
        provider_name: &str,
        final_url: Option<&str>,
        headers: Option<&serde_json::Value>,
        bytes: Vec<u8>,
        info_hash_hint: Option<String>,
    ) -> AppResult<ResolvedDownloadArtifact> {
        if final_url.is_some_and(is_valid_magnet_uri) {
            let uri = final_url.unwrap().trim().to_string();
            return Ok(ResolvedDownloadArtifact::Magnet {
                info_hash_hint: resolved_magnet_info_hash_hint(info_hash_hint, &uri),
                uri,
            });
        }
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let trimmed = text.trim();
            if is_valid_magnet_uri(trimmed) {
                let uri = trimmed.to_string();
                return Ok(ResolvedDownloadArtifact::Magnet {
                    info_hash_hint: resolved_magnet_info_hash_hint(info_hash_hint, &uri),
                    uri,
                });
            }
        }

        let torrent_info_hash_v1 = torrent_info_hash_v1(&bytes);
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
                || value.contains("application/octet-stream") && torrent_info_hash_v1.is_some()
        }) || final_path
            .as_deref()
            .is_some_and(|path| path.ends_with(".torrent"))
            || file_name_lower
                .as_deref()
                .is_some_and(|name| name.ends_with(".torrent"))
            || torrent_info_hash_v1.is_some()
        {
            if torrent_info_hash_v1.is_none() {
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
                info_hash_hint: if preserves_v2_info_hash_hint(info_hash_hint.as_deref()) {
                    info_hash_hint
                } else {
                    torrent_info_hash_v1.or(info_hash_hint)
                },
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
            seeding_profile_id: Self::read_trimmed_string(
                config
                    .get("seedingProfileId")
                    .or_else(|| config.get("seeding_profile_id")),
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

        let all_clients = self.download_client_configs.list(None).await?;
        let mut clients = all_clients
            .iter()
            .filter(|config| config.is_enabled)
            .cloned()
            .collect::<Vec<_>>();
        let any_globally_enabled = !clients.is_empty();
        let mut disabled_scope = None;

        match resolved_routing.as_ref() {
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
            all_clients,
            disabled_scope,
            routing: resolved_routing,
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

    /// The single grab-time choke point: the routing entry for the client this
    /// grab was actually routed to decides category and queue priority, and —
    /// since it also carries the routing-level seeding profile — this is the
    /// only place with everything the seeding-goal resolver needs. Resolving
    /// here rather than at each add-request construction site keeps the
    /// precedence rules in one place and makes the routing level meaningful.
    async fn apply_selected_client_routing(
        &self,
        request: &DownloadClientAddRequest,
        client_id: &str,
    ) -> AppResult<(DownloadClientAddRequest, ResolvedSeedGoals)> {
        let mut effective_request = request.clone();
        let routing_entry = self
            .routing_entry_for_client(&request.title, client_id)
            .await?;

        effective_request.category = routing_entry
            .as_ref()
            .and_then(|entry| entry.category.clone())
            .or_else(|| Self::normalized_request_category(request));
        let routing_seeding_profile_id = routing_entry
            .as_ref()
            .and_then(|entry| entry.seeding_profile_id.clone());

        let is_recent = request.is_recent.unwrap_or(false);
        effective_request.queue_priority = routing_entry.and_then(|entry| {
            if is_recent {
                entry.recent_queue_priority
            } else {
                entry.older_queue_priority
            }
        });

        let seed_goals = self
            .resolve_seed_goals(request, client_id, routing_seeding_profile_id)
            .await;
        if seed_goals.is_resolved() {
            effective_request.seed_goal_ratio = seed_goals.seed_goal_ratio;
            effective_request.seed_goal_seconds = seed_goals.seed_goal_seconds;
        }

        Ok((effective_request, seed_goals))
    }

    /// Torrent-only, and never fatal: an unreadable profile catalog leaves the
    /// grab without goals instead of failing a download that would otherwise
    /// have succeeded.
    async fn resolve_seed_goals(
        &self,
        request: &DownloadClientAddRequest,
        client_id: &str,
        routing_seeding_profile_id: Option<String>,
    ) -> ResolvedSeedGoals {
        let Some(seeding_profiles) = self.seeding_profiles.clone() else {
            return ResolvedSeedGoals::default();
        };
        if !matches!(
            Self::request_source_kind(request),
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
        ) {
            return ResolvedSeedGoals::default();
        }

        let resolver = SeedGoalResolver::new(
            seeding_profiles,
            self.indexer_configs.clone(),
            self.settings.clone(),
        );
        let goal_request = SeedGoalRequest {
            indexer_id: request.indexer_id.clone(),
            routing_seeding_profile_id,
            season_pack: request.season_pack.unwrap_or(false),
            tracker_min_seed_ratio: request.tracker_min_seed_ratio,
            tracker_min_seed_time_minutes: request.tracker_min_seed_time_minutes,
            season_pack_min_seed_ratio: request.season_pack_seed_ratio,
            season_pack_min_seed_time_minutes: request.season_pack_seed_time_minutes,
        };
        match resolver.resolve(&goal_request).await {
            Ok(resolved) => {
                if resolved.is_resolved() {
                    info!(
                        client_id,
                        indexer_id = request.indexer_id.as_deref().unwrap_or(""),
                        seeding_profile_id = resolved.seeding_profile_id.as_deref().unwrap_or(""),
                        resolution_source = resolved.resolution_source.as_str(),
                        seed_goal_ratio = resolved.seed_goal_ratio.unwrap_or(0.0),
                        seed_goal_seconds = resolved.seed_goal_seconds.unwrap_or(0),
                        season_pack = request.season_pack.unwrap_or(false),
                        never_remove = resolved.never_remove,
                        "resolved torrent seeding goals"
                    );
                }
                resolved
            }
            Err(error) => {
                warn!(
                    client_id,
                    indexer_id = request.indexer_id.as_deref().unwrap_or(""),
                    error = %error,
                    "seeding goal resolution failed; submitting without seeding goals"
                );
                ResolvedSeedGoals::default()
            }
        }
    }

    fn persisted_seed_goals(
        request: &DownloadClientAddRequest,
        grab: &DownloadGrabResult,
        seed_goals: &ResolvedSeedGoals,
    ) -> Option<PersistedSeedGoals> {
        if !seed_goals.is_resolved() {
            return None;
        }
        Some(PersistedSeedGoals {
            seeding_profile_id: seed_goals.seeding_profile_id.clone(),
            seed_goal_ratio: seed_goals.seed_goal_ratio,
            seed_goal_seconds: seed_goals.seed_goal_seconds,
            never_remove: seed_goals.never_remove,
            goal_met_action: seed_goals.goal_met_action,
            post_import_tracking: seed_goals.post_import_tracking,
            resolution_source: seed_goals.resolution_source,
            info_hash: grab
                .info_hash
                .clone()
                .or_else(|| request.info_hash_hint.clone()),
        })
    }

    fn routing_scope_label(routing: Option<&ResolvedDownloadClientRouting>) -> &'static str {
        match routing.map(|value| value.scope) {
            Some(DownloadClientRoutingScope::Library) => "library",
            Some(DownloadClientRoutingScope::Facet) => "facet",
            None => "global",
        }
    }

    fn mapped_routing_failure(
        indexer: &scryer_domain::IndexerConfig,
        client: Option<&DownloadClientConfig>,
        client_id: &str,
        scope: &str,
        reason: impl Into<String>,
    ) -> AppError {
        let reason = reason.into();
        let client_name = client
            .map(|config| config.name.as_str())
            .unwrap_or(client_id);
        let client_type = client
            .map(|config| config.client_type.as_str())
            .unwrap_or("unavailable");
        warn!(
            routing_mode = "mapped",
            indexer_id = indexer.id.as_str(),
            indexer_name = indexer.name.as_str(),
            client_id,
            client_name,
            client_type,
            effective_scope = scope,
            failure_reason = reason.as_str(),
            "indexer download-client routing failed"
        );
        AppError::download_submit_unavailable(format!(
            "indexer '{}' ({}) mapped to download client '{}' ({client_id}) is unavailable in {scope} routing: {reason}",
            indexer.name, indexer.id, client_name
        ))
    }

    async fn mapped_routing_failure_with_cleanup(
        &self,
        staged_nzb: Option<&StagedNzbRef>,
        indexer: &scryer_domain::IndexerConfig,
        client: Option<&DownloadClientConfig>,
        client_id: &str,
        scope: &str,
        reason: impl Into<String>,
    ) -> AppError {
        self.delete_staged_nzb(staged_nzb, "mapped_routing_failure")
            .await;
        Self::mapped_routing_failure(indexer, client, client_id, scope, reason)
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

    /// Load the proxy assigned to a download client.
    ///
    /// Fail-closed: an assignment that cannot be resolved (no repository, a
    /// deleted proxy, a disabled proxy) is an error, never "carry on
    /// unproxied". This mirrors how the indexer path resolves its own proxy in
    /// `prepare_download_request`.
    async fn proxy_for_download_client(
        &self,
        config: &DownloadClientConfig,
    ) -> AppResult<Option<ProxyConfig>> {
        let Some(proxy_config_id) = config
            .proxy_config_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        let Some(proxy_configs) = self.proxy_configs.as_ref() else {
            return Err(AppError::Validation(format!(
                "download client {} is assigned a proxy but the proxy repository is not wired",
                config.id
            )));
        };
        let proxy_config = proxy_configs
            .get_by_id(proxy_config_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "download client {} references proxy configuration {proxy_config_id}, which was not found",
                    config.id
                ))
            })?;
        if !proxy_config.is_enabled {
            return Err(AppError::Validation(format!(
                "download client {} is assigned proxy {proxy_config_id}, which is disabled",
                config.id
            )));
        }
        Ok(Some(proxy_config))
    }

    fn client_from_config(
        config: &DownloadClientConfig,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
        feedback_read_timeout: Duration,
        proxy_config: Option<&ProxyConfig>,
    ) -> AppResult<Arc<dyn DownloadClient>> {
        if let Some(provider) = plugin_provider
            && let Some(client) = provider.client_for_config_with_proxy(config, proxy_config)
        {
            return Ok(Self::wrap_feedback_client(client, feedback_read_timeout));
        }

        // Native clients build their own reqwest client, so the proxy has to be
        // attached here rather than in the plugin HTTP host. A tunnel fails to
        // build, which stops the client being created at all rather than
        // creating one that egresses directly.
        let http_client = native_download_client_http_client(&config.name, proxy_config)?;

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
                )
                .with_http_client(http_client);
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
                )
                .with_http_client(http_client);
                Self::wrap_feedback_client(Arc::new(client), feedback_read_timeout)
            }
            "weaver" => {
                let client = WeaverDownloadClient::from_config_with_staged_nzb_store(
                    config,
                    staged_nzb_store,
                    staged_nzb_pipeline_limit,
                )?
                .with_http_client(http_client);
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
            let proxy_config = match self.proxy_for_download_client(&config).await {
                Ok(proxy_config) => proxy_config,
                Err(error) => {
                    warn!(
                        routing_mode = "automatic",
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        error = %error,
                        "download client skipped while routing queue action: assigned proxy is unusable"
                    );
                    continue;
                }
            };
            match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
                proxy_config.as_ref(),
            ) {
                Ok(client) => clients.push((config, client)),
                Err(error) => {
                    warn!(
                        routing_mode = "automatic",
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
                        routing_mode = "automatic",
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

            let proxy_config = self.proxy_for_download_client(&config).await?;
            return Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
                proxy_config.as_ref(),
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

            let proxy_config = self.proxy_for_download_client(&config).await?;
            return Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
                proxy_config.as_ref(),
            )
            .map(Some);
        }

        Ok(None)
    }
}

#[async_trait]
impl DownloadClient for PrioritizedDownloadClientRouter {
    /// Resolve the release's file without submitting it (D17).
    ///
    /// The indexer is resolved exactly as the submit path resolves it, so an
    /// assigned challenge solver or a private tracker's own grab flow still
    /// owns the fetch. A magnet comes back as-is; refusing it belongs to the
    /// caller, which knows the release title to name in the message.
    async fn fetch_release_artifact(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let indexer_config = self
            .load_indexer_config_for_submission(request)
            .await
            .map_err(AppError::into_download_submit_unavailable)?;
        let prepared = self
            .prepare_download_request_with(
                request,
                indexer_config.as_ref(),
                ArtifactFetch::HostSide,
            )
            .await?;
        prepared.resolved_download_artifact.ok_or_else(|| {
            AppError::Validation("release has no download artifact to fetch".to_string())
        })
    }

    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        // Load indexer provenance once. The same live config drives proxy
        // resolution and the current indexer -> download-client mapping.
        let indexer_config = match self.load_indexer_config_for_submission(request).await {
            Ok(indexer_config) => indexer_config,
            Err(error) => {
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "indexer_lookup_failure")
                    .await;
                return Err(error.into_download_submit_unavailable());
            }
        };
        let mapped_client_id = indexer_config
            .as_ref()
            .and_then(|config| config.download_client_id.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let request = match self
            .prepare_download_request(request, indexer_config.as_ref())
            .await
        {
            Ok(request) => request,
            Err(error) => {
                if error.is_download_source_gone() {
                    self.delete_staged_nzb(request.staged_nzb.as_ref(), "artifact_source_gone")
                        .await;
                    return Err(error);
                }
                if let (Some(indexer), Some(mapped_client_id)) =
                    (indexer_config.as_ref(), mapped_client_id)
                {
                    return Err(self
                        .mapped_routing_failure_with_cleanup(
                            request.staged_nzb.as_ref(),
                            indexer,
                            None,
                            mapped_client_id,
                            "artifact",
                            format!("indexer artifact resolution failed: {error}"),
                        )
                        .await);
                }
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "proxy_resolution_failure")
                    .await;
                return Err(error.into_download_submit_unavailable());
            }
        };
        let request = &request;
        // Pillar D1 for NZB bytes Scryer already holds (indexer-proxied
        // artifacts). URL-sourced NZBs are gated as they stream in, inside
        // `stage_nzb_from_url`, so no payload is ever fetched twice.
        if let Some(ResolvedDownloadArtifact::Nzb { bytes, .. }) =
            request.resolved_download_artifact.as_ref()
        {
            let head_len = bytes.len().min(scryer_application::NZB_HEAD_PROBE_BYTES);
            if let Err(error) = scryer_application::enforce_nzb_category_gate(
                &bytes[..head_len],
                request
                    .search_facet
                    .as_ref()
                    .unwrap_or(&request.title.facet),
            ) {
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "nzb_category_rejected")
                    .await;
                return Err(error);
            }
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
                if let Some(mapped_client_id) = mapped_client_id {
                    let indexer = indexer_config
                        .as_ref()
                        .expect("mapped client requires indexer configuration");
                    return Err(self
                        .mapped_routing_failure_with_cleanup(
                            request.staged_nzb.as_ref(),
                            indexer,
                            None,
                            mapped_client_id,
                            "unknown",
                            format!("effective client policy could not be loaded: {error}"),
                        )
                        .await);
                }
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "client_policy_failure")
                    .await;
                return Err(error.into_download_submit_unavailable());
            }
        };

        let mut clients = if let Some(pinned_client_id) = request
            .pinned_download_client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            // An unlinked grab (D8) has no title to route by, so the operator
            // named the client outright. That choice outranks the indexer
            // mapping and the routing order; only the client's own routing
            // entry still applies, for category and queue priority.
            let Some(config) = selection
                .all_clients
                .iter()
                .find(|config| config.id == pinned_client_id)
            else {
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "pinned_client_missing")
                    .await;
                return Err(AppError::Validation(format!(
                    "download client {pinned_client_id} does not exist"
                )));
            };
            if !config.is_enabled {
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "pinned_client_disabled")
                    .await;
                return Err(AppError::Validation(format!(
                    "download client {} is disabled",
                    config.name
                )));
            }
            vec![config.clone()]
        } else if let Some(mapped_client_id) = mapped_client_id {
            let indexer = indexer_config
                .as_ref()
                .expect("mapped client requires indexer configuration");
            let scope = Self::routing_scope_label(selection.routing.as_ref());
            let Some(config) = selection
                .all_clients
                .iter()
                .find(|config| config.id == mapped_client_id)
            else {
                return Err(self
                    .mapped_routing_failure_with_cleanup(
                        request.staged_nzb.as_ref(),
                        indexer,
                        None,
                        mapped_client_id,
                        scope,
                        "mapped download client does not exist",
                    )
                    .await);
            };
            if !config.is_enabled {
                return Err(self
                    .mapped_routing_failure_with_cleanup(
                        request.staged_nzb.as_ref(),
                        indexer,
                        Some(config),
                        mapped_client_id,
                        scope,
                        "mapped download client is globally disabled",
                    )
                    .await);
            }
            let enabled_in_scope = selection
                .routing
                .as_ref()
                .map(|routing| {
                    routing
                        .routing_object
                        .get(mapped_client_id)
                        .map(|entry| Self::read_bool(entry.get("enabled"), true))
                        .unwrap_or(matches!(routing.scope, DownloadClientRoutingScope::Facet))
                })
                .unwrap_or(true);
            if !enabled_in_scope {
                return Err(self
                    .mapped_routing_failure_with_cleanup(
                        request.staged_nzb.as_ref(),
                        indexer,
                        Some(config),
                        mapped_client_id,
                        scope,
                        "mapped download client is disabled in the effective scope",
                    )
                    .await);
            }
            vec![config.clone()]
        } else {
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
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "scope_routing_unavailable")
                    .await;
                return Err(AppError::download_submit_unavailable(message));
            }

            let clients = selection.clients;
            if clients.is_empty() {
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "no_enabled_clients")
                    .await;
                return Err(AppError::download_submit_unavailable(
                    "no enabled download clients configured",
                ));
            }
            clients
        };

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
                if let Some(mapped_client_id) = mapped_client_id {
                    let indexer = indexer_config
                        .as_ref()
                        .expect("mapped client requires indexer configuration");
                    let mapped_client = selection
                        .all_clients
                        .iter()
                        .find(|config| config.id == mapped_client_id);
                    return Err(self
                        .mapped_routing_failure_with_cleanup(
                            request.staged_nzb.as_ref(),
                            indexer,
                            mapped_client,
                            mapped_client_id,
                            Self::routing_scope_label(selection.routing.as_ref()),
                            format!(
                                "mapped download client cannot accept the resolved {}",
                                Self::artifact_kind_label(artifact_kind)
                            ),
                        )
                        .await);
                }
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "artifact_incompatible")
                    .await;
                return Err(AppError::download_submit_unavailable(format!(
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
                if let Some(mapped_client_id) = mapped_client_id {
                    let indexer = indexer_config
                        .as_ref()
                        .expect("mapped client requires indexer configuration");
                    let mapped_client = selection
                        .all_clients
                        .iter()
                        .find(|config| config.id == mapped_client_id);
                    return Err(self
                        .mapped_routing_failure_with_cleanup(
                            request.staged_nzb.as_ref(),
                            indexer,
                            mapped_client,
                            mapped_client_id,
                            Self::routing_scope_label(selection.routing.as_ref()),
                            format!(
                                "mapped download client cannot handle {} releases",
                                Self::source_kind_label(source_kind)
                            ),
                        )
                        .await);
                }
                self.delete_staged_nzb(request.staged_nzb.as_ref(), "source_kind_incompatible")
                    .await;
                return Err(AppError::download_submit_unavailable(format!(
                    "no enabled download client can handle {} releases",
                    Self::source_kind_label(source_kind)
                )));
            }
        }

        let mut last_error: Option<AppError> = None;
        let mut staged_nzb = if let Some(staged_nzb) = request.staged_nzb.clone() {
            if let Err(error) = self
                .staged_nzb_store
                .mark_artifact_active(&staged_nzb.compressed_path)
            {
                if let (Some(indexer), Some(mapped_client_id)) =
                    (indexer_config.as_ref(), mapped_client_id)
                {
                    return Err(self
                        .mapped_routing_failure_with_cleanup(
                            Some(&staged_nzb),
                            indexer,
                            clients.first(),
                            mapped_client_id,
                            Self::routing_scope_label(selection.routing.as_ref()),
                            format!("staged NZB could not be activated: {error}"),
                        )
                        .await);
                }
                self.delete_staged_nzb(Some(&staged_nzb), "staged_nzb_activation_failure")
                    .await;
                return Err(error.into_download_submit_unavailable());
            }
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
            info!(
                routing_mode = if mapped_client_id.is_some() {
                    "mapped"
                } else {
                    "automatic"
                },
                indexer_id = indexer_config
                    .as_ref()
                    .map(|indexer| indexer.id.as_str())
                    .unwrap_or(""),
                indexer_name = indexer_config
                    .as_ref()
                    .map(|indexer| indexer.name.as_str())
                    .unwrap_or(""),
                client_id = config.id.as_str(),
                client_name = config.name.as_str(),
                client_type = config.client_type.as_str(),
                effective_scope = Self::routing_scope_label(selection.routing.as_ref()),
                "selected download client route"
            );
            // Fail-closed: a selected client whose proxy will not resolve is a
            // routing failure, not a client to use unproxied.
            let proxy_config = self.proxy_for_download_client(&config).await?;
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
                proxy_config.as_ref(),
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
                    if let Some(mapped_client_id) = mapped_client_id {
                        self.delete_staged_nzb(
                            staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                            "mapped_client_config_failure",
                        )
                        .await;
                        let indexer = indexer_config
                            .as_ref()
                            .expect("mapped client requires indexer configuration");
                        return Err(Self::mapped_routing_failure(
                            indexer,
                            Some(&config),
                            mapped_client_id,
                            Self::routing_scope_label(selection.routing.as_ref()),
                            format!("download client configuration could not be built: {error}"),
                        ));
                    }
                    last_error = Some(error);
                    continue;
                }
            };

            let (effective_request, resolved_seed_goals) = match self
                .apply_selected_client_routing(request, &config.id)
                .await
            {
                Ok((mut effective_request, resolved_seed_goals)) => {
                    if Self::is_native_nzb_client_type(&config.client_type)
                        && Self::request_uses_nzb_payload(&effective_request)
                    {
                        if staged_nzb.is_none() {
                            if let Some(ResolvedDownloadArtifact::Nzb { bytes, .. }) =
                                effective_request.resolved_download_artifact.clone()
                            {
                                let wire_download_id =
                                    effective_request.download_id.map(|id| id.to_wire());
                                let source_label = wire_download_id
                                    .as_deref()
                                    .or(effective_request.source_title.as_deref())
                                    .unwrap_or("proxied-nzb");
                                let staged = stage_nzb_from_bytes(
                                    &self.staged_nzb_store,
                                    &self.staged_nzb_pipeline_limit,
                                    source_label,
                                    Some(&request.title.id),
                                    bytes,
                                )
                                .await;
                                staged_nzb = Some(match staged {
                                    Ok(staged) => staged,
                                    Err(error) => {
                                        if matches!(
                                            &error,
                                            AppError::Validation(_)
                                                | AppError::DownloadSubmitRejected(_)
                                        ) {
                                            return Err(error);
                                        }
                                        if let (Some(indexer), Some(mapped_client_id)) =
                                            (indexer_config.as_ref(), mapped_client_id)
                                        {
                                            return Err(Self::mapped_routing_failure(
                                                indexer,
                                                Some(&config),
                                                mapped_client_id,
                                                Self::routing_scope_label(
                                                    selection.routing.as_ref(),
                                                ),
                                                format!("NZB staging failed: {error}"),
                                            ));
                                        }
                                        return Err(error.into_download_submit_unavailable());
                                    }
                                });
                            } else {
                                let source_hint =
                                    match request_source_hint_for_nzb(&effective_request) {
                                        Ok(source_hint) => source_hint,
                                        Err(error) => {
                                            if matches!(
                                                &error,
                                                AppError::Validation(_)
                                                    | AppError::DownloadSubmitRejected(_)
                                            ) {
                                                return Err(error);
                                            }
                                            if let (Some(indexer), Some(mapped_client_id)) =
                                                (indexer_config.as_ref(), mapped_client_id)
                                            {
                                                return Err(Self::mapped_routing_failure(
                                                    indexer,
                                                    Some(&config),
                                                    mapped_client_id,
                                                    Self::routing_scope_label(
                                                        selection.routing.as_ref(),
                                                    ),
                                                    format!(
                                                        "NZB source could not be resolved: {error}"
                                                    ),
                                                ));
                                            }
                                            return Err(error.into_download_submit_unavailable());
                                        }
                                    };
                                let staged = stage_nzb_from_url(
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
                                .await;
                                staged_nzb = Some(match staged {
                                    Ok(staged) => staged,
                                    Err(error) => {
                                        if matches!(
                                            &error,
                                            AppError::Validation(_)
                                                | AppError::DownloadSubmitRejected(_)
                                        ) {
                                            return Err(error);
                                        }
                                        if let (Some(indexer), Some(mapped_client_id)) =
                                            (indexer_config.as_ref(), mapped_client_id)
                                        {
                                            return Err(Self::mapped_routing_failure(
                                                indexer,
                                                Some(&config),
                                                mapped_client_id,
                                                Self::routing_scope_label(
                                                    selection.routing.as_ref(),
                                                ),
                                                format!("NZB download or staging failed: {error}"),
                                            ));
                                        }
                                        return Err(error.into_download_submit_unavailable());
                                    }
                                });
                            }
                        }
                        effective_request.staged_nzb =
                            staged_nzb.as_ref().map(|lease| lease.staged_nzb.clone());
                    }
                    (effective_request, resolved_seed_goals)
                }
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped because routing configuration could not be resolved"
                    );
                    if let Some(mapped_client_id) = mapped_client_id {
                        self.delete_staged_nzb(
                            staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                            "mapped_routing_failure",
                        )
                        .await;
                        let indexer = indexer_config
                            .as_ref()
                            .expect("mapped client requires indexer configuration");
                        return Err(Self::mapped_routing_failure(
                            indexer,
                            Some(&config),
                            mapped_client_id,
                            Self::routing_scope_label(selection.routing.as_ref()),
                            format!("effective routing settings could not be resolved: {error}"),
                        ));
                    }
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
                    let mut grab = DownloadGrabResult {
                        download_id: effective_request.download_id,
                        job_id: result.job_id,
                        client_id: Some(config.id.clone()),
                        client_type: config.client_type.clone(),
                        info_hash: result
                            .info_hash
                            .or_else(|| resolved_v1_info_hash(&effective_request)),
                        seed_goals: None,
                    };
                    grab.seed_goals =
                        Self::persisted_seed_goals(&effective_request, &grab, &resolved_seed_goals);
                    return Ok(grab);
                }
                Err(error) => {
                    if let Some(mapped_client_id) = mapped_client_id {
                        if error.is_download_submit_ambiguous() {
                            warn!(
                                routing_mode = "mapped",
                                indexer_id = indexer_config
                                    .as_ref()
                                    .map(|indexer| indexer.id.as_str())
                                    .unwrap_or("unknown"),
                                indexer_name = indexer_config
                                    .as_ref()
                                    .map(|indexer| indexer.name.as_str())
                                    .unwrap_or("unknown"),
                                client_id = config.id.as_str(),
                                client_name = config.name.as_str(),
                                client_type = config.client_type.as_str(),
                                effective_scope = Self::routing_scope_label(selection.routing.as_ref()),
                                failure_reason = %error,
                                failover = false,
                                "mapped download client submit is ambiguous; preserving ambiguity without fallback"
                            );
                            self.delete_staged_nzb(
                                staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                                "mapped_ambiguous_submit",
                            )
                            .await;
                            return Err(error.with_ambiguous_download_submission_client(
                                Some(config.id.clone()),
                                config.client_type.clone(),
                            ));
                        }
                        warn!(
                            routing_mode = "mapped",
                            indexer_id = indexer_config
                                .as_ref()
                                .map(|indexer| indexer.id.as_str())
                                .unwrap_or("unknown"),
                            indexer_name = indexer_config
                                .as_ref()
                                .map(|indexer| indexer.name.as_str())
                                .unwrap_or("unknown"),
                            client_id = config.id.as_str(),
                            client_name = config.name.as_str(),
                            client_type = config.client_type.as_str(),
                            effective_scope = Self::routing_scope_label(selection.routing.as_ref()),
                            failure_reason = %error,
                            failover = false,
                            "mapped download client enqueue failed without fallback"
                        );
                        self.delete_staged_nzb(
                            staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                            "mapped_submit_failure",
                        )
                        .await;
                        let indexer = indexer_config
                            .as_ref()
                            .expect("mapped client requires indexer configuration");
                        return Err(Self::mapped_routing_failure(
                            indexer,
                            Some(&config),
                            mapped_client_id,
                            Self::routing_scope_label(selection.routing.as_ref()),
                            format!("download submission failed: {error}"),
                        ));
                    }
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
                    if error.is_download_submit_ambiguous() {
                        return Err(error.with_ambiguous_download_submission_client(
                            Some(config.id.clone()),
                            config.client_type.clone(),
                        ));
                    }
                    return Err(error);
                }
            }
        }

        self.delete_staged_nzb(
            staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
            "submit_failure",
        )
        .await;

        // Every eligible client in the routing order was tried (or none was
        // eligible). The typed variant is what the acquisition layer keys its
        // retry-later decision on; the final client error rides along as
        // display-only context.
        Err(match last_error {
            Some(error) => AppError::download_submit_failover_exhausted(format!(
                "all prioritized download clients failed to enqueue this release; last client error: {error}"
            )),
            None => AppError::download_submit_failover_exhausted(
                "all prioritized download clients failed to enqueue this release",
            ),
        })
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.list_queue_excluding_client_types(&[]).await
    }

    async fn list_queue_excluding_client_types(
        &self,
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<DownloadQueueItem>> {
        Ok(self
            .queue_listing_excluding_client_types(excluded_client_types)
            .await?
            .items)
    }

    async fn list_queue_with_read_report(&self) -> AppResult<DownloadClientListing> {
        self.queue_listing_excluding_client_types(&[]).await
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        let reads = self
            .poll_feedback_clients(
                clients,
                DownloadFeedbackReadKind::TitleQueue,
                "title-scoped queue listing",
                |client, scope| async move {
                    client
                        .list_queue_for_title_with_feedback_scope(title_id, &scope)
                        .await
                },
            )
            .await;
        for (config, elapsed, result) in reads {
            match result {
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
                        elapsed,
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
        Ok(self.history_listing().await?.items)
    }

    async fn list_history_with_read_report(&self) -> AppResult<DownloadClientListing> {
        self.history_listing().await
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
        let mut client_priorities = HashMap::new();
        let mut read_summary = FeedbackReadSummary::default();
        let reads = self
            .poll_feedback_clients(
                clients,
                DownloadFeedbackReadKind::RecentActivity,
                "recent activity listing",
                |client, scope| async move {
                    client
                        .list_recent_activity_with_feedback_scope(limit, &scope)
                        .await
                },
            )
            .await;
        for (config, elapsed, result) in reads {
            client_priorities.insert(config.id.clone(), config.client_priority);
            match result {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_success();
                    items.truncate(limit);
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
                        elapsed,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items
            .sort_by(|left, right| compare_history_items_desc(left, right, &client_priorities));
        Ok(all_items)
    }

    async fn list_snapshot_outcome_excluding_client_types(
        &self,
        recent_activity_limit: usize,
        excluded_client_types: &[&str],
    ) -> AppResult<DownloadClientSnapshotOutcome> {
        let clients = self
            .list_enabled_clients_by_priority_excluding(excluded_client_types)
            .await?;
        if clients.is_empty() {
            return Ok(DownloadClientSnapshotOutcome {
                any_client_read_succeeded: true,
                ..Default::default()
            });
        }

        let mut queue_items = Vec::new();
        let mut queue_successes = HashSet::new();
        let mut any_client_read_succeeded = false;
        let queue_reads = self
            .poll_feedback_clients(
                clients.clone(),
                DownloadFeedbackReadKind::Queue,
                "download snapshot queue listing",
                |client, scope| async move { client.list_queue_with_feedback_scope(&scope).await },
            )
            .await;
        for (config, elapsed, result) in queue_reads {
            match result {
                Ok(mut items) => {
                    self.record_feedback_read_success(&config.id, DownloadFeedbackReadKind::Queue);
                    any_client_read_succeeded = true;
                    queue_successes.insert(config.id.clone());
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    queue_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::Queue,
                        elapsed,
                    );
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list queue for download snapshot");
                }
            }
        }

        let mut activity_items = Vec::new();
        let mut activity_successes = HashSet::new();
        let mut client_priorities = HashMap::new();
        if recent_activity_limit > 0 {
            let activity_reads = self
                .poll_feedback_clients(
                    clients,
                    DownloadFeedbackReadKind::RecentActivity,
                    "download snapshot recent activity listing",
                    |client, scope| async move {
                        client
                            .list_recent_activity_with_feedback_scope(recent_activity_limit, &scope)
                            .await
                    },
                )
                .await;
            for (config, elapsed, result) in activity_reads {
                client_priorities.insert(config.id.clone(), config.client_priority);
                match result {
                    Ok(mut items) => {
                        self.record_feedback_read_success(
                            &config.id,
                            DownloadFeedbackReadKind::RecentActivity,
                        );
                        any_client_read_succeeded = true;
                        activity_successes.insert(config.id.clone());
                        items.truncate(recent_activity_limit);
                        for item in &mut items {
                            item.client_id = config.id.clone();
                            item.client_name = config.name.clone();
                        }
                        activity_items.extend(items);
                    }
                    Err(error) => {
                        self.record_feedback_read_failure(
                            &config.id,
                            DownloadFeedbackReadKind::RecentActivity,
                            elapsed,
                        );
                        tracing::warn!(client_id = %config.id, error = %error, "failed to list recent activity for download snapshot");
                    }
                }
            }
        }

        let mut seen = HashSet::with_capacity(activity_items.len());
        activity_items.retain(|item| seen.insert(download_queue_history_key(item)));
        activity_items
            .sort_by(|left, right| compare_history_items_desc(left, right, &client_priorities));
        queue_items.extend(activity_items);

        Ok(DownloadClientSnapshotOutcome {
            items: queue_items,
            authoritative_client_ids: queue_successes
                .intersection(&activity_successes)
                .cloned()
                .collect(),
            any_client_read_succeeded,
        })
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
        let mut client_priorities = HashMap::new();
        let mut read_summary = FeedbackReadSummary::default();
        let reads = self
            .poll_feedback_clients(
                clients,
                DownloadFeedbackReadKind::RecentActivity,
                "type-scoped recent activity listing",
                |client, scope| async move {
                    client
                        .list_recent_activity_with_feedback_scope(limit, &scope)
                        .await
                },
            )
            .await;
        for (config, elapsed, result) in reads {
            client_priorities.insert(config.id.clone(), config.client_priority);
            match result {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_success();
                    items.truncate(limit);
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
                        elapsed,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list type-scoped recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items
            .sort_by(|left, right| compare_history_items_desc(left, right, &client_priorities));
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
        let mut client_priorities = HashMap::new();
        let mut read_summary = FeedbackReadSummary::default();
        let reads = self
            .poll_feedback_clients(
                clients,
                DownloadFeedbackReadKind::TitleRecentActivity,
                "title-scoped recent activity listing",
                |client, scope| async move {
                    client
                        .list_recent_activity_for_title_with_feedback_scope(title_id, limit, &scope)
                        .await
                },
            )
            .await;
        for (config, elapsed, result) in reads {
            client_priorities.insert(config.id.clone(), config.client_priority);
            match result {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::TitleRecentActivity,
                    );
                    read_summary.record_success();
                    items.truncate(limit);
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
                        elapsed,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list title-scoped recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items
            .sort_by(|left, right| compare_history_items_desc(left, right, &client_priorities));
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
        let mut client_priorities = HashMap::new();
        let mut read_summary = FeedbackReadSummary::default();
        let reads = self
            .poll_feedback_clients(
                clients,
                DownloadFeedbackReadKind::History,
                "paged history listing",
                |client, scope| async move {
                    client
                        .list_history_page_with_feedback_scope(0, fetch_limit, &scope)
                        .await
                },
            )
            .await;
        for (config, elapsed, result) in reads {
            client_priorities.insert(config.id.clone(), config.client_priority);
            match result {
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
                        elapsed,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list paged history");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items
            .sort_by(|left, right| compare_history_items_desc(left, right, &client_priorities));

        Ok(all_items.into_iter().skip(offset).take(limit).collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return Ok(Vec::new());
        }
        let mut all_items = Vec::new();
        let reads = self
            .poll_feedback_clients(
                clients,
                DownloadFeedbackReadKind::RecentCompletedDownloads,
                "completed downloads listing",
                |client, scope| async move {
                    client
                        .list_completed_downloads_with_feedback_scope(&scope)
                        .await
                },
            )
            .await;
        for (config, elapsed, result) in reads {
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
            match result {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentCompletedDownloads,
                    );
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
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::RecentCompletedDownloads,
                        elapsed,
                    );
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
        let clients = clients
            .into_iter()
            .filter(|config| {
                if !has_scope {
                    return true;
                }
                let type_key = config.client_type.trim().to_ascii_lowercase();
                let id_matches =
                    !scoped_client_ids.is_empty() && scoped_client_ids.contains(config.id.trim());
                let type_matches = !scoped_client_types.is_empty()
                    && scoped_client_types.contains(type_key.as_str());
                id_matches || type_matches
            })
            .collect::<Vec<_>>();
        let mut all_items = Vec::new();
        let reads = self
            .poll_feedback_clients(
                clients,
                DownloadFeedbackReadKind::RecentCompletedDownloads,
                "recent completed downloads listing",
                |client, scope| async move {
                    client
                        .list_recent_completed_downloads_with_feedback_scope(limit, &scope)
                        .await
                },
            )
            .await;
        for (config, elapsed, result) in reads {
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
            match result {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentCompletedDownloads,
                    );
                    let raw_count = items.len();
                    items.truncate(limit);
                    tracing::debug!(
                        client = %config.name,
                        client_type = %config.client_type,
                        recent_completed_strategy = recent_completed_strategy_label(&config.client_type),
                        raw_count,
                        returned_count = items.len(),
                        limit,
                        saturated = raw_count >= limit,
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
                        elapsed,
                    );
                    tracing::warn!(client_id = %config.id, client = %config.name, error = %error, "failed to list recent completed downloads");
                }
            }
        }

        all_items.sort_by(compare_completed_downloads_desc);
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

        let proxy_config = self.proxy_for_download_client(config).await?;
        let client = Self::client_from_config(
            config,
            self.staged_nzb_store.clone(),
            self.staged_nzb_pipeline_limit.clone(),
            self.plugin_provider.as_ref(),
            self.feedback_read_timeout,
            proxy_config.as_ref(),
        )?;
        let started_at = Instant::now();
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
                    started_at.elapsed(),
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

    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, is_history).await? {
            return client.delete_queue_item(id, is_history, remove_data).await;
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
        remove_data: bool,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.delete_queue_item(id, is_history, remove_data).await;
        }
        Err(AppError::Validation(format!(
            "download client not found: {client_id}"
        )))
    }

    async fn mark_imported_non_destructive_for_client_id(
        &self,
        client_id: &str,
        request: &scryer_application::DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.mark_imported_non_destructive(request).await;
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
        remove_data: bool,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_type(client_type).await? {
            return client.delete_queue_item(id, is_history, remove_data).await;
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
        let proxy_config = self.proxy_for_download_client(&config).await?;
        let client = Self::client_from_config(
            &config,
            self.staged_nzb_store.clone(),
            self.staged_nzb_pipeline_limit.clone(),
            self.plugin_provider.as_ref(),
            self.feedback_read_timeout,
            proxy_config.as_ref(),
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

fn parse_history_timestamp(value: Option<&str>) -> Option<i64> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(timestamp) = value.parse::<i64>() {
        const UNIX_MILLISECONDS_THRESHOLD: u64 = 100_000_000_000;
        return if timestamp.unsigned_abs() >= UNIX_MILLISECONDS_THRESHOLD {
            Some(timestamp)
        } else {
            timestamp.checked_mul(1_000)
        };
    }

    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn compare_history_items_desc(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
    client_priorities: &HashMap<String, i64>,
) -> std::cmp::Ordering {
    parse_history_timestamp(right.last_updated_at.as_deref())
        .cmp(&parse_history_timestamp(left.last_updated_at.as_deref()))
        .then_with(|| {
            client_priorities
                .get(&left.client_id)
                .copied()
                .unwrap_or(i64::MAX)
                .cmp(
                    &client_priorities
                        .get(&right.client_id)
                        .copied()
                        .unwrap_or(i64::MAX),
                )
        })
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

    #[test]
    fn history_timestamps_support_seconds_milliseconds_and_rfc3339() {
        assert_eq!(
            parse_history_timestamp(Some("1700000000")),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_history_timestamp(Some("1700000000000")),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_history_timestamp(Some("2023-11-14T22:13:20Z")),
            Some(1_700_000_000_000)
        );
        assert_eq!(parse_history_timestamp(Some("not-a-timestamp")), None);

        let mut seconds = test_queue_item("seconds");
        seconds.last_updated_at = Some("1700000001".to_string());
        let mut milliseconds = test_queue_item("milliseconds");
        milliseconds.last_updated_at = Some("1700000000500".to_string());
        let mut rfc3339 = test_queue_item("rfc3339");
        rfc3339.last_updated_at = Some("2023-11-14T22:13:20Z".to_string());
        let mut malformed = test_queue_item("malformed");
        malformed.last_updated_at = Some("not-a-timestamp".to_string());

        let mut items = vec![malformed, rfc3339, milliseconds, seconds];
        items.sort_by(|left, right| compare_history_items_desc(left, right, &HashMap::new()));
        assert_eq!(
            items
                .into_iter()
                .map(|item| item.download_client_item_id)
                .collect::<Vec<_>>(),
            vec!["seconds", "milliseconds", "rfc3339", "malformed"]
        );

        let mut high_priority = test_queue_item("aaa");
        high_priority.client_id = "high".to_string();
        high_priority.last_updated_at = Some("1700000000".to_string());
        let mut low_priority = test_queue_item("zzz");
        low_priority.client_id = "low".to_string();
        low_priority.last_updated_at = Some("1700000000".to_string());
        let client_priorities = HashMap::from([("high".to_string(), 0), ("low".to_string(), 10)]);
        let mut tied = [low_priority, high_priority];
        tied.sort_by(|left, right| compare_history_items_desc(left, right, &client_priorities));
        assert_eq!(tied[0].client_id, "high");
    }

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
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_add_request(
        source_hint: &str,
        source_kind: Option<DownloadSourceKind>,
    ) -> DownloadClientAddRequest {
        DownloadClientAddRequest {
            title: test_title(),
            search_facet: None,
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            download_id: None,
            source_hint: Some(source_hint.to_string()),
            staged_nzb: None,
            resolved_download_artifact: None,
            source_kind,
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
            tracker_min_seed_ratio: None,
            tracker_min_seed_time_minutes: None,
            season_pack_seed_ratio: None,
            season_pack_seed_time_minutes: None,
            is_recent: None,
            season_pack: None,
            pinned_download_client_id: None,
        }
    }

    #[test]
    fn routing_entry_parses_optional_seeding_profile_id() {
        let camel = PrioritizedDownloadClientRouter::parse_routing_entry(&serde_json::json!({
            "enabled": true,
            "seedingProfileId": "  profile-1  ",
        }));
        assert_eq!(camel.seeding_profile_id.as_deref(), Some("profile-1"));

        let snake = PrioritizedDownloadClientRouter::parse_routing_entry(&serde_json::json!({
            "enabled": true,
            "seeding_profile_id": "profile-2",
        }));
        assert_eq!(snake.seeding_profile_id.as_deref(), Some("profile-2"));

        let absent = PrioritizedDownloadClientRouter::parse_routing_entry(&serde_json::json!({
            "enabled": true,
            "category": "Movies",
            "removeComplete": true,
        }));
        assert_eq!(absent.seeding_profile_id, None);
        assert_eq!(absent.category.as_deref(), Some("Movies"));
        assert!(absent.remove_completed);
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

    #[test]
    fn classifier_accepts_extensionless_torrent_with_plain_content_type() {
        let artifact = PrioritizedDownloadClientRouter::classify_resolved_download_artifact(
            "Indexer",
            Some("https://indexer.example/download?id=1"),
            Some(&serde_json::json!({"content-type": "text/plain"})),
            b"d4:infod4:name4:testee".to_vec(),
            None,
        )
        .expect("valid metainfo is sufficient to classify a torrent");
        let ResolvedDownloadArtifact::TorrentFile { info_hash_hint, .. } = artifact else {
            panic!("valid metainfo should classify as a torrent");
        };
        assert_eq!(
            info_hash_hint.as_deref(),
            Some("1ade8a1a581f338e4fce4ce784da3f7d03f81f3a")
        );
    }

    fn transport_proxy_config(
        provider_type: scryer_domain::ProxyProviderType,
        base_url: String,
    ) -> ProxyConfig {
        let now = chrono::Utc::now();
        ProxyConfig {
            id: "transport-1".to_string(),
            name: "House VPN".to_string(),
            provider_type,
            protocol: None,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url,
            request_timeout_seconds: 30,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
        }
    }

    /// Minimal in-process HTTP proxy built from raw tokio: records the
    /// absolute-form request line it was handed and answers itself.
    async fn spawn_recording_artifact_proxy(
        body: &'static [u8],
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("proxy double should bind");
        let address = listener.local_addr().expect("proxy double should be bound");
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let recorder = Arc::clone(&recorder);
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 8192];
                    let mut received = Vec::new();
                    loop {
                        let Ok(read) = stream.read(&mut buffer).await else {
                            return;
                        };
                        if read == 0 {
                            break;
                        }
                        received.extend_from_slice(&buffer[..read]);
                        if received.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    if let Some(line) = String::from_utf8_lossy(&received).lines().next() {
                        recorder
                            .lock()
                            .expect("proxy recorder lock")
                            .push(line.to_string());
                    }
                    let mut response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/x-bittorrent\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(body);
                    let _ = stream.write_all(&response).await;
                    let _ = stream.flush().await;
                });
            }
        });
        (format!("http://{address}"), seen)
    }

    #[tokio::test]
    async fn artifact_fetch_egresses_through_an_assigned_transport_proxy() {
        let (proxy_url, seen) = spawn_recording_artifact_proxy(b"d4:infod4:name4:testee").await;
        let proxy_config =
            transport_proxy_config(scryer_domain::ProxyProviderType::Http, proxy_url);

        let artifact = no_client_router()
            .resolve_download_artifact_via_transport_proxy(
                &proxy_config,
                // Deliberately unresolvable: only the proxy can reach it.
                "http://indexer.example/download?id=1",
                None,
                &PluginEgressPolicy::default(),
            )
            .await
            .expect("the proxied artifact fetch should resolve");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::TorrentFile { .. }
        ));
        let seen = seen.lock().expect("proxy recorder lock").clone();
        assert_eq!(seen.len(), 1, "expected one proxied fetch: {seen:?}");
        assert!(
            seen[0].contains("http://indexer.example/download?id=1"),
            "expected an absolute-form proxied request line, got {}",
            seen[0]
        );
    }

    #[tokio::test]
    async fn an_unreachable_transport_proxy_names_itself_rather_than_the_indexer() {
        let proxy_config = transport_proxy_config(
            scryer_domain::ProxyProviderType::Socks5,
            "socks5://127.0.0.1:1".to_string(),
        );

        let error = no_client_router()
            .resolve_download_artifact_via_transport_proxy(
                &proxy_config,
                "http://indexer.example/download?id=1",
                None,
                &PluginEgressPolicy::default(),
            )
            .await
            .expect_err("an unreachable proxy must fail the fetch");

        let message = error.to_string();
        assert!(
            message.contains("proxy House VPN unreachable:"),
            "the failure must name the proxy, got: {message}"
        );
    }

    /// Was `a_tunnel_proxy_fails_closed_on_an_artifact_fetch` while there was
    /// no engine. The artifact fetch now travels through the tunnel: the SSH
    /// double records the destination it forwarded, so this proves the grab
    /// took the tunnel rather than merely that it did not go direct.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_tunnel_proxy_carries_an_artifact_fetch() {
        let server = scryer_tunnel::test_support::SshServerDouble::start(
            scryer_tunnel::test_support::SshServerOptions::default(),
        )
        .await;
        let origin = scryer_tunnel::test_support::TunnelledOrigin::start_with_content_type(
            "application/x-bittorrent",
            "d4:infod4:name4:testee",
        )
        .await;

        let mut proxy_config = transport_proxy_config(
            scryer_domain::ProxyProviderType::SshTunnel,
            format!("ssh://{}", server.addr()),
        );
        proxy_config.id = "router-tunnel".to_string();
        proxy_config.username_encrypted = Some("operator".to_string());
        proxy_config.password_encrypted = Some("s3cret".to_string());

        let resolved = no_client_router()
            .resolve_download_artifact_via_transport_proxy(
                &proxy_config,
                &format!("http://{}/download?id=1", origin.addr()),
                None,
                &PluginEgressPolicy::default(),
            )
            .await
            .expect("the tunnel must carry the artifact fetch");
        assert!(
            matches!(resolved, ResolvedDownloadArtifact::TorrentFile { .. }),
            "the artifact must come back classified through the tunnel: {resolved:?}"
        );

        assert_eq!(
            server.forwarded_targets(),
            vec![("127.0.0.1".to_string(), origin.addr().port())],
            "the artifact must have been fetched through the SSH server"
        );
        assert_eq!(origin.request_lines().len(), 1);

        scryer_application::tunnel_proxy::stop_tunnel("router-tunnel");
    }

    /// The fail-closed half, kept: a tunnel that will not come up fails the
    /// fetch instead of falling back to a direct one. The destination is a live
    /// server here, so a dropped assignment would show up as a successful
    /// download.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unreachable_tunnel_fails_closed_on_an_artifact_fetch() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let origin = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-bittorrent")
                    .set_body_bytes(b"d4:infod4:name4:testee"),
            )
            .mount(&origin)
            .await;
        let dead_ssh_port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            drop(listener);
            port
        };

        let mut proxy_config = transport_proxy_config(
            scryer_domain::ProxyProviderType::SshTunnel,
            format!("ssh://127.0.0.1:{dead_ssh_port}"),
        );
        proxy_config.id = "router-tunnel-dead".to_string();
        proxy_config.username_encrypted = Some("operator".to_string());
        proxy_config.password_encrypted = Some("s3cret".to_string());

        let error = no_client_router()
            .resolve_download_artifact_via_transport_proxy(
                &proxy_config,
                &format!("{}/download?id=1", origin.uri()),
                None,
                &PluginEgressPolicy::default(),
            )
            .await
            .expect_err("an unreachable tunnel must fail the fetch");

        let message = error.to_string();
        assert!(
            message.contains("proxy House VPN unreachable:"),
            "the failure must name the proxy, got: {message}"
        );
        assert!(
            origin
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "the artifact must never be fetched directly when a tunnel is assigned"
        );

        scryer_application::tunnel_proxy::stop_tunnel("router-tunnel-dead");
    }

    #[tokio::test]
    async fn direct_artifact_fetch_follows_relative_redirect_with_same_origin_headers() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .and(header("x-indexer-session", "secret"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/artifact"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifact"))
            .and(header("x-indexer-session", "secret"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_bytes(b"d4:infod4:name4:testee"),
            )
            .mount(&server)
            .await;

        let fetched = no_client_router()
            .fetch_download_artifact_direct(
                "Indexer",
                &format!("{}/start", server.uri()),
                &[("x-indexer-session".to_string(), "secret".to_string())],
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
            )
            .await
            .expect("relative redirect should resolve");
        let artifact = PrioritizedDownloadClientRouter::classify_resolved_download_artifact(
            "Indexer",
            fetched.final_url.as_deref(),
            fetched.headers.as_ref(),
            fetched.bytes,
            None,
        )
        .expect("redirect target should classify as a torrent");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::TorrentFile { .. }
        ));
    }

    #[tokio::test]
    async fn direct_artifact_fetch_strips_headers_after_cross_origin_redirect() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let target = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/artifact"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"d4:infod4:name4:testee"))
            .mount(&target)
            .await;
        let origin = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .and(header("x-indexer-session", "secret"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/artifact", target.uri())),
            )
            .mount(&origin)
            .await;

        no_client_router()
            .fetch_download_artifact_direct(
                "Indexer",
                &format!("{}/start", origin.uri()),
                &[("x-indexer-session".to_string(), "secret".to_string())],
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
            )
            .await
            .expect("cross-origin redirect should resolve without credentials");

        let requests = target.received_requests().await.expect("recorded requests");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].headers.get("x-indexer-session").is_none());
    }

    #[tokio::test]
    async fn direct_artifact_fetch_resolves_magnet_redirect_and_rejects_loops() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";
        Mock::given(method("GET"))
            .and(path("/magnet"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", magnet))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
            .mount(&server)
            .await;

        let fetched = no_client_router()
            .fetch_download_artifact_direct(
                "Indexer",
                &format!("{}/magnet", server.uri()),
                &[],
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
            )
            .await
            .expect("magnet redirect should resolve");
        assert_eq!(fetched.final_url.as_deref(), Some(magnet));

        let error = no_client_router()
            .fetch_download_artifact_direct(
                "Indexer",
                &format!("{}/loop", server.uri()),
                &[],
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
            )
            .await
            .expect_err("redirect loops must fail");
        assert!(error.to_string().contains("redirect looped"));

        let uppercase_btih = "MAGNET:?XT=URN:BTIH:0123456789ABCDEF0123456789ABCDEF01234567";
        let prepared = no_client_router()
            .prepare_download_request(
                &test_add_request(uppercase_btih, Some(DownloadSourceKind::MagnetUri)),
                None,
            )
            .await
            .expect("uppercase btih magnet should resolve");
        assert!(matches!(
            prepared.resolved_download_artifact,
            Some(ResolvedDownloadArtifact::Magnet {
                info_hash_hint: Some(ref hash),
                ..
            }) if hash == "0123456789abcdef0123456789abcdef01234567"
        ));

        let btmh = format!("magnet:?xt=urn:btmh:1220{}", "ab".repeat(32));
        let prepared = no_client_router()
            .prepare_download_request(
                &test_add_request(&btmh, Some(DownloadSourceKind::MagnetUri)),
                None,
            )
            .await
            .expect("btmh magnet should resolve");
        assert!(matches!(
            prepared.resolved_download_artifact,
            Some(ResolvedDownloadArtifact::Magnet {
                info_hash_hint: Some(ref hash),
                ..
            }) if hash == &"ab".repeat(32)
        ));
    }

    #[tokio::test]
    async fn fetch_download_artifact_resolves_an_nzb_url_the_submit_path_leaves_alone() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const NZB: &[u8] =
            br#"<?xml version="1.0" encoding="UTF-8"?><nzb xmlns="http://www.newzbin.com/DTD/2003/nzb"><file subject="release"><segments><segment bytes="1" number="1">a@b</segment></segments></file></nzb>"#;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/release.nzb"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-nzb")
                    .set_body_bytes(NZB),
            )
            .mount(&server)
            .await;

        let router = no_client_router();
        let request = test_add_request(
            &format!("{}/release.nzb", server.uri()),
            Some(DownloadSourceKind::NzbUrl),
        );

        let artifact = router
            .fetch_release_artifact(&request)
            .await
            .expect("host-side fetch should resolve the NZB");
        let ResolvedDownloadArtifact::Nzb { bytes, .. } = artifact else {
            panic!("an application/x-nzb body should classify as an NZB");
        };
        assert_eq!(bytes, NZB);

        // The submit path is untouched: the download client still fetches the
        // URL itself.
        let prepared = router
            .prepare_download_request(&request, None)
            .await
            .expect("submit preparation should still leave the NZB URL alone");
        assert!(prepared.resolved_download_artifact.is_none());
        assert_eq!(prepared.source_hint, request.source_hint);
    }

    #[tokio::test]
    async fn direct_preparation_resolves_http_torrent_but_preserves_nzb_url() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/download"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"d4:infod4:name4:testee"))
            .mount(&server)
            .await;
        let router = no_client_router();
        let mut torrent_request = test_add_request(
            &format!("{}/download", server.uri()),
            Some(DownloadSourceKind::MagnetUri),
        );
        torrent_request.info_hash_hint =
            Some("abcdef0123456789abcdef0123456789abcdef01".to_string());
        let prepared = router
            .prepare_download_request(&torrent_request, None)
            .await
            .expect("HTTP torrent should be resolved even with a known hash");
        assert!(matches!(
            prepared.resolved_download_artifact,
            Some(ResolvedDownloadArtifact::TorrentFile { .. })
        ));
        assert_eq!(prepared.source_kind, Some(DownloadSourceKind::TorrentFile));
        assert_eq!(
            prepared.info_hash_hint.as_deref(),
            Some("1ade8a1a581f338e4fce4ce784da3f7d03f81f3a")
        );

        let derived = router
            .prepare_download_request(
                &test_add_request(
                    &format!("{}/download", server.uri()),
                    Some(DownloadSourceKind::MagnetUri),
                ),
                None,
            )
            .await
            .expect("HTTP torrent should derive its v1 hash when the indexer omitted it");
        assert_eq!(
            derived.info_hash_hint.as_deref(),
            Some("1ade8a1a581f338e4fce4ce784da3f7d03f81f3a")
        );

        let v2_hint = "ab".repeat(32);
        let mut v2_request = test_add_request(
            &format!("{}/download", server.uri()),
            Some(DownloadSourceKind::TorrentFile),
        );
        v2_request.info_hash_hint = Some(v2_hint.clone());
        let prepared = router
            .prepare_download_request(&v2_request, None)
            .await
            .expect("HTTP torrent should preserve an explicit v2 hint");
        assert_eq!(prepared.info_hash_hint.as_deref(), Some(v2_hint.as_str()));

        let nzb_request = test_add_request(
            "http://127.0.0.1:1/release.nzb",
            Some(DownloadSourceKind::NzbUrl),
        );
        let preserved = router
            .prepare_download_request(&nzb_request, None)
            .await
            .expect("direct NZB URLs must not be fetched by torrent normalization");
        assert_eq!(preserved.source_hint, nzb_request.source_hint);
        assert!(preserved.resolved_download_artifact.is_none());
    }

    #[tokio::test]
    async fn hexadecimal_v1_magnets_override_stale_hints_across_resolution_paths() {
        const STALE_HASH: &str = "abcdef0123456789abcdef0123456789abcdef01";
        const CANONICAL_HASH: &str = "0123456789abcdef0123456789abcdef01234567";
        const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";

        let router = no_client_router();
        let mut request = test_add_request(MAGNET, Some(DownloadSourceKind::MagnetUri));
        request.info_hash_hint = Some(STALE_HASH.to_string());

        let direct = router
            .prepare_download_request(&request, None)
            .await
            .expect("direct hexadecimal btih magnet should resolve");
        assert_eq!(direct.info_hash_hint.as_deref(), Some(CANONICAL_HASH));
        assert!(matches!(
            direct.resolved_download_artifact.as_ref(),
            Some(ResolvedDownloadArtifact::Magnet {
                info_hash_hint: Some(hash),
                ..
            }) if hash == CANONICAL_HASH
        ));

        let from_final_url = PrioritizedDownloadClientRouter::classify_resolved_download_artifact(
            "Indexer",
            Some(MAGNET),
            None,
            Vec::new(),
            Some(STALE_HASH.to_string()),
        )
        .expect("final magnet URL should resolve");
        let prepared = router
            .prepare_resolved_request(&request, from_final_url)
            .expect("final magnet URL should prepare");
        assert_eq!(prepared.info_hash_hint.as_deref(), Some(CANONICAL_HASH));

        let from_body = PrioritizedDownloadClientRouter::classify_resolved_download_artifact(
            "Indexer",
            None,
            None,
            MAGNET.as_bytes().to_vec(),
            Some(STALE_HASH.to_string()),
        )
        .expect("magnet response body should resolve");
        let prepared = router
            .prepare_resolved_request(&request, from_body)
            .expect("magnet response body should prepare");
        assert_eq!(prepared.info_hash_hint.as_deref(), Some(CANONICAL_HASH));
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
        let proxy = ProxyConfig {
            id: "trawl-1".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::ProxyProviderType::Trawl,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_proxy(
                &proxy,
                &download_url,
                None,
                &PluginEgressPolicy::default(),
            )
            .await
            .expect("Trawl should resolve embedded NZB content");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::Nzb { bytes, .. } if bytes == b"<nzb></nzb>"
        ));
        let requests = server.received_requests().await.expect("recorded requests");
        let solver_request = requests
            .iter()
            .find(|request| request.url.path() == "/v1")
            .expect("solver request");
        assert_eq!(
            solver_request
                .headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok()),
            Some(scryer_outbound_http::PROXY_USER_AGENT)
        );
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
        let proxy = ProxyConfig {
            id: "trawl-direct-first".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::ProxyProviderType::Trawl,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_proxy(
                &proxy,
                &download_url,
                None,
                &PluginEgressPolicy::default(),
            )
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
        let proxy = ProxyConfig {
            id: "trawl-1".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::ProxyProviderType::Trawl,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_proxy(
                &proxy,
                &download_url,
                None,
                &PluginEgressPolicy::default(),
            )
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
        let proxy = ProxyConfig {
            id: "byparr-refetch".into(),
            name: "Byparr".into(),
            provider_type: scryer_domain::ProxyProviderType::Byparr,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
        };

        let artifact = no_client_router()
            .resolve_download_artifact_via_proxy(
                &proxy,
                &download_url,
                None,
                &PluginEgressPolicy::default(),
            )
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
        let proxy = ProxyConfig {
            id: "trawl-unavailable".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::ProxyProviderType::Trawl,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
        };

        let error = no_client_router()
            .resolve_download_artifact_via_proxy(
                &proxy,
                &download_url,
                None,
                &PluginEgressPolicy::default(),
            )
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
            scryer_domain::ProxyProviderType::Byparr,
            scryer_domain::ProxyProviderType::Trawl,
        ] {
            let proxy = ProxyConfig {
                id: format!("{}-1", provider_type.as_str()),
                name: solver::solver_provider_name(provider_type).to_string(),
                provider_type,
                protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
                username_encrypted: None,
                password_encrypted: None,
                remote_dns: false,
                base_url: "http://127.0.0.1:1".to_string(),
                request_timeout_seconds: 1,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
                host_key_fingerprint: None,
                host_key_pinned_at: None,
                private_key_encrypted: None,
                private_key_passphrase_encrypted: None,
                peer_public_key: None,
                preshared_key_encrypted: None,
                tunnel_public_key: None,
                tunnel_addresses: Vec::new(),
                tunnel_dns_servers: Vec::new(),
                tunnel_mtu: None,
                tunnel_keepalive_seconds: None,
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
                        Duration::from_secs(1),
                        &PluginEgressPolicy::default(),
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
                    .resolve_download_artifact_via_proxy(
                        &proxy,
                        target,
                        None,
                        &PluginEgressPolicy::default(),
                    )
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

    struct RoutingIndexerConfigRepository {
        configs: Vec<scryer_domain::IndexerConfig>,
    }

    #[async_trait]
    impl IndexerConfigRepository for RoutingIndexerConfigRepository {
        async fn list(
            &self,
            _provider_type: Option<String>,
        ) -> AppResult<Vec<scryer_domain::IndexerConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::IndexerConfig>> {
            Ok(self.configs.iter().find(|config| config.id == id).cloned())
        }

        async fn create(
            &self,
            config: scryer_domain::IndexerConfig,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(
            &self,
            _update: scryer_application::IndexerConfigUpdate,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Validation("not implemented in test".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct EmptyProxyConfigRepository;

    #[async_trait]
    impl ProxyConfigRepository for EmptyProxyConfigRepository {
        async fn list(
            &self,
            _provider_type: Option<scryer_domain::ProxyProviderType>,
        ) -> AppResult<Vec<scryer_domain::ProxyConfig>> {
            Ok(Vec::new())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<scryer_domain::ProxyConfig>> {
            Ok(None)
        }

        async fn create(
            &self,
            config: scryer_domain::ProxyConfig,
        ) -> AppResult<scryer_domain::ProxyConfig> {
            Ok(config)
        }

        async fn update(
            &self,
            config: scryer_domain::ProxyConfig,
        ) -> AppResult<scryer_domain::ProxyConfig> {
            Ok(config)
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }

        async fn record_health(
            &self,
            _id: &str,
            _status: scryer_domain::ProxyHealthStatus,
            _error_message: Option<String>,
            _error_at: Option<chrono::DateTime<chrono::Utc>>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn pin_host_key(
            &self,
            _id: &str,
            _fingerprint: &str,
            _pinned_at: chrono::DateTime<chrono::Utc>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn clear_host_key(&self, _id: &str) -> AppResult<()> {
            Ok(())
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
        deleted: Mutex<Vec<(String, bool, bool)>>,
        marked_imported: Mutex<Vec<scryer_application::DownloadClientMarkImportedRequest>>,
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
                download_id: None,
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "mock".to_string(),
                info_hash: None,
                seed_goals: None,
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

        async fn delete_queue_item(
            &self,
            id: &str,
            is_history: bool,
            remove_data: bool,
        ) -> AppResult<()> {
            self.deleted
                .lock()
                .unwrap()
                .push((id.to_string(), is_history, remove_data));
            Ok(())
        }

        async fn mark_imported(
            &self,
            request: &scryer_application::DownloadClientMarkImportedRequest,
        ) -> AppResult<()> {
            self.marked_imported.lock().unwrap().push(request.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailingCompletedDownloadClient {
        recent_completed_calls: AtomicUsize,
        targeted_completed_calls: AtomicUsize,
    }

    impl FailingCompletedDownloadClient {
        fn recent_completed_call_count(&self) -> usize {
            self.recent_completed_calls.load(Ordering::SeqCst)
        }

        fn targeted_completed_call_count(&self) -> usize {
            self.targeted_completed_calls.load(Ordering::SeqCst)
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

        async fn get_completed_download_for_source(
            &self,
            _client_id: &str,
            _client_type: &str,
            _download_client_item_id: &str,
        ) -> AppResult<Option<scryer_domain::CompletedDownload>> {
            self.targeted_completed_calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Repository(
                "targeted completed lookup unavailable".to_string(),
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
                release_name: None,
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

    struct FeedbackScopeQueueDownloadClient {
        scopes: Mutex<Vec<Vec<String>>>,
        queue_items: Vec<DownloadQueueItem>,
    }

    #[async_trait]
    impl DownloadClient for FeedbackScopeQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".to_string()))
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Err(AppError::Repository(
                "unscoped queue listing should not be used".to_string(),
            ))
        }

        async fn list_queue_with_feedback_scope(
            &self,
            scope: &DownloadClientFeedbackScope,
        ) -> AppResult<Vec<DownloadQueueItem>> {
            self.scopes.lock().unwrap().push(scope.categories.clone());
            Ok(self.queue_items.clone())
        }
    }

    #[async_trait]
    impl DownloadClient for DelayedQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Ok(DownloadGrabResult {
                download_id: None,
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "delayed".to_string(),
                info_hash: None,
                seed_goals: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            tokio::time::sleep(self.delay).await;
            Ok(self.queue_items.clone())
        }
    }

    struct CoordinatedQueueDownloadClient {
        barrier: Arc<tokio::sync::Barrier>,
        delay_after_barrier: Duration,
        queue_items: Vec<DownloadQueueItem>,
    }

    #[async_trait]
    impl DownloadClient for CoordinatedQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".to_string()))
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            self.barrier.wait().await;
            tokio::time::sleep(self.delay_after_barrier).await;
            Ok(self.queue_items.clone())
        }
    }

    struct GatedQueueDownloadClient {
        started: Option<Arc<tokio::sync::Notify>>,
        release: Option<Arc<tokio::sync::Notify>>,
        queue_items: Vec<DownloadQueueItem>,
    }

    #[async_trait]
    impl DownloadClient for GatedQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".to_string()))
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            if let Some(started) = &self.started {
                started.notify_one();
            }
            if let Some(release) = &self.release {
                release.notified().await;
            }
            Ok(self.queue_items.clone())
        }
    }

    struct DelayedCompletedDownloadClient {
        delay: Duration,
    }

    impl DelayedCompletedDownloadClient {
        async fn delay(&self) {
            tokio::time::sleep(self.delay).await;
        }
    }

    #[async_trait]
    impl DownloadClient for DelayedCompletedDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".to_string()))
        }

        async fn list_completed_downloads(
            &self,
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            self.delay().await;
            Ok(Vec::new())
        }

        async fn list_recent_completed_downloads(
            &self,
            _limit: usize,
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            self.delay().await;
            Ok(Vec::new())
        }

        async fn list_recent_completed_downloads_excluding_client_types(
            &self,
            _limit: usize,
            _excluded_client_types: &[&str],
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            self.delay().await;
            Ok(Vec::new())
        }

        async fn list_recent_completed_downloads_for_client_scope(
            &self,
            _limit: usize,
            _client_ids: &[String],
            _client_types: &[String],
            _excluded_client_types: &[&str],
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            self.delay().await;
            Ok(Vec::new())
        }

        async fn get_completed_download_for_source(
            &self,
            _client_id: &str,
            _client_type: &str,
            _download_client_item_id: &str,
        ) -> AppResult<Option<scryer_domain::CompletedDownload>> {
            self.delay().await;
            Ok(None)
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
                download_id: None,
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "failing".to_string(),
                info_hash: None,
                seed_goals: None,
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

    struct ActivityFailingDownloadClient {
        queue_items: Vec<DownloadQueueItem>,
    }

    #[async_trait]
    impl DownloadClient for ActivityFailingDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Ok(DownloadGrabResult {
                download_id: None,
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "failing-activity".to_string(),
                info_hash: None,
                seed_goals: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.queue_items.clone())
        }

        async fn list_recent_activity(&self, _limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
            Err(AppError::Repository(
                "recent activity unavailable".to_string(),
            ))
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
            proxy_config_id: None,
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

    #[derive(Default)]
    struct RecordingStagedNzbStore {
        deleted_ids: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl StagedNzbStore for RecordingStagedNzbStore {
        async fn create_pending_staged_nzb(
            &self,
            _source_url: &str,
            _title_id: Option<&str>,
        ) -> AppResult<scryer_application::PendingStagedNzb> {
            Err(AppError::Repository("not used in this test".into()))
        }

        async fn finalize_pending_staged_nzb(
            &self,
            _pending: scryer_application::PendingStagedNzb,
            _raw_size_bytes: u64,
        ) -> AppResult<StagedNzbRef> {
            Err(AppError::Repository("not used in this test".into()))
        }

        async fn delete_staged_nzb(&self, staged_nzb: &StagedNzbRef) -> AppResult<bool> {
            self.deleted_ids.lock().unwrap().push(staged_nzb.id.clone());
            Ok(true)
        }

        async fn prune_staged_nzbs_older_than(
            &self,
            _older_than: chrono::DateTime<Utc>,
        ) -> AppResult<u32> {
            Ok(0)
        }

        fn mark_artifact_active(&self, _path: &Path) -> AppResult<()> {
            Ok(())
        }

        fn mark_artifact_inactive(&self, _path: &Path) -> AppResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn mapped_indexer_selects_lower_priority_client_and_applies_scope_policy() {
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
        let mut indexer = test_indexer_config("https://indexer.example/api");
        indexer.download_client_id = Some("secondary".to_string());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 10),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": true },
                        "secondary": {
                            "enabled": true,
                            "category": "Movies",
                            "recentQueuePriority": "high"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![indexer],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );

        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Mapped Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());
        request.is_recent = Some(true);
        router
            .submit_download(&request)
            .await
            .expect("mapped client should accept compatible release");

        assert!(primary.submissions.lock().unwrap().is_empty());
        let submissions = secondary.submissions.lock().unwrap();
        assert_eq!(submissions.len(), 1);
        assert_eq!(submissions[0].category.as_deref(), Some("Movies"));
        assert_eq!(submissions[0].queue_priority.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn a_pinned_client_wins_over_routing_order_and_keeps_its_routing_category() {
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
                    test_config("secondary", "Secondary", "qbittorrent", 10),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": true },
                        "secondary": { "enabled": true, "category": "Unlinked" }
                    }"#
                    .to_string(),
                )]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Pinned Release".to_string()),
            None,
            None,
        );
        request.pinned_download_client_id = Some("secondary".to_string());
        router
            .submit_download(&request)
            .await
            .expect("pinned client should accept the release");

        assert!(
            primary.submissions.lock().unwrap().is_empty(),
            "the higher-priority client must not be routed to"
        );
        let submissions = secondary.submissions.lock().unwrap();
        assert_eq!(submissions.len(), 1);
        assert_eq!(submissions[0].category.as_deref(), Some("Unlinked"));
    }

    #[tokio::test]
    async fn a_pinned_client_that_is_missing_or_disabled_is_refused() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let mut retired = test_config("retired", "Retired", "qbittorrent", 5);
        retired.is_enabled = false;
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0), retired],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        for pinned in ["retired", "does-not-exist"] {
            let mut request = DownloadClientAddRequest::from_legacy(
                &test_title(),
                Some("https://indexer.example/release.nzb".to_string()),
                Some(DownloadSourceKind::NzbUrl),
                Some("Pinned Release".to_string()),
                None,
                None,
            );
            request.pinned_download_client_id = Some(pinned.to_string());
            let error = router
                .submit_download(&request)
                .await
                .expect_err("an unusable pinned client must not fall back");
            assert!(
                matches!(error, AppError::Validation(_)),
                "{pinned}: {error:?}"
            );
        }
        assert!(
            primary.submissions.lock().unwrap().is_empty(),
            "a refused pin must never fall back to another client"
        );
    }

    #[tokio::test]
    async fn indexed_submission_without_mapping_preserves_automatic_failover() {
        let primary = Arc::new(MockDownloadClient {
            submit_error: Mutex::new(Some(MockSubmitError::Repository)),
            ..Default::default()
        });
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
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![test_indexer_config("https://indexer.example/api")],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );
        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Automatic Indexed Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());
        let first_attempt_id = scryer_domain::download_identity::DownloadId::parse(
            "00000000-0000-4000-8000-000000000011",
        )
        .expect("fixed UUID should parse");
        request.download_id = Some(first_attempt_id);

        let result = router
            .submit_download(&request)
            .await
            .expect("automatic route should fail over for an unmapped indexer");
        assert_eq!(result.client_id.as_deref(), Some("secondary"));
        let primary_submissions = primary.submissions.lock().unwrap();
        let secondary_submissions = secondary.submissions.lock().unwrap();
        assert_eq!(primary_submissions.len(), 1);
        assert_eq!(secondary_submissions.len(), 1);
        let second_attempt_id = secondary_submissions[0]
            .download_id
            .expect("fallback attempt should carry an ID");
        assert_eq!(primary_submissions[0].download_id, Some(first_attempt_id));
        assert_eq!(second_attempt_id, first_attempt_id);
        assert_eq!(result.download_id, Some(first_attempt_id));
    }

    #[tokio::test]
    async fn indexed_submission_fails_closed_without_indexer_repository() {
        let client = Arc::new(MockDownloadClient::default());
        let router = nzb_bytes_router(client.clone());
        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Missing Wiring Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("missing-indexer".to_string());

        let error = router
            .submit_download(&request)
            .await
            .expect_err("indexed submission must not fall back to automatic routing");
        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("missing-indexer") && message.contains("not wired")
        ));
        assert!(client.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn indexed_submission_fails_closed_when_indexer_row_is_missing() {
        let client = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), client.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: Vec::new(),
            }),
            Arc::new(EmptyProxyConfigRepository),
        );
        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Missing Row Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("missing-row".to_string());

        let error = router
            .submit_download(&request)
            .await
            .expect_err("missing indexer rows must fail closed");
        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("missing-row") && message.contains("not found")
        ));
        assert!(client.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mapped_enqueue_failure_does_not_fallback_and_names_route() {
        let mapped = Arc::new(MockDownloadClient {
            submit_error: Mutex::new(Some(MockSubmitError::Repository)),
            ..Default::default()
        });
        let fallback = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("mapped".to_string(), mapped.clone()),
                    ("fallback".to_string(), fallback.clone()),
                ],
            });
        let mut indexer = test_indexer_config("https://indexer.example/api");
        indexer.download_client_id = Some("mapped".to_string());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("mapped", "Mapped", "qbittorrent", 0),
                    test_config("fallback", "Fallback", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![indexer],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );

        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Mapped Failure Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());

        let error = router
            .submit_download(&request)
            .await
            .expect_err("mapped enqueue failures must not fall back");
        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("indexer-1")
                    && message.contains("mapped")
                    && message.contains("download submission failed")
        ));
        assert_eq!(mapped.submissions.lock().unwrap().len(), 1);
        assert!(fallback.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mapped_ambiguous_submit_preserves_ambiguity_without_fallback() {
        let mapped = Arc::new(MockDownloadClient {
            submit_error: Mutex::new(Some(MockSubmitError::Ambiguous)),
            ..Default::default()
        });
        let fallback = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("mapped".to_string(), mapped.clone()),
                    ("fallback".to_string(), fallback.clone()),
                ],
            });
        let mut indexer = test_indexer_config("https://indexer.example/api");
        indexer.download_client_id = Some("mapped".to_string());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("mapped", "Mapped", "qbittorrent", 0),
                    test_config("fallback", "Fallback", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![indexer],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );
        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Ambiguous Mapped Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());

        assert!(
            router
                .submit_download(&request)
                .await
                .is_err_and(|error| error.is_download_submit_ambiguous())
        );
        assert_eq!(mapped.submissions.lock().unwrap().len(), 1);
        assert!(fallback.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_mapped_client_cleans_staged_nzb_without_fallback() {
        let fallback = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("fallback".to_string(), fallback.clone())],
            });
        let mut indexer = test_indexer_config("https://indexer.example/api");
        indexer.download_client_id = Some("deleted-client".to_string());
        let staged_store = Arc::new(RecordingStagedNzbStore::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("fallback", "Fallback", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            staged_store.clone(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![indexer],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );

        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Missing Mapped Client Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());
        request.staged_nzb = Some(StagedNzbRef {
            id: "staged-missing-client".to_string(),
            compressed_path: std::path::PathBuf::from("/tmp/staged-missing-client.nzb.zst"),
            raw_size_bytes: 128,
        });

        let error = router
            .submit_download(&request)
            .await
            .expect_err("missing mapped clients must fail closed");
        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("Indexer")
                    && message.contains("deleted-client")
                    && message.contains("does not exist")
        ));
        assert_eq!(
            staged_store.deleted_ids.lock().unwrap().as_slice(),
            &["staged-missing-client".to_string()]
        );
        assert!(fallback.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn incompatible_mapped_client_cleans_staged_nzb_without_fallback() {
        let mapped = Arc::new(MockDownloadClient::default());
        let fallback = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_url".to_string()],
                clients: vec![
                    ("mapped".to_string(), mapped.clone()),
                    ("fallback".to_string(), fallback.clone()),
                ],
            });
        let mut indexer = test_indexer_config("https://indexer.example/api");
        indexer.download_client_id = Some("mapped".to_string());
        let staged_store = Arc::new(RecordingStagedNzbStore::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("mapped", "Mapped", "qbittorrent", 0),
                    test_config("fallback", "Fallback", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            staged_store.clone(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![indexer],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );

        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Incompatible Mapped Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());
        request.staged_nzb = Some(StagedNzbRef {
            id: "staged-incompatible".to_string(),
            compressed_path: std::path::PathBuf::from("/tmp/staged-incompatible.nzb.zst"),
            raw_size_bytes: 128,
        });

        let error = router
            .submit_download(&request)
            .await
            .expect_err("artifact-incompatible mapped clients must fail closed");
        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("Indexer")
                    && message.contains("Mapped")
                    && message.contains("cannot handle NZB URL releases")
        ));
        assert_eq!(
            staged_store.deleted_ids.lock().unwrap().as_slice(),
            &["staged-incompatible".to_string()]
        );
        assert!(mapped.submissions.lock().unwrap().is_empty());
        assert!(fallback.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mapped_client_disabled_in_effective_scope_fails_closed() {
        let mapped = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("mapped".to_string(), mapped.clone())],
            });
        let mut indexer = test_indexer_config("https://indexer.example/api");
        indexer.download_client_id = Some("mapped".to_string());
        let staged_store = Arc::new(RecordingStagedNzbStore::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("mapped", "Mapped", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{"mapped": {"enabled": false}}"#.to_string(),
                )]),
            }),
            staged_store.clone(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![indexer],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );

        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Scoped Disabled Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());
        request.staged_nzb = Some(StagedNzbRef {
            id: "staged-scope-disabled".to_string(),
            compressed_path: std::path::PathBuf::from("/tmp/staged-scope-disabled.nzb.zst"),
            raw_size_bytes: 128,
        });

        let error = router
            .submit_download(&request)
            .await
            .expect_err("scope-disabled mapped clients must fail closed");
        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("indexer-1")
                    && message.contains("mapped")
                    && message.contains("disabled in the effective scope")
        ));
        assert_eq!(
            staged_store.deleted_ids.lock().unwrap().as_slice(),
            &["staged-scope-disabled".to_string()]
        );
        assert!(mapped.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn globally_disabled_mapped_client_cleans_staged_nzb_without_fallback() {
        let mapped = Arc::new(MockDownloadClient::default());
        let fallback = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("mapped".to_string(), mapped.clone()),
                    ("fallback".to_string(), fallback.clone()),
                ],
            });
        let mut indexer = test_indexer_config("https://indexer.example/api");
        indexer.download_client_id = Some("mapped".to_string());
        let staged_store = Arc::new(RecordingStagedNzbStore::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    disabled_test_config("mapped", "Mapped", "qbittorrent", 0),
                    test_config("fallback", "Fallback", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            staged_store.clone(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![indexer],
            }),
            Arc::new(EmptyProxyConfigRepository),
        );

        let mut request = DownloadClientAddRequest::from_legacy(
            &test_title(),
            Some("https://indexer.example/release.nzb".to_string()),
            Some(DownloadSourceKind::NzbUrl),
            Some("Disabled Mapped Release".to_string()),
            None,
            None,
        );
        request.indexer_id = Some("indexer-1".to_string());
        request.staged_nzb = Some(StagedNzbRef {
            id: "staged-1".to_string(),
            compressed_path: std::path::PathBuf::from("/tmp/staged-1.nzb.zst"),
            raw_size_bytes: 128,
        });

        let error = router
            .submit_download(&request)
            .await
            .expect_err("globally disabled mapped clients must fail closed");
        assert!(matches!(
            error,
            AppError::DownloadSubmitUnavailable(message)
                if message.contains("Indexer")
                    && message.contains("Mapped")
                    && message.contains("globally disabled")
        ));
        assert_eq!(
            staged_store.deleted_ids.lock().unwrap().as_slice(),
            &["staged-1".to_string()]
        );
        assert!(mapped.submissions.lock().unwrap().is_empty());
        assert!(fallback.submissions.lock().unwrap().is_empty());
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
            seeding: None,
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

    struct ResolvingIndexerClient {
        calls: Mutex<Vec<String>>,
        artifact: Option<ResolvedDownloadArtifact>,
    }

    #[async_trait]
    impl scryer_application::IndexerClient for ResolvingIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<scryer_application::IndexerRoutingPlan>,
            _mode: scryer_application::SearchMode,
            _operation: scryer_application::IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<scryer_application::IndexerSearchLearningContext>,
            _cancel_token: tokio_util::sync::CancellationToken,
        ) -> AppResult<scryer_application::IndexerSearchResponse> {
            Err(AppError::Repository(
                "search is not used in this test".to_string(),
            ))
        }

        async fn resolve_download(
            &self,
            download_url: &str,
        ) -> AppResult<Option<ResolvedDownloadArtifact>> {
            self.calls.lock().unwrap().push(download_url.to_string());
            Ok(self.artifact.clone())
        }
    }

    struct ResolvingIndexerProvider {
        client: Arc<dyn scryer_application::IndexerClient>,
    }

    impl IndexerPluginProvider for ResolvingIndexerProvider {
        fn client_for_provider(
            &self,
            _config: &scryer_domain::IndexerConfig,
        ) -> Option<Arc<dyn scryer_application::IndexerClient>> {
            Some(Arc::clone(&self.client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["cardigann".to_string()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }
    }

    fn resolving_router(
        artifact: Option<ResolvedDownloadArtifact>,
    ) -> (PrioritizedDownloadClientRouter, Arc<ResolvingIndexerClient>) {
        let client = Arc::new(ResolvingIndexerClient {
            calls: Mutex::new(Vec::new()),
            artifact,
        });
        let router =
            no_client_router().with_indexer_plugin_provider(Arc::new(ResolvingIndexerProvider {
                client: client.clone(),
            }));
        (router, client)
    }

    #[tokio::test]
    async fn indexer_owned_download_resolution_runs_before_direct_fallback() {
        let download_url = "https://indexer.example/download/1";
        let (router, indexer_client) =
            resolving_router(Some(ResolvedDownloadArtifact::TorrentFile {
                bytes: b"torrent bytes".to_vec(),
                file_name: Some("release.torrent".to_string()),
                content_type: Some("application/x-bittorrent".to_string()),
                info_hash_hint: Some("0123456789012345678901234567890123456789".to_string()),
            }));

        let prepared = router
            .prepare_download_request(
                &test_add_request(download_url, Some(DownloadSourceKind::TorrentFile)),
                Some(&test_indexer_config("https://indexer.example")),
            )
            .await
            .expect("indexer-owned resolution should avoid the direct fallback");

        assert_eq!(
            indexer_client.calls.lock().unwrap().as_slice(),
            &[download_url.to_string()]
        );
        assert!(matches!(
            prepared.resolved_download_artifact,
            Some(ResolvedDownloadArtifact::TorrentFile { ref bytes, .. }) if bytes == b"torrent bytes"
        ));
        assert_eq!(prepared.source_kind, Some(DownloadSourceKind::TorrentFile));
        assert!(prepared.source_hint.is_none());
    }

    #[tokio::test]
    async fn a_magnet_source_hint_never_reaches_the_indexer_grab_flow() {
        let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
        let (router, indexer_client) = resolving_router(None);

        let prepared = router
            .prepare_download_request(
                &test_add_request(magnet, Some(DownloadSourceKind::MagnetUri)),
                Some(&test_indexer_config("https://indexer.example")),
            )
            .await
            .expect("a magnet needs no resolution at all");

        assert!(indexer_client.calls.lock().unwrap().is_empty());
        assert!(matches!(
            prepared.resolved_download_artifact,
            Some(ResolvedDownloadArtifact::Magnet { .. })
        ));
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
            router.delete_queue_item("job-1", false, false).await,
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
                source_hint: None,
                staged_nzb: None,
                resolved_download_artifact: Some(ResolvedDownloadArtifact::TorrentFile {
                    bytes: b"d4:infod4:name4:testee".to_vec(),
                    file_name: Some("file.torrent".to_string()),
                    content_type: Some("application/x-bittorrent".to_string()),
                    info_hash_hint: None,
                }),
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
 <meta type="name">Tide.Chart.S02.DANiSH.JAPANESE.1080p.WEB.H264</meta>
 <meta type="category">TV &gt; Anime</meta>
</head>
<file poster="poster@example.invalid" date="1700000000" subject="[1/1] - &quot;tide.chart.par2&quot;"></file>
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
            source_title: Some("Tide.Chart.S02.DANiSH.JAPANESE.1080p.WEB.H264".to_string()),
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
            tracker_min_seed_ratio: None,
            tracker_min_seed_time_minutes: None,
            season_pack_seed_ratio: None,
            season_pack_seed_time_minutes: None,
            is_recent: None,
            season_pack: None,
            pinned_download_client_id: None,
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
                source_hint: None,
                staged_nzb: None,
                resolved_download_artifact: Some(ResolvedDownloadArtifact::TorrentFile {
                    bytes: b"d4:infod4:name4:testee".to_vec(),
                    file_name: Some("file.torrent".to_string()),
                    content_type: Some("application/x-bittorrent".to_string()),
                    info_hash_hint: None,
                }),
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
            })
            .await
            .expect_err("ambiguous submit errors should stop router failover");

        assert!(error.is_download_submit_ambiguous());
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
                source_hint: None,
                staged_nzb: None,
                resolved_download_artifact: Some(ResolvedDownloadArtifact::TorrentFile {
                    bytes: b"d4:infod4:name4:testee".to_vec(),
                    file_name: Some("file.torrent".to_string()),
                    content_type: Some("application/x-bittorrent".to_string()),
                    info_hash_hint: None,
                }),
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
            })
            .await
            .expect_err("rejected submit errors should stop router failover");

        assert!(matches!(error, AppError::DownloadSubmitRejected(_)));
        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn submit_download_all_failover_clients_failed_returns_failover_exhausted() {
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
        let download_id = scryer_domain::download_identity::DownloadId::new();

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                search_facet: None,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: Some(download_id),
                source_hint: None,
                staged_nzb: None,
                resolved_download_artifact: Some(ResolvedDownloadArtifact::TorrentFile {
                    bytes: b"d4:infod4:name4:testee".to_vec(),
                    file_name: Some("file.torrent".to_string()),
                    content_type: Some("application/x-bittorrent".to_string()),
                    info_hash_hint: None,
                }),
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
            })
            .await
            .expect_err("exhausted failover clients should fail");

        // The typed variant is the decision; the final client's error is only
        // human-readable context inside it.
        let AppError::DownloadSubmitFailoverExhausted(message) = &error else {
            panic!("expected DownloadSubmitFailoverExhausted, got {error:?}");
        };
        assert!(
            message.contains("all prioritized download clients failed")
                && message.contains("client submit unavailable"),
            "{message}"
        );
        assert!(error.is_retryable_download_submit_failure());
        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
        assert_eq!(
            primary.submissions.lock().unwrap()[0].download_id,
            Some(download_id)
        );
        assert_eq!(
            secondary.submissions.lock().unwrap()[0].download_id,
            Some(download_id)
        );
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
            })
            .await
            .expect_err("magnet request should fail when only nzb clients are enabled");

        match error {
            AppError::DownloadSubmitUnavailable(message) => {
                assert!(message.contains("magnet"));
            }
            other => panic!("expected unavailable error, got {other:?}"),
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: Some(true),
                season_pack: None,
                pinned_download_client_id: None,
            })
            .await
            .expect("request should be routed");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.category.as_deref(), Some("Movies"));
        assert_eq!(request.queue_priority.as_deref(), Some("high"));
    }

    struct MockSeedingProfileRepository {
        profiles: Vec<scryer_domain::SeedingProfile>,
    }

    #[async_trait]
    impl SeedingProfileRepository for MockSeedingProfileRepository {
        async fn list(&self) -> AppResult<Vec<scryer_domain::SeedingProfile>> {
            Ok(self.profiles.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::SeedingProfile>> {
            Ok(self
                .profiles
                .iter()
                .find(|profile| profile.id == id)
                .cloned())
        }

        async fn create(
            &self,
            profile: scryer_domain::SeedingProfile,
        ) -> AppResult<scryer_domain::SeedingProfile> {
            Ok(profile)
        }

        async fn update(
            &self,
            profile: scryer_domain::SeedingProfile,
        ) -> AppResult<scryer_domain::SeedingProfile> {
            Ok(profile)
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    fn test_seeding_profile(id: &str) -> scryer_domain::SeedingProfile {
        let now = Utc::now();
        scryer_domain::SeedingProfile {
            id: id.to_string(),
            name: id.to_string(),
            ratio: Some(1.5),
            seed_time_minutes: Some(60),
            season_pack_mode: scryer_domain::SeasonPackSeedMode::Inherit,
            season_pack_ratio: None,
            season_pack_seed_time_minutes: None,
            honor_tracker_minimums: true,
            goal_met_action: scryer_domain::SeedGoalMetAction::RemoveEntry,
            never_remove: false,
            minimum_seeders: None,
            post_import_tracking: scryer_domain::PostImportTracking::Park,
            created_at: now,
            updated_at: now,
        }
    }

    fn seeding_router(
        client: Arc<MockDownloadClient>,
        client_type: &str,
        accepted_inputs: Vec<String>,
        routing_json: &str,
    ) -> PrioritizedDownloadClientRouter {
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs,
                clients: vec![("primary".to_string(), client)],
            });
        PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", client_type, 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([("movie".to_string(), routing_json.to_string())]),
            }),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        )
        .with_indexer_config_repositories(
            Arc::new(RoutingIndexerConfigRepository {
                configs: vec![scryer_domain::IndexerConfig {
                    seeding_profile_id: Some("indexer-profile".to_string()),
                    ..test_indexer_config("https://indexer.invalid")
                }],
            }),
            Arc::new(EmptyProxyConfigRepository),
        )
        .with_seed_goal_resolution(Arc::new(MockSeedingProfileRepository {
            profiles: vec![
                test_seeding_profile("indexer-profile"),
                scryer_domain::SeedingProfile {
                    ratio: Some(0.25),
                    seed_time_minutes: None,
                    ..test_seeding_profile("routing-profile")
                },
            ],
        }))
    }

    fn torrent_add_request(indexer_id: Option<&str>) -> DownloadClientAddRequest {
        DownloadClientAddRequest {
            title: test_title(),
            search_facet: None,
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            download_id: Some(scryer_domain::download_identity::DownloadId::new()),
            source_hint: None,
            staged_nzb: None,
            resolved_download_artifact: Some(ResolvedDownloadArtifact::TorrentFile {
                bytes: b"d4:infod4:name4:testee".to_vec(),
                file_name: Some("file.torrent".to_string()),
                content_type: Some("application/x-bittorrent".to_string()),
                info_hash_hint: None,
            }),
            source_kind: Some(DownloadSourceKind::TorrentFile),
            source_title: Some("Test Release".to_string()),
            source_password: None,
            category: None,
            queue_priority: None,
            download_directory: None,
            release_title: None,
            indexer_name: None,
            indexer_id: indexer_id.map(str::to_string),
            info_hash_hint: Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string()),
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            tracker_min_seed_ratio: None,
            tracker_min_seed_time_minutes: None,
            season_pack_seed_ratio: None,
            season_pack_seed_time_minutes: None,
            is_recent: None,
            season_pack: None,
            pinned_download_client_id: None,
        }
    }

    #[tokio::test]
    async fn submit_download_resolves_and_persists_seeding_goals_for_torrents() {
        let primary = Arc::new(MockDownloadClient::default());
        let router = seeding_router(
            primary.clone(),
            "qbittorrent",
            vec!["torrent_file".to_string(), "magnet_uri".to_string()],
            r#"{"primary": {"enabled": true, "seedingProfileId": "routing-profile"}}"#,
        );

        let mut request = torrent_add_request(Some("indexer-1"));
        // Tracker demands more than the indexer profile's 1.5.
        request.tracker_min_seed_ratio = Some(3.0);
        let expected_download_id = request.download_id.expect("test request has an ID");
        let grab = router
            .submit_download(&request)
            .await
            .expect("torrent request should route");
        assert_eq!(grab.download_id, Some(expected_download_id));

        let sent = primary.submissions.lock().unwrap();
        let sent = sent.first().expect("submission should be recorded");
        // Indexer assignment beats the routing entry, then the tracker minimum
        // clamps the ratio up.
        assert_eq!(sent.seed_goal_ratio, Some(3.0));
        assert_eq!(sent.seed_goal_seconds, Some(3600));

        let recorded = grab.seed_goals.as_ref().expect("goals should be returned");
        assert_eq!(
            recorded.seeding_profile_id.as_deref(),
            Some("indexer-profile")
        );
        assert_eq!(recorded.seed_goal_ratio, Some(3.0));
        assert_eq!(recorded.seed_goal_seconds, Some(3600));
        assert_eq!(
            recorded.resolution_source,
            scryer_application::SeedGoalResolutionSource::Indexer
        );
        assert_eq!(
            recorded.info_hash.as_deref(),
            Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01")
        );
    }

    #[tokio::test]
    async fn submit_download_falls_back_to_the_routing_entry_seeding_profile() {
        let primary = Arc::new(MockDownloadClient::default());
        let router = seeding_router(
            primary.clone(),
            "qbittorrent",
            vec!["torrent_file".to_string(), "magnet_uri".to_string()],
            r#"{"primary": {"enabled": true, "seedingProfileId": "routing-profile"}}"#,
        );

        let canonical_hash = "0123456789abcdef0123456789abcdef01234567";
        let mut request = torrent_add_request(None);
        request.info_hash_hint = Some(canonical_hash.to_string());
        if let Some(ResolvedDownloadArtifact::TorrentFile { info_hash_hint, .. }) =
            request.resolved_download_artifact.as_mut()
        {
            *info_hash_hint = Some(canonical_hash.to_string());
        }

        let grab = router
            .submit_download(&request)
            .await
            .expect("torrent request should route");
        assert_eq!(grab.info_hash.as_deref(), Some(canonical_hash));

        let recorded = grab.seed_goals.as_ref().expect("goals should be returned");
        assert_eq!(
            recorded.seeding_profile_id.as_deref(),
            Some("routing-profile")
        );
        assert_eq!(
            recorded.resolution_source,
            scryer_application::SeedGoalResolutionSource::RoutingEntry
        );
        assert_eq!(recorded.seed_goal_ratio, Some(0.25));
        assert_eq!(recorded.seed_goal_seconds, None);
    }

    #[tokio::test]
    async fn submit_download_leaves_usenet_requests_without_seeding_goals() {
        let primary = Arc::new(MockDownloadClient::default());
        // A torrent-capable client type deliberately fed an NZB payload: the
        // resolver keys off the request's source kind, not the client.
        let router = seeding_router(
            primary.clone(),
            "qbittorrent",
            vec!["nzb_url".to_string()],
            r#"{"primary": {"enabled": true, "seedingProfileId": "routing-profile"}}"#,
        );

        let mut request = torrent_add_request(Some("indexer-1"));
        request.resolved_download_artifact = None;
        request.source_kind = Some(DownloadSourceKind::NzbUrl);
        request.source_hint = Some("https://example.invalid/release.nzb".to_string());
        request.info_hash_hint = None;

        let grab = router
            .submit_download(&request)
            .await
            .expect("usenet request should route");

        let sent = primary.submissions.lock().unwrap();
        let sent = sent.first().expect("submission should be recorded");
        assert_eq!(sent.seed_goal_ratio, None);
        assert_eq!(sent.seed_goal_seconds, None);
        assert!(grab.seed_goals.is_none());
    }

    #[tokio::test]
    async fn submit_download_without_any_seeding_profile_wiring_keeps_todays_behavior() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string(), "magnet_uri".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&torrent_add_request(None))
            .await
            .expect("torrent request should route");

        let sent = primary.submissions.lock().unwrap();
        let sent = sent.first().expect("submission should be recorded");
        assert_eq!(sent.seed_goal_ratio, None);
        assert_eq!(sent.seed_goal_seconds, None);
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: Some(false),
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
            })
            .await
            .expect_err("facet-disabled clients should fail fast");

        match error {
            AppError::DownloadSubmitUnavailable(message) => {
                assert!(message.contains("no download client enabled"));
            }
            other => panic!("expected unavailable error, got {other:?}"),
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: Some(true),
                season_pack: None,
                pinned_download_client_id: None,
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
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: None,
            })
            .await
            .expect_err("library override should fail fast when every client is disabled");

        match error {
            AppError::DownloadSubmitUnavailable(message) => {
                assert!(message.contains("no download client enabled for library"));
            }
            other => panic!("expected unavailable error, got {other:?}"),
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
            .delete_queue_item("SABnzbd_nzo_hist01", true, false)
            .await
            .expect("history delete should route to sabnzbd client");

        assert!(nzb_client.deleted.lock().unwrap().is_empty());
        assert_eq!(
            sab_client.deleted.lock().unwrap().as_slice(),
            [("SABnzbd_nzo_hist01".to_string(), true, false)]
        );
    }

    /// The data-removal flag is the caller's decision (the terminal-cleanup
    /// executor's), so the router has to carry it to the client it resolved
    /// rather than deciding anything itself.
    #[tokio::test]
    async fn delete_forwards_the_data_removal_flag_to_the_resolved_client() {
        let torrent_client = Arc::new(MockDownloadClient::default());
        torrent_client
            .history_items
            .lock()
            .unwrap()
            .push(test_queue_item("torrent-1"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["magnet_uri".to_string()],
                clients: vec![("qbit".to_string(), torrent_client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("qbit", "qBittorrent", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .delete_queue_item("torrent-1", true, true)
            .await
            .expect("history delete should route to the torrent client");
        router
            .delete_queue_item_for_client_id("qbit", "torrent-1", true, true)
            .await
            .expect("client-id delete should route to the torrent client");
        router
            .delete_queue_item_for_client("qbittorrent", "torrent-1", true, true)
            .await
            .expect("client-type delete should route to the torrent client");

        assert_eq!(
            torrent_client.deleted.lock().unwrap().as_slice(),
            [
                ("torrent-1".to_string(), true, true),
                ("torrent-1".to_string(), true, true),
                ("torrent-1".to_string(), true, true)
            ]
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
    async fn list_recent_activity_applies_limit_per_client() {
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
        assert_eq!(
            ids,
            vec![
                "a-1".to_string(),
                "b-1".to_string(),
                "a-2".to_string(),
                "b-2".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn recent_feedback_limit_allows_300_rows_from_each_client() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());
        let now = Utc::now();

        for index in 0..301 {
            let mut a = test_queue_item(&format!("activity-a-{index:03}"));
            a.last_updated_at = Some((1_800_000_000_i64 - index).to_string());
            client_a.history_items.lock().unwrap().push(a);
            let mut b = test_queue_item(&format!("activity-b-{index:03}"));
            b.last_updated_at = Some((1_700_000_000_i64 - index).to_string());
            client_b.history_items.lock().unwrap().push(b);

            for (client, prefix) in [(&client_a, "a"), (&client_b, "b")] {
                client
                    .completed_downloads
                    .lock()
                    .unwrap()
                    .push(scryer_domain::CompletedDownload {
                        client_type: "qbittorrent".to_string(),
                        client_id: String::new(),
                        download_client_item_id: format!("completed-{prefix}-{index:03}"),
                        download_id: None,
                        name: format!("Completed {prefix} {index}"),
                        release_name: None,
                        dest_dir: format!("/downloads/{prefix}/{index}"),
                        category: None,
                        size_bytes: None,
                        completed_at: Some(now - chrono::Duration::seconds(index)),
                        parameters: Vec::new(),
                    });
            }
        }

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a),
                    ("client-b".to_string(), client_b),
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

        assert_eq!(router.list_recent_activity(300).await.unwrap().len(), 600);
        assert_eq!(
            router
                .list_recent_completed_downloads(300)
                .await
                .unwrap()
                .len(),
            600
        );
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
    async fn list_recent_completed_downloads_applies_limit_per_client() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        client_a.completed_downloads.lock().unwrap().extend([
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "a-1".to_string(),
                download_id: None,
                name: "A 1".to_string(),
                release_name: None,
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
                release_name: None,
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
                release_name: None,
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
                release_name: None,
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
        assert_eq!(
            ids,
            vec![
                "a-1".to_string(),
                "b-1".to_string(),
                "a-2".to_string(),
                "b-2".to_string(),
            ]
        );
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
                release_name: None,
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
                release_name: None,
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
                release_name: None,
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
                release_name: None,
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
                release_name: None,
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
                release_name: None,
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

    #[test]
    fn download_client_feedback_timeout_configuration_uses_positive_seconds() {
        let default = scryer_outbound_http::DEFAULT_DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT;

        assert_eq!(
            parse_download_client_feedback_timeout(None, default),
            default
        );
        assert_eq!(
            parse_download_client_feedback_timeout(Some(" 600 "), default),
            Duration::from_secs(600)
        );
        assert_eq!(
            parse_download_client_feedback_timeout(Some("0"), default),
            default
        );
        assert_eq!(
            parse_download_client_feedback_timeout(Some("invalid"), default),
            default
        );
    }

    #[test]
    fn feedback_backoff_retains_exponential_growth_for_fast_failures() {
        let timeout = Duration::from_secs(90);
        let elapsed = Duration::from_secs(1);

        assert_eq!(
            PrioritizedDownloadClientRouter::feedback_backoff_duration(1, timeout, elapsed),
            Duration::from_secs(15)
        );
        assert_eq!(
            PrioritizedDownloadClientRouter::feedback_backoff_duration(2, timeout, elapsed),
            Duration::from_secs(30)
        );
        assert_eq!(
            PrioritizedDownloadClientRouter::feedback_backoff_duration(3, timeout, elapsed),
            Duration::from_secs(60)
        );
        assert_eq!(
            PrioritizedDownloadClientRouter::feedback_backoff_duration(4, timeout, elapsed),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn feedback_backoff_accounts_for_expensive_failures() {
        assert_eq!(
            PrioritizedDownloadClientRouter::feedback_backoff_duration(
                1,
                Duration::from_secs(300),
                Duration::from_secs(240),
            ),
            Duration::from_secs(240)
        );
    }

    #[test]
    fn feedback_backoff_uses_dynamic_ceiling() {
        assert_eq!(
            PrioritizedDownloadClientRouter::feedback_backoff_duration(
                10,
                Duration::from_secs(300),
                Duration::from_secs(1),
            ),
            Duration::from_secs(300)
        );
        assert_eq!(
            PrioritizedDownloadClientRouter::feedback_backoff_duration(
                10,
                Duration::from_secs(90),
                Duration::from_secs(500),
            ),
            Duration::from_secs(120)
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
                if message == "download feedback timed out after 5ms; queue status is temporarily unavailable"
        ));
    }

    #[tokio::test]
    async fn download_client_timeout_wrapper_times_out_all_completion_reads() {
        let wrapped = FeedbackTimeoutDownloadClient::new(
            Arc::new(DelayedCompletedDownloadClient {
                delay: Duration::from_millis(25),
            }),
            Duration::from_millis(5),
        );
        let client_ids = vec!["client-a".to_string()];
        let client_types = vec!["qbittorrent".to_string()];

        let errors = [
            wrapped
                .list_completed_downloads()
                .await
                .expect_err("completed download reads should time out"),
            wrapped
                .list_recent_completed_downloads(10)
                .await
                .expect_err("recent completed download reads should time out"),
            wrapped
                .list_recent_completed_downloads_excluding_client_types(10, &["sabnzbd"])
                .await
                .expect_err("excluded-type completed download reads should time out"),
            wrapped
                .list_recent_completed_downloads_for_client_scope(
                    10,
                    &client_ids,
                    &client_types,
                    &[],
                )
                .await
                .expect_err("client-scoped completed download reads should time out"),
            wrapped
                .get_completed_download_for_source("client-a", "qbittorrent", "item-a")
                .await
                .expect_err("targeted completed download reads should time out"),
        ];

        for error in errors {
            assert!(matches!(
                error,
                AppError::DownloadFeedbackTimeout(ref message)
                    if message == "download feedback timed out after 5ms; queue status is temporarily unavailable"
            ));
        }
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
    async fn list_queue_delivers_only_each_clients_case_preserving_category_scope() {
        let first = Arc::new(FeedbackScopeQueueDownloadClient {
            scopes: Mutex::new(Vec::new()),
            queue_items: vec![test_queue_item("first")],
        });
        let second = Arc::new(FeedbackScopeQueueDownloadClient {
            scopes: Mutex::new(Vec::new()),
            queue_items: vec![test_queue_item("second")],
        });
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_url".to_string()],
                clients: vec![
                    ("first".to_string(), first.clone()),
                    ("second".to_string(), second.clone()),
                ],
            });
        let store = DownloadClientCategorySnapshotStore::default();
        store
            .replace(
                scryer_application::DownloadClientCategoryAdmissionSnapshot::from_feedback_categories(
                    HashMap::from([
                        (
                            "first".to_string(),
                            vec!["Movies".to_string(), "TV / Anime".to_string()],
                        ),
                        ("second".to_string(), vec!["Series-HD".to_string()]),
                    ]),
                ),
            )
            .await;

        let router = PrioritizedDownloadClientRouter::with_feedback_read_timeout(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("first", "First", "qbittorrent", 0),
                    test_config("second", "Second", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
            Duration::from_secs(1),
        )
        .with_download_client_category_snapshot_store(store);

        let items = router.list_queue().await.expect("scoped queue listing");
        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["first", "second"]);
        assert_eq!(
            *first.scopes.lock().unwrap(),
            vec![vec!["Movies".to_string(), "TV / Anime".to_string()]]
        );
        assert_eq!(
            *second.scopes.lock().unwrap(),
            vec![vec!["Series-HD".to_string()]]
        );
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
    async fn snapshot_outcome_keeps_healthy_items_when_another_client_queue_fails() {
        let healthy_client = Arc::new(MockDownloadClient::default());
        healthy_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("healthy"));
        let failing_client = Arc::new(FailingQueueDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("healthy".to_string(), healthy_client),
                    ("failing".to_string(), failing_client),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("healthy", "Healthy", "qbittorrent", 0),
                    test_config("failing", "Failing", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let outcome = router
            .list_snapshot_outcome_excluding_client_types(300, &[])
            .await
            .expect("a partial snapshot should remain available");

        assert!(outcome.any_client_read_succeeded);
        assert!(
            outcome
                .items
                .iter()
                .any(|item| item.download_client_item_id == "healthy")
        );
        assert_eq!(
            outcome.authoritative_client_ids,
            HashSet::from(["healthy".to_string()])
        );
    }

    #[tokio::test]
    async fn snapshot_outcome_requires_recent_activity_before_authorizing_absence() {
        let healthy_client = Arc::new(MockDownloadClient::default());
        healthy_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("healthy"));
        let activity_failing_client: Arc<dyn DownloadClient> =
            Arc::new(ActivityFailingDownloadClient {
                queue_items: vec![test_queue_item("activity-failing")],
            });
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("healthy".to_string(), healthy_client),
                    ("activity-failing".to_string(), activity_failing_client),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("healthy", "Healthy", "qbittorrent", 0),
                    test_config("activity-failing", "Activity Failing", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let outcome = router
            .list_snapshot_outcome_excluding_client_types(300, &[])
            .await
            .expect("queue data should remain available after an activity failure");

        assert!(outcome.any_client_read_succeeded);
        assert!(
            outcome
                .items
                .iter()
                .any(|item| item.download_client_item_id == "activity-failing")
        );
        assert_eq!(
            outcome.authoritative_client_ids,
            HashSet::from(["healthy".to_string()])
        );
    }

    #[tokio::test]
    async fn list_queue_polls_clients_concurrently_and_preserves_priority_order() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let first: Arc<dyn DownloadClient> = Arc::new(CoordinatedQueueDownloadClient {
            barrier: barrier.clone(),
            delay_after_barrier: Duration::from_millis(40),
            queue_items: vec![test_queue_item("first")],
        });
        let second: Arc<dyn DownloadClient> = Arc::new(CoordinatedQueueDownloadClient {
            barrier,
            delay_after_barrier: Duration::ZERO,
            queue_items: vec![test_queue_item("second")],
        });
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("first".to_string(), first), ("second".to_string(), second)],
            });
        let router = PrioritizedDownloadClientRouter::with_feedback_read_timeout(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("first", "First", "qbittorrent", 0),
                    test_config("second", "Second", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
            Duration::from_secs(5),
        );

        let items = tokio::time::timeout(Duration::from_secs(1), router.list_queue())
            .await
            .expect("both feedback reads should reach the barrier concurrently")
            .expect("parallel queue listing should succeed");
        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["first".to_string(), "second".to_string()]);
    }

    #[tokio::test]
    async fn list_queue_refills_polling_slots_and_preserves_priority_order() {
        let first_release = Arc::new(tokio::sync::Notify::new());
        let fifth_started = Arc::new(tokio::sync::Notify::new());
        let first: Arc<dyn DownloadClient> = Arc::new(GatedQueueDownloadClient {
            started: None,
            release: Some(first_release.clone()),
            queue_items: vec![test_queue_item("first")],
        });
        let immediate = |id| -> Arc<dyn DownloadClient> {
            Arc::new(GatedQueueDownloadClient {
                started: None,
                release: None,
                queue_items: vec![test_queue_item(id)],
            })
        };
        let fifth: Arc<dyn DownloadClient> = Arc::new(GatedQueueDownloadClient {
            started: Some(fifth_started.clone()),
            release: None,
            queue_items: vec![test_queue_item("fifth")],
        });
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("first".to_string(), first),
                    ("second".to_string(), immediate("second")),
                    ("third".to_string(), immediate("third")),
                    ("fourth".to_string(), immediate("fourth")),
                    ("fifth".to_string(), fifth),
                ],
            });
        let router = Arc::new(PrioritizedDownloadClientRouter::with_feedback_read_timeout(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("first", "First", "qbittorrent", 0),
                    test_config("second", "Second", "qbittorrent", 1),
                    test_config("third", "Third", "qbittorrent", 2),
                    test_config("fourth", "Fourth", "qbittorrent", 3),
                    test_config("fifth", "Fifth", "qbittorrent", 4),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
            Duration::from_secs(5),
        ));

        let poll = tokio::spawn({
            let router = router.clone();
            async move { router.list_queue().await }
        });
        tokio::time::timeout(Duration::from_secs(1), fifth_started.notified())
            .await
            .expect("client five should start while client one remains blocked");
        first_release.notify_one();

        let items = tokio::time::timeout(Duration::from_secs(1), poll)
            .await
            .expect("queue polling should finish after client one is released")
            .expect("queue polling task should not panic")
            .expect("queue listing should succeed");
        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string(),
                "fourth".to_string(),
                "fifth".to_string(),
            ]
        );
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
                if message == "download feedback timed out after 5ms; queue status is temporarily unavailable"
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

    /// The aggregate read degrades a failing client to an empty contribution
    /// so one dead client cannot blind the others, which leaves the report as
    /// the only trace of it: a client that errored and a client skipped while
    /// in feedback backoff are both named as unreadable.
    #[tokio::test]
    async fn read_report_names_failing_and_backed_off_clients() {
        let failing_client = Arc::new(FailingQueueDownloadClient::default());
        let healthy_client = Arc::new(MockDownloadClient::default());

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("failing".to_string(), failing_client.clone()),
                    ("healthy".to_string(), healthy_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("failing", "Failing", "qbittorrent", 0),
                    test_config("healthy", "Healthy", "qbittorrent", 10),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let queue = router
            .list_queue_with_read_report()
            .await
            .expect("a partial read is not an error");
        assert_eq!(queue.polled_client_count, 2);
        assert_eq!(queue.unreadable_client_ids, vec!["failing".to_string()]);
        assert!(!queue.all_unreadable());

        // The failure armed backoff: the next read skips the client without
        // asking it, and it is still reported as unreadable.
        let queue = router.list_queue_with_read_report().await.unwrap();
        assert_eq!(queue.unreadable_client_ids, vec!["failing".to_string()]);
        assert_eq!(failing_client.list_queue_call_count(), 1);

        // History: the failing client has no history read at all.
        let history = router.list_history_with_read_report().await.unwrap();
        assert_eq!(history.polled_client_count, 2);
        assert_eq!(history.unreadable_client_ids, vec!["failing".to_string()]);
    }

    /// A read nobody answered is reported as such without becoming an error:
    /// the plain listing keeps its "empty, not Err" shape for every caller
    /// that never asked for the report.
    #[tokio::test]
    async fn read_report_marks_a_read_nobody_answered_without_erroring() {
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

        let queue = router
            .list_queue_with_read_report()
            .await
            .expect("non-timeout failures degrade instead of erroring");
        assert!(queue.all_unreadable());
        assert!(queue.items.is_empty());
        assert!(router.list_queue().await.unwrap().is_empty());
        assert!(
            router
                .list_history_with_read_report()
                .await
                .unwrap()
                .all_unreadable()
        );
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

    #[tokio::test]
    async fn targeted_completed_lookup_bypasses_feedback_backoff_and_preserves_errors() {
        let failing_client = Arc::new(FailingCompletedDownloadClient::default());
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

        let completed = router
            .list_recent_completed_downloads(50)
            .await
            .expect("background completed read should degrade to empty");
        assert!(completed.is_empty());
        let error = router
            .get_completed_download_for_source("failing", "qbittorrent", "item-1")
            .await
            .expect_err("targeted lookup must preserve the provider error");

        assert!(
            error
                .to_string()
                .contains("targeted completed lookup unavailable")
        );
        assert_eq!(failing_client.recent_completed_call_count(), 1);
        assert_eq!(failing_client.targeted_completed_call_count(), 1);
    }
}
